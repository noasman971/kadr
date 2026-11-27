# Changelog

All notable changes to Kadr are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project uses [Semantic Versioning](https://semver.org/). Each validated phase is tagged
(`v0.1.0` for phase 0, `v0.2.0` for phase 1, and so on).

## [Unreleased]

### Added (phase 7a: blur tools for Claude)

- MCP tools `blur_region`, `get_blur_status` and `cancel_blur`: Claude gives the
  rectangle around a face on one frame, the analysis runs in the background (task
  `blur-1`), the tracked blur is added to the project as one undo step, and the status
  lists the ranges to check in timeline seconds. `blur_region` with an `effect_id`
  corrects a blur: tracking restarts from the redrawn rectangle.
- `kadr-engine`: `RegionTrackingRequest::for_correction` and `corrected_tracked_blur`
  re-track from a correction up to the neighbouring anchors (the selection and other
  corrections) and keep, frame by frame, the surer of the old and new answers;
  `RegionTrackingRequest::check_selection` checks a selection without decoding;
  `find_tracked_blur`.
- `kadr-core`: `Clip::timeline_time_showing`, the timeline moment showing a source
  moment.
- `kadr-mcp` adds finished analyses to the project every second, so the file is up to
  date even if Claude makes no further call.

### Added (phase 7b: blur in the desktop app)

- Blur tool on the preview (**B**, the preview's button, or *Add* in the inspector):
  draw a box on a face, move and resize it, ellipse or rectangle, *Blur and follow*
  (Enter). The analysis runs in the background with its progress and *Stop*; the
  result is one undo step, announced with *Undo*.
- The inspector's *Blur* section: style (blur, pixels, mask), strength, shape, margin,
  on / off, delete; how many moments the tracker was unsure about.
- The preview draws each blur's box at the playhead for the selected clip (dashed
  with ⚠ where the tracker was unsure); blurred clips get a badge on the timeline.
- `kadr_engine::clip_geometry` and `kadr_render::SourceToCanvasMapping`: where a
  clip's source picture is on the canvas, for interfaces drawing on top of it.

### Added (phase 7c: checking and correcting a blur in the app)

- The moments a blur's tracker was unsure about are striped on the clip in the
  timeline (amber uncertain, red lost, with ⚠ and a tooltip); a click goes there.
  **Shift+B** and the inspector's *Next* go to the next one.
- Correcting: drag a blur's box on the preview (or *Correct the box here*, then the
  arrow keys) where the face is, then *Correct from here* (Enter): the tracking runs
  again from that frame up to the neighbouring corrections or the selection, and keeps
  the surer answer frame by frame. One undo step, announced with what is left to
  check.

### Added (phase 7d: Claude works live in the open app)

- When the desktop app is open, `kadr-mcp` passes Claude's calls to it through a local
  pipe (`kadr-app-link`: `\\.\pipe\kadr-app-<user>` on Windows, a private Unix socket
  elsewhere), and the app runs them on the project its window shows: same undo
  history, every change shown at once with *"Claude a modifié le projet : …"*, Undo
  and the changed clips outlined; projects Claude opens or creates open in the window.
  With the app closed, file mode as before; the first answer after a switch says so.
- Claude may only undo and redo its own steps when the app is open.
- `kadr-tools`: the tools reach the project through a `ProjectHost`
  (`EditableProject`, `FileProjectHost`, `run_tool_steps`), so the app can run them on
  its own project; `KadrToolbox::call_tool_on`.
- Decision record 0030.

### Added (phase 7e: Connect to Claude)

- Settings → *Connect to Claude…*: the folders Claude may use (kept in
  `claude-access.json`, read by `kadr-mcp --kadr-settings` before every call, so a
  change applies at once), Claude Desktop's configuration found automatically (or
  chosen) and written in one click (only the `kadr` entry, with a backup; an invalid
  file is never touched), and the `claude mcp add` command for Claude Code.
- `kadr-mcp --kadr-settings <file>`; the README explains connecting from the app
  first, and by hand second.
- Decision record 0031.

### Added (phase 8a: transitions and text animations)

