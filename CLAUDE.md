# CLAUDE.md — working memory for Kadr

Read this first in every session. Keep it up to date: architecture changes, useful
commands, conventions, and every pitfall you hit.

## What Kadr is

A free, open source video editor in Rust (GPL-3.0-or-later). Two signature features:

1. **Driven from a Claude conversation through a local MCP server.** No Claude API, no
   API key: the user's Claude subscription is enough. Kadr never talks to Anthropic.
2. **Face blur by classic region tracking.** No AI, no neural network, no model file.

Desktop first (Windows priority), then Android with the same engine.

The full specification, in French, is [PROMPT_CLAUDE_CODE.md](PROMPT_CLAUDE_CODE.md).
It is the source of truth for scope, phases and rules.

## Current state

- **Phase 0 (foundations): done and validated**, tagged `v0.1.0`.
- **Phase 1 (data model): done and validated**, tagged `v0.2.0`.
- **Phase 2 (media and export): done and validated**, tagged `v0.3.0`.
- **Phase 3 (GPU rendering): done and validated**, tagged `v0.4.0` (PRs #3, #4, #5).
- **Phases 4 (face blur by tracking), 5 (MCP server) and 6 (desktop app): done**,
  tagged `v0.5.0`, `v0.6.0` and `v0.7.0` at the owner's request on 2026-11-27.
- **Phases 7 (blur and Claude in the app) and 8 (features) are in progress**, each in
  small PRs whose description starts with "Status for resuming". Phase 9 (Android) is
  paused by the owner; phase 10 (iOS) needs a Mac.
- **[docs/status.md](docs/status.md) says where each phase stands, how to resume on
  another computer, and the prompt to start each session with: read it first.**
  Versions are tagged in phase order (`v0.8.0` = phase 7, `v0.9.0` = phase 8, …),
  each after the owner validates it.

## Decisions taken with the owner

- License: **GPL-3.0-or-later**.
- Language: **everything in English** (code, comments, docs, commit messages). The
  specification file stays in French.
- Git: own repository in this folder, public GitHub repository `noasman971/kadr`.
- Phase 0 was committed directly on `main` at the owner's request. Since phase 1, each
  phase goes on its own branch with a pull request (see
  [docs/contributing.md](docs/contributing.md)).
- **Merging (owner's instruction, 2026-11-26):** Claude merges the pull requests it
  opened while working with the owner, once CI is green and there is no conflict.
  Never merge pull requests from other contributors. Merging does not replace phase
  validation: at the end of a phase, still stop and wait for the owner's OK before
  tagging the version and starting the next phase.
- **`main` is protected on GitHub** (owner's request, 2026-11-26), administrators
  included: pull request required (0 approvals), the six CI checks required
  ("Formatting", the three "Build, lint, test and docs (…)" jobs, "Test video fixtures
  can be generated", and since 2026-11-26 "Desktop front end (types, tests, build)"),
  conversations resolved, no force push, no deletion. Direct pushes
  to `main` are refused. If a CI job is renamed, update the required checks too
  (`gh api repos/noasman971/kadr/branches/main/protection`), or every PR will wait for
  a check that never runs. Tags can still be pushed.
- Architecture decisions are recorded in [docs/decisions/](docs/decisions/). While
  phases run in parallel, record numbers are reserved to avoid clashes: phase 4 uses
  0007–0009, phase 5 0010–0019, phase 6 0020–0029, phase 7 0030–0039, phase 9
  0040–0049, phase 8 0050–0059.

## Commands

```powershell
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo doc --workspace --no-deps
powershell -ExecutionPolicy Bypass -File scripts/setup-windows.ps1
powershell -ExecutionPolicy Bypass -File scripts/generate-test-fixtures.ps1
```

Desktop app, from `apps/desktop/` (Node.js 20.19+):

```powershell
npm ci
npm run tauri dev
npm run check
npm test
```

End-of-phase checklist: build without warnings, clippy, fmt, tests, `cargo doc`, docs
updated, CHANGELOG updated, summary to the owner, **stop and wait for validation**.

## Architecture in one paragraph

`kadr-core` (data model, commands, undo, validation, `.kadr` file) has no Kadr
dependency; read [docs/data-model.md](docs/data-model.md) before changing it.
`kadr-media` exposes `MediaBackend` traits implemented with FFmpeg (MediaCodec later)
and the `VideoFrame` (YUV or RGBA) and colour-space types; `kadr-render` turns layers
into BT.709 YUV frames on the GPU (`wgpu`) or the CPU (reference); `kadr-vision`
(classic tracking) will do the face blur. `kadr-engine` is the only API interfaces
use (it re-exports the media backend types). `kadr-tools` defines the tools Claude can
call (`KadrToolbox::call_tool`, no MCP dependency); `kadr-mcp` serves them over stdio
with `rmcp`. Read [docs/mcp-server.md](docs/mcp-server.md) before changing a tool.
`kadr-app-link` is the local pipe through which `kadr-mcp` passes Claude's calls
to the open desktop app, which runs them on its own project (decision 0030).
`kadr-cli` is for testing. The desktop app `apps/desktop` (Tauri 2) has a Svelte front
end and a Rust back end, the crate `kadr-desktop`, which owns the project and answers
with project views, frames and sound ([docs/desktop-app.md](docs/desktop-app.md)).
Details: [docs/architecture.md](docs/architecture.md).

## Conventions (short version — full version in docs/contributing.md)

- Clear, complete names: verb + object for functions (`split_clip_at_time`), domain
  nouns for types (`TimelineClip`), units in names when ambiguous (`duration_in_seconds`).
- `///` on every public item, `//!` at the top of every module. `missing_docs` is on.
- No `unwrap`/`expect` outside tests (clippy warns, CI fails on warnings).
- `unsafe` is denied workspace-wide; a crate needing FFI opts out explicitly with a
  justification.
- Short functions, named constants, strong types (`ClipId`, not `u64`).
- Timeline time is exact (integers / rationals), never `f64`.
- Conventional Commits: `feat(core): add command grouping for undo`.

## Pitfalls found so far

- **This folder sits inside another Git repository** (`Desktop/Project`, remote
  `CVGenerator`). Kadr has its own `.git`. Always run Git commands from inside
  `kadr/`, and never touch the parent repository.
- **Cargo warns about unused dependencies** (`cargo::unused_dependencies`, default since
  recent Cargo versions), and CI turns warnings into errors. Only declare a dependency
  when the crate actually uses it.
- In Git Bash, Rust tools live in `~/.cargo/bin`; a shell opened before installing
  rustup needs `export PATH="$HOME/.cargo/bin:$PATH"`.
- FFmpeg: the owner's machine has the Gyan **static** CLI (for scripts) and the BtbN
  **shared** 9.0 build (winget `BtbN.FFmpeg.GPL.Shared.9.0`) with `FFMPEG_DIR` set as a
  user variable. Git Bash sessions opened before that need `FFMPEG_DIR`,
  `LIBCLANG_PATH` and the BtbN `bin` folder exported by hand. `ffmpeg-next` 9.1
  supports FFmpeg up to 9.x; Kadr requires FFmpeg 7+ (channel layout API).
- `ffmpeg-next`'s scaler (`software::scaling::Context`) is **not `Send`**: video sources
  and export sinks live on the thread that created them (`VideoFrameSource` is not Send).
- Always set a packet duration when muxing MP4 video: without it the last frame gets a
  zero duration and is dropped by players (found by the round-trip test).
- The audio filter graph returns `Eof` once `atrim` has passed the end of the range:
  that is the normal end, not an error.
- FFmpeg's `avg_frame_rate` is wrong for short files (3600/119 for 4 s at 30 fps): probe
  uses the nominal `r_frame_rate` first.
- `atempo` (pitch-keeping speed change) shifts sounds by up to ~15 ms without drifting:
  sync tests use 20 ms tolerance with speed changes, 10 ms without.
- Export speed on the owner's laptop varies 2–3× between runs (power state). Measured
  per 1080p frame: YUV→RGBA 6 ms, RGBA→YUV 7 ms (swscale, single-threaded), CPU
  composite 5 ms (one-to-one) to 25 ms (scaled sources), decode 1 ms, QSV encode 1 ms.
  Phase 3a fixed it: YUV end to end + GPU compositing (see docs/rendering.md).
- `wgpu` 30 API notes: `Instance::new(InstanceDescriptor::new_without_display_handle())`
  (by value), `request_adapter`/`request_device` return `Result`,
  `PipelineLayoutDescriptor { bind_group_layouts: &[Some(&layout)], immediate_size }`,
  `RenderPassDescriptor` and `RenderPipelineDescriptor` need `multiview_mask: None`,
  `get_mapped_range()` returns a `Result`, `PollType::wait_indefinitely()`.
- GPU tests: `KADR_GPU_ADAPTER=software` forces WARP / lavapipe, `KADR_REQUIRE_GPU=1`
  turns "no GPU" into a failure. To check GPU vs CPU on a real export, compare both
  files with `ffmpeg -i a.mp4 -i b.mp4 -lavfi psnr -f null -` (47 dB today).
- GPU-vs-CPU test pictures: the GPU interpolates the half-size colour planes, the CPU
  reuses each 2 × 2 block, so a sharp edge between two very different colours (red
  and green, about 130 apart in V) puts 13 % of V values more than 10 apart with no
  bug at all. Make test edges sharp in brightness (compared at full resolution) and
  milder in colour, rather than loosening the shared thresholds.
- On the desktop PC (RTX 4060), WARP (`KADR_GPU_ADAPTER=software`) crashes with an
  access violation (`0xC0000005`) in about half the parallel runs of
  `gpu_matches_cpu`, `main`'s tests alone included; `--test-threads=1` passes, the
  RTX 4060 never crashes, and CI's WARP does not. Treat such a crash there as the
  machine's, after a single-threaded rerun.
- Inter 4's variable font declares the family **"Inter Variable"**, not "Inter": a
  request for "Inter" silently fell back to Segoe UI until `text.rs` redirected it.
  `TextRasterizer::has_font_family` and a test now guard it. Look at rendered text
  (export a frame, extract it with ffmpeg) rather than trusting pixel counts alone.
- For visual checks, use detailed pictures (FFmpeg's `mandelbrot` source): blurring the
  flat colour bars of a test pattern only shows at their borders.
- Clippy 1.100 wants `as_chunks::<N>()` instead of `chunks_exact(N)` for constant sizes,
  and flags enum variants much larger than the others (box them).
- MCP over stdio: **never print to stdout** in `kadr-mcp` except protocol messages.
  The test `kadr-mcp/tests/stdio_binary.rs` runs the real binary and fails otherwise.
- `rmcp` 3.11: implement `ServerHandler::list_tools` / `call_tool` by hand (tools are
  defined in `kadr-tools`, not with `#[tool]` macros, see ADR 0010). Return tool
  failures as `CallToolResult::error`, never as `ErrorData`: clients hide protocol
  errors from the model. Do not implement `get_tool`: it turns on `rmcp`'s own
  argument validation, whose errors are less clear than ours.
- `schemars` keeps the line breaks of wrapped doc comments and puts the struct's doc
  in the schema: `catalog.rs` removes the root description and joins the lines.
- Tool parameter structs use `#[serde(deny_unknown_fields)]` so a misspelt argument
  is reported to Claude instead of silently ignored.
- Times shown to Claude are rounded to the microsecond; tools snap clip edges given
  within half a frame of another edge (`time_input::snap_to_nearby_clip_edge`), or a
  copied `1.999999` would overlap the previous clip by a fraction of a microsecond.
- `kadr-core` invariants: every command's `apply` must check everything **before** the
  first mutation, and `revert` must restore the project exactly (including the
  `IdAllocator`). The helper `commands::test_support::apply_and_check_revert` checks
  apply → revert → redo for any command: use it in every new command test.
- New validation rules go in `validation.rs` only, never scattered in commands.
- Snapshot tests (`insta`): after an intended change of the file format, regenerate with
  `$env:INSTA_UPDATE='always'; cargo test -p kadr-core` (PowerShell), then **review the
  `.snap` diff** before committing. In CI, a stale snapshot fails the build.
- Clippy's `allow-unwrap-in-tests` only covers `#[test]` functions: integration test
  files with helper functions carry `#![allow(clippy::unwrap_used, clippy::expect_used)]`
  with a comment explaining why.
- **Disk:** each session's build folder (`C:/Users/noasman/kadr-build/<session>`)
  grows to 25–35 GB. Seven of them filled the desktop PC's 1 TB disk on 2026-11-27
  (link errors, then "no space left"). Delete the build folder of a finished
  session, with the owner's OK.
- **Parallel sessions:** each worktree has its own `target/`, so every session
  compiles the whole workspace. Five at once filled 32 GB of memory and made
  everything 10 to 20 times slower. Run two or three sessions at most, with
  `CARGO_BUILD_JOBS=4`.
- **Disk:** each session's build folder grows to 10–30 GB (debug, release, Tauri).
  On 2026-11-27 five of them filled the desktop PC's disk and builds failed with
  `link.exe` error 1318. Delete a finished session's build folder, with the owner's
  OK; never one that a running session uses.
- **Repository inside OneDrive** (the desktop PC's `Bureau` folder): keep the build
  output out of it, or OneDrive syncs gigabytes and can lock files mid-build. Each
  checkout or worktree gets a machine-local `.cargo/config.toml` with
  `[build] target-dir = "<a folder outside OneDrive>"`, excluded from git through
  `.git/info/exclude` (shared by every worktree); never commit it.
- **Tools installed while the Claude app is open:** sessions inherit the app's old
  `PATH`. Until the app restarts, start build commands with
  `$env:Path = [Environment]::GetEnvironmentVariable('Path','Machine') + ';' + [Environment]::GetEnvironmentVariable('Path','User'); $env:FFMPEG_DIR = [Environment]::GetEnvironmentVariable('FFMPEG_DIR','User')`.
- **Tracking:** synthetic scenes are not enough. A real 1080p video
  (`scripts/generate-tracking-benchmark.ps1`, measured with the `track_video`
  example and `scripts/compare-tracking-with-truth.py`) showed drifts that every
  synthetic scenario missed, while the PSR stayed "confident". Measure changes there.
- Tracking, drift: a high PSR only means the filter found what it learnt. With
  MOSSE's usual learning rate (0.125) the region slid off a real video's object by
  2–4 px/s while every frame stayed "confident". The filter now learns at 0.02 and
  every confident frame is anchored to the selection's appearance (decision 0008):
  re-measure on the benchmark before touching either.
- Tracking, size: MOSSE tried at several scales drifts (it learns the slightly wrong
  size every frame), and DSST on grey levels cannot see a zoom at all. The size comes
  from the optical flow points (Median Flow): see decision 0007 before changing it.
- Tracking, optical flow: on coarse pyramid levels a 15 × 15 window is larger than the
  region and follows the background. The coarsest level is limited by the region's
  size, and the search starts from the previous frame's motion.
- Halving an image by averaging 2 × 2 blocks aliases badly; the pyramid blurs with
  weights 1, 3, 3, 1 first.
- `rustfft`'s inverse transform is not normalised (results are N times larger).
- Tracked blur corrections: `TrackedBlur::region_at` at a sample's exact time takes
  the less sure of that sample and the next one. Code that needs a frame's own
  confidence (the correction merge) reads the sample itself.
- Tool answers end with a "Meanwhile, in the background" text block when an analysis
  finished; tests that parse a tool's JSON details must not expect it (it is left out
  of `get_blur_status` answers).
- `cargo doc` runs with `-D warnings` in CI: public docs must not link to private
  modules (`[crate::optical_flow]`); name the file in backticks instead.
- Long Python patch scripts in a Bash heredoc can fail with "unexpected EOF": write
  the script to the scratchpad and run it from there.
- Since the desktop crate joined the workspace, its procedural macros enable more of
  `syn`, and a debug `bindgen` overflowed the 1 MB stack of Windows build scripts
  (`ffmpeg-sys-next`, `STATUS_STACK_OVERFLOW`). Build scripts are now compiled with
  `opt-level = 3` (`[profile.*.build-override]` in `Cargo.toml`): keep it.
- Tauri: synchronous commands run on the main thread and freeze the window; every
  command is `async`, and heavy work goes to `spawn_blocking` or a worker thread.
  Large data (frames, sound) is answered as `tauri::ipc::Response` bytes, never JSON.
- Tauri's file drop (`dragDropEnabled`, needed to get dropped file paths) disables
  HTML5 drag and drop inside the page on Windows: in-app drags (library → timeline,
  clips) use pointer events.
- `@lucide/svelte` (thousands of files) stalled `npm install` on a slow network; the
  few icons used are copied into `apps/desktop/src/lib/ui/icons.ts`.
- In Git Bash, `tasklist` can hang: use PowerShell's `Get-Process` to see running
  `cargo`/`rustc`. Several sessions building at once slow every build down a lot.
- `gh` sometimes fails with `Post "https://api.github.com/graphql": EOF`: retry; check
  with `gh pr view` whether the action went through before repeating it.
- Merging `origin/main` can put a CHANGELOG entry under the wrong version once a
  phase is tagged: check where each entry lands after a merge.
- Desktop app, interface size: the setting scales `body` with CSS `zoom`. Pointer
  positions and `getBoundingClientRect()` are then zoomed, element sizes and
  `scrollLeft` are not: turn pointer positions into timeline pixels with
  `pointerOffsetX` (`src/lib/ui/pointer.ts`), or seeks land 1.5 times too far at 150 %.
- CSS `repeat(0, …)` is invalid and drops the whole `grid-template-rows`: build the
  track list in code (an empty timeline had a 70 px ruler).
- Checking the running app from a script (DevTools port, faking the file dialog and
  file drops): see docs/desktop-app.md, "Driving the running app from a script".
  When another session's app is running, start yours on other ports **and** with
  its own `WEBVIEW2_USER_DATA_FOLDER`, or WebView2 fails with `0x8007139F`.
- In scripts, `.picture` also matches the library's thumbnails: aim at the preview
  through `svg.surface` (the blur overlay) or `section.preview`.
- Claude's link to the app: tests must never reach a real Kadr open on the
  computer. `KadrMcpServer::new` is file mode only; the relay test and the stdio test
  use their own pipe (`KADR_APP_LINK_ADDRESS`). Git Bash rewrites environment values
  like `\\.\pipe\…`: set that variable from PowerShell.
- `open_project` only reads files but changes which project is open: hosts must not
  run it on a read-only snapshot (`KadrToolbox::only_reads_the_open_project`).
  Computer use may be refused; the script route needs no permission.
- Svelte components that close while a field is focused: WebView2 fires `blur` (and
  `change`, for an open colour picker) on the field **while it is being removed**, and
  by then the component's props may already be `undefined` or another item's. Handlers
  that commit on blur or change must not read props: `ClipInspector` keeps the clip's
  id from its creation (`untrack`) and reads the clip's settings from `editor.view`;
  `NumberField` compares the typed text with the value it had on focus, not with
  `value`. Reproduce such bugs by dispatching the event from a patched
  `Element.prototype.remove` in the page (DevTools port).
- Editable number text must read back as the same number: `Intl` grouping turns 1200
  into "1,200" in English, which parses as 1.2. Use `formatEditableNumber` /
  `parseEditableNumber` (`src/lib/ui/numberText.ts`) for anything typed back.
- Cutting ranges out of a clip (silences, and later filler words or bad takes):
  `RemoveClipParts`, one command and one undo step, never a loop of `SplitClip` /
  `RemoveClip` / `MoveClip` (decision 0051). It only closes up the clip's own track.
- Transitions (phase 8a, decision 0050) live on clip **edges** (`entry_transition`,
  `exit_transition`), never as separate objects: no command has to update them when
  clips move. The clip touching the edge is drawn underneath by
  `kadr-engine/src/export/clip_layers.rs`; its source continues past its cut. Keep
  that file the only place deciding which clips a track shows.
- File format changes: bump `CURRENT_FORMAT_VERSION`, add a `Migration`, keep the
  previous snapshot as a fixture in `crates/kadr-core/tests/fixtures/` (format 1:
  `vertical_interview_format_1.kadr`) with a test that it still opens. A new enum
  variant (effect kind…) needs a version bump too, even with a no-op migration: an
  older Kadr must say "newer version" rather than "damaged file".
- Desktop edit requests must never carry settings the user did not change: the view
  the front end draws can be behind the back end (an earlier change not answered
  yet, or Claude's change). Crop, colour and text requests take `changes` with
  optional fields, applied to the project as it is then. `editor.edit`, `undo` and
  `redo` go one at a time, in order (`inOrder`): Tauri runs commands on a thread
  pool, so two requests sent together can be applied in either order. To reproduce
  such races, wrap `window.fetch` in the page and hold back `apply_edit` requests or
  answers (DevTools port).
- Running the app while another session runs its own: port 1420 is then taken, and
  a `tauri dev` whose Vite fails still starts its window on the **other** session's
  front end. Use other ports and a separate WebView2 folder: a `--config` file
  setting `build.devUrl` to `http://localhost:1430` and `build.beforeDevCommand` to
  `npx vite --port 1430 --strictPort`, with `WEBVIEW2_USER_DATA_FOLDER` set and
  another `--remote-debugging-port`. A running `kadr.exe` cannot be deleted but can
  be renamed, which unblocks a rebuild.
