//! Turning the tracked frames into the samples stored in the project.
//!
//! A one-minute clip has 1 800 frames; storing a sample for each would make
//! project files large for nothing, because the blur interpolates linearly
//! between samples anyway. A frame is dropped when the straight line between the
//! samples kept around it gives its region within
//! [`SIMPLIFICATION_TOLERANCE_IN_PIXELS`]. Frames where the confidence changes,
//! and the user's selection, are always kept, so that the stored confidences
//! (and the uncertain ranges shown on the timeline) stay exact; so are the
//! user's selection and corrections.

use kadr_core::effect::{PixelRegion, RegionSample, RegionSampleOrigin};

use super::TrackedFrame;

/// Largest difference, on any side of the region, between a dropped frame and
/// the line between the samples kept around it.
pub(super) const SIMPLIFICATION_TOLERANCE_IN_PIXELS: f32 = 1.0;

/// Longest gap between two kept samples, in frames, to keep the search short.
const MAXIMUM_FRAMES_BETWEEN_SAMPLES: usize = 150;

/// The frames worth storing, as samples.
pub(super) fn simplified_samples(frames: &[TrackedFrame]) -> Vec<RegionSample> {
    let Some(last) = frames.len().checked_sub(1) else {
        return Vec::new();
    };
    let must_keep = |index: usize| {
        let differs_from = |other: usize| frames[other].confidence != frames[index].confidence;
        index == 0
            || index == last
            || frames[index].origin != RegionSampleOrigin::Tracker
            || differs_from(index - 1)
            || differs_from(index + 1)
    };
    let mut kept_indices = vec![0];
    let mut anchor = 0;
    while anchor < last {
        let mut end = anchor + 1;
        while end < last
            && end - anchor < MAXIMUM_FRAMES_BETWEEN_SAMPLES
            && !must_keep(end)
            && line_reproduces(frames, anchor, end + 1)
        {
            end += 1;
        }
        kept_indices.push(end);
        anchor = end;
    }
    kept_indices
        .into_iter()
        .map(|index| {
            let frame = &frames[index];
            RegionSample {
                source_time: frame.source_time,
                region: frame.region,
                confidence: frame.confidence,
                origin: frame.origin,
            }
        })
        .collect()
}

/// Whether the straight line from frame `start` to frame `end` gives every
/// frame between them within the tolerance.
fn line_reproduces(frames: &[TrackedFrame], start: usize, end: usize) -> bool {
    let (from, to) = (&frames[start].region, &frames[end].region);
    (start + 1..end).all(|index| {
        let fraction = (index - start) as f32 / (end - start) as f32;
        largest_side_difference(
            &from.interpolated_towards(to, fraction),
            &frames[index].region,
        ) <= SIMPLIFICATION_TOLERANCE_IN_PIXELS
    })
}

fn largest_side_difference(first: &PixelRegion, second: &PixelRegion) -> f32 {
    let sides = |region: &PixelRegion| {
        [
            region.left_in_pixels,
            region.top_in_pixels,
            region.left_in_pixels + region.width_in_pixels,
            region.top_in_pixels + region.height_in_pixels,
        ]
    };
    sides(first)
        .iter()
        .zip(sides(second))
        .map(|(first, second)| (first - second).abs())
        .fold(0.0, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kadr_core::effect::TrackingConfidence;
    use kadr_core::time::FrameRate;

    fn frame(index: u64, left: f32, confidence: TrackingConfidence) -> TrackedFrame {
        TrackedFrame {
            source_time: FrameRate::FPS_30.timestamp_of_frame(index),
            region: PixelRegion {
                left_in_pixels: left,
                top_in_pixels: 50.0,
                width_in_pixels: 100.0,
                height_in_pixels: 120.0,
            },
            confidence,
            origin: RegionSampleOrigin::Tracker,
            peak_to_sidelobe_ratio: Some(20.0),
        }
    }

    #[test]
    fn a_straight_motion_keeps_only_its_ends() {
        let frames: Vec<TrackedFrame> = (0..40)
            .map(|index| {
                frame(
                    index,
                    10.0 + 3.0 * index as f32,
                    TrackingConfidence::Confident,
                )
            })
            .collect();
        let samples = simplified_samples(&frames);
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[1].source_time, frames[39].source_time);
    }

    #[test]
    fn a_curve_keeps_enough_samples_to_stay_within_a_pixel() {
        let left_at = |index: u64| 200.0 + 80.0 * (index as f32 * 0.1).sin();
        let frames: Vec<TrackedFrame> = (0..120)
            .map(|index| frame(index, left_at(index), TrackingConfidence::Confident))
            .collect();
        let samples = simplified_samples(&frames);
        assert!(
            samples.len() > 4 && samples.len() < 60,
            "{} samples",
            samples.len()
        );
        // Between samples, the linear interpolation stays within the tolerance.
        for pair in samples.windows(2) {
            let (from, to) = (&pair[0], &pair[1]);
            let start = FrameRate::FPS_30.frame_index_at(from.source_time);
            let end = FrameRate::FPS_30.frame_index_at(to.source_time);
            for index in start..=end {
                let fraction = (index - start) as f32 / (end - start).max(1) as f32;
                let interpolated = from.region.interpolated_towards(&to.region, fraction);
                assert!((interpolated.left_in_pixels - left_at(index)).abs() <= 1.0 + 1e-3);
            }
        }
    }

    #[test]
    fn confidence_changes_and_the_selection_are_kept() {
        let mut frames: Vec<TrackedFrame> = (0..30)
            .map(|index| frame(index, 100.0, TrackingConfidence::Confident))
            .collect();
        for frame in &mut frames[10..15] {
            frame.confidence = TrackingConfidence::Lost;
        }
        frames[20].origin = RegionSampleOrigin::UserSelection;
        let kept: Vec<u64> = simplified_samples(&frames)
            .iter()
            .map(|sample| FrameRate::FPS_30.frame_index_at(sample.source_time))
            .collect();
        assert_eq!(kept, vec![0, 9, 10, 14, 15, 20, 29]);
    }

    #[test]
    fn no_frames_give_no_samples() {
        assert!(simplified_samples(&[]).is_empty());
    }
}