- `kadr-core`: entry and exit **transitions** on videos and images (`fade`, `slide`
  in four directions, `zoom` in or out) and entry and exit **text animations** on texts
  (`fade`, `slide`, `pop`, `type_on`, `word_by_word`), with the undoable commands
  `SetClipTransition` and `SetTextAnimation` and their validation rules. A clip
  touching a transition's edge on the same track shows underneath, so a fade between
  two clips is a dissolve and a slide pushes the previous clip out. Splitting keeps
  the entry on the first part and the exit on the second.
- File format version 2, with a migration from version 1.
- `kadr-render`: `transition_motion` and `text_animation_frame` move layers (opacity,
  position, scale) on both renderers; `TextRasterizer::rasterize_revealed` draws
  part of a text for type-on and word-by-word animations.
- `kadr-engine`: exports draw transitions, reading the touching clip's source past its
  cut (holding the file's last or first frame where it ends).
- `kadr-cli assemble --transition … --title … --title-animation …` to try them.
- `docs/transitions-and-text-animations.md`, decision record 0050.

### Added (phase 8b: silence removal)

- `kadr-core`: `RemoveClipParts` cuts timeline ranges out of a clip in one undoable
  command: what is left goes end to end, the later clips of its track move earlier,
  other tracks do not move (`Clip::without_parts`, `TimeRange`,
  `FrameRate::frame_start_at_or_before` / `_after`).
- `kadr-engine`: `silence_removal` finds a clip's silences and shortens each to a
  pause (0.2 s by default, half on each side), with cuts on frame starts; the
  analysis reports progress and can be stopped.
- Claude's tool `remove_silences` shortens every silence of a clip in one step;
  `detect_silences` and `split_clip` point to it.
- Desktop app: *Remove silences…* in the inspector's Sound section opens a dialog
  with three settings, a strip of what will go and a summary; one click, one undo
  step.
- `kadr-cli remove-silences video.mp4 -o shorter.mp4 [--threshold -45]`.
- Decision record 0051.

### Added (phase 8c: colour grading with `.cube` LUTs)

- `kadr-core`: the `color_lut` effect names a `.cube` file and a strength from 0 to 1.
  It applies after the colour adjustments, whatever the effects list order. Only the
  path is stored. File format 3, with a migration from 2 that changes nothing.
- `kadr-render`: `color_lookup_table` reads `.cube` files:
  - 3D up to 65, 1D up to 65 536;
  - domains, Resolve's input ranges, comments, Windows line endings and byte-order
    marks;
  - messages give line numbers and never quote lines.
  
  Tables apply on the CPU and the GPU with the same trilinear arithmetic. On the
  GPU, a `Rgba32Float` 3D texture read with `textureLoad`, kept while frames use it.
  The pass-through and the CPU's opaque-row copy now ask
  `VisualLayer::changes_colors`, so a lone canvas-sized video still gets its LUT.
- `kadr-engine`: `color_grading::load_color_lookup_table` refuses files over 32 MB
  and gives a `ColorTableSummary`.
  - Exports and previews keep the tables they read, and read a file again when it
    changes.
  - A LUT that cannot be read is skipped with a warning in the export report (clip,
    effect, file), never an error. So is a second LUT on a clip.
- Claude's tool `add_color_lut` (with `effect_id`: change the file or the strength):
  - the file must be inside the allowed folders and named `.cube`, and is read before
    the project changes;
  - one LUT per clip;
  - `find_media_files` lists `.cube` files with `kind` `color_lut`, and `import_media`
    points them to `add_color_lut`;
  - LUT paths stored in a project are checked against the allowed folders, and
    `open_project` warns about them.
- Desktop app: the inspector's Colour section chooses a `.cube` file (read in the
  background first), its strength (0–100 %), another file, or none. It says when the
  file has gone. *Reset* also removes the LUT.
- `kadr-cli assemble --lut film.cube [--lut-strength 0.8]` grades every video and
  image.
- Decision record 0052.

### Added (phase 8e: editable keyframes with curves)

