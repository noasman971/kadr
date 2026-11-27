# 0056. Subtitles: cues on subtitle tracks, styled by the track, following the timeline

- Status: Accepted
- Date: 2026-11-27

## Context

The specification's phase 8 asks for subtitles with SRT import and editing ("Sous-titres :
import/édition SRT"). Automatic transcription is out of scope: it needs AI, and the owner
will discuss it later. The owner gave the go-ahead on 2026-11-27.

What users need:

- an `.srt` file (from a transcription service, YouTube, a translator) burned into the
  video;
- a few cues fixed by hand or by Claude (a typo, a timing, a cue too long);
- the corrected `.srt` written back, for platforms that show subtitles themselves.

Five questions had to be answered:

1. how cues and their look are modelled;
2. what cues do when the edit changes (cuts, moves, silence removal);
3. where `.srt` files are read and written;
4. how cues are drawn identically by both renderers;
5. what Claude, the app and the command line can do with them.

## Options considered

1. **Cues as ordinary text clips** on a video track. Everything already works on them,
   but each cue carries its own style: restyling 400 cues is 400 changes, and nothing
   keeps them alike.
2. **A separate list of cues** beside the tracks. Clean, but every editing gesture
   (drag, trim, split, delete, snapping, undo) would have to be written again, in the
   core, the app and Claude's tools.
3. **A subtitle track kind whose clips are cues**: a new `TrackKind::Subtitle`, a new
   clip content `ClipContent::Subtitle(SubtitleCue)` (words and duration only), and one
   `SubtitleStyle` on the track. Clip commands and gestures work on cues; the style is
   shared by construction.
4. **Cues follow a clip** (anchored to the source time of the video they transcribe,
   like tracked blurs). They would survive any edit of that video. But an `.srt` file
   describes the times of a finished video, a cue may span several clips, and editing a
   cue's time would mean converting through whichever clip lies under it.
5. **Cues follow the timeline**, like the times of an `.srt` file and the caption
   tracks of most editors. Cutting, trimming or moving a video then leaves cues where
   they are. Only edits that **remove time** from the edit can desynchronise them.
6. **Removed time**: leave the subtitles alone, or remove the same moments from them.

## Decision

- **Model** (option 3), file format 5 (a migration adds `"subtitle_style": null` to
  every track of a version 4 file):
  - a subtitle track has a `SubtitleStyle`, other tracks have none (rule in
    `validation.rs`);
  - a cue is `ClipContent::Subtitle(SubtitleCue { text, duration })`, only on subtitle
    tracks, which hold nothing else;
  - a cue has no placement, opacity, crop, effect or transition of its own: the rules
    refuse them and point to the track's style.
- **Text**: the light markup of `.srt` files is kept as it is: line breaks, `<i>` and
  `<b>`. Only those four tags are tags (`parse_cue_markup`); anything else, `<` included,
  is shown as written, so a cue reads back exactly as it was typed or imported.
