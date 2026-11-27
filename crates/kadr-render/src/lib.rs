//! # kadr-render — composing the image of a timeline
//!
//! Given the pictures visible at one moment and how each is placed, this crate
//! produces the finished frame: it stacks the layers and applies their position,
//! scale, rotation, crop and opacity, then hands the encoder YUV 4:2:0 planes in
//! BT.709.
//!
//! - [`create_renderer`] picks the [`GpuRenderer`] (wgpu) when a GPU works, the
//!   [`CpuRenderer`] otherwise; both implement [`FrameRenderer`].
//! - [`TextRasterizer`] draws text clips with the bundled Inter font.
//! - [`LayerPlacement::of_clip_at`] reads a clip's (possibly animated)
//!   placement at a given moment.
//! - [`transition_motion`] and [`text_animation_frame`] move layers during
//!   transitions and text animations.
//! - [`ColorLookupTable`] reads `.cube` colour LUTs; a layer applies one after its
//!   colour adjustments ([`LayerColorLut`]).
//! - A single video that already fills the canvas is passed through untouched.
//!
//! ## Rules for this crate
//!
//! - The same renderers serve the live preview and the export, so what the user
//!   sees is what they get.
//! - The CPU renderer is the reference: the GPU renderer must produce the same
//!   picture within rounding (checked by tests).
//! - Colour conversions use the coefficients of `kadr_media::YuvColorSpace`, on
//!   both sides.
//!
//! See `docs/rendering.md`.

pub mod color_adjustment;
pub mod color_lookup_table;
pub mod cpu_compositor;
pub mod gpu;
pub mod layers;
pub mod region_effects;
pub mod renderer;
pub mod text;
pub mod transitions;

pub use color_adjustment::{MAXIMUM_COLOR_ADJUSTMENTS_PER_LAYER, adjust_color};
pub use color_lookup_table::{
    ColorLookupTable, CubeFileError, LayerColorLut, LookupTableShape, MAXIMUM_1D_SIZE,
    MAXIMUM_3D_SIZE, MAXIMUM_COLOR_LUTS_PER_LAYER,
};
pub use cpu_compositor::{CpuRenderer, composite_layers_to_rgba};
pub use gpu::GpuRenderer;
pub use layers::{
    CanvasDescription, LayerPlacement, LayerSizing, SourceToCanvasMapping, VisualLayer,
};
pub use region_effects::{HiddenRegion, hide_regions};
pub use renderer::{FrameRenderer, RenderError, RendererPreference, create_renderer};
pub use text::{BUNDLED_FONT_FAMILY, TextRasterizer};
pub use transitions::{
    LayerMotion, TextReveal, TransitionRole, text_animation_frame, transition_motion,
};
