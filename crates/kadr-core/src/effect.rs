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

    /// Where the region is at `source_time`, interpolated between the two
    /// nearest samples, with the confidence of the less sure of the two.
    /// Before the first sample or after the last one, the nearest sample is kept
    /// (better to hide too much than to reveal a face). `None` without samples.
    pub fn region_at(&self, source_time: Timestamp) -> Option<(PixelRegion, TrackingConfidence)> {
        let samples = &self.region_samples;
        let index_of_next = samples.partition_point(|sample| sample.source_time <= source_time);
        let (previous, next) = match (
            index_of_next
                .checked_sub(1)
                .and_then(|index| samples.get(index)),
            samples.get(index_of_next),
        ) {
            (Some(previous), Some(next)) => (previous, next),
            (Some(only), None) | (None, Some(only)) => return Some((only.region, only.confidence)),
            (None, None) => return None,
        };
        let elapsed = source_time.span_since(previous.source_time)?;
        let between = next.source_time.span_since(previous.source_time)?;
        let fraction = elapsed.fraction_of(between) as f32;
        Some((
            previous.region.interpolated_towards(&next.region, fraction),
            previous.confidence.least_sure(next.confidence),
        ))
    }

    /// The area to hide at `source_time`: the tracked region grown by the margin,
    /// and by half as much again when the tracker was not confident.
    pub fn hidden_area_at(&self, source_time: Timestamp) -> Option<PixelRegion> {
        let (region, confidence) = self.region_at(source_time)?;
        let margin_in_percent = match confidence {
            TrackingConfidence::Confident => self.margin_in_percent,
            TrackingConfidence::Uncertain | TrackingConfidence::Lost => {
                self.margin_in_percent * UNSURE_MARGIN_FACTOR
            }
        };
        Some(region.grown_by_percent(margin_in_percent))
    }

    /// The stretches of samples where the tracker was not confident, in time
    /// order: what the timeline highlights and what Claude checks.
    pub fn unsure_ranges(&self) -> Vec<UnsureRange> {
        let mut ranges = Vec::new();
        let mut current: Option<UnsureRange> = None;
        for sample in &self.region_samples {
            if sample.confidence == TrackingConfidence::Confident {
                ranges.extend(current.take());
                continue;
            }
            current = Some(match current {
                Some(range) => UnsureRange {
                    last_source_time: sample.source_time,
                    confidence: range.confidence.least_sure(sample.confidence),
                    ..range
                },
                None => UnsureRange {
                    first_source_time: sample.source_time,
                    last_source_time: sample.source_time,
                    confidence: sample.confidence,
                },
            });
        }
        ranges.extend(current);
        ranges
    }
}

/// A stretch of a tracked blur where the tracker was not confident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsureRange {
    /// Source time of the first unsure sample.
    pub first_source_time: Timestamp,
    /// Source time of the last unsure sample.
    pub last_source_time: Timestamp,
    /// The least sure confidence in the stretch: uncertain or lost.
    pub confidence: TrackingConfidence,
}

