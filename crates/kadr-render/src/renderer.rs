//! The interface every renderer offers, and the choice between GPU and CPU.

use kadr_media::{VideoFrame, YuvColorSpace};

use crate::cpu_compositor::CpuRenderer;
use crate::gpu::GpuRenderer;
use crate::layers::{CanvasDescription, LayerGeometry, VisualLayer};

/// Why a frame could not be rendered.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RenderError {
    /// No usable GPU was found (only reported when the GPU is required).
    #[error("no usable graphics adapter: {reason}")]
    NoGraphicsAdapter {
        /// What went wrong.
        reason: String,
    },
    /// The GPU failed while rendering.
    #[error("the GPU failed to render a frame: {reason}")]
    GpuFailure {
        /// What went wrong.
        reason: String,
    },
}

/// Draws the layers visible at one moment into a finished frame.
///
/// Every renderer returns YUV 4:2:0 in BT.709 limited range, the format the
/// encoder takes, so no conversion is left for later.
pub trait FrameRenderer {
    /// What does the rendering, for reports: for example `"GPU: Intel(R) Arc(TM) Graphics (Vulkan)"`.
    fn description(&self) -> String;

    /// Draws `layers` in order (the first one at the bottom) over the canvas background.
    ///
    /// # Errors
    ///
    /// Fails when the GPU fails.
    fn render_frame(
        &mut self,
        canvas: &CanvasDescription,
        layers: &[VisualLayer<'_>],
    ) -> Result<VideoFrame, RenderError>;
}

/// Which renderer to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RendererPreference {
    /// The GPU if one works, otherwise the CPU.
    #[default]
    GpuIfAvailable,
    /// Only the GPU: fail if there is none.
    GpuOnly,
    /// Only the CPU (slower; the reference implementation).
    CpuOnly,
}

/// Creates the renderer matching `preference`.
///
/// # Errors
///
/// Only with [`RendererPreference::GpuOnly`], when no GPU can be used.
pub fn create_renderer(
    preference: RendererPreference,
) -> Result<Box<dyn FrameRenderer>, RenderError> {
    match preference {
        RendererPreference::CpuOnly => Ok(Box::new(CpuRenderer)),
        RendererPreference::GpuOnly => Ok(Box::new(GpuRenderer::new()?)),
        RendererPreference::GpuIfAvailable => match GpuRenderer::new() {
            Ok(gpu_renderer) => Ok(Box::new(gpu_renderer)),
            Err(_) => Ok(Box::new(CpuRenderer)),
        },
    }
}

/// The most common case, handled without drawing anything: a single opaque
/// video already at the canvas size, already in the output colour space. The
/// frame is then the source picture itself.
pub(crate) fn pass_through_frame(
    canvas: &CanvasDescription,
    layers: &[VisualLayer<'_>],
) -> Option<VideoFrame> {
    let [layer] = layers else {
        return None;
    };
    is_drawn_as_it_is(canvas, layers).then(|| layer.picture.clone())
}

/// Whether rendering `layers` on `canvas` draws nothing and hands over the one
/// picture as it is (a single opaque video at the canvas size, in the output
/// colour space, colours unchanged): the cheapest frame there is.
pub fn is_drawn_as_it_is(canvas: &CanvasDescription, layers: &[VisualLayer<'_>]) -> bool {
    let [layer] = layers else {
        return false;
    };
    let VideoFrame::Yuv420(picture) = layer.picture else {
        return false;
    };
    if picture.color_space() != YuvColorSpace::BT709_LIMITED
        || layer.placement.clamped_opacity() < 1.0
        || layer.changes_colors()
    {
        return false;
    }
    LayerGeometry::new(
        canvas.size,
        picture.width(),
        picture.height(),
        &layer.placement,
    )
    .is_some_and(|geometry| {
        geometry.maps_pixels_one_to_one(
            canvas.size,
            picture.width(),
            picture.height(),
            &layer.placement,
        )
    })
}
