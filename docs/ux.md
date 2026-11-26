# User experience of the desktop app

How the Kadr desktop app is meant to feel and behave, screen by screen. This is the
reference for phase 6 (desktop MVP); phase 7 adds face blur and Claude to the same
layout.

- **Mockups:** open [mockups/index.html](mockups/index.html) in a browser. Its top bar
  switches between screens, dark and light themes, French and English.
- **Visual language** (colours, spacing, type, components):
  [design-system.md](design-system.md).

## Goals

1. **A beginner makes a first edit without a tutorial.** Import, place, cut, export:
   each step is visible on screen, with a hint where the next action happens.
2. **An experienced editor works fast with the keyboard.** Every frequent action has a
   shortcut, shown in its tooltip and in the `?` sheet.
3. **The picture is the star.** The preview and the timeline take most of the window;
   the side panels can be resized and folded, and the layout is remembered.
4. **Nothing is serious.** Every edit can be undone with Ctrl+Z, the project saves
   itself, and nothing on disk is overwritten without asking.

## Screens

```mermaid
flowchart LR
    home[Home<br/>new project, recent projects] --> editor
    editor[Editor<br/>library · preview · inspector · timeline] --> export[Export dialog<br/>settings → progress → done]
    editor --> settings[Settings]
    editor --> shortcuts["Shortcuts sheet (?)"]
    export --> editor
```

### Home

Shown when the app starts, and when the user clicks the Kadr logo in the editor.

- **New project**, with a format choice: *Automatic* (selected by default: the project
  takes the size and frame rate of the first video placed on the timeline),
  *Landscape 16:9* (1920 × 1080), *Vertical 9:16* (1080 × 1920), *Square 1:1*.
- **Open…** a `.kadr` file.
- **Recent projects**: thumbnail, name, last change, length, format. Clicking opens.
- A reminder that videos never leave the computer.