- `kadr-core`: keyframe curves `EaseIn` (p²), `EaseOut` (1 − (1 − p)²) and
  `CubicBezier`, a CSS-like timing curve.
  - The rules keep its handles from 0 to 1, so every value stays between its two
    keyframes.
  - File format 4, with a migration from 3 that changes nothing.
  - `MoveKeyframe` moves a keyframe in time, keeping its value and curve.
  - `RemoveKeyframe::keeping_the_value_when_last` keeps the last keyframe's value.
  - `AnimatedValue::interpolation_in_effect_at` and `Interpolation::as_bezier_curve`.
- Desktop app: position, scale, rotation, opacity and volume can be animated by
  mouse in the inspector.
  - A diamond adds or removes a keyframe at the playhead, and the field shows and
    sets the value at the playhead.
  - A lane per animated value: click to go to a keyframe, drag to move it (snapped
    to frames), ← / → and Delete from the keyboard; previous / next keyframe.
  - The *Curve* menu (linear, ease in, ease out, ease in and out, hold, custom
    curve) and a small curve editor with two handles.
  - The timeline shows a clip's keyframes as diamonds: click to go there, drag to
    move every keyframe at that moment.
  - Each gesture is one undo step, and the preview is drawn by the engine, as the
    export is.
- Claude's tool `set_keyframes` sets one property's keyframes with their curves, in
  canvas coordinates. `get_clip` now shows animated positions in canvas coordinates
  too (they were shown from the canvas centre), with each keyframe's `curve`.
- Decision record 0054.

### Added (phase 8g: subtitles, SRT import and editing)

- `kadr-core`: subtitle tracks (`TrackKind::Subtitle`) whose cues
  (`ClipContent::Subtitle`) share the track's `SubtitleStyle`:
  - font, size, bold, colour, an outline or a box, bottom or top, a margin, the
    widest line;
  - sized from the canvas: a vertical canvas keeps them a quarter of the height
    above the bottom, clear of TikTok's, Reels' and Shorts' buttons.
  
  Cue texts keep `<i>`, `<b>` and line breaks. File format 5, with a migration from 4.
- `kadr-core::srt` reads `.srt` files forgivingly:
  - a byte-order mark, Windows or old Mac line endings, `,` or `.` milliseconds;
  - multi-line cues, numbering gaps, cues out of order or overlapping;
  - `<font>`, `<u>` and `{\an8}` removed with a warning.
  
  It also decodes UTF-16 and Windows-1252 files. Errors give line numbers. It writes
  cues back as `.srt`.
- Commands: `AddSubtitleCues`, `SetSubtitleStyle`, `SetSubtitleCueText`,
  `SetSubtitleCueTiming`, `MergeSubtitleCues`, `RemoveTimeFromSubtitleTracks`.
  `SplitClip` shares a cue's words between its parts. Subtitles follow the timeline,
  and removing parts of a clip (silence removal) removes the same moments from them.
- `kadr-render`: `TextRasterizer::rasterize_subtitle` draws a cue:
  - lines wrapped and centred, italic and bold words;
  - an outline from an exact distance transform, or a box per line;
  - placed on whole pixels at the margin, so the GPU and the CPU show the same subtitles.
- `kadr-engine`: subtitle tracks are burned into exports, previews and Claude's
  frames; export sizes and format changes refit their style. `subtitle_files` reads
  (16 MB at most) and writes `.srt` files and imports one as a track in one undo step.
- Claude's tools `import_subtitles`, `list_subtitles`, `edit_subtitles` (edit, add,
  remove, split and merge cues in one undo step), `set_subtitle_style` and
  `export_subtitles`. `get_timeline` summarises subtitle tracks, `add_track` makes
  one, `remove_clip` with `close_gap` closes them up too.
- Desktop app:
  - *Import subtitles (.srt)…* in the Text tab, or drop an `.srt` file;
  - cues on their own track in the timeline (moved, trimmed, split with S, deleted
    like clips);
  - the inspector edits a cue's words, start and end, splits it at the playhead,
    merges it with the next one, and changes the track's style;
  - *Add a subtitle at the playhead*, *Export as .srt…*.
- `kadr-cli import-subtitles edit.kadr film.srt [--offset 2] [--backdrop box]
  [--position top]` and `kadr-cli export-subtitles edit.kadr corrected.srt`.
- Decision record 0056.

### Fixed

