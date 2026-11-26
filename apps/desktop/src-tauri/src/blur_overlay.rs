//! What the preview draws on top of a clip to blur it (docs/ux.md, "Blurring a
//! face"): where the clip's source picture is on the canvas, and each tracked
//! blur's box at the playhead.
//!
//! Boxes are in the **source video's pixels**, the coordinates tracked blurs are
//! stored in; the front end draws them through `source_to_canvas`, and turns a
//! box the user draws back into source pixels with its inverse.

use kadr_core::effect::{EffectKind, PixelRegion, TrackingConfidence};
use kadr_core::ids::{ClipId, EffectId};
use kadr_core::project::Project;
use kadr_core::time::Timestamp;
use kadr_engine::clip_geometry::clip_source_placement;
use kadr_engine::region_tracking::minimum_selection_side_in_pixels;
use serde::{Deserialize, Serialize};

use crate::command_error::CommandResult;
use crate::editing::BlurShapeChoice;

/// A box in the source video's pixels (origin at the top-left corner).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRegion {
    /// Left edge.
    pub left: f32,
    /// Top edge.
    pub top: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

impl From<PixelRegion> for SourceRegion {
    fn from(region: PixelRegion) -> Self {
        Self {
            left: region.left_in_pixels,
            top: region.top_in_pixels,
            width: region.width_in_pixels,
            height: region.height_in_pixels,
        }
    }
}

impl From<SourceRegion> for PixelRegion {
    fn from(region: SourceRegion) -> Self {
        Self {
            left_in_pixels: region.left,
            top_in_pixels: region.top,
            width_in_pixels: region.width,
            height_in_pixels: region.height,
        }
    }
}

/// How sure the tracker was, for the box's look.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfidenceView {
    /// Clearly found.
    Confident,
    /// Probably found.
    Uncertain,
    /// Lost (hidden, out of the picture).
    Lost,
}

impl From<TrackingConfidence> for ConfidenceView {
    fn from(confidence: TrackingConfidence) -> Self {
        match confidence {
            TrackingConfidence::Confident => Self::Confident,
            TrackingConfidence::Uncertain => Self::Uncertain,
            TrackingConfidence::Lost => Self::Lost,
        }
    }
}

/// One tracked blur at the playhead.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlurBoxView {
    /// The blur.
    pub effect_id: EffectId,
    /// Whether it is drawn.
    pub is_enabled: bool,
    /// Ellipse or rectangle.
    pub shape: BlurShapeChoice,
    /// The tracked box.
    pub region: SourceRegion,
    /// What is hidden: the box grown by the margin.
    pub hidden_area: SourceRegion,
    /// How sure the tracker was here.
    pub confidence: ConfidenceView,
}

/// What the preview needs to draw on top of a clip at one moment.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipOverlay {
    /// The clip.
    pub clip_id: ClipId,
    /// The source picture's width, in pixels.
    pub source_width: u32,
    /// The source picture's height, in pixels.
    pub source_height: u32,
    /// The moment of the file shown, in ticks.
    pub source_time_ticks: u64,
    /// Source pixels to canvas pixels, in SVG's `matrix(a, b, c, d, e, f)` order.
    pub source_to_canvas: [f32; 6],
    /// The visible part of the source: left, top, right, bottom.
    pub visible_source: [f32; 4],
    /// The shortest side a box may have to be followed, in source pixels.
    pub minimum_region_side: f32,
    /// The clip's tracked blurs at that moment.
    pub blur_boxes: Vec<BlurBoxView>,
}

/// The overlay of clip `clip_id` at `timeline_time`, or `None` when the clip
/// shows no source picture then (a title, a sound, outside the clip).
///
/// # Errors
///
/// When the clip does not exist.
pub fn clip_overlay_at(
    project: &Project,
    clip_id: ClipId,
    timeline_time: Timestamp,
) -> CommandResult<Option<ClipOverlay>> {
    let Some(placement) = clip_source_placement(project, clip_id, timeline_time)? else {
        return Ok(None);
    };
    let (_, clip) = project.require_clip(clip_id)?;
    let blur_boxes = clip
        .effects
        .iter()
        .filter_map(|effect| {
            let EffectKind::TrackedBlur(blur) = &effect.kind else {
                return None;
            };
            let (region, confidence) = blur.region_at(placement.source_time)?;
            Some(BlurBoxView {
                effect_id: effect.id,
                is_enabled: effect.is_enabled,
                shape: match blur.shape {
                    kadr_core::effect::BlurShape::Ellipse => BlurShapeChoice::Ellipse,
                    kadr_core::effect::BlurShape::Rectangle => BlurShapeChoice::Rectangle,
                },
                region: region.into(),
                hidden_area: blur.hidden_area_at(placement.source_time)?.into(),
                confidence: confidence.into(),
            })
        })
        .collect();
    Ok(Some(ClipOverlay {
        clip_id,
        source_width: placement.picture_size.width_in_pixels,
        source_height: placement.picture_size.height_in_pixels,
        source_time_ticks: placement.source_time.ticks(),
        source_to_canvas: placement.mapping.matrix,
        visible_source: placement.mapping.visible_source,
        minimum_region_side: minimum_selection_side_in_pixels(
            placement.picture_size.width_in_pixels,
        ),
        blur_boxes,
    }))
}