### Editor

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ K  Project name ✓ Saved            ↶ ↷ │ 16:9 · 1920×1080 · 30 fps │ ? ⚙ [Export] │
├──────────────┬────────────────────────────────────────────┬──────────────────┤
│ Media │ Text │                                            │ Inspector        │
│ [search]   + │              Preview                       │ (selected clip,  │
│ ▣ ▣          │                                            │  or the project) │
│ ▣ ▣          │                                            │                  │
│ ▣ ▣          │ 00:07:12 / 00:24:00   ⏮ ◀ ▶ ▶ ⏭   quality  │                  │
├──────────────┴────────────────────────────────────────────┴──────────────────┤
│ ✂ 🗑 │ T Add a title                                  🧲 │ − ═══○══ + │ fit    │
│        0:00   0:02   0:04   0:06 ▼ 0:08                                       │
│ T Titles     [ Lisbon, day 1 ]                                                │
│ ▶ Video 1  👁🔊[ tram ][ miradouro ][ pasteis 1.5× ][ logo ]                   │
│ ♪ Music      🔊[ fado-guitar.mp3 ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~ ]            │
└──────────────────────────────────────────────────────────────────────────────┘
```

| Area | Content |
|---|---|
| **Top bar** | Logo (back to Home), project name (rename in place), save state, undo and redo (tooltip names the step: "Undo: split clip"), project format chip (opens the project in the inspector), shortcuts `?`, settings, **Export** |
| **Library** (left) | *Media* tab: imported files as thumbnails with kind icon, length, a check when used in the timeline, and a **+** to add at the end. *Text* tab: title styles to add |
| **Preview** (centre) | The project frame at the playhead, letterboxed to the project format. Transport: timecode, go to start, previous frame, play/pause, next frame, go to end, preview quality |
| **Inspector** (right) | The selected clip's settings, in folding sections; the project's settings when nothing is selected |
| **Timeline** (bottom) | Toolbar (split, delete, add a title, snapping, zoom, fit), ruler, track headers, tracks, playhead |

Minimum window: 1280 × 720. Below 1360 px wide, side panels get narrower; each
panel can be folded to a thin strip, and the timeline height is resizable.

## Main flow: import → edit → export

### 1. Start a project

The user clicks **New project** (format *Automatic* by default). The editor opens
empty:

- the library is one big drop zone: *"Drag your videos here"*, with an **Import
  files** button (Ctrl+I) and the accepted formats;
- the preview says where the picture will appear;
- the timeline shows faint empty tracks and *"Drag a media file here to start your
  edit, or click + on a library thumbnail"*;
- the inspector shows the project format.

The project is saved automatically, from the first change, in
`Documents/Kadr/<project name>.kadr`. **Save as…** moves it elsewhere.

### 2. Import media

- Drag files from the file explorer **anywhere on the window**, or use Import.
- Each file appears at once as a tile with a spinner (*"Reading…"*), then its
  thumbnail and length. Importing never blocks the interface.
- A file Kadr cannot read becomes a red tile: *"Unreadable: notes.txt"*. Its tooltip
  and a notification explain: *"notes.txt is not a video, a picture or a sound Kadr
  can read. Accepted formats: MP4, MOV, MKV, WebM, MP3, WAV, M4A, PNG, JPG."* The
  tile has a remove button.
- Importing the same file twice reuses the existing tile.

### 3. Put clips on the timeline

Two ways, from simplest to most precise:

- **Click + on a tile**: the clip goes at the end of the first track of the right
  kind (videos and pictures on *Video 1*, sounds on *Audio 1*); the track is created
  if needed.
- **Drag a tile onto the timeline**: a ghost shows where the clip will land and
  snaps to clip edges and the playhead (yellow guide line). Dropping:
  - on a free spot of a track of the right kind → the clip goes there;
  - on a spot that is taken → the ghost is red and says why (*"Overlaps
    tram-28.mp4: drop next to it or on another track"*); releasing does nothing;
  - in the *"new track"* area below the tracks → a new track is created for it.

With the *Automatic* format, the first video placed sets the project's size and
frame rate; a notification says so and offers to undo.

### 4. Watch and move around

| Action | Mouse | Keyboard |
|---|---|---|
| Play / pause | Play button | Space |
| Play backwards / pause / forwards | — | J / K / L (press J or L again: 2×, then 4×) |
| One frame back / forward | Arrow buttons | ← / → |
| One second back / forward | — | Shift + ← / → |
| Previous / next cut | — | ↑ / ↓ |
| Jump anywhere | Click or drag on the ruler | Home / End |

Playback plays sound in sync and drops frames rather than slowing down when the
computer cannot keep up. The preview shows exactly what the export will produce:
both use the same renderer.

### 5. Cut and arrange

- **Select**: click a clip (it gets a bright outline and edge handles); click an
  empty spot or press Esc to deselect. Shift + click adds clips to the selection.
- **Split** (S or ✂): cuts the selected clip at the playhead; with nothing
  selected, cuts every clip under the playhead.
- **Delete** (Del or 🗑): removes the selected clips and leaves the gap. A
  notification offers *Undo*.
- **Move**: drag a clip along its track or to another track of the same kind. Same
  ghost, snapping and refusal rules as when dropping from the library. Alt while
  dragging turns snapping off for that gesture.
- **Trim**: drag a selected clip's edge. A bubble shows the new length; the preview
  shows the frame at the edge. A clip cannot be stretched past the end of its file.
- **Snapping** (🧲 or N): to clip edges, the playhead and the start of the timeline,
  within 8 screen pixels. On by default.
- **Zoom**: slider, + / −, Ctrl + mouse wheel (centred on the mouse), *Fit* (Shift+Z).
- **Tracks**: hide a video track (👁), mute any track with sound (🔊). An empty
  track can be removed from its header's menu.
- **Undo / redo**: Ctrl+Z / Ctrl+Y (also Ctrl+Shift+Z), or the top-bar arrows whose
  tooltips name the step.

Clips show what they contain, never by colour alone: an icon and the file name on
every clip; thumbnails along video clips; a waveform along anything with sound; the
speed (*1.5×*) and a changed volume (*60 %*) as badges.

### 6. Adjust a clip (inspector)

Changes show in the preview immediately; each finished gesture (a slider released, a
value typed) is one undo step.

| Section | Settings | Shown for |
|---|---|---|
| **Picture** | Position X/Y (pixels), scale (%), rotation (°), opacity (%). *Advanced: crop* (left, top, right, bottom) | Videos, pictures, titles |
| **Sound** | Volume (0–400 %) | Clips with sound |
| **Speed** | 0.5×, 1×, 1.5×, 2×, or a custom value. Voices keep their pitch; the clip's length changes, later clips do not move (a longer clip that would overlap is refused with a message) | Videos and sounds |
| **Colour** | Exposure, contrast, saturation, temperature; off until first touched | Videos and pictures |
| **Text** | Words, font, size, colour, alignment, duration | Titles |

Number fields accept typing, arrow keys (Shift for bigger steps) and dragging
sideways on the field. Each section has a *Reset*.

A value animated with keyframes (by Claude, for example) shows *"Animated (3
keyframes)"* and cannot be changed to a single value from the inspector yet:
keyframe editing arrives in phase 8.

With nothing selected, the inspector shows the **project**: format (16:9, 9:16, 1:1
or a custom size), frame rate, background colour, and the project's length.

### 7. Add a title

*Add a title* (toolbar), the *Text* tab or **T** adds a title at the playhead, on a
*Titles* track above the videos (created if needed), 3 seconds long, selected, with
the inspector's text field focused. The *Text* tab offers a few ready-made styles
(title, subtitle at the bottom, big centred word).

### 8. Export

**Export** (Ctrl+E) opens a dialog:

- **Presets** on the left: *Project format* (suggested), *YouTube 1080p*, *YouTube 4K*,
  *TikTok · Reels · Shorts* (1080 × 1920), *Square*. A preset whose shape differs from
  the project (vertical for a horizontal project) shows: *"This preset is vertical;
  your project is horizontal. Kadr will switch the project to vertical and refit the
  clips. You can undo it with Ctrl+Z."*
- **File name** (the project name) and **folder** (the Videos folder, or the last one
  used), with *Choose…*. An existing file is never replaced without a confirmation.
- **Quality**: *High* (suggested) or *Fast (larger file)*. *Advanced*: hardware
  encoder when possible, or software only.
- A summary: length, estimated size, MP4 · H.264 · AAC.

Then **progress**: percentage, frame count, elapsed and remaining time, the encoder
used, *Cancel export*, and *Keep editing* (the dialog folds into a progress pill in
the top bar; the export uses the project as it was when it started).

Then **done**: the file with its size and export time, *Show in folder*, *Play the
video*. Or a **human error**: what happened, why, what to do (*"The disk is full…
Free some space or pick another folder, then try again. Your previous file was not
touched."*), with *Try again* and *Copy the details*.

## States every screen handles

| State | What the user sees |
|---|---|
| Empty | An explanation and the next action (drop zone, *"Drag a media file here…"*) |
| Loading | A spinner where the content will be; the rest stays usable |
| Error | What happened, why, what to do; never a raw code. Engine messages are translated for the cases the app knows, and shown as they are otherwise |
| Long text | Names are cut with an ellipsis and shown whole in a tooltip; French, about 20 % longer than English, is the reference for widths |
| Missing file | A media file moved since the project was saved: its tile and clips are marked *Missing*, the preview shows a placeholder. Relinking arrives after the MVP |

## Notifications

Small cards at the bottom of the preview, for 5 seconds (longer when they hold an
action), never stacking more than three:

- after destructive or surprising actions, with **Undo**: clip deleted, format changed
  by the first video, format changed for an export preset;
- background work finished: export done (with *Show in folder*);
- errors that are not tied to a dialog (an unreadable imported file).

## Keyboard shortcuts

The full list is also in the app (`?`). Shortcuts do not fire while typing in a field.

| Group | Action | Keys |
|---|---|---|
| Playback | Play / pause | Space |
| | Backwards / pause / forwards (repeat for 2×, 4×) | J / K / L |
| | Previous / next frame | ← / → |
| | One second back / forward | Shift + ← / → |
| | Previous / next cut | ↑ / ↓ |
| | Start / end | Home / End |
| Editing | Split at the playhead | S |
| | Delete the selection | Delete, Backspace |
| | Undo / redo | Ctrl+Z / Ctrl+Y, Ctrl+Shift+Z |
| | Deselect | Esc |
| | Add a title at the playhead | T |
| Timeline | Zoom in / out | + / − , Ctrl + wheel |
| | Fit the whole edit | Shift+Z |
| | Snapping on / off | N |
| | Drag without snapping | Alt (held) |
| Project | Import files | Ctrl+I |
| | Export | Ctrl+E |
| | Save (automatic anyway) / save as | Ctrl+S / Ctrl+Shift+S |
| | New / open | Ctrl+N / Ctrl+O |
| Help | This list | ? |

Customisable shortcuts are planned after the MVP; the sheet already lists them in
one place.

## Languages

French and English from the start. The language follows Windows (French if Windows
is in French, English otherwise) and can be changed in the settings without
restarting. Every string lives in a translation file, keyed by meaning; numbers,
sizes and dates use the language's conventions (*17,6 Mo* / *17.6 MB*).

## Accessibility

- Contrast at least WCAG AA in both themes (checked token by token in
  [design-system.md](design-system.md)).
- Everything works with the keyboard: logical tab order, visible focus ring, panels
  reachable with F6, the timeline navigable with arrows when focused.
- Icons always have a label (tooltip and `aria-label`); information is never carried
  by colour alone.
- Interface size 80–150 % in the settings; `prefers-reduced-motion` is respected.

## What the MVP leaves for later

| Later | Phase |
|---|---|
| Blurring a face: draw a box on the preview, tracking with progress, uncertain ranges coloured on the timeline, correction | 7 |
| Claude working on the open project, *"Claude changed the project"* notifications, *Connect to Claude* screen | 7 |
| Moving and resizing clips directly in the preview, with guides | 7 |
| Transitions, keyframe curves, silence removal, stabilisation, LUTs, subtitles | 8 |
| First-launch tour and sample project; customisable shortcuts; relinking moved files | after the MVP |

The layout keeps room for them: phase 7's blur tools live in the inspector (a
*Blur* section) and on the preview, Claude's status in the top bar.