- Desktop inspector, *Blur* section: two quick changes (a wider margin, then the
  rectangle shape, before the first answer arrived) undid each other. Blur
  changes now name only the settings they change, like the other settings since
  #18.
- Desktop inspector: two quick changes to the same clip could make the second undo
  the first. New words typed in a title, then a click on an alignment button before
  the first answer arrived, brought the old words back; the same could happen to
  the font, size and colour of a title, to the colour settings and to the crop.
  Crop, colour and text requests now name only what changes, and the back end
  keeps the rest as the project has it.
- Desktop app: edits, undo and redo are sent one at a time, in order. Two requests
  sent together could be applied in either order (left then right alignment could
  end up left), and an undo pressed right after a change could be ignored.
- Desktop inspector: words being typed in a title were replaced by the saved words
  when the answer to an earlier change (a font size, say) arrived.

## [0.7.0] — Phase 6: desktop app

### Added (phase 6a: desktop app design)

- `docs/ux.md`: the desktop app's screens and flows (import → edit → export), states,
  notifications, keyboard shortcuts, languages and accessibility.
- `docs/design-system.md`: colour, spacing, type and motion tokens for the dark and
  light themes, with measured WCAG contrast, and the component list.
- `docs/mockups/index.html`: static mockups of every key screen, in both themes and
  both languages.
- Decision records 0020 (Svelte 5 and TypeScript), 0021 (the app's layout in the
  repository) and 0022 (how the front end and the engine talk).

### Added (phase 6b: desktop app skeleton)

- `kadr-engine`: `preview` (the export's frame composition at any time, shrunk to a
  panel's size), `timeline_sound` (any range of the timeline's sound, mixed exactly
  like the export, decoded clips kept), `media_overview` (thumbnails with decoders kept
  open, waveform peaks) and `picture_scaling` (box-filter shrinking). Export behaviour
  is unchanged; its frame composer now takes the project per frame.
- `apps/desktop`: the Tauri 2 desktop app. Rust back end (crate `kadr-desktop`) owning
  the project, saved after every change, with preview and thumbnail threads; Svelte 5
  front end in French and English, dark and light themes, interface size setting.
- Home screen (new project with its format, open, recent projects), media import by
  drag and drop or dialog with reading and error tiles, + to add at the end of the
  timeline, automatic format from the first video, preview drawn with WebGL, playback
  with sound (Space, J/K/L, frame and second steps), timeline with zoom, undo and redo.
- CI: WebKitGTK on Linux for the desktop crate, and a job checking, testing and
  building the front end.

### Added (phase 6c: timeline editing)

- Clips on the timeline show thumbnails along pictures and a waveform along sound,
  with badges for a changed speed or volume.
- Drag a file from the library onto a track or onto the "new track" zone; move
  clips along their track, to another track or to a new one; trim either edge
  (the preview shows the frame at the edge). A ghost shows where the clip would
  land, or turns red and says why it cannot (overlap, wrong kind of track).
- Snapping to clip edges, the playhead and the start (N, or Alt held to drag
  without it); split at the playhead (S); delete the selection (Delete, with an
  Undo notice); previous and next cut (↑ / ↓); hide, mute and remove tracks.
- Every gesture is one undo step whose name appears in the undo button and notices.

### Added (phase 6d: inspector, titles and export)

- Inspector for the selected clip: picture (position, scale, rotation, opacity,
  crop), sound (volume), speed (0.5× to 2× or any value), colour (exposure, contrast,
  saturation, temperature) and text (words, font, size, colour, alignment, length),
  each with a Reset; number fields take typing, arrow keys and dragging.
- Titles: T, the timeline's button or three ready-made styles in the Text tab, on a
  Titles track.
- Export dialog (Ctrl+E): presets (project format, YouTube 1080p and 4K, TikTok ·
  Reels · Shorts, square), file name and folder, quality, encoder, estimated size;
  progress with time left, cancel, *Keep editing* with a progress pill; the finished
  file with *Show in folder* and *Play the video*, or a clear error with *Try again*.
- `kadr_engine::export_presets`: the export sizes and the scaling of a project to
  another size, shared by Claude's tools and the app, and `refit_clips_to_canvas`,
  which keeps titles in place when the canvas changes shape.

