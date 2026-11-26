//! Effects applied to a clip: colour adjustment and tracked blur.
//!
//! Effects only *describe* what to do. Rendering them is the job of
//! `kadr-render`, and computing the tracked regions is the job of `kadr-vision`.

use serde::{Deserialize, Serialize};

use crate::color::Color;
use crate::ids::EffectId;
use crate::time::Timestamp;

/// An effect attached to a clip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    /// Identifier inside the project.
    pub id: EffectId,
    /// A disabled effect is kept but not rendered.
    pub is_enabled: bool,
    /// What the effect does.
    pub kind: EffectKind,
}

/// The different effects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EffectKind {
    /// Exposure, contrast, saturation and temperature.
    ColorAdjustment(ColorAdjustment),
    /// Hides a moving region (usually a face) by following it through the clip.
    TrackedBlur(TrackedBlur),
}

impl EffectKind {
    /// A short name for messages, for example `"tracked blur"`.
    pub fn display_name(&self) -> &'static str {
        match self {
            EffectKind::ColorAdjustment(_) => "colour adjustment",
            EffectKind::TrackedBlur(_) => "tracked blur",
        }
    }

    /// Whether the effect needs pictures (it cannot apply to sound or text alone).
    pub fn needs_picture(&self) -> bool {
        match self {
            EffectKind::ColorAdjustment(_) | EffectKind::TrackedBlur(_) => true,
        }
    }
}

/// Basic colour correction. [`ColorAdjustment::NEUTRAL`] changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ColorAdjustment {
    /// Brightness change in photographic stops: +1 doubles the light, −1 halves it.
    pub exposure_in_stops: f32,
    /// Contrast multiplier: 1 unchanged, 0 flat grey, 2 strong.
    pub contrast_multiplier: f32,
    /// Saturation multiplier: 1 unchanged, 0 black and white, 2 vivid.
    pub saturation_multiplier: f32,
    /// White balance shift: negative is cooler (bluer), positive warmer (more orange).
    /// From −1 to 1.
    pub temperature_shift: f32,
}

impl ColorAdjustment {
    /// No change at all.
    pub const NEUTRAL: Self = Self {
        exposure_in_stops: 0.0,
        contrast_multiplier: 1.0,
        saturation_multiplier: 1.0,
        temperature_shift: 0.0,
    };
}

/// Hides a region that moves, such as a face, by following it frame after frame.
///
/// The region is chosen by the user (or by Claude looking at a frame) and then
/// followed by the classic tracker of `kadr-vision`. The result is a list of
/// [`RegionSample`]s indexed by **source time**, so the blur stays on the face
/// even if the clip is trimmed, split or moved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackedBlur {
    /// How the region is hidden.
    pub style: BlurStyle,
    /// Shape of the hidden area.
    pub shape: BlurShape,
    /// Extra space around the tracked region, as a percentage of its size.
    /// Better to hide too much than to reveal a face.
    pub margin_in_percent: f32,
    /// Width of the soft transition at the edge of the hidden area.
    pub edge_softness_in_pixels: f32,
    /// Where the region is over time, sorted by source time (empty until tracked).
    pub region_samples: Vec<RegionSample>,
}

impl TrackedBlur {
    /// A gaussian-blurred ellipse with a 20 % margin, before any tracking.
    pub fn with_default_style() -> Self {
        Self {
            style: BlurStyle::GaussianBlur {
                radius_in_pixels: 30.0,
            },
            shape: BlurShape::Ellipse,
            margin_in_percent: 20.0,
            edge_softness_in_pixels: 8.0,
            region_samples: Vec::new(),
        }
    }
}

/// How a region is hidden.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BlurStyle {
    /// Soft blur.
    GaussianBlur {
        /// Blur strength.
        radius_in_pixels: f32,
    },
    /// Big square pixels.
    Pixelate {
        /// Size of each square.
        block_size_in_pixels: u32,
    },
    /// A plain colour over the region.
    SolidFill {
        /// The colour of the mask.
        color: Color,
    },
}

/// Shape of the hidden area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlurShape {
    /// Follows the tracked box exactly.
    Rectangle,
    /// An ellipse inside the tracked box, closer to the shape of a face.
    Ellipse,
}

/// A rectangle in the pixels of the **source** picture. Origin at the top-left
/// corner, x to the right, y downwards. Fractions allow smooth sub-pixel motion.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PixelRegion {
    /// Distance from the left edge of the picture.
    pub left_in_pixels: f32,
    /// Distance from the top edge of the picture.
    pub top_in_pixels: f32,
    /// Width of the region.
    pub width_in_pixels: f32,
    /// Height of the region.
    pub height_in_pixels: f32,
}

impl PixelRegion {
    /// Whether every number is finite and the size is positive.
    pub fn is_well_formed(&self) -> bool {
        let all_finite = [
            self.left_in_pixels,
            self.top_in_pixels,
            self.width_in_pixels,
            self.height_in_pixels,
        ]
        .iter()
        .all(|value| value.is_finite());
        all_finite && self.width_in_pixels > 0.0 && self.height_in_pixels > 0.0
    }
}

/// How sure the tracker was about a region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackingConfidence {
    /// The region was clearly found.
    Confident,
    /// The region was probably found; the margin is widened and the range is
    /// highlighted on the timeline for the user to check.
    Uncertain,
    /// The region was lost (hidden or out of frame); the user should check it.
    Lost,
}

/// Who decided where a region is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionSampleOrigin {
    /// The region the tracking started from.
    UserSelection,
    /// Computed by the tracker.
    Tracker,
    /// Moved or resized by hand afterwards.
    ManualCorrection,
}

/// Where the tracked region is at one moment of the source file.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RegionSample {
    /// The moment, in the source file's time.
    pub source_time: Timestamp,
    /// The region at that moment.
    pub region: PixelRegion,
    /// How sure the tracker was.
    pub confidence: TrackingConfidence,
    /// Who placed it.
    pub origin: RegionSampleOrigin,
}
