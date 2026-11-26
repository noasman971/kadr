//! # kadr-render — composing the image of a timeline
//!
//! Given the pictures visible at one moment and how each is placed, this crate
//! produces the final image: it stacks the layers and applies their position,
//! scale, rotation, crop and opacity.
//!
//! - [`composite_layers`] draws a list of [`VisualLayer`]s over a background;
//! - [`LayerPlacement::of_clip_at`] reads a clip's (possibly animated)
//!   placement at a given moment.
//!
//! ## Rules for this crate
//!
//! - The same renderer is used for the live preview and for the export, so what
//!   the user sees is what they get.
//! - Phase 2 renders on the CPU ([`cpu_compositor`]). Phase 3 moves rendering to
//!   the GPU through `wgpu` (Vulkan, DirectX 12, Metal, OpenGL ES for Android)
//!   and adds text, transitions and blurs.
//!
//! See `docs/architecture.md`.

pub mod cpu_compositor;

pub use cpu_compositor::{LayerPlacement, VisualLayer, composite_layers};
