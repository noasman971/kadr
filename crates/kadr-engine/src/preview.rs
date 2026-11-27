//! Pictures of the timeline for an interface's preview.
//!
//! The preview uses **the same frame composition as the export** (same decoders,
//! same renderer, same effects), so what the user sees is what the export writes.
//! It composes on the project's canvas, halved as long as the half still covers
//! the size the interface asks for (a 4K project shown 1280 pixels wide is
//! composed at 1920 × 1080: a quarter of the work), then shrinks the frame to
//! that size.
//!
//! A [`PreviewRenderer`] keeps decoders open between frames, so playing forward
//! only decodes each frame once, and jumping around (scrubbing) reopens only what
//! it must. The project is handed over on every call: after an edit, the next
//! frame simply uses the new version.
//!
//! Like video sources, a preview renderer stays on the thread that created it: an
//! interface runs it on a dedicated preview thread.
//!
//! Given [`ReadyProxies`] ([`PreviewRenderer::use_proxies`]), the preview shows
//! heavy videos from their proxies, smaller and quicker to decode (decision
//! record 0055); the export never does.

use std::sync::Arc;

use kadr_core::media_asset::PictureSize;
use kadr_core::project::Project;
use kadr_core::time::Timestamp;
use kadr_media::{MediaBackend, MediaError, YuvColorSpace, YuvPicture};
use kadr_render::{RenderError, RendererPreference, create_renderer};

use crate::export::frame_composer::FrameComposer;
use crate::picture_scaling::{shrink_yuv_picture_to_fit, size_fitting_inside};
use crate::proxies::ReadyProxies;

/// Why a preview frame could not be drawn.
#[derive(Debug, thiserror::Error)]
pub enum PreviewError {
    /// A source file could not be read at that point (missing, damaged…).
    #[error(transparent)]
    Media(#[from] MediaError),
    /// The renderer failed.
    #[error(transparent)]
    Render(#[from] RenderError),
}

/// Draws preview frames of a project, keeping decoders open between frames.
pub struct PreviewRenderer {
    composer: FrameComposer,
}

impl PreviewRenderer {
    /// A preview renderer using the GPU or the CPU as `preference` says.
    ///
    /// # Errors
    ///
    /// Only with [`RendererPreference::GpuOnly`], when no GPU can be used.
    pub fn new(preference: RendererPreference) -> Result<Self, RenderError> {
        Ok(Self {
            composer: FrameComposer::new(create_renderer(preference)?),
        })
    }

    /// Shows videos listed in `proxies` from their proxies from the next frame on.
    pub fn use_proxies(&mut self, proxies: Arc<ReadyProxies>) {
        self.composer.use_proxies(proxies);
    }

    /// Closes every open decoder; the next frame reopens what it needs. Call it
    /// before deleting proxy files, which cannot be deleted while open.
    pub fn close_open_files(&mut self) {
        self.composer.close_open_sources();
    }

    /// What renders the frames, for example `"GPU: Intel(R) Arc(TM) Graphics (Vulkan)"`.
    pub fn renderer_description(&self) -> String {
        self.composer.renderer_description()
    }

    /// The picture of `project`'s timeline at `timeline_time`, shrunk to fit inside
    /// `maximum_size` (never enlarged), in BT.709 limited range like exports.
    ///
    /// # Errors
    ///
    /// See [`PreviewError`].
    pub fn render_frame(
        &mut self,
        project: &Project,
        backend: &dyn MediaBackend,
        timeline_time: Timestamp,
        maximum_size: PictureSize,
    ) -> Result<YuvPicture, PreviewError> {
        let drawn_size = composition_size_for(project.settings().canvas_size, maximum_size);
        let frame = self.composer.compose_frame_at_size::<PreviewError>(
            project,
            backend,
            timeline_time,
            drawn_size,
        )?;
        let picture = frame.to_yuv420(YuvColorSpace::BT709_LIMITED);
        Ok(shrink_yuv_picture_to_fit(&picture, maximum_size))
    }
}

/// The size a preview frame is composed at: `canvas_size`, halved as long as
/// the half (with even sides) still covers the picture shown inside
/// `maximum_size`.
pub fn composition_size_for(canvas_size: PictureSize, maximum_size: PictureSize) -> PictureSize {
    let shown = size_fitting_inside(
        canvas_size.width_in_pixels,
        canvas_size.height_in_pixels,
        maximum_size,
    );
    let mut drawn = canvas_size;
    loop {
        let can_halve =
            drawn.width_in_pixels.is_multiple_of(4) && drawn.height_in_pixels.is_multiple_of(4);
        let half = PictureSize {
            width_in_pixels: drawn.width_in_pixels / 2,
            height_in_pixels: drawn.height_in_pixels / 2,
        };
        let half_covers_shown = half.width_in_pixels >= shown.width_in_pixels
            && half.height_in_pixels >= shown.height_in_pixels;
        if !can_halve || !half_covers_shown {
            return drawn;
        }
        drawn = half;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(width_in_pixels: u32, height_in_pixels: u32) -> PictureSize {
        PictureSize {
            width_in_pixels,
            height_in_pixels,
        }
    }

    #[test]
    fn big_canvases_are_composed_at_the_smallest_half_covering_the_preview() {
        assert_eq!(
            composition_size_for(size(3840, 2160), size(1280, 720)),
            size(1920, 1080)
        );
        assert_eq!(
            composition_size_for(size(3840, 2160), size(640, 360)),
            size(960, 540)
        );
        assert_eq!(
            composition_size_for(size(3840, 2160), size(1920, 1080)),
            size(1920, 1080)
        );
        assert_eq!(
            composition_size_for(size(3840, 2160), size(2000, 1200)),
            size(3840, 2160)
        );
        assert_eq!(
            composition_size_for(size(1080, 1920), size(800, 450)),
            size(270, 480)
        );
    }

    #[test]
    fn canvases_close_to_the_preview_are_composed_at_their_size() {
        assert_eq!(
            composition_size_for(size(1920, 1080), size(1280, 720)),
            size(1920, 1080)
        );
        assert_eq!(
            composition_size_for(size(1366, 768), size(320, 180)),
            size(1366, 768)
        );
    }
}