### Changed

- Build scripts and procedural macros are compiled with optimisations, so `bindgen`
  no longer overflows the stack of Windows build scripts.

### Fixed

- Desktop inspector: a change made just before the inspector closed (a click on the
  empty timeline or on another clip) was lost, often with an error in the console.
  It happened to a title's words typed and not yet confirmed, to a colour chosen in
  the picker and to every number field (position, scale, opacity, crop, volume,
  speed, colour, font size, length): the field sends its value while it is being
  removed, when the inspector no longer has its clip. The change now goes to the
  clip the inspector was showing.
- Number fields: after ↑ or ↓, leaving the field put the old value back.
- Number fields in English: a value of 1000 or more was shown as "1,200", read back
  as 1.2, so just entering and leaving the field changed it (Position X 1200 px
  became 1 px). Editable numbers are now shown without thousands separators.

## [0.6.0] — Phase 5: MCP server

### Added (phase 5: MCP server, file mode)

- `kadr-mcp`: a local MCP server over stdio, built on the official Rust SDK `rmcp`
  3.11, that Claude Desktop and Claude Code start with `--allowed-folder <folder>`
  arguments. Nothing but the protocol is written to stdout; logs go to stderr.
- `kadr-tools`: 35 tools defined once (name, description for a language model, JSON
  schema generated with `schemars`, code), independent of MCP: project
  (`create_project`, `open_project`, `save_project`, `get_project_info`,
  `set_project_settings`, `undo`, `redo`), media (`find_media_files`, `probe_media`,
  `import_media`, `list_media`), timeline (`get_timeline`, `get_clip`, `add_track`,
  `remove_track`, `add_clip`, `remove_clip`, `move_clip`, `trim_clip`, `split_clip`),
  clip properties (`set_clip_speed`, `set_clip_volume`, `set_transform`), content
  (`add_text`, `edit_text`, `add_transition`, `remove_transitions`,
  `add_color_adjustment`, `remove_effect`), pictures (`get_frame`,
  `get_contact_sheet`), sound (`detect_silences`) and export (`export`,
  `get_export_status`, `cancel_export`).
- One tool call is one undo step labelled "Claude: …"; the project file is saved
  after every change and never overwritten if another program changed it.
- File access limited to the allowed folders (links and `..` resolved), no file
  replaced without `"overwrite": true`, no file deleted.
- Pictures for Claude are JPEG with the longest side at most 1024 pixels, with the
  original size and the conversion factor stated.
- Exports run in the background with a task id; presets for TikTok / Reels / Shorts,
  YouTube 1080p and 4K, with automatic scaling when the shape matches.
- `kadr-engine::still_frames`: single pictures of a file, a clip or the finished
  timeline, and labelled contact sheets.
- Silence detection: `kadr-vision::silence_detection` (RMS level per 10 ms window,
  incremental), `kadr-engine::silence_analysis` (sound decoded piece by piece at
  16 kHz), the `detect_silences` tool.
- Integration tests driving the server through an MCP client, including the real
  binary as a child process; `docs/mcp-server.md`; decision records 0010–0012;
  README section "Editing with Claude".

## [0.5.0] — Phase 4: face blur by region tracking

### Added (phase 4a: region tracker)

- `kadr-vision`: `RegionTracker` follows a selected region frame after frame, without
  AI:
  - a MOSSE correlation filter finds its position;
  - the peak-to-sidelobe ratio rates each frame as confident, uncertain or lost;
  - pyramidal Lucas-Kanade optical flow predicts fast motion and measures changes of
    size (Median Flow);
  - normalised cross-correlation with the memorised appearance finds a lost region
    again.
- `smooth_track`: One Euro smoothing, forwards and backwards, so the blur neither
  shakes nor lags.
- `GreyImage`: brightness images built straight from a video's Y plane, optionally
  shrunk to about 640 pixels wide (`shrink_factor_for_tracking`).
- `synthetic_scenes`: textured scenes with a known truth, for tests and benchmarks.
- Tracking scenarios (motion, acceleration, coming closer, moving away, camera pan,
  occlusion, leaving the frame, backwards) and `criterion` benchmarks
  (`cargo bench -p kadr-vision`).
