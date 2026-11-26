//! Pictures of the timeline for an interface's preview.
//!
//! The preview uses **the same frame composition as the export** (same decoders,
//! same renderer, same effects), so what the user sees is what the export writes.
//! The finished frame is then shrunk to the size the interface asks for.
//!
//! A [`PreviewRenderer`] keeps decoders open between frames, so playing forward
//! only decodes each frame once, and jumping around (scrubbing) reopens only what
//! it must. The project is handed over on every call: after an edit, the next
//! frame simply uses the new version.
//!
//! Like video sources, a preview renderer stays on the thread that created it: an
//! interface runs it on a dedicated preview thread.

use kadr_core::media_asset::PictureSize;
use kadr_core::project::Project;
use kadr_core::time::Timestamp;
use kadr_media::{MediaBackend, MediaError, YuvColorSpace, YuvPicture};
use kadr_render::{RenderError, RendererPreference, create_renderer};

use crate::export::frame_composer::FrameComposer;
use crate::picture_scaling::shrink_yuv_picture_to_fit;

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
        let frame =
            self.composer
                .compose_frame_at::<PreviewError>(project, backend, timeline_time)?;
        let picture = frame.to_yuv420(YuvColorSpace::BT709_LIMITED);
        Ok(shrink_yuv_picture_to_fit(&picture, maximum_size))
    }
}
