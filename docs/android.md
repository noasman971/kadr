# Android

Kadr's engine is written once and runs on desktop and on Android phones. This page
explains how to build the engine for Android, what works there today and what is
left. The Android **app** (Tauri 2, touch interface) comes later in phase 9, after
the desktop app of phase 6.

## What runs on Android

| Crate | On Android |
|---|---|
| `kadr-core` | Builds and runs as is: no platform code |
| `kadr-vision` | Builds and runs as is: plain Rust arithmetic |
| `kadr-render` | `wgpu` picks Vulkan, or OpenGL ES when the phone has no usable Vulkan driver; the CPU renderer is the fallback |
| `kadr-media` | The FFmpeg backend is left out (`ffmpeg` feature off). The MediaCodec backend arrives in phase 9b |
| `kadr-engine` | Builds without FFmpeg (`--no-default-features`); `create_platform_media_backend()` picks the backend |
| `kadr-cli`, `kadr-mcp`, `kadr-tools` | Desktop only |

The oldest supported Android is **Android 9 (API level 28)**; see
[decision 0041](decisions/0041-minimum-android-api-level.md). Media backend features:
[decision 0040](decisions/0040-media-backend-features.md).

## Setting up the toolchain (Windows)

Everything below is free. About 3 GB on disk with Android Studio's SDK.

1. **Android SDK.** Install [Android Studio](https://developer.android.com/studio),
   which puts the SDK in `%LOCALAPPDATA%\Android\Sdk`, or only Google's
   [command-line tools](https://developer.android.com/studio#command-line-tools-only)
   unpacked into `%LOCALAPPDATA%\Android\Sdk\cmdline-tools\latest`.
2. **Android NDK** (C toolchain and system libraries for Android), version r28 or
   newer (16 KB page alignment, required by Android 15 phones). Kadr is tested with
   **r30 (30.0.16248370)**. Either:
   - Android Studio → *Settings* → *Languages & Frameworks* → *Android SDK* →
     *SDK Tools* → *NDK (Side by side)*; or
   - the command-line tools (version 23 replaced `sdkmanager` by `android sdk`):

     ```powershell
     & "$env:LOCALAPPDATA\Android\Sdk\cmdline-tools\latest\bin\android.exe" --sdk="$env:LOCALAPPDATA\Android\Sdk" sdk install ndk/30.0.16248370
     ```

3. **Tell the tools where they are** (user variables, then open a new terminal):

   ```powershell
   [Environment]::SetEnvironmentVariable('ANDROID_HOME', "$env:LOCALAPPDATA\Android\Sdk", 'User')
   [Environment]::SetEnvironmentVariable('ANDROID_NDK_HOME', "$env:LOCALAPPDATA\Android\Sdk\ndk\30.0.16248370", 'User')
   ```

4. **Rust targets and `cargo-ndk`**: phones are `aarch64`; the emulator on a PC is
   `x86_64`.

   ```powershell
   rustup target add aarch64-linux-android x86_64-linux-android
   ```

   ```powershell
   cargo install cargo-ndk --locked
   ```

On Linux and macOS the steps are the same, with the SDK in `~/Android/Sdk` (Linux)
or `~/Library/Android/sdk` (macOS).

## Building

From the repository root:

```powershell
cargo ndk --target arm64-v8a --platform 28 build -p kadr-core -p kadr-vision -p kadr-render -p kadr-engine --no-default-features --lib
```

- `--target arm64-v8a`: phones. Use `x86_64` for the emulator.
- `--platform 28`: the minimum API level (decision 0041).
- `--no-default-features`: leaves FFmpeg out. **Forgetting it fails inside
  `ffmpeg-sys-next`'s build script**, with an error about FFmpeg or `libclang` that
  does not mention Android.
- `--lib`: the engine's tests decode and encode real files with FFmpeg on the
  development machine, so they are not built for Android.

Lint and link the test programs (linking checks that every system library the code
needs exists on Android):

```powershell
cargo ndk --target arm64-v8a --platform 28 clippy -p kadr-core -p kadr-vision -p kadr-render -p kadr-engine --no-default-features --lib -- -D warnings
```

```powershell
cargo ndk --target arm64-v8a --platform 28 test --no-run -p kadr-core -p kadr-vision -p kadr-render
```

CI runs the same three commands in the job "Android build (aarch64-linux-android)",
with the NDK preinstalled on GitHub's Linux runners. The job is **not a required
check** of `main` yet.

## What is left

- **MediaCodec backend** (phase 9b): probing, hardware decoding and encoding, sound,
  behind the same `MediaBackend` traits as FFmpeg.
- **Running the tests on a phone or the emulator** (phase 9b): the test programs build
  and link, but nothing runs them on Android yet.
- **The Android app** (phase 9, after phase 6): Tauri 2, touch timeline, gallery
  import, export to the gallery, permissions. Built as a `cdylib` loaded by the app.
- **Claude on the phone**: a local MCP server cannot be reached from the phone's
  Claude app. The options will be studied and presented to the owner later.
