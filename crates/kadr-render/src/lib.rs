//! # kadr-render — composing the image of a timeline
//!
//! Given a project and a time on the timeline, this crate produces the final
//! image: it stacks the visible clips, applies their transforms (position, scale,
//! rotation, crop), opacity, text, transitions and blurs.
//!
//! ## Rules for this crate
//!
//! - Rendering runs on the GPU through `wgpu` (Vulkan, DirectX 12, Metal and
//!   OpenGL ES for Android). Shaders are written in WGSL.
//! - The same renderer is used for the live preview and for the export, so what
//!   the user sees is what they get.
//!
//! A simple CPU version arrives in phase 2, the GPU version in phase 3.
//! See `docs/architecture.md`.