/// How much the margin grows where the tracker was not confident.
const UNSURE_MARGIN_FACTOR: f32 = 1.5;

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

    /// The region `fraction` of the way to `target` (0 = this one, 1 = `target`).
    pub fn interpolated_towards(&self, target: &PixelRegion, fraction: f32) -> PixelRegion {
        let mix = |from: f32, to: f32| from + (to - from) * fraction;
        PixelRegion {
            left_in_pixels: mix(self.left_in_pixels, target.left_in_pixels),
            top_in_pixels: mix(self.top_in_pixels, target.top_in_pixels),
            width_in_pixels: mix(self.width_in_pixels, target.width_in_pixels),
            height_in_pixels: mix(self.height_in_pixels, target.height_in_pixels),
        }
    }

    /// The same region, wider and taller by `percent` of its size, around the same centre.
    pub fn grown_by_percent(&self, percent: f32) -> PixelRegion {
        let growth_factor = 1.0 + percent / 100.0;
        let (new_width, new_height) = (
            self.width_in_pixels * growth_factor,
            self.height_in_pixels * growth_factor,
        );
        PixelRegion {
            left_in_pixels: self.left_in_pixels - (new_width - self.width_in_pixels) / 2.0,
            top_in_pixels: self.top_in_pixels - (new_height - self.height_in_pixels) / 2.0,
            width_in_pixels: new_width,
            height_in_pixels: new_height,
        }
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

impl TrackingConfidence {
    /// The less sure of two confidences.
    pub fn least_sure(self, other: TrackingConfidence) -> TrackingConfidence {
        let rank = |confidence| match confidence {
            TrackingConfidence::Confident => 0,
            TrackingConfidence::Uncertain => 1,
            TrackingConfidence::Lost => 2,
        };
        if rank(other) > rank(self) {
            other
        } else {
            self
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn region(left: f32, top: f32, width: f32, height: f32) -> PixelRegion {
        PixelRegion {
            left_in_pixels: left,
            top_in_pixels: top,
            width_in_pixels: width,
            height_in_pixels: height,
        }
    }

    fn assert_regions_close(found: PixelRegion, expected: PixelRegion) {
        let pairs = [
            (found.left_in_pixels, expected.left_in_pixels),
            (found.top_in_pixels, expected.top_in_pixels),
            (found.width_in_pixels, expected.width_in_pixels),
            (found.height_in_pixels, expected.height_in_pixels),
        ];
        assert!(
            pairs.iter().all(|(a, b)| (a - b).abs() < 1e-3),
            "{found:?} vs {expected:?}"
        );
    }

    fn sample(at_second: u64, area: PixelRegion, confidence: TrackingConfidence) -> RegionSample {
        RegionSample {
            source_time: Timestamp::from_whole_seconds(at_second),
            region: area,
            confidence,
            origin: RegionSampleOrigin::Tracker,
        }
    }

    #[test]
    fn unsure_stretches_are_listed_with_their_least_sure_confidence() {
        use TrackingConfidence::{Confident, Lost, Uncertain};
        let area = region(0.0, 0.0, 10.0, 10.0);
        let blur = TrackedBlur {
            region_samples: [Confident, Uncertain, Lost, Uncertain, Confident, Uncertain]
                .into_iter()
                .enumerate()
                .map(|(second, confidence)| sample(second as u64, area, confidence))
                .collect(),
            ..TrackedBlur::with_default_style()
        };
        let at = Timestamp::from_whole_seconds;
        assert_eq!(
            blur.unsure_ranges(),
            vec![
                UnsureRange {
                    first_source_time: at(1),
                    last_source_time: at(3),
                    confidence: Lost,
                },
                UnsureRange {
                    first_source_time: at(5),
                    last_source_time: at(5),
                    confidence: Uncertain,
                },
            ]
        );
        assert!(TrackedBlur::with_default_style().unsure_ranges().is_empty());
    }

    fn blur_with_samples(samples: Vec<RegionSample>) -> TrackedBlur {
        TrackedBlur {
            region_samples: samples,
            ..TrackedBlur::with_default_style()
        }
    }

    #[test]
    fn regions_are_interpolated_between_samples() {
        let blur = blur_with_samples(vec![
            sample(
                1,
                region(100.0, 100.0, 50.0, 50.0),
                TrackingConfidence::Confident,
            ),
            sample(
                3,
                region(200.0, 100.0, 70.0, 50.0),
                TrackingConfidence::Uncertain,
            ),
        ]);
        let (halfway, confidence) = blur.region_at(Timestamp::from_whole_seconds(2)).unwrap();
        assert_eq!(halfway, region(150.0, 100.0, 60.0, 50.0));
        assert_eq!(confidence, TrackingConfidence::Uncertain);
    }

    #[test]
    fn the_nearest_sample_is_kept_outside_the_tracked_range() {
        let first = region(10.0, 10.0, 20.0, 20.0);
        let last = region(90.0, 10.0, 20.0, 20.0);
        let blur = blur_with_samples(vec![
            sample(2, first, TrackingConfidence::Confident),
            sample(4, last, TrackingConfidence::Confident),
        ]);
        assert_eq!(blur.region_at(Timestamp::ZERO).unwrap().0, first);
        assert_eq!(
            blur.region_at(Timestamp::from_whole_seconds(9)).unwrap().0,
            last
        );
        assert_eq!(
            blur.region_at(Timestamp::from_whole_seconds(4)).unwrap().0,
            last
        );
        assert!(
            blur_with_samples(Vec::new())
                .region_at(Timestamp::ZERO)
                .is_none()
        );
    }

    #[test]
    fn the_margin_grows_where_the_tracker_was_unsure() {
        let tracked = region(100.0, 100.0, 100.0, 100.0);
        let confident = blur_with_samples(vec![sample(0, tracked, TrackingConfidence::Confident)]);
        let unsure = blur_with_samples(vec![sample(0, tracked, TrackingConfidence::Lost)]);
        // Default margin 20 %: 120 px wide; unsure: 30 %, 130 px wide. Same centre.
        assert_regions_close(
            confident.hidden_area_at(Timestamp::ZERO).unwrap(),
            region(90.0, 90.0, 120.0, 120.0),
        );
        assert_regions_close(
            unsure.hidden_area_at(Timestamp::ZERO).unwrap(),
            region(85.0, 85.0, 130.0, 130.0),
        );
    }
}
