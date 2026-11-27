# 0052. Colour LUTs: a path to a `.cube` file, read by the engine, applied after the colour adjustments

- Status: Accepted
- Date: 2026-11-27

## Context

Colour grading with LUT files is the next phase 8 feature in the specification. A LUT
("lookup table") maps every colour to another. Grading tools (DaVinci Resolve,
Premiere, most camera makers) exchange looks as `.cube` text files, Adobe's "Cube LUT
Specification 1.0": a 3D cube of N × N × N colours (N is usually 17, 33 or 65), or
one 1D curve per channel.

Six questions had to be answered:

- what the project stores;
- which crate reads and parses the file;
- where the LUT sits among the other effects;
- how both renderers apply it identically;
- what happens when the file is missing or broken;
- how Claude, the app and the command line choose a file safely.

## Options considered

1. **Store the table in the project** (35 937 colours for a 33³ cube, 274 625 for
   65³). The project would open anywhere. But every change is kept in the undo
   history and saved (file mode saves after every change): megabytes per step. And a
   look exported again from Resolve would not show up.
2. **Store the path**, as media files are. The project stays small, a look rewritten
   by the grading tool shows on the next frame, and a moved file is the same
   problem as a moved video.
3. **Parse in `kadr-core`**: core describes and never reads files.
4. **Parse in `kadr-render`**: render owns the colour arithmetic and the GPU
   upload, and needs no FFmpeg (it builds for Android).
5. **A missing or broken LUT** fails the export, or is skipped with a warning in
   the export report.
6. **Interpolation**: trilinear (8 neighbours) or tetrahedral (4).
7. **On the GPU**: the hardware's filtering of an `Rgba16Float` 3D texture, or
   `Rgba32Float` read with `textureLoad` and interpolated in the shader.

## Decision

- **Core** stores `EffectKind::ColorLut(ColorLut { file_path, strength_fraction })`
  (option 2), JSON tag `color_lut`, file format 3 with a no-op migration from 2.
  - Its rules check only the form: a non-empty path ending in `.cube` (any case), a
    finite strength from 0 to 1.
  - They never touch the disk: validation runs after every change and on load, and
    a moved LUT must not make a project impossible to open.
- **Order**, whatever the order of the effects list:
  1. tracked blurs hide their areas in the source picture;
  2. the colour adjustments apply, in list order;
  3. then the LUT.
- One LUT per layer is drawn (`MAXIMUM_COLOR_LUTS_PER_LAYER = 1`). Further ones are
  reported by the export. Claude's tool refuses a second one, and the app shows the
  first.
- **Render** parses (option 4). `kadr_render::color_lookup_table`:
  - `ColorLookupTable::from_cube_text` reads `TITLE`, `LUT_3D_SIZE` / `LUT_1D_SIZE`,
    `DOMAIN_MIN` / `DOMAIN_MAX` and Resolve's `LUT_*_INPUT_RANGE`. It also handles
    comments, Windows line endings and a byte-order mark.
  - Sizes: 2 to 65 in 3D, 2 to 65 536 in 1D. Shaper files (1D and 3D in one file)
    are refused with a clear message.
  - Error messages give line numbers and never quote the file's lines: a project
    may name a file the user did not choose.
- **Arithmetic**, on the gamma-encoded, full-range R'G'B' the colour adjustments work
  on, never linearised (what Rec.709 LUTs expect):
  1. each channel is placed in the domain and clamped;
  2. trilinear interpolation (option 6, first part), the format's reference and what
     most tools do;
  3. mixed with the input by the strength, then clamped.
- **GPU**: an `Rgba32Float` 3D texture at binding 5, read with `textureLoad` and
  interpolated by hand in the same order as the CPU (option 7, second part).
  - `Rgba32Float` is not filterable without the `FLOAT32_FILTERABLE` feature, which
    not every adapter (WARP included) offers.
  - Hardware filtering of 16-bit floats would also differ slightly from the CPU.
  - Textures are kept by the table's fingerprint while frames use them. A 1×1×1
    placeholder is bound for layers without a LUT.
- **Engine** reads (`kadr_engine::color_grading`):
  - `load_color_lookup_table` refuses files over 32 MB before reading them;
  - the frame composer keeps the tables it read, by file, and reads a file again
    when its modification time or length changes;
  - failures are kept too, and files the project no longer names are forgotten.
- **A LUT that cannot be read is skipped** (option 5, second part): the clip is drawn
  without its look, and the export report names the clip, the effect and the file.
  The preview shows the clip ungraded; an export never fails because of a LUT.
- **Interfaces read the file before adding it**, so a wrong file is reported at once:
  - `kadr-cli assemble --lut` stores the full path;
  - Claude's `add_color_lut` checks the path against the allowed folders and the
    `.cube` extension, then parses before changing the project;
  - the app's `choose_clip_lut_file` parses in the background before the edit.
  - LUT paths stored in a project are checked against the allowed folders before
    Claude's tools show or export the timeline. A missing file is accepted there,
    since the engine skips it.

## Consequences

- A project names its LUT files the way it names its media. Moving a LUT shows as
  a missing file: the app's inspector says so, the export report too.
- A look rewritten by the grading tool is picked up by the next preview frame,
  without reopening the project.
- CPU and GPU give the same colours (the `gpu_matches_cpu` tests compare graded
  layers). A 65³ table weighs 4.4 MB on the GPU.
- Not done, for later if asked:
  - shaper LUTs;
  - tetrahedral interpolation;
  - several LUTs per clip;
  - LUTs on adjustment layers or the whole timeline;
  - `.3dl` and other formats;
  - an animated strength.
- `get_frame` with a `clip_id` shows the source picture, before colour. Tool texts send
  Claude to the timeline picture to check a LUT.
- On Android (phase 9), pickers give `content://` addresses, not paths: LUT files will
  have to be copied into the app's storage, or read through a new entry point taking
  the file's text.
