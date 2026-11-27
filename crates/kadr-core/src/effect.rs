//! Effects applied to a clip: colour adjustment, colour LUT, tracked blur and
//! stabilisation.
//!
//! Effects only *describe* what to do. Rendering them is the job of
//! `kadr-render`, reading LUT files the job of `kadr-engine`, and computing the
//! tracked regions and the camera's path the job of `kadr-vision`.
//!
//! Whatever the order of a clip's effects list, they apply in a fixed order:
//! tracked blurs hide their areas on the source picture, then the stabilisation
//! moves the picture inside the layer's window, then the colour adjustments
//! apply (in list order), then the colour LUT.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::color::Color;
use crate::ids::EffectId;
use crate::media_asset::PictureSize;
use crate::time::{TimeSpan, Timestamp};

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
    /// A look from a colour lookup table file (`.cube`), after the colour adjustments.
    ColorLut(ColorLut),
    /// Hides a moving region (usually a face) by following it through the clip.
    TrackedBlur(TrackedBlur),
    /// Steadies a shaky camera: its measured path, smoothed when the clip is drawn.
    Stabilization(Stabilization),
}

impl EffectKind {
    /// A short name for messages, for example `"tracked blur"`.
    pub fn display_name(&self) -> &'static str {
        match self {
            EffectKind::ColorAdjustment(_) => "colour adjustment",
            EffectKind::ColorLut(_) => "colour LUT",
            EffectKind::TrackedBlur(_) => "tracked blur",
            EffectKind::Stabilization(_) => "stabilisation",
        }
    }

    /// Whether the effect needs pictures (it cannot apply to sound or text alone).
    pub fn needs_picture(&self) -> bool {
        match self {
            EffectKind::ColorAdjustment(_)
            | EffectKind::ColorLut(_)
            | EffectKind::TrackedBlur(_)
            | EffectKind::Stabilization(_) => true,
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

/// A look from a colour lookup table (LUT) in the `.cube` format, the format
/// DaVinci Resolve, Premiere and most grading tools export.
///
/// Only the path is stored, never the table: a 33 × 33 × 33 table holds 35 937
/// colours, and every change of the project is kept in the undo history and
/// saved. Reading and checking the file is the engine's job (a moved file must
/// not make the project impossible to open); the rules only check that the
/// path names a `.cube` file and that the strength is between 0 and 1.
///
/// It applies after the clip's colour adjustments, whatever the order of the
/// effects list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColorLut {
    /// Where the `.cube` file is, as a full path.
    pub file_path: PathBuf,
    /// How much of the look is applied: 0 keeps the picture as the colour
    /// adjustments leave it, 1 applies the LUT fully.
    pub strength_fraction: f32,
}

impl ColorLut {
    /// The extension of LUT files, without the dot.
    pub const FILE_EXTENSION: &'static str = "cube";

    /// The LUT of `file_path`, fully applied.
    pub fn at_full_strength(file_path: impl Into<PathBuf>) -> Self {
        Self {
            file_path: file_path.into(),
            strength_fraction: 1.0,
        }
    }
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

/// A stretch of a tracked blur where the tracker was not confident, or of a
/// stabilisation's camera path where the motion could not be measured.
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

/// Steadies a shaky camera.
///
/// The analysis (`kadr-engine`, with the algorithms of `kadr-vision`) measures
/// how the camera moved from one frame to the next and stores the **raw**
/// cumulative path: one [`CameraPathSample`] per analysed frame, indexed by
/// **source time** like the tracked blur's regions. The path is smoothed when
/// the clip is drawn, so changing the smoothing needs no new analysis, and a
/// split, a trim or a speed change never changes how a frame is corrected.
///
/// The analysis covers the clip's source range and a margin around it. Frames
/// outside the analysed part keep no correction (they are only enlarged by the
/// zoom); [`Stabilization::covers`] tells whether a range was analysed.
///
/// The path is shared, not copied, by splits and undo records (an hour of
/// video at 60 fps holds 216 000 samples); comparing two stabilisations
/// compares the paths' contents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stabilization {
    /// Standard deviation of the Gaussian smoothing the camera's path, in
    /// seconds of the source file. From
    /// [`Stabilization::MINIMUM_SMOOTHING_IN_SECONDS`] to
    /// [`Stabilization::MAXIMUM_SMOOTHING_IN_SECONDS`].
    pub smoothing_in_seconds: f32,
    /// How much the steadied picture is enlarged so its moving edges stay
    /// outside the frame (1 = not enlarged, at most
    /// [`Stabilization::MAXIMUM_ZOOM_FACTOR`]). Derived from the path and the
    /// smoothing by `kadr-engine`, never set by hand; the renderer never shows
    /// an edge, even if it is stale.
    pub zoom_factor: f32,
    /// Size of the pictures the path was measured in (the file's picture size),
    /// in pixels.
    pub analysed_picture_size: PictureSize,
    /// The camera's measured path: one sample per analysed frame, in strictly
    /// increasing source time, never empty. Never simplified or interpolated:
    /// the small motions it would drop are the shake itself.
    pub camera_path: Arc<[CameraPathSample]>,
}

/// Frame starts are rounded up to whole ticks (`FrameRate::timestamp_of_frame`),
/// so two spacings of the same frame rate can differ by one tick: the end of the
/// last analysed frame, estimated from the last spacing, can be a tick early.
const FRAME_START_ROUNDING: TimeSpan = TimeSpan::from_ticks(1);

impl Stabilization {
    /// The smoothing of a new stabilisation, in seconds.
    pub const DEFAULT_SMOOTHING_IN_SECONDS: f32 = 0.5;
    /// The lightest smoothing, in seconds.
    pub const MINIMUM_SMOOTHING_IN_SECONDS: f32 = 0.1;
    /// The strongest smoothing, in seconds.
    pub const MAXIMUM_SMOOTHING_IN_SECONDS: f32 = 10.0;
    /// The zoom factor of a picture that is not enlarged.
    pub const MINIMUM_ZOOM_FACTOR: f32 = 1.0;
    /// The largest enlargement: a shake needing more is only partly corrected.
    pub const MAXIMUM_ZOOM_FACTOR: f32 = 1.2;
    /// The smallest size a camera path sample may have, relative to the first frame.
    pub const MINIMUM_PATH_SCALE_FACTOR: f32 = 0.25;
    /// The largest size a camera path sample may have, relative to the first frame.
    pub const MAXIMUM_PATH_SCALE_FACTOR: f32 = 4.0;

    /// The index of the sample whose frame shows at `source_time`: the last
    /// sample at or before it (the decoder's rule), or the first sample for a
    /// time before the path starts. Never interpolated: a frame shown twice (a
    /// slowed-down clip) gets the same correction twice. 0 for an empty path.
    pub fn sample_index_at(&self, source_time: Timestamp) -> usize {
        self.camera_path
            .partition_point(|sample| sample.source_time <= source_time)
            .saturating_sub(1)
    }

    /// The moment to decode to show `source_time`: inside the analysed part
    /// (from the first sample to the end of the last sample's frame), the time
    /// of the sample whose frame shows then; outside it, `source_time` itself.
    ///
    /// The analysis decoded each frame at its sample's time, so decoding at the
    /// same time shows exactly the frame that was measured, even in files whose
    /// frames are not evenly spaced.
    pub fn frame_time_for(&self, source_time: Timestamp) -> Timestamp {
        let (first_time, _) = self.analysed_range();
        let is_inside = self
            .analysed_end()
            .is_some_and(|analysed_end| first_time <= source_time && source_time < analysed_end);
        if !is_inside {
            return source_time;
        }
        self.camera_path
            .get(self.sample_index_at(source_time))
            .map_or(source_time, |sample| sample.source_time)
    }

    /// The times of the first and last samples (both zero for an empty path).
    pub fn analysed_range(&self) -> (Timestamp, Timestamp) {
        match (self.camera_path.first(), self.camera_path.last()) {
            (Some(first), Some(last)) => (first.source_time, last.source_time),
            _ => (Timestamp::ZERO, Timestamp::ZERO),
        }
    }

    /// Whether every frame shown from `source_start` to `source_end` (excluded)
    /// was analysed: the range starts at or after the first sample and ends by
    /// the end of the last sample's frame.
    pub fn covers(&self, source_start: Timestamp, source_end: Timestamp) -> bool {
        let (first_time, _) = self.analysed_range();
        self.analysed_end().is_some_and(|analysed_end| {
            first_time <= source_start && source_end <= analysed_end + FRAME_START_ROUNDING
        })
    }

    /// The stretches where the camera's motion could not be measured (a cut, a
    /// frame too blurred or too uniform to follow): the samples after the first
    /// whose `motion_was_measured` is false, consecutive ones joined, each with
    /// the confidence [`TrackingConfidence::Lost`].
    pub fn unmeasured_ranges(&self) -> Vec<UnsureRange> {
        let mut ranges = Vec::new();
        let mut current: Option<UnsureRange> = None;
        for sample in self.camera_path.iter().skip(1) {
            if sample.motion_was_measured {
                ranges.extend(current.take());
                continue;
            }
            current = Some(match current {
                Some(range) => UnsureRange {
                    last_source_time: sample.source_time,
                    ..range
                },
                None => UnsureRange {
                    first_source_time: sample.source_time,
                    last_source_time: sample.source_time,
                    confidence: TrackingConfidence::Lost,
                },
            });
        }
        ranges.extend(current);
        ranges
    }

    /// Where the last sample's frame ends (excluded), estimated from the spacing
    /// of the last two samples: the last sample's time for a one-sample path,
    /// `None` for an empty one.
    fn analysed_end(&self) -> Option<Timestamp> {
        let last = self.camera_path.last()?;
        let last_spacing = match self.camera_path.last_chunk::<2>() {
            Some([before_last, last]) => last
                .source_time
                .span_since(before_last.source_time)
                .unwrap_or(TimeSpan::ZERO),
            None => TimeSpan::ZERO,
        };
        Some(last.source_time + last_spacing)
    }
}

/// Where the camera was at one analysed frame, relative to the path's first
/// frame, about the centre of the picture, in the pixels of
/// [`Stabilization::analysed_picture_size`].
///
/// A point of the content seen in the path's first frame at `u` is at
/// `C + scale · R(rotation) · (u − C) + shift` in this frame, `C` being the
/// picture's centre and `R` a clockwise rotation (y downwards).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraPathSample {
    /// Start of the analysed frame in the file: the time the analysis decoded
    /// it at (`FrameRate::timestamp_of_frame` of its index).
    pub source_time: Timestamp,
    /// How far the content moved right (positive) since the path's first
    /// frame, in source pixels.
    pub shift_x_in_pixels: f32,
    /// How far the content moved down (positive) since the path's first frame,
    /// in source pixels.
    pub shift_y_in_pixels: f32,
    /// How much the content turned clockwise (y downwards) since the path's
    /// first frame, in degrees.
    pub rotation_in_degrees: f32,
    /// The content's size relative to the path's first frame (1 = same size).
    pub scale_factor: f32,
    /// Whether the motion since the previous sample was measured. False on the
    /// first sample, and after a cut or a frame that could not be followed: the
    /// smoothing restarts there.
    pub motion_was_measured: bool,
}

