# Subtitles

How subtitles are imported from `.srt` files, edited, drawn into the video and
written back. Code:

- `kadr-core`: `subtitle.rs` (cues, style, `<i>`/`<b>` markup), `srt.rs` (the file
  format), `commands/subtitle_commands.rs`;
- `kadr-render`: `subtitles.rs` (drawing a cue);
- `kadr-engine`: `subtitle_files.rs` (files on disk, an import as one undo step) and
  `export/frame_composer.rs`;
- `kadr-tools`: `tools/subtitle_tools.rs` (Claude);
- the desktop app: `editing/subtitles.rs` and `SubtitleInspector.svelte`;
- `kadr-cli`: `subtitles.rs`.

Why it works this way: [decision record 0056](decisions/0056-subtitle-tracks-following-the-timeline.md).
Automatic transcription is not part of Kadr: it needs AI. Subtitles come from an `.srt`
file (a transcription service, YouTube, a translator) or are typed.

## Subtitle tracks and cues

A **subtitle track** holds **cues**: words shown from a moment of the timeline for
some time. It holds nothing else, and cues go nowhere else.

- Cues never overlap on a track (like any clips). Two people talking at once need two
  subtitle tracks, one at the top for example.
- Every cue of a track looks the same: the track has **one style**. Restyling 400 cues
  is one change.
- A cue has no position, size, opacity, effect or transition of its own: the rules
  refuse them and point to the track's style.
- Subtitle tracks are drawn where they are in the stack, like video tracks. A new
  one goes on top, so subtitles cover the videos and titles below.

### The words

A cue's text keeps the light markup of `.srt` files:

| Written | Shown |
|---|---|
| a line break | a new line |
| `<i>words</i>` | italic (Inter has no italic face: the letters are slanted) |
| `<b>words</b>` | bold (black when the whole style is bold) |

Only these four tags are tags, in any case. Anything else is shown as it is written:
`1 < 2` stays `1 < 2`. An unclosed `<i>` runs to the end of the cue.

### The style

| Setting | Default |
|---|---|
| Font | Inter (bundled; any installed font works) |
| Size | 1/18 of the canvas's shorter side: 60 px in 1080p, landscape or vertical |
| Bold | On for vertical canvases, off otherwise |
| Letters' colour | White |
| Behind the letters | A black **outline** (8 % of the font size), a **box** behind each line (black, 70 % opaque by default), or nothing |
| Position | Bottom (or top) |
| Margin | From that edge of the canvas to the nearest line: **25 % of the height on vertical canvases**, which keeps subtitles clear of the captions and buttons TikTok, Reels and Shorts draw over the bottom fifth; 8 % on landscape and square ones (inside television's title-safe area) |
| Widest line | 80 % of the canvas width; longer lines wrap between words. Lines are centred |

The margin is measured to the lines' full height, so a cue with descenders
("jumping") sits exactly where one without ("ABC") does.

When the canvas changes (an export size of the same shape, or a new format in the app),
the style follows: size, outline and margin keep their proportion to the new canvas's
defaults. A landscape project turned vertical moves its subtitles up, clear of the
apps' buttons.

## Time: subtitles follow the timeline

A cue at 12 s shows at 12 s of the edit, like the times of an `.srt` file.

| Edit | Subtitles |
|---|---|
| Split, trim or move a video | Do not move |
| Remove silences (or other parts of a clip, closing the gaps) | Lose the same moments: a cue inside a removed part disappears, a cue overlapping one gets shorter, later cues move earlier. The words stay under the voice |
| Claude's `remove_clip` with `close_gap` | The same, for the removed clip's time |
| Remove a clip without closing the gap | Do not move |

So the usual order is free: import the `.srt` of the interview, then remove its
silences; or remove the silences first and import subtitles made for the shorter
video. A video moved by hand leaves its subtitles behind: move the cues too.

## Editing cues

| Change | Command | In the app | Claude |
|---|---|---|---|
| Words | `SetSubtitleCueText` | Inspector's text field | `edit_subtitles`, action `edit` |
| Start and end | `SetSubtitleCueTiming` | Inspector's *Start* / *End*, or drag and trim on the timeline | `edit` |
| Add | `AddSubtitleCues` | *Add a subtitle at the playhead* (2 s, less if the next cue comes sooner) | `add` |
| Split | `SplitClip` | S, or *Split at the playhead* | `split` |
| Merge with the next | `MergeSubtitleCues` | *Merge with the next* | `merge` |
| Delete | `RemoveClip` | Delete | `remove` |
| Style | `SetSubtitleStyle` | Inspector's *Style of the track* | `set_subtitle_style` |

Splitting a cue shares its words: the cut falls at the space or line break nearest
to the cut's share of the cue's letters, and tags open at the cut are closed and
opened again. A single word cannot be cut: both parts keep it. Merging joins the
words with a space, from the first cue's start to the second's end.

Every change is one undo step. Claude's `edit_subtitles` takes a list of changes and
applies them as **one** step: fixing ten typos is one undo.

## `.srt` files

### Reading

`kadr_core::srt::parse_srt` is forgiving where files in the wild differ:

- a byte-order mark; Windows (`\r\n`), Unix or old Mac (`\r`) line endings;
- `,` or `.` before the milliseconds, one to three digits of them (`01,5` is 1.5 s),
  hours left out (`01:02,500`), position coordinates after the times (ignored);
- missing numbers, numbers with gaps or out of order (numbers are ignored);
- blank lines inside a cue's text (the text goes on until the next number or timing
  line);
