# kadr-render

Draws the pictures visible at one moment into the finished frame (YUV 4:2:0, BT.709),
with their position, scale, rotation, crop and opacity. The same code drives the
preview and the export.

- `GpuRenderer` (`wgpu`: Vulkan, DirectX 12, Metal, OpenGL ES) and `CpuRenderer` (the
  reference), both behind `FrameRenderer`; `create_renderer` picks the GPU when one
  works.
- A single video already at canvas size passes through untouched.
- Benchmarks: `cargo bench -p kadr-render`.

- **Depends on:** `kadr-core`, `kadr-media` (without FFmpeg), `wgpu`, `image`, `rayon`.
- **Used by:** `kadr-engine`.
- **Status:** phase 3a (GPU compositing). Text (3b), colour, blur and pixelation
  effects (3c) next.

See [docs/rendering.md](../../docs/rendering.md).
