# Changelog

All notable changes to Kadr are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project uses [Semantic Versioning](https://semver.org/). Each validated phase is tagged
(`v0.1.0` for phase 0, `v0.2.0` for phase 1, and so on).

## [Unreleased]

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
- `docs/transitions-and-text-animations.md`, decision record 0030.

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
