# 0042. MediaCodec through the NDK media API, with the `ndk` crate

- Status: Accepted
- Date: 2026-11-26

## Context

On Android, Kadr cannot use FFmpeg (decision 0040) and must use the phone's own
codecs, which Android exposes through `MediaCodec` (decoders and encoders, hardware
ones included), `MediaExtractor` (reading MP4, MKV, WebM, 3GP…) and `MediaMuxer`
(writing MP4). Kadr's engine is Rust and runs in the app's process, so it should call
them without going through Java.

The `MediaBackend` traits (decision 0005) must keep the same guarantees as on
desktop: the exact frame at any time, sound of the exact requested length at any
speed with its pitch kept, H.264 + AAC MP4 files in BT.709.

## Options considered

1. **Java `MediaCodec` through JNI** (`jni` crate). The most complete API (including
   `Image` planes), but every frame crosses JNI, the engine needs a Java VM and an
   `Activity` context, and tests cannot run as plain programs in `adb shell`.
2. **The NDK media API (`libmediandk`) through the `ndk` crate** (rust-mobile, MIT or
   Apache-2.0). C functions callable from any thread, no Java needed, test programs
   run in `adb shell`. The crate wraps `AMediaCodec` and `AMediaFormat` safely, but
   not `AMediaExtractor` nor `AMediaMuxer`, which need a little `unsafe` FFI (through
   `ndk-sys`, the crate's raw bindings).
3. **FFmpeg built for Android, with its `mediacodec` hardware wrappers.** One backend
   for every platform, but FFmpeg must be cross-built for each ABI and shipped in the
   APK (tens of MB), and its MediaCodec support goes through JNI anyway.

## Decision

Option 2, in `kadr-media`'s **`mediacodec` feature** (module `src/mediacodec/`):

- `ndk` 0.9 with its `media` and `api-level-28` features (decision 0041), default
  features off; `ndk-sys` 0.6 with `media`. Both are Android-only dependencies
  (`[target.'cfg(target_os = "android")'.dependencies]`).
- **`unsafe` stays in two small files**, `extractor.rs` and `muxer.rs`, which opt out
  of the workspace's `unsafe_code = "deny"` with `#![allow(unsafe_code)]` and a module
  comment. Each `unsafe` block has a `// SAFETY:` comment. They wrap the C objects in
  types that own them (deleted once, in `Drop`) and keep the file descriptor they read
  or write open as long as they live. Everything else uses the `ndk` crate's safe API.
- Codecs run in **byte-buffer mode** (no `Surface`): decoded pictures are copied out of
  the decoder's buffer in its own layout, described by the format's `image-data`
  (`MediaImage2`) entry, or by `color-format`, `stride` and `slice-height` for the
  planar and semi-planar layouts. Encoders are asked for "flexible YUV 4:2:0", which
  every Android encoder must accept, and describe their layout back the same way.
- What FFmpeg's filters do on desktop is written in Rust and tested on the
  development machine: down-mixing to stereo, **windowed-sinc resampling**, and
  pitch-keeping speed changes with **WSOLA** (classic DSP algorithms, small, no
  dependency). Still pictures (PNG, JPEG, WebP) are decoded by the `image` crate's
  pure-Rust decoders, since `MediaExtractor` reads no pictures.
- `VideoEncoderPreference::HardwareFirst` takes the encoder Android prefers for
  H.264 (the hardware one on phones), then Android's software encoders;
  `SoftwareOnly` takes only the software ones (`c2.android.avc.encoder`, then
  `OMX.google.h264.encoder`).

## Consequences

- No Java and no JNI in the engine: the Android app will only have to load the Rust
  library. Test programs run in `adb shell` on a phone or the emulator
  (`scripts/run-android-tests.ps1`).
- Copying pictures out of the decoder costs one memory copy per decoded frame
  (about 1 ms for 1080p). Zero-copy decoding into a GPU texture (`AImageReader` +
  `AHardwareBuffer` + Vulkan import) is possible later, like option 4 of decision 0006.
- A decoder whose layout is neither described by `image-data` nor a standard planar
  or semi-planar format (some old Qualcomm tiled formats) is refused with a clear
  error naming its colour format. Reading through `AImageReader` would cover it.
- The `ndk` crate panics in a few "impossible" cases (a null buffer for an index
  Android just returned, a failure while deleting a codec). Kadr does not catch them.
- `EncodingSpeed` changes nothing on Android yet: phone encoders have no portable
  speed / quality setting.
