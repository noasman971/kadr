# The desktop app

How the Kadr desktop app is built. What it looks like and how it behaves is in
[ux.md](ux.md) and [design-system.md](design-system.md); the choices behind it are
decision records [0020](decisions/0020-svelte-typescript-front-end.md),
[0021](decisions/0021-desktop-app-layout.md) and
[0022](decisions/0022-front-end-and-engine-communication.md).

![The editor in the dark theme, in French](screenshots/desktop-editor-dark-fr.jpg)

![The editor in the light theme, in English](screenshots/desktop-editor-light-en.jpg)

## Running it

You need the Rust and FFmpeg setup of the [README](../README.md#setting-up-windows)
and Node.js 20.19 or newer. From `apps/desktop/`:

```powershell
npm ci
```

```powershell
npm run tauri dev
```

The first build compiles the whole engine and Tauri (several minutes); later ones
take seconds. The front end reloads by itself when a `.svelte` or `.ts` file changes;
a change in Rust rebuilds and restarts the app.

| Command (in `apps/desktop/`) | What it does |
|---|---|
| `npm run tauri dev` | The app with live reloading |
| `npm run tauri build` | An optimised app and its installer (`target/release/bundle/`) |
| `npm run check` | Type check of the front end (`svelte-check`, no warning allowed) |
| `npm test` | Front-end unit tests (`vitest`) |
| `npm run build` | Builds the front end alone into `dist/` |

The Rust side is a workspace crate like the others: `cargo clippy`, `cargo test` and
`cargo doc` at the root cover it.

### Driving the running app from a script

To check the app without clicking by hand, start it with the WebView's debugging port
open, then talk to it with the Chrome DevTools Protocol (Node 22 has `WebSocket`
built in):

```powershell
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
npm run tauri dev
```

A second app running at the same time (another session's) holds port 1420, port
9222 and the WebView2 data folder: start this one with its own, or WebView2 refuses
to start (`0x8007139F`):

```powershell
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9237'
$env:WEBVIEW2_USER_DATA_FOLDER = 'C:\Users\you\kadr-build\webview-2'
node node_modules/@tauri-apps/cli/tauri.js dev --config other-ports.json
```

with `other-ports.json` setting `build.devUrl` to `http://localhost:1437` and
`build.beforeDevCommand` to `npm run dev -- --port 1437` (`npm run tauri dev --
--config …` loses the option in PowerShell). Vite reloads the page when a
translation file changes: the app goes back to the home screen.

`http://127.0.0.1:9222/json` lists the page; `Runtime.evaluate`, `Input.dispatchKeyEvent`,
`Input.dispatchMouseEvent` and `Page.captureScreenshot` do the rest. Things learned
doing it:

- `window.__TAURI_INTERNALS__.invoke` is read-only: it cannot be replaced to fake an
  answer. Every call goes through `fetch("http://ipc.localhost/<command>")`, which can
  be wrapped, for example to answer `plugin:dialog|open` with chosen files instead of
  opening the native dialog.
- Ctrl+I without that opens the real dialog, which a script cannot close.
- A file drop from the explorer can be imitated by emitting Tauri's own event:
  `invoke("plugin:event|emit", { event: "tauri://drag-drop", payload: { paths, position } })`.
- Real clicks (`Input.dispatchMouseEvent`) are better than `element.click()`: they
  also show hover states and go through pointer events.

## The two halves

```mermaid
flowchart LR
    subgraph webview[Webview: apps/desktop/src]
        ui[Svelte components] --> state[State: editor, playback, preferences]
        state --> api[api/backend.ts]
        canvas[Preview canvas, WebGL] 
        audio[Web Audio]
    end
    subgraph rust[Rust: apps/desktop/src-tauri]
        commands[Tauri commands] --> open[OpenProject: edit history + file]
        commands --> preview[Preview thread]
        commands --> thumbnails[Thumbnail thread]
        commands --> sound[Timeline sound mixer]
    end
    api -- "invoke (JSON)" --> commands
    commands -- "project view (JSON)" --> api
    preview -- "YUV frame (bytes)" --> canvas
    sound -- "PCM (bytes)" --> audio
    rust --> engine[kadr-engine, kadr-core, kadr-media]
```

### Back end (`src-tauri/src/`)

| Module | Role |
|---|---|
| `open_project.rs` | The project being edited: `EditSession` (undo / redo), its `.kadr` file, the revision counter, the snapshot shared with the worker threads. **Every change goes through `OpenProject::edit`**, which saves the file afterwards |
| `editing.rs` | `EditRequest`: what the interface can ask, and the rules each request follows (for example *add at the end of the first track of the right kind*). One request is one undo step; undo labels are keys the front end translates |
| `project_view.rs` | `ProjectView`: the description the front end draws, rebuilt after every change, with tracks in display order and computed clip lengths |
| `media_workers.rs` | The preview and thumbnail threads, which own their decoders (FFmpeg's are not `Send`). The preview thread renders only the newest waiting request |
| `binary_messages.rs` | The byte layouts of frames, thumbnails, waveforms and sound |
| `commands/` | The Tauri commands, thin calls into the above and `kadr-engine` |
| `recent_projects.rs` | The home screen's list, in the app's configuration folder |
| `command_error.rs` | Errors with a stable code the front end translates, plus the engine's details |

### Front end (`src/`)

| Folder | Role |
|---|---|
| `lib/api/` | Typed calls to the commands (`backend.ts`), the shared types (`types.ts`), binary decoders |
| `lib/state/` | `editor` (project view, playhead, selection, imports), `playback` (clock and sound), `preferences` (language, theme, interface size), `toasts` |
| `lib/components/` | Screens and panels: home, top bar, library, preview, inspector, timeline, dialogs |
| `lib/ui/` | Reusable components: buttons, icon buttons, dialog, key caps, notifications, icons |
| `lib/i18n/` | French and English texts (`messages.en.ts` defines the keys, `messages.fr.ts` must translate all of them) |
| `lib/styles/` | Design tokens (`tokens.css`) and base styles |
| `lib/time/`, `lib/timeline/` | Ticks, frames and timecodes; time ↔ pixels and ruler steps (unit-tested) |

The front end holds **no project data of its own**: it draws the latest project view
and sends requests. Selection, playhead and zoom are its only state.

## Preview

1. The preview panel asks for the frame at the playhead, at the size it shows it
   (panel size × screen pixel ratio, or a fixed fraction of the canvas when the
   quality is set to full, 1/2 or 1/4).
2. `render_preview_frame` takes a snapshot of the project and queues a job on the
   preview thread. If newer jobs are already waiting, older ones are answered
   *superseded*: scrubbing never builds a backlog.
3. The thread renders with `kadr_engine::preview::PreviewRenderer`, which uses the
   **same frame composition as the export** (decoders, renderer, effects), then
   shrinks the frame with a box filter.
4. The YUV planes travel as raw bytes; WebGL 2 uploads them as three textures and a
   shader converts BT.709 limited range to RGB.

The panel asks for one frame at a time and always for the newest position, so a slow
frame makes the preview skip, never lag.

## Playback

Web Audio is the clock. Pressing play:

1. fetches the first second of mixed sound (`get_timeline_sound`, mixed by
   `kadr_engine::timeline_sound::TimelineSoundMixer` exactly like the export);
2. starts the clock 60 ms later and schedules that second at an exact time;
3. at every screen refresh, computes the playhead from the audio clock, and fetches
   the next second whenever less than two are scheduled ahead.

A chunk that arrives late starts part-way through so it stays in sync. Each clip's
sound is decoded once, whole, and kept (up to about ten minutes of sound), so
playing again starts at once. J and L play at 2× and 4× when pressed again (the
sound is sped up with its pitch); backwards play has no sound and uses
`performance.now()` as its clock. Any edit restarts playback from the playhead with
the new sound.

## Thumbnails and waveforms

- Thumbnails come from the thumbnail thread (`ThumbnailMaker` keeps the six most
  recent decoders open) as RGBA bytes, drawn on small canvases. The front end keeps
  the last 600 in memory, by file path, time and size.
- Waveforms are decoded once per file at 8 kHz and reduced to 100 peaks per second
  (one byte each), kept by the back end and the front end.

## Projects on disk

- New projects go to `Documents/Kadr/<name>.kadr`; a taken name gets a number.
- The project is saved after **every** change (atomic write: a temporary file, then a
  rename). A failed save shows *Not saved* in the top bar with the reason; the next
  change, or Ctrl+S, tries again.
- Renaming the project renames its file. The recent list follows renames.

## Editing on the timeline

Every gesture is worked out in the front end while it happens and sent as **one**
request when it ends, so one gesture is one undo step:

- **Rules during a gesture** live in `src/lib/timeline/placement.ts`, as pure,
  tested functions: snapping (clip edges, the playhead and the start, within 8 screen
  pixels), overlaps, which tracks accept a clip, trimming limits (the other edge, the
  file's length, the neighbours), and the cut times for ↑ / ↓.
- **`TimelinePanel`** shows the result before anything is sent: a ghost where the
  clip would land (red, with the reason, when it cannot), the yellow snapping guide,
  the length while trimming. Clips are dragged with pointer events, not HTML drag
  and drop (see the pitfalls in `CLAUDE.md`).
- **Media dragged from the library** cross two panels: `state/mediaDrag.svelte.ts`
  holds the gesture, the library starts it, the timeline previews and places the
  clip, and the editor screen draws the badge under the pointer.
- **The back end** (`editing.rs`, `editing/arrangement.rs`) turns each request into
  `kadr-core` commands in one transaction. The engine checks everything again: the
  front-end rules only make refusals visible before the release.
- Pointer positions go through `pointerOffsetX` (`src/lib/ui/pointer.ts`), because
  the interface size setting zooms the page.

![A file dragged from the library: ghost, snapping guide and badge](screenshots/desktop-timeline-drop.jpg)

![Trimming a clip: its length, and the frame at the edge in the preview](screenshots/desktop-timeline-trim.jpg)

## Inspector and titles

The inspector shows the project when nothing is selected, and the settings of the
clip when one is (`ClipInspector.svelte`): picture, sound, speed, colour and text, only
where they apply. Each change sends one request (`editing/clip_settings.rs`), so one
undo step:

- `NumberField` takes typing, ↑ / ↓ (Shift for steps ten times bigger) and dragging
  its label sideways; the value is sent when the gesture ends. A value animated with
  keyframes shows *Animated (n keyframes)* and is not editable yet (phase 8).
- The colour adjustment is an effect added on the first change, replaced afterwards,
  and removed by *Reset*.
- A speed change keeps the clip's start; a clip that would then overlap the next one
  is refused by the engine.

Titles (`editing/titles.rs`) are text clips added at the playhead, 3 s long, on the
first *Titles* track free at that time (a new one on top otherwise), in one of three
styles sized from the canvas height. The inspector then focuses their words.

![The inspector of a video clip](screenshots/desktop-inspector.jpg)

![Titles from the Text tab](screenshots/desktop-titles.jpg)

## Export

The dialog (`components/export/ExportDialog.svelte`) gathers the settings; the export
itself is `state/exportJob.svelte.ts` on the front end and `export_job.rs` on the back
end:

- **Sizes.** The presets come from `kadr_engine::export_presets`, shared with Claude's
  tools. A preset of the project's shape exports a scaled copy of the project; one of
  another shape first changes the project's format as an undoable step
  (`setCanvasSize`), which also refits titles (`refit_clips_to_canvas`).
- **Running.** `start_export` takes a snapshot of the project, so editing during the
  export changes nothing in the file, and runs the engine's `export_project` on its
  own thread. Progress (a few times a second) and the result come back through a
  Tauri channel. One export at a time.
- **Files.** An existing file is replaced only after the user agrees
  (`exportFileExists`, then a confirmation). The engine writes to a temporary file
  and renames it at the end, so a failed or cancelled export never leaves a broken
  file. Only files exported during the session can be shown in the explorer or
  played from the app (`reveal_exported_file`, `open_exported_file`).
- **Keep editing** folds the dialog into a progress pill in the top bar; when the
  export ends, a notice says so.

![The export settings](screenshots/desktop-export-settings.jpg)

![An export finished](screenshots/desktop-export-done.jpg)

## Blurring a face

The flow is in [ux.md](ux.md#blurring-a-face-phase-7); the tracker itself in
[tracking-and-blur.md](tracking-and-blur.md).

```mermaid
sequenceDiagram
    participant User
    participant Front as Preview (BlurOverlay)
    participant Back as Back end
    participant Thread as Tracking thread
    User->>Front: B, then draws a box
    Front->>Back: get_clip_overlay(clip, playhead)
    Back-->>Front: source → canvas mapping, boxes of existing blurs
    User->>Front: Blur and follow (Enter)
    Front->>Back: start_region_tracking(clip, time, box in source pixels, shape)
    Back->>Thread: track_region (forwards, then backwards)
    Thread-->>Front: progress (channel)
    Thread->>Back: add the tracked blur (one undo step)
    Thread-->>Front: finished, with the new project view
```

- **Coordinates.** A blur lives in the **source video's pixels**. `get_clip_overlay`
  answers with the renderer's own map from source pixels to canvas pixels for the clip
  at the playhead (`kadr_engine::clip_geometry`, keyframes included) and the visible
  part of the source. The front end multiplies it by the display scale to draw, and
  inverts it to turn the pointer into source pixels (`lib/blur/sourceMapping.ts`). A
  box is an upright rectangle in the source: on a rotated clip it shows rotated.
- **The tool** (`lib/state/blurTool.svelte.ts`): off → drawing → adjusting →
  tracking. It works on the selected video clip under the playhead, or the top one
  there (`lib/blur/blurTarget.ts`). Moving the playhead while adjusting keeps the box
  in source pixels over the new frame; the analysis starts from the frame under the
  playhead.
- **The analysis** (`tracking_job.rs`) runs on its own thread with copies of what it
  needs, one at a time. When it ends, it locks the open project, checks it is still
  the same opening (`OpenProject::identity`, kept by renaming), adds the effect as the
  undo step `blurRegion`, and sends the new view. Editing goes on meanwhile.
- **Errors** before the analysis (box too small or outside the picture, clip not on
  screen) come back from `start_region_tracking`, and the box stays to be fixed;
  errors found while analysing (no detail to follow) come as a `failed` event, with
  the box kept too. Each has its own error code and sentence.
- **Settings** are ordinary edit requests (`setBlurSettings`, `setBlurEnabled`,
  `removeBlur`, `editing/blur_settings.rs`); they never touch the samples.
- The preview draws the boxes of the selected clip's blurs at the playhead, when not
  playing (they would lag behind the picture): solid when the tracker was sure,
  dashed and marked ⚠ when not. Boxes follow clip transforms, not the motion of
  phase 8's transitions.

### Checking and correcting

- The project view gives each blur's **unsure ranges in timeline ticks**
  (`project_view.rs`: source times mapped through the clip's range and speed, each
  range to the end of its last frame). The timeline draws them as striped bands
  under the clip's name, amber (uncertain) or red (lost), with ⚠ and a tooltip;
  clicking one puts the playhead in its middle. **Shift+B** and the inspector's
  *Next* go to the next moment to check (`lib/blur/unsureMoments.ts`, round the
  edit).
- **Correcting** is the same tool with `correctedEffectId` set: dragging a blur's box
  on the preview (or *Correct the box here* in the inspector, then the arrow keys,
  Shift for bigger steps) puts the box where the face is, and *Correct from here*
  (Enter) calls `start_region_tracking` with the blur's `effectId`. The back end
  builds the request with `RegionTrackingRequest::for_correction` (only up to the
  neighbouring anchors) and `blur_results.rs` merges the analysis into the blur **as
  it is then** with `corrected_tracked_blur`, as the undo step `correctBlur`.

| Drawing the box | Following it |
|---|---|
| ![The box drawn on the preview, with its handles and the Blur and follow bar](screenshots/desktop-blur-draw.jpg) | ![The analysis' progress with Stop](screenshots/desktop-blur-following.jpg) |
| **A moment the tracker lost (dashed, ⚠) and the Blur section** | **Light theme** |
| ![The inspector's Blur section next to a lost box](screenshots/desktop-blur-inspector.jpg) | ![The same in the light theme](screenshots/desktop-blur-light.jpg) |
| **Correcting: the box dragged onto the target, the lost stretch striped on the clip** | **Corrected: the stretch is now only where the target was hidden** |
| ![Correcting a lost box](screenshots/desktop-blur-correct.jpg) | ![After the correction](screenshots/desktop-blur-corrected.jpg) |

## Adding an edit

1. Add a variant to `EditRequest` in `editing.rs`, with a test, and a branch in
   `apply_edit_request` that runs `kadr-core` commands inside one transaction whose
   label is a key (`"splitClip"`).
2. Add the same shape to `EditRequest` in `src/lib/api/types.ts`.
3. Add `undo.<label>` to both message files.
4. Call `editor.edit({ type: "…", … })` from the component.

## Languages

`t("key", { name })` returns the text in the current language. The language follows
the system (French if it is French, English otherwise) until the user picks one in
the settings. Default track names from the engine ("Video 2") and undo labels are
translated on display; error codes from the back end map to `error.<code>` texts.