- **Style**: font, size, bold, text colour, a backdrop (an outline around the letters,
  a box behind each line, or nothing), bottom or top, a margin from that edge, and the
  widest line as a fraction of the canvas width (longer lines wrap between words; lines
  are centred).
  - `SubtitleStyle::for_canvas` sizes it from the canvas: a font of 1/18 of the shorter
    side (60 px in 1080p), a black outline of 8 % of the font;
  - the margin is 25 % of the height on a vertical canvas, clear of the captions and
    buttons TikTok, Reels and Shorts draw over the bottom fifth, and 8 % otherwise
    (inside television's title-safe area);
  - white bold letters on vertical canvases, regular on others.
- **Time** (option 5): cues follow the timeline. Splitting, trimming or moving a video
  changes no cue.
  - Removing time **does** reach the subtitles (option 6, second part):
    `RemoveClipParts` (silence removal, and later filler words or bad takes) removes the
    same moments from every subtitle track. Claude's `remove_clip` with `close_gap` does
    the same through `RemoveTimeFromSubtitleTracks`.
  - A moment `t` moves earlier by the removed time before it: a cue inside a removed
    range disappears, a cue overlapping one gets shorter, later cues keep their length.
    The words stay under the voice that says them.
  - Other tracks still do not move (decision 0051).
- **Commands**:
  - `AddTrack` (kind subtitle, with the canvas's default style or
    `with_subtitle_style`);
  - `AddSubtitleCues` (any number, one step);
  - `SetSubtitleStyle`, `SetSubtitleCueText`, `SetSubtitleCueTiming` (start and end in
    one step);
  - `MergeSubtitleCues` (with the next cue of the track: from the first's start to the
    second's end, words joined by a space);
  - `RemoveTimeFromSubtitleTracks`.
  - `SplitClip` on a cue cuts its words at the space or line break nearest to the cut's
    share of the cue's characters (tags reopened on the second part); a word without a
    space stays whole in both parts.
  - `MoveClip`, `TrimClip` and `RemoveClip` work on cues unchanged.
- **Files**: the format is text, so its reader and writer are in `kadr-core`
  (`kadr_core::srt`), like the `.kadr` format. The reader is forgiving:
  - a byte-order mark, `\r\n` or `\r`, `,` or `.` before one to three digits of
    milliseconds, hours left out, coordinates after the times;
  - missing or out-of-order numbers, blank lines inside a cue's text;
  - cues out of order (sorted) or overlapping (the earlier one ends when the next one
    starts; cues starting together become one, one under the other);
  - `<u>`, `<font>`, `<s>` and `{\an8}` removed with a warning.

  Errors give the line number and never quote the file.
  `decode_subtitle_file_bytes` reads UTF-8 (with or without a mark), UTF-16 with a mark,
  and Windows-1252 (with a warning) for older Western European files. The writer numbers
  from 1, writes `HH:MM:SS,mmm` and Windows line endings. Reading and writing files
  (16 MB at most, atomic writes) and adding an import as one undo step are in
  `kadr_engine::subtitle_files`, shared by every interface.
- **Drawing** (`TextRasterizer::rasterize_subtitle`, `kadr-render`): the cue is laid
  out by `cosmic-text`, wrapped and centred; italic words are slanted (Inter has no
  italic face), bold ones use Inter's weight axis (`<b>` in a bold style is black).
  - The outline comes from an exact Euclidean distance transform of the letters' ink
    (an anti-aliased disc of the outline's width around each pixel of ink).
  - A box is drawn behind each line, the lines' boxes touching.
  - The picture is made on the CPU and placed on whole pixels at the margin, measured
    to the lines' layout box so that a cue with descenders sits where one without does.
  - Both renderers only copy it: the GPU and the CPU show the same subtitles (tested).
  - Subtitle tracks are drawn where they are in the stack (new ones go on top).
  - The frame composer keeps only the pictures of the cues on screen.
- **Canvas changes**: export sizes of the same shape and changes of the canvas's shape
  refit the style (`refitted_from_canvas_to_canvas`): its size, outline and margin keep
  their proportion to the canvas's defaults, so a project turned vertical moves its
  subtitles clear of the apps' buttons.
- **Interfaces**:
  - Claude gets `import_subtitles`, `list_subtitles`, `edit_subtitles` (edits, adds,
    removals, splits and merges, in one undo step), `set_subtitle_style` and
    `export_subtitles`. `get_timeline` summarises subtitle tracks instead of listing
    hundreds of cues.
  - The app shows cues on their track, edits them in the inspector, and imports and
    exports `.srt` files.
  - `kadr-cli` gets `import-subtitles` and `export-subtitles`.

## Consequences

- An imported `.srt` file becomes a track that is edited like any other, with undo, by
  the user and by Claude. Restyling is one change.
- Subtitles stay in sync through silence removal. They do not follow a video that is
  moved by hand: the user moves the cues too (or imports after editing).
- One cue at a time per track: overlapping cues of a file are shortened. Two speakers
  at once need two subtitle tracks (one at the top, say).
- Per-cue positions (`{\an8}`), colours, karaoke timing, WebVTT, `.ass` and burned-in
  styles from the file are not supported. They are for later, if asked.
- Automatic transcription, when the owner decides on it, can produce `TimedSubtitleCue`s
  and reuse everything here.
- On Android (phase 9), pickers give `content://` addresses: the engine will need an
  entry point taking the file's bytes (`decode_subtitle_file_bytes` and `parse_srt`
  already do).
