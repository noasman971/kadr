# 0041. Android 9 (API level 28) is the oldest supported Android

- Status: Accepted
- Date: 2026-11-26

## Context

Every Android build names a minimum API level: the oldest Android version the app
installs on. A lower level reaches more phones, but every NDK function newer than it
is unavailable (the program would not even load on older phones). Kadr needs:

- the NDK media API (`AMediaExtractor`, `AMediaCodec`, `AMediaMuxer`): API 21. Several
  functions the MediaCodec backend relies on arrive in **API 28**:
  `AMediaCodec_getName` (to report which encoder is used, like the FFmpeg backend
  does), `AMediaCodec_getBufferFormat` and `AMediaFormat_getRect` (the exact picture
  layout of each decoded frame, including the crop rectangle),
  `AMediaExtractor_getSampleSize`;
- `wgpu` on Vulkan (API 24 for Vulkan 1.0; Vulkan 1.1 is required on 64-bit phones
  launched with Android 10 or later) or on OpenGL ES 3 (any recent phone);
- Tauri 2 for the app (phase 9, later), whose default minimum is API 24.

In 2026, phones older than Android 9 (released in August 2018) are a few percent of
those in use, and are too slow for video editing anyway.

## Options considered

1. **API 24 (Android 7)**, Tauri's default. A few more phones, but no
   `AMediaCodec_getBufferFormat` nor `AMediaFormat_getRect`: decoded frames would be
   read with the layout announced once for the whole stream, which some decoders
   change on the way.
2. **API 28 (Android 9).** Everything the backend needs, while still covering almost
   every phone in use.
3. **API 29 or newer.** Nothing more that Kadr needs today.

## Decision

Option 2: **API 28**.

- Builds pass `--platform 28` to `cargo ndk` (CI does too).
- The `ndk` crate is used with its `api-level-28` feature, which unlocks the
  functions above and refuses newer ones at compile time.
- The Android app (Tauri) will set `minSdkVersion 28`.

## Consequences

- Phones on Android 8.1 or older cannot install Kadr.
- Raising the level later is easy (one number in the build commands and the app);
  lowering it would mean replacing the API 28 functions.
- NDK r28 and newer align native libraries on 16 KB pages, which Android 15 phones
  with 16 KB pages require: use a recent NDK (docs/android.md says which).
