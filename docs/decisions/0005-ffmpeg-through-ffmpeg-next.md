# 0005. FFmpeg through `ffmpeg-next`, behind backend traits

- Status: Accepted
- Date: 2026-11-26

## Context

Kadr must read almost any video, audio or image file and write H.264 / AAC MP4 files,
using hardware encoders when available. No pure-Rust library covers this. On Android
the system's `MediaCodec` will be used instead (phase 9).

## Options considered

1. **Run the `ffmpeg` command line** and pipe raw frames. Simple to build, but slow
   frame-accurate seeking, fragile process handling, and impossible on Android.
2. **`ffmpeg-next`** (safe Rust wrapper of the FFmpeg libraries, WTFPL). Mature, in
   maintenance mode, supports FFmpeg 3.4 to 9.x (version 9.1).
3. **`ffmpeg-the-third`** (a fork of the same lineage) or **`rsmpeg`** (thinner, more
   `unsafe`-oriented). Both viable; no clear advantage for our needs.

## Decision

Option 2, `ffmpeg-next` 9.1, behind the `MediaBackend` traits of `kadr-media`, in an
optional `ffmpeg` feature (on by default).

- Required FFmpeg: **7 or newer** (the code uses the channel-layout API introduced in
  FFmpeg 7). CI uses the BtbN FFmpeg 9.0 shared builds on Windows and Linux and
  Homebrew's FFmpeg on macOS.
- Frame accuracy: seek to the keyframe before the target, decode forward, keep one
  decoded frame ahead to know when the current frame stops being on screen.
- Sound: one filter graph per segment (`atrim`, `asetpts`, `atempo`, `aresample`,
  `aformat`), result padded or cut to the exact clip length.
- Encoders tried in order: `h264_nvenc`, `h264_qsv`, `h264_amf`, `h264_videotoolbox`,
  `libx264`, `libopenh264`. Missing hardware simply makes an encoder fail to open.
- No `unsafe` code in Kadr: everything goes through the safe wrapper.

## Consequences

- Hardware **encoding** works now (Quick Sync on the owner's Intel Arc laptop).
- Hardware **decoding** is postponed to phase 3: `ffmpeg-next` does not wrap hardware
  frame contexts (it would need `unsafe` FFI), and its benefit only appears when frames
  stay on the GPU for GPU compositing.
- `ffmpeg-next`'s scaler is not `Send`, so video sources and export sinks stay on the
  thread that created them.
- Colour conversions (YUV ↔ RGBA) run in swscale on one core and are the main export
  cost today; phase 3 moves them to the GPU.
- Developers need FFmpeg development files and LLVM (`libclang`) to build; the README
  and `scripts/setup-windows.ps1` cover it.