- cues out of order (sorted), overlapping cues (the earlier one ends when the next
  starts), cues starting together (combined, one under the other);
- empty cues and cues lasting no time (left out);
- `<u>`, `<s>`, `<font …>` and `{\an8}` codes (removed: the style sets the look).

Every change is listed in warnings, shown by the app and Claude's answer.

It is precise where a file cannot be read. Errors give the line, never quote it:

```text
line 6: this timing line cannot be read: the two times must be separated by "-->", as in 00:00:01,000 --> 00:00:03,500
line 2: this subtitle ends (00:00:02,000) before it starts (00:00:05,000)
line 1: expected a subtitle number or a timing line such as 00:00:01,000 --> 00:00:03,500, but found text
```

Files are decoded by `decode_subtitle_file_bytes`:

- UTF-8 with or without a byte-order mark;
- UTF-16 with one;
- otherwise Windows-1252, the usual encoding of older French, Spanish or German
  files, with a warning.

`kadr_engine::subtitle_files::read_subtitle_file` refuses files over 16 MB.

### Writing

`write_srt` numbers cues from 1, writes `HH:MM:SS,mmm` (rounded to the millisecond,
never less than one millisecond long), Windows line endings, UTF-8 without a mark.
Files are written atomically (a temporary file renamed over the target).

Times are exact: a file imported then exported gives the same milliseconds back.

## How cues are drawn

1. For each subtitle track, the frame composer finds the cue under the playhead.
2. `TextRasterizer::rasterize_subtitle` lays it out with `cosmic-text`: wrapped at the
   widest line, centred, italic and bold runs from the markup.
3. The backdrop is drawn behind the letters:
   - **outline**: an exact Euclidean distance transform of the letters' ink, giving
     an anti-aliased disc of the outline's width around every pixel of ink;
   - **box**: a rectangle behind each line, the lines' boxes touching.
4. The picture is placed on whole pixels, centred, at the margin.
5. Both renderers only copy it. The GPU and the CPU show the same subtitles
   (`gpu_matches_cpu`, `subtitles_export`).

Only the pictures of the cues on screen are kept, so a film with a thousand cues does
not keep a thousand pictures. Exports, the app's preview and Claude's `get_frame` all
go through the same composer.

## Claude

Tools, in [mcp-server.md](mcp-server.md#subtitles):

- `import_subtitles`;
- `list_subtitles`;
- `edit_subtitles`;
- `set_subtitle_style`;
- `export_subtitles`.

> **User:** Add the subtitles in my Videos folder to the interview, fix the typos, put
> them in a box, and give me the corrected file.
>
> **Claude:** `find_media_files` (`kind` `subtitles`) → `import_subtitles` → `list_subtitles` → `edit_subtitles` (all the fixes in
> one call) → `set_subtitle_style` (`backdrop` `box`) → `get_frame` at a cue's time to
> check → `export_subtitles` → `export` for the video with the subtitles burned in.

## Command line

```powershell
.\target\release\kadr-cli import-subtitles edit.kadr interview.srt --backdrop box
```

```powershell
.\target\release\kadr-cli render edit.kadr edit.mp4
```

```powershell
.\target\release\kadr-cli export-subtitles edit.kadr interview.corrected.srt
```

`import-subtitles` changes the project file in place; options:

- `--offset <seconds>` moves every cue later;
- `--backdrop outline|box|none`;
- `--position bottom|top`.

`export-subtitles` takes `--track <id>` when the project has several subtitle tracks.

## Known limits

- One cue at a time per track (overlapping cues of a file are shortened).
- Not kept from files:
  - per-cue positions (`{\an8}`);
  - colours and fonts written in the file;
  - karaoke timing.
- WebVTT, `.ass` and `.ssa` are not read.
- Cues do not follow a video moved by hand.
- Subtitles are always burned in: an MP4 subtitle stream (soft subtitles) is not
  written. For platforms that show subtitles themselves, export the `.srt` file.
