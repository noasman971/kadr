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
- **Phase 3 (GPU rendering): in progress**, in three pull requests: 3a GPU compositing
  and YUV colour pipeline (merged, PR #3), 3b text with the bundled Inter font (the
  owner chose to bundle it; branch `feature/phase-3b-text`), 3c colour adjustment,
  blur and pixelation effects. Tag `v0.4.0` when all three are merged and validated.

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
- Architecture decisions are recorded in [docs/decisions/](docs/decisions/).

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

End-of-phase checklist: build without warnings, clippy, fmt, tests, `cargo doc`, docs
updated, CHANGELOG updated, summary to the owner, **stop and wait for validation**.

## Architecture in one paragraph

`kadr-core` (data model, commands, undo, validation, `.kadr` file) has no Kadr
dependency; read [docs/data-model.md](docs/data-model.md) before changing it.
`kadr-media` exposes `MediaBackend` traits implemented with FFmpeg (MediaCodec later)
and the `VideoFrame` (YUV or RGBA) and colour-space types; `kadr-render` turns layers
into BT.709 YUV frames on the GPU (`wgpu`) or the CPU (reference); `kadr-vision`
(classic tracking) will do the face blur. `kadr-engine` is the only API interfaces use. `kadr-tools` defines the tools
Claude can call; `kadr-mcp` serves them over stdio. `kadr-cli` is for testing. The
desktop app (Tauri 2) arrives in phase 6. Details: [docs/architecture.md](docs/architecture.md).

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
- Inter 4's variable font declares the family **"Inter Variable"**, not "Inter": a
  request for "Inter" silently fell back to Segoe UI until `text.rs` redirected it.
  `TextRasterizer::has_font_family` and a test now guard it. Look at rendered text
  (export a frame, extract it with ffmpeg) rather than trusting pixel counts alone.
- Clippy 1.100 wants `as_chunks::<N>()` instead of `chunks_exact(N)` for constant sizes,
  and flags enum variants much larger than the others (box them).
- MCP over stdio: **never print to stdout** in `kadr-mcp` except protocol messages.
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