- `docs/tracking-and-blur.md`, decision record 0007.

### Added (phase 4b: blur analysis)

- `kadr-engine`: `region_tracking`:
  - `RegionTrackingRequest::for_clip` turns a selection on a clip into a request;
  - `track_region` tracks it forwards and backwards from the selected frame. The
    backward pass decodes in chunks. It reports progress and can be cancelled;
  - the result is smoothed and stored as few samples as possible (within a pixel);
  - `export_tracking_debug_video` draws the tracked box and its confidence.
- `TrackedBlur::unsure_ranges` lists where the tracker was uncertain or lost.
- `kadr-vision`: `TrackingError::RegionWithoutDetail` for a uniform region on a
  textured background. A recovered region now counts as uncertain, and the
  recovery search starts at exactly one region size.
- `kadr-cli blur video.mp4 --at 2.5 --rect 420,180,160,200 -o out.mp4 [--debug]`.
- `scripts/generate-tracking-benchmark.ps1`, the `track_video` example and
  `scripts/compare-tracking-with-truth.py`: measure the tracker on a 60 s 1080p video
  whose true path is known.

### Fixed (phase 4b: tracking drift on a real video)

On the 1080p benchmark the region slid off the object by 2 to 4 pixels per second
while every frame stayed "confident" (mean centre error 197 px, size error 24 %). Now
2 px and 1.7 % (decision record 0008):

- Median Flow measures the size only from points moving with the region, and the
  correlation filter's window is weighted towards the region.
- The correlation filter learns at 0.02 instead of 0.125: it no longer forgets the
  selection within a few frames.
- Every confident frame is anchored to the selection's appearance: searched a few
  cells around the answer at three sizes, the region is pulled towards it where it
  matches clearly.
- A confident frame that does not look like the selection at all is reported as
  uncertain, and the region is re-anchored where the selection is found again nearby.

## [0.4.0] — Phase 3: GPU rendering

### Added (phase 3a: GPU rendering)

- `VideoFrame` is now `Rgba` or `Yuv420`: decoders hand over their YUV planes and the
  encoder takes YUV planes, so most pictures are never converted on the CPU.
- `YuvColorSpace` (BT.601 / BT.709, limited / full range) with one set of conversion
  coefficients shared by the CPU code and the GPU shaders.
- `kadr-render`: `FrameRenderer` with a `GpuRenderer` (`wgpu` 30: Vulkan, DirectX 12,
  Metal, OpenGL ES) and the `CpuRenderer` reference; the GPU is used when available.
- A single video already at canvas size is passed through without drawing.
- Tests checking the GPU draws the same pictures as the CPU (WARP / lavapipe in CI).
- `criterion` benchmarks of 1080p and 4K frames (`cargo bench -p kadr-render`).
- `kadr-cli render --cpu`; the export report names the renderer.
- `docs/rendering.md`, decision record 0006.

### Added (phase 3b: text)

- Text clips are drawn: `TextRasterizer` (`cosmic-text`) lays out and rasterises the
  words, with left, centre or right alignment and line breaks.
- The Inter font is bundled (SIL Open Font License 1.1); system fonts serve other
  families and characters Inter lacks, such as emoji.
- `LayerSizing::NativePixels`: texts are drawn at their size in canvas pixels, videos
  and images keep fitting inside the canvas.
- Export tests drawing a title with the CPU and the GPU renderers.

### Added (phase 3c: effects)

- Colour adjustment (exposure, contrast, saturation, temperature) on the GPU and the
  CPU with the same formula; up to four per layer, in order.
- Tracked blur rendering: gaussian blur, pixelation or solid fill, in a rectangle or an
  ellipse with soft edges, applied in the source picture's space on YUV or RGBA.
- `TrackedBlur::region_at` and `hidden_area_at` in `kadr-core`: the area at any source
  time, interpolated between samples, with a wider margin where tracking was unsure.
- The export reports tracked blurs without regions and ignored colour adjustments.
- Tests: region effects, colour formula, GPU/CPU parity with adjustments, exports of a
  moving hidden area and of a black-and-white adjustment.

### Changed

- Exports are BT.709 limited range and tagged as such in the MP4 (phase 2 exports were
  BT.601 untagged, which shifted HD colours).
