# 0006. GPU rendering with `wgpu`, YUV from end to end, CPU as reference

- Status: Accepted
- Date: 2026-11-26

## Context

Phase 2 measured where an export spends its time, per 1080p frame: YUV→RGBA
conversion 6 ms and RGBA→YUV 7 ms (swscale, one core), compositing 5–25 ms, decoding
1 ms, hardware encoding 1 ms. Exports ran at 0.5–1.2× real time. Phase 2 exports were
also BT.601 without colour tags, which shifted HD colours.

## Options considered

1. **Optimise the CPU path** (SIMD, multi-threaded swscale). Some gain, but compositing
   with scaling and rotation stays expensive, and 4K would remain slow.
2. **GPU compositing on RGBA**, keeping swscale conversions. Fixes compositing only.
3. **YUV from end to end, GPU compositing.** Decoders hand over their YUV planes, the
   GPU converts YUV→RGB while sampling, composites, and converts back to YUV planes;
   the encoder takes them directly. Conversions disappear from the CPU entirely.
4. **Zero-copy hardware decode → GPU → hardware encode** (D3D11/Vulkan interop).
   Fastest in theory, but platform-specific, needs `unsafe` FFI, and is far more
   complex.

## Decision

Option 3:

- `VideoFrame` becomes `Rgba | Yuv420`, with an explicit `YuvColorSpace`
  (BT.601/BT.709, limited/full). One set of coefficients serves CPU and GPU.
- `kadr-render` offers `FrameRenderer` with two implementations: `GpuRenderer`
  (`wgpu` 30, WGSL shaders) and `CpuRenderer` (the reference). The GPU is used when
  available; tests check both agree.
- Exports are BT.709 limited range and tagged as such.
- A single video already at canvas size passes through without any drawing.
- `kadr-render` depends on `kadr-media` without its FFmpeg feature, so rendering stays
  portable to Android.

## Consequences

- Measured on the owner's laptop: GPU frames take 10 ms in 1080p and 25 ms in 4K
  (5–18× faster than the CPU renderer); a mixed real export went from 0.6× to 1.6× real
  time, a same-size export to 2.1–2.7× real time.
- GPU and CPU exports differ by a PSNR of 47 dB: invisible.
- CI needs a software GPU (WARP, lavapipe) to test the GPU path.
- Hardware decoding stays postponed: at about 1 ms per 1080p frame, decoding is not the
  bottleneck. Option 4 can be revisited for 4K HEVC sources.
