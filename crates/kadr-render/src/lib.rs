//! # kadr-render — composing the image of a timeline
//!
//! Given the pictures visible at one moment and how each is placed, this crate
//! produces the finished frame: it stacks the layers and applies their position,
//! scale, rotation, crop and opacity, then hands the encoder YUV 4:2:0 planes in
//! BT.709.
//!
//! - [`create_renderer`] picks the [`GpuRenderer`] (wgpu) when a GPU works, the
//!   [`CpuRenderer`] otherwise; both implement [`FrameRenderer`].
//! - [`LayerPlacement::of_clip_at`] reads a clip's (possibly animated)
//!   placement at a given moment.
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

pub mod cpu_compositor;
pub mod gpu;
pub mod layers;
pub mod renderer;

pub use cpu_compositor::{CpuRenderer, composite_layers_to_rgba};
pub use gpu::GpuRenderer;
pub use layers::{CanvasDescription, LayerPlacement, VisualLayer};
pub use renderer::{FrameRenderer, RenderError, RendererPreference, create_renderer};