#[cfg(test)]
pub(crate) mod test_samples {
    //! Ready-made stabilisations for the tests of every module.

    use super::*;
    use crate::time::FrameRate;

    /// A stabilisation of a 1080p, 30 fps file whose path has one sample per
    /// frame from `first_frame` to `last_frame` (included), drifting right by a
    /// pixel per frame, every motion measured.
    pub(crate) fn stabilization_of_frames(first_frame: u64, last_frame: u64) -> Stabilization {
        let camera_path: Vec<CameraPathSample> = (first_frame..=last_frame)
            .map(|frame_index| CameraPathSample {
                source_time: FrameRate::FPS_30.timestamp_of_frame(frame_index),
                shift_x_in_pixels: (frame_index - first_frame) as f32,
                shift_y_in_pixels: 0.0,
                rotation_in_degrees: 0.0,
                scale_factor: 1.0,
                motion_was_measured: frame_index != first_frame,
            })
            .collect();
        Stabilization {
            smoothing_in_seconds: Stabilization::DEFAULT_SMOOTHING_IN_SECONDS,
            zoom_factor: 1.05,
            analysed_picture_size: PictureSize {
                width_in_pixels: 1920,
                height_in_pixels: 1080,
            },
            camera_path: camera_path.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_samples::stabilization_of_frames;
    use super::*;
    use crate::time::FrameRate;

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

    fn one_tick_before(time: Timestamp) -> Timestamp {
        time.checked_sub(TimeSpan::from_ticks(1)).unwrap()
    }

    #[test]
    fn stabilization_lookup_is_a_step() {
        // At 2997/100 fps (how some files give 29.97), frames do not start on
        // whole ticks: the lookup must still find each frame's own sample at its
        // start, and the previous one a tick before.
        let frame_rate = FrameRate::new(2_997, 100).unwrap();
        let camera_path: Vec<CameraPathSample> = (30..=40)
            .map(|frame_index| CameraPathSample {
                source_time: frame_rate.timestamp_of_frame(frame_index),
                ..stabilization_of_frames(0, 0).camera_path[0]
            })
            .collect();
        let stabilization = Stabilization {
            camera_path: camera_path.into(),
            ..stabilization_of_frames(0, 0)
        };
        for frame_index in 31..=40 {
            let frame_start = frame_rate.timestamp_of_frame(frame_index);
            let expected_index = (frame_index - 30) as usize;
            assert_eq!(stabilization.sample_index_at(frame_start), expected_index);
            assert_eq!(
                stabilization.sample_index_at(one_tick_before(frame_start)),
                expected_index - 1
            );
        }
        assert_eq!(stabilization.sample_index_at(Timestamp::ZERO), 0);
        assert_eq!(
            stabilization.sample_index_at(Timestamp::from_whole_seconds(60)),
            10
        );
    }

    #[test]
    fn frame_time_snaps_only_inside_the_analysed_range() {
        let frame_start = |frame_index| FrameRate::FPS_30.timestamp_of_frame(frame_index);
        let stabilization = stabilization_of_frames(30, 40);
        assert_eq!(
            stabilization.analysed_range(),
            (frame_start(30), frame_start(40))
        );

        let inside_frame_35 = frame_start(35) + TimeSpan::from_milliseconds(10);
        assert_eq!(
            stabilization.frame_time_for(inside_frame_35),
            frame_start(35)
        );
        // The last sample's frame lasts until the next frame would start.
        let inside_frame_40 = one_tick_before(frame_start(41));
        assert_eq!(
            stabilization.frame_time_for(inside_frame_40),
            frame_start(40)
        );
        // Outside the analysed part, times are decoded as they are.
        for outside in [
            frame_start(41),
            frame_start(41) + TimeSpan::from_milliseconds(10),
            one_tick_before(frame_start(30)),
            Timestamp::ZERO,
        ] {
            assert_eq!(stabilization.frame_time_for(outside), outside);
        }
    }

    #[test]
    fn a_path_covers_its_frames_up_to_the_end_of_the_last_one() {
        let frame_start = |frame_index| FrameRate::FPS_30.timestamp_of_frame(frame_index);
        let stabilization = stabilization_of_frames(30, 40);
        assert!(stabilization.covers(frame_start(30), frame_start(41)));
        assert!(stabilization.covers(frame_start(32), frame_start(35)));
        assert!(!stabilization.covers(frame_start(29), frame_start(35)));
        assert!(!stabilization.covers(frame_start(32), frame_start(42)));

        // When frames do not start on whole ticks, the last spacing can be a
        // tick shorter than the next one: a range ending at the next frame's
        // start is still covered.
        let frame_rate = FrameRate::new(2_997, 100).unwrap();
        for last_frame in 1..40 {
            let camera_path: Vec<CameraPathSample> = (0..=last_frame)
                .map(|frame_index| CameraPathSample {
                    source_time: frame_rate.timestamp_of_frame(frame_index),
                    ..stabilization.camera_path[0]
                })
                .collect();
            let ntsc_stabilization = Stabilization {
                camera_path: camera_path.into(),
                ..stabilization.clone()
            };
            assert!(
                ntsc_stabilization.covers(
                    Timestamp::ZERO,
                    frame_rate.timestamp_of_frame(last_frame + 1)
                ),
                "{last_frame}"
            );
        }
    }

    #[test]
    fn unmeasured_ranges_skip_the_first_sample() {
        let measured = [false, true, false, false, true, false];
        let base = stabilization_of_frames(0, 5);
        let camera_path: Vec<CameraPathSample> = base
            .camera_path
            .iter()
            .zip(measured)
            .map(|(sample, motion_was_measured)| CameraPathSample {
                motion_was_measured,
                ..*sample
            })
            .collect();
        let stabilization = Stabilization {
            camera_path: camera_path.into(),
            ..base
        };
        let frame_start = |frame_index| FrameRate::FPS_30.timestamp_of_frame(frame_index);
        assert_eq!(
            stabilization.unmeasured_ranges(),
            vec![
                UnsureRange {
                    first_source_time: frame_start(2),
                    last_source_time: frame_start(3),
                    confidence: TrackingConfidence::Lost,
                },
                UnsureRange {
                    first_source_time: frame_start(5),
                    last_source_time: frame_start(5),
                    confidence: TrackingConfidence::Lost,
                },
            ]
        );
        assert!(stabilization_of_frames(0, 5).unmeasured_ranges().is_empty());
    }

    #[test]
    fn a_stabilization_reads_back_from_json_with_its_path() {
        let stabilization = EffectKind::Stabilization(stabilization_of_frames(0, 2));
        let json_text = serde_json::to_string(&stabilization).unwrap();
        assert!(
            json_text.starts_with(r#"{"type":"stabilization","#),
            "{json_text}"
        );
        let read_back: EffectKind = serde_json::from_str(&json_text).unwrap();
        assert_eq!(read_back, stabilization);
    }
}
