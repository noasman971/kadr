# kadr-render

The compositor: draws the pictures visible at one moment onto the canvas, with their
position, scale, rotation, crop and opacity. The same code drives the preview and the
export.

- **Depends on:** `kadr-core`, `image`, `rayon`.
- **Used by:** `kadr-engine`.
- **Status:** CPU compositor with parallel rows (phase 2). GPU (`wgpu`), text,
  transitions and blurs in phase 3.

See [docs/media-and-export.md](../../docs/media-and-export.md).
