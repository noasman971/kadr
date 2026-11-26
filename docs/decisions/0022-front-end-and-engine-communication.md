# 0022. How the front end and the engine talk

- Status: Accepted
- Date: 2026-11-26

## Context

The desktop front end runs in a webview; the project, the decoders and the renderer
live in Rust. The interface must answer every action in under 100 ms, play the edit
smoothly with its sound in sync, and show **exactly** what the export will produce.
Everything must keep working on Android (phase 9) and let Claude change the open
project later (phase 7).

## Options considered

For the preview, which is the hard part:

1. **The engine renders, the webview displays.** The same renderer as the export draws
   each preview frame; the frame travels to the webview as raw bytes through Tauri's
   binary IPC and is drawn on a `<canvas>` with WebGL. The engine also mixes the
   sound, which the webview plays with Web Audio. Exactly what the export gives, the
   same path on every platform; costs one copy per frame, so the preview resolution
   is limited to what the panel shows.
2. **A native GPU surface under the webview.** `wgpu` draws straight into a native
   window placed behind a transparent hole in the webview. No copy and full
   resolution, but fragile: window stacking, resizing and DPI kept in sync by hand,
   menus and tooltips unable to overlap the picture, and a different solution on
   Android.
3. **The webview decodes with `<video>`.** Cheap, but the preview would not be the
   export (effects, text, colour pipeline), and the codecs depend on the webview.

## Decision

Option 1, validated by the owner on 2026-11-26. The rules:

**The Rust back end owns the project.** It holds the `EditSession` (project, undo,
redo) and the file. The front end never edits a project itself: it sends an *edit
request* (`split_clips_at { time }`, `move_clip { clip, track, start }`…), the back
end turns it into `kadr-core` commands (one undo step per user gesture), saves the
file, and answers with a fresh **project view**: a JSON description made for the
interface (tracks, clips with their computed start, length and badges, media with
their state, undo and redo labels). The view carries a revision number; the front
end draws from it and nothing else. A refused edit returns the engine's reason,
which the front end shows in plain words.

**Time crosses the boundary as integer ticks** (705 600 000 per second, see
[data-model.md](../data-model.md)), safe in JavaScript numbers for any realistic
length. The front end only converts to seconds and pixels for display.

**Pictures and sound cross as raw binary** (`tauri::ipc::Response`), never as JSON
or base64:

| Data | Format | Produced by |
|---|---|---|
| Preview frame | Width, height, then Y, U, V planes (4:2:0, BT.709 limited range); the WebGL shader converts to RGB | `kadr-engine` preview, at the preview size the panel asks for |
| Thumbnail | Width, height, then RGBA | `kadr-engine` media overview |
| Waveform | Peaks per second, then one byte per peak | `kadr-engine` media overview |
| Playback sound | Interleaved stereo 32-bit float at the project rate | `kadr-engine` timeline sound |

**Threads.** Video sources are not `Send` (see `CLAUDE.md`), so preview rendering
runs on one dedicated thread that owns its decoders, text cache and renderer;
requests queue there and a newer request replaces an older one still waiting, so
scrubbing never builds a backlog. Thumbnails run on their own thread. Imports, sound
and waveforms run on blocking worker threads. Exports run on their own thread with
a copy of the project, report progress through Tauri events, and are cancelled with
`ExportCancellation`.

**Playback clock.** Web Audio is the clock: while playing, the front end schedules
sound chunks a few seconds ahead and asks, at each screen refresh, for the frame at
the sound's current position (skipping frames when the computer cannot keep up, never
drifting). Without sound (reverse play, muted edit), the same clock runs on
`performance.now()`.

## Consequences

- The preview cannot differ from the export: same composition, same renderer.
- One copy per frame: about 0.8 MB at 960 × 540. Measured numbers go in
  `docs/desktop-app.md`; the preview size follows the panel and the quality setting.
- A 4K project is rendered at full size and then scaled down for the preview; a
  renderer output scale is a later optimisation.
- Claude changing the open project (phase 7) only needs the back end to publish a new
  project view: the front end already redraws from views.
