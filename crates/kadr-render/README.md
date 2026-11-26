# kadr-render

Draws the pictures visible at one moment into the finished frame (YUV 4:2:0, BT.709),
with their position, scale, rotation, crop and opacity. The same code drives the
preview and the export.

- `GpuRenderer` (`wgpu`: Vulkan, DirectX 12, Metal, OpenGL ES) and `CpuRenderer` (the
  reference), both behind `FrameRenderer`; `create_renderer` picks the GPU when one
  works.
- A single video already at canvas size passes through untouched.
- `TextRasterizer` draws text clips with `cosmic-text` and the bundled Inter font
  (`assets/fonts/`, SIL Open Font License 1.1).
- Benchmarks: `cargo bench -p kadr-render`.

- **Depends on:** `kadr-core`, `kadr-media` (without FFmpeg), `wgpu`, `cosmic-text`, `image`, `rayon`.
- **Used by:** `kadr-engine`.
- **Status:** phase 3a (GPU compositing) and 3b (text). Colour, blur and pixelation
  effects (3c) next.

See [docs/rendering.md](../../docs/rendering.md).
