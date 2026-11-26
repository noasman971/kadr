# 0040. The media backend is chosen with Cargo features

- Status: Accepted
- Date: 2026-11-26

## Context

`kadr-media` hides FFmpeg behind the `MediaBackend` traits (decision 0005), and its
`ffmpeg` feature was already optional. But `kadr-engine` turned that feature on
unconditionally, so nothing that depends on the engine could be built without FFmpeg.

Android cannot use that backend:

- `ffmpeg-sys-next` needs FFmpeg's headers and libraries, built for Android, and
  `libclang` to generate bindings. Nobody ships those, and building FFmpeg for four
  Android architectures would be a project of its own.
- An FFmpeg build bundled in the APK would add tens of megabytes and still not use
  the phone's hardware codecs, which Android only exposes through `MediaCodec`.

So the Android build must leave FFmpeg out and use a `MediaCodec` backend (phase 9),
while desktop builds stay exactly as they are.

## Options considered

1. **Cargo features, FFmpeg on by default.** `kadr-media` and `kadr-engine` each get
   an `ffmpeg` feature (default) and, with the MediaCodec backend, a `mediacodec`
   feature. Android builds pass `--no-default-features --features mediacodec`.
   Explicit, testable on the host (a build without backend, the MediaCodec helpers),
   and the desktop commands do not change.
2. **Target-specific dependencies.** `kadr-engine` enables `kadr-media/ffmpeg` under
   `[target.'cfg(not(target_os = "android"))'.dependencies]` and
   `kadr-media/mediacodec` under the Android one. No flags at all, but the choice is
   hidden in `Cargo.toml`, a desktop build without FFmpeg (or Android with it) becomes
   impossible, and the MediaCodec code could not be compiled on the host for its tests.
3. **Both backends compiled in, chosen at run time.** Impossible on Android, where
   FFmpeg does not build.
4. **One crate per backend** (`kadr-media-ffmpeg`, `kadr-media-mediacodec`). A clean
   boundary, but two more crates for one module each, and the engine would still need
   a switch to pick one.

## Decision

Option 1.

- `kadr-engine`: `default = ["ffmpeg"]`, `ffmpeg = ["kadr-media/ffmpeg"]`, then
  `mediacodec = ["kadr-media/mediacodec"]` with the MediaCodec backend.
- `kadr_engine::create_platform_media_backend()` returns the backend the build was
  made with, so interfaces never name a backend. With no backend compiled in, it
  fails at once with a message saying which feature to enable.
- The engine's **tests always use FFmpeg** (a dev-dependency on `kadr-media` with
  `ffmpeg`): they decode and encode real files on the development machine. Android
  builds check the engine library alone (`--lib`).
- `kadr-cli` stays a desktop tool and keeps depending on FFmpeg directly.

## Consequences

- Desktop: same commands, same result (`cargo build --workspace`).
- Android: `cargo ndk … build -p kadr-engine --no-default-features --lib`
  (see [docs/android.md](../android.md)); a non-required CI job checks it.
- Code in `kadr-engine/src` that names `FfmpegBackend` must sit behind
  `#[cfg(feature = "ffmpeg")]`, or the Android build breaks. The CI job catches it.
- A build with default features for an Android target fails inside
  `ffmpeg-sys-next`'s build script, before Kadr's code: the error does not say "turn
  off the default features". `docs/android.md` says it instead.