- FFmpeg only prints fatal messages; Kadr reports every failure itself.
- Measured on the owner's laptop: GPU frames take 10 ms (1080p) and 25 ms (4K); a mixed
  1080p export runs at 1.6× real time instead of 0.6×.

## [0.3.0] — Phase 2: media and export

### Added

- `kadr-media`: the `MediaBackend` traits (probe, frame-accurate video source, audio
  segment decoding, export sink) and their FFmpeg implementation (`ffmpeg-next`,
  FFmpeg 7 or newer):
  - probing of video, audio and image files;
  - frame-accurate reading, in order or with jumps, keeping one frame ahead;
  - sound decoding of any range at any speed, pitch preserved (`atempo`), stereo,
    padded or cut to the exact clip length;
  - MP4 writing (H.264 + AAC) choosing the first working encoder: NVIDIA NVENC,
    Intel Quick Sync, AMD AMF, Apple VideoToolbox, then libx264.
- `kadr-media::test_patterns`: barcode frames and beeps to check exports exactly.
- `kadr-render`: CPU compositor with fit, position, scale, rotation, crop and
  opacity, rows rendered in parallel.
- `kadr-engine`: atomic `.kadr` saving and loading, timeline sound mixing with
  animated volume, frame-by-frame export with progress and cancellation (a failed or
  cancelled export never leaves a broken file), and quick assembly of files end to end.
- `kadr-cli`: `probe`, `assemble` and `render` commands; Ctrl+C cancels an export cleanly.
- `FrameRate::frame_count_covering` in `kadr-core`.
- Acceptance test: three cuts, a 1.5× speed change and a music track export with
  every frame taken from the exact expected source frame and every beep within 20 ms.
- CI installs FFmpeg 9 on Windows, Linux and macOS.
- Decision record 0005 (FFmpeg through `ffmpeg-next`).

## [0.2.0] — Phase 1: data model

### Added

- `kadr-core` data model:
  - exact time in integer ticks (flicks) with `Timestamp`, `TimeSpan` and exact
    `FrameRate` fractions;
  - one identifier counter for media, tracks, clips and effects, with a distinct type
    per kind;
  - project settings with landscape, vertical and square presets;
  - media assets (video, audio, image), tracks stacked bottom to top, media and text
    clips with exact playback speeds;
  - volume, opacity, position, scale and rotation animatable by keyframes (linear,
    ease-in-out, hold), and crop;
  - colour adjustment and tracked blur effects, the blur regions indexed by source time.
- 22 undoable commands covering media, tracks, clips, keyframes, effects and settings.
- `EditSession` with undo, redo, a history limit, and transactions that group several
  commands into one undo step, all or nothing.
- `validate_project`: every rule of a valid project in one place, run after every
  command and on every loaded file. Error messages name the objects involved and list
  what exists when something is missing.
- Versioned `.kadr` JSON file format with a migration mechanism, refusing files from
  newer versions.
- Tests: unit tests for every module and command (apply, revert and redo must give
  identical projects), an end-to-end editing scenario, 400 pseudo-random edits fully
  undone, and a snapshot of a complete project file.
- `docs/data-model.md`.

## [0.1.0] — Phase 0: foundations

### Added

- Cargo workspace with eight documented crates: `kadr-core`, `kadr-media`,
  `kadr-render`, `kadr-vision`, `kadr-engine`, `kadr-tools`, `kadr-mcp`, `kadr-cli`.
- Workspace-wide lints: missing documentation, `unsafe`, `unwrap` and `expect` are
  reported; CI fails on any warning.
- `kadr-cli` with `--help` and `--version`, and its tests.
- Continuous integration on Windows, Linux and macOS: formatting, build, clippy, tests,
  documentation, and test video generation.
- `scripts/setup-windows.ps1`: read-only check of the Windows development environment.
- `scripts/generate-test-fixtures.ps1`: synthesises the test videos with FFmpeg.
- Documentation: README, architecture, contribution guide, first architecture decision
  records, third-party licenses.
- GPL-3.0-or-later license.

### Changed

- CI uses `actions/checkout` v7, which runs on Node 24.
