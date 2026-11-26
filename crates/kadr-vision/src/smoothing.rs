//! Steadying the tracked region, so that the blur does not shake.
//!
//! The tracker's answer jitters by a fraction of a pixel from one frame to the
//! next; a blur following it exactly would visibly tremble. The One Euro filter
//! smooths it: a low-pass filter whose cutoff frequency rises with speed, so it
//! smooths strongly when the region is still (no jitter) and little when it moves
//! fast (little lag).
//!
//! Reference: G. Casiez, N. Roussel and D. Vogel, "1€ Filter: A Simple Speed-based
//! Low-pass Filter for Noisy Input in Interactive Systems", CHI 2012.
//!
//! A filter running forward in time always lags behind. Since the whole track is
//! known once the analysis is over, it is filtered twice, forward and backward,
//! and the two results are averaged: their lags have opposite signs and cancel
//! out (a "zero-phase" filter). Both ends are padded with the straight line that
//! best fits the first (and last) frames, which continues a steady motion
//! exactly, so that the filters have settled by the first real frame.
//!
//! Lost frames are left alone, and each run of tracked frames between them is
//! smoothed on its own. Values are measured in region sizes (divided by the run's
//! average longer side), so the same settings work at any resolution.

use std::ops::Range;

use crate::geometry::Region;
use crate::tracker::{TrackedRegion, TrackingState};

/// Cutoff frequency when the region is still.
const MINIMUM_CUTOFF_IN_HERTZ: f32 = 1.0;

/// How much the cutoff rises with speed, per region size per second.
const CUTOFF_INCREASE_PER_SPEED: f32 = 0.5;

/// Cutoff frequency of the speed estimate itself.
const SPEED_CUTOFF_IN_HERTZ: f32 = 1.0;

/// Length mirrored at each end of a run (when the run is long enough): enough
/// for both filters to settle.
const PADDING_IN_SECONDS: f32 = 1.0;

/// Smooths the position and size of every run of tracked (not lost) frames,
/// without lag. `frames_per_second` is the rate at which the frames were tracked.
pub fn smooth_track(track: &[TrackedRegion], frames_per_second: f32) -> Vec<TrackedRegion> {
    let mut smoothed = track.to_vec();
    if frames_per_second.is_nan() || frames_per_second <= 0.0 {
        return smoothed;
    }
    let seconds_per_frame = 1.0 / frames_per_second;
    let padding_in_frames = (PADDING_IN_SECONDS * frames_per_second).ceil() as usize;
    for run in runs_of_tracked_frames(track) {
        smooth_run(&mut smoothed[run], seconds_per_frame, padding_in_frames);
    }
    smoothed
}

fn runs_of_tracked_frames(track: &[TrackedRegion]) -> Vec<Range<usize>> {
    let mut runs = Vec::new();
    let mut run_start = None;
    for (index, tracked) in track.iter().enumerate() {
        match (tracked.state == TrackingState::Lost, run_start) {
            (false, None) => run_start = Some(index),
            (true, Some(start)) => {
                runs.push(start..index);
                run_start = None;
            }
            _ => {}
        }
    }
    if let Some(start) = run_start {
        runs.push(start..track.len());
    }
    runs
}

fn smooth_run(run: &mut [TrackedRegion], seconds_per_frame: f32, padding_in_frames: usize) {
    if run.len() < 2 {
        return;
    }
    let region_size = run
        .iter()
        .map(|tracked| tracked.region.longer_side())
        .sum::<f32>()
        / run.len() as f32;
    if region_size <= 0.0 {
        return;
    }
    let smoothed_component = |component: fn(&Region) -> f32| {
        let values: Vec<f32> = run
            .iter()
            .map(|tracked| component(&tracked.region) / region_size)
            .collect();
        smooth_without_lag(&values, seconds_per_frame, padding_in_frames)
    };
    let centers_x = smoothed_component(|region| region.center_x);
    let centers_y = smoothed_component(|region| region.center_y);
    let widths = smoothed_component(|region| region.width);
    for (index, tracked) in run.iter_mut().enumerate() {
        let aspect_ratio = tracked.region.height / tracked.region.width;
        let width = widths[index] * region_size;
        tracked.region = Region {
            center_x: centers_x[index] * region_size,
            center_y: centers_y[index] * region_size,
            width,
            height: width * aspect_ratio,
        };
    }
}

/// Frames used to fit the straight line that pads each end.
const LINE_FIT_LENGTH_IN_FRAMES: usize = 10;

/// Forward and backward One Euro passes over the padded values, averaged.
fn smooth_without_lag(values: &[f32], seconds_per_frame: f32, padding: usize) -> Vec<f32> {
    let fit_length = LINE_FIT_LENGTH_IN_FRAMES.min(values.len());
    let (start_value, start_slope) = fitted_line(&values[..fit_length]);
    let (end_intercept, end_slope) = fitted_line(&values[values.len() - fit_length..]);
    let end_value = end_intercept + end_slope * (fit_length - 1) as f32;
    let padded: Vec<f32> = (1..=padding)
        .rev()
        .map(|distance| start_value - start_slope * distance as f32)
        .chain(values.iter().copied())
        .chain((1..=padding).map(|distance| end_value + end_slope * distance as f32))
        .collect();
    let mut forward_filter = OneEuroFilter::default();
    let forward: Vec<f32> = padded
        .iter()
        .map(|&value| forward_filter.filter(value, seconds_per_frame))
        .collect();
    let mut backward_filter = OneEuroFilter::default();
    let mut backward: Vec<f32> = padded
        .iter()
        .rev()
        .map(|&value| backward_filter.filter(value, seconds_per_frame))
        .collect();
    backward.reverse();
    forward[padding..padding + values.len()]
        .iter()
        .zip(&backward[padding..padding + values.len()])
        .map(|(forward_value, backward_value)| (forward_value + backward_value) / 2.0)
        .collect()
}

/// The least-squares line through `values` (at positions 0, 1, 2…): its value
/// at position 0 and its slope per position.
fn fitted_line(values: &[f32]) -> (f32, f32) {
    let count = values.len() as f32;
    let mean_position = (count - 1.0) / 2.0;
    let mean_value = values.iter().sum::<f32>() / count;
    let (mut covariance, mut variance) = (0.0_f32, 0.0_f32);
    for (position, value) in values.iter().enumerate() {
        let offset = position as f32 - mean_position;
        covariance += offset * (value - mean_value);
        variance += offset * offset;
    }
    let slope = if variance > 0.0 {
        covariance / variance
    } else {
        0.0
    };
    (mean_value - slope * mean_position, slope)
}

/// The One Euro filter, for one value.
#[derive(Debug, Clone, Copy, Default)]
struct OneEuroFilter {
    /// The previous smoothed value and smoothed speed.
    previous: Option<(f32, f32)>,
}

impl OneEuroFilter {
    fn filter(&mut self, value: f32, seconds_per_frame: f32) -> f32 {
        let Some((previous_value, previous_speed)) = self.previous else {
            self.previous = Some((value, 0.0));
            return value;
        };
        let speed = (value - previous_value) / seconds_per_frame;
        let smoothed_speed = blend(
            previous_speed,
            speed,
            smoothing_factor(SPEED_CUTOFF_IN_HERTZ, seconds_per_frame),
        );
        let cutoff = MINIMUM_CUTOFF_IN_HERTZ + CUTOFF_INCREASE_PER_SPEED * smoothed_speed.abs();
        let smoothed_value = blend(
            previous_value,
            value,
            smoothing_factor(cutoff, seconds_per_frame),
        );
        self.previous = Some((smoothed_value, smoothed_speed));
        smoothed_value
    }
}

/// The weight of the new value in a first-order low-pass filter.
fn smoothing_factor(cutoff_in_hertz: f32, seconds_per_frame: f32) -> f32 {
    let time_constant = 1.0 / (std::f32::consts::TAU * cutoff_in_hertz);
    1.0 / (1.0 + time_constant / seconds_per_frame)
}

fn blend(previous: f32, new: f32, weight_of_new: f32) -> f32 {
    previous + (new - previous) * weight_of_new
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAMES_PER_SECOND: f32 = 30.0;

    fn tracked(center_x: f32, center_y: f32, state: TrackingState) -> TrackedRegion {
        TrackedRegion {
            region: Region {
                center_x,
                center_y,
                width: 50.0,
                height: 60.0,
            },
            state,
            peak_to_sidelobe_ratio: 20.0,
        }
    }

    fn standard_deviation(values: impl Iterator<Item = f32> + Clone) -> f32 {
        let count = values.clone().count() as f32;
        let mean = values.clone().sum::<f32>() / count;
        (values.map(|value| (value - mean).powi(2)).sum::<f32>() / count).sqrt()
    }

    #[test]
    fn a_still_region_stops_jittering() {
        let track: Vec<TrackedRegion> = (0..90)
            .map(|index| {
                let jitter = if index % 2 == 0 { 0.6 } else { -0.6 };
                tracked(100.0 + jitter, 80.0 - jitter, TrackingState::Confident)
            })
            .collect();
        let smoothed = smooth_track(&track, FRAMES_PER_SECOND);
        let before = standard_deviation(track.iter().map(|tracked| tracked.region.center_x));
        let after = standard_deviation(smoothed.iter().map(|tracked| tracked.region.center_x));
        assert!(after < before / 5.0, "before {before}, after {after}");
    }

    #[test]
    fn a_steady_motion_is_followed_without_lag() {
        let track: Vec<TrackedRegion> = (0..60)
            .map(|index| tracked(20.0 + 6.0 * index as f32, 50.0, TrackingState::Confident))
            .collect();
        let smoothed = smooth_track(&track, FRAMES_PER_SECOND);
        for (original, smoothed) in track.iter().zip(&smoothed) {
            assert!(
                (original.region.center_x - smoothed.region.center_x).abs() < 0.2,
                "{original:?} became {smoothed:?}"
            );
            assert_eq!(smoothed.region.height / smoothed.region.width, 1.2);
        }
    }

    #[test]
    fn lost_frames_are_left_alone_and_split_the_track() {
        let mut track: Vec<TrackedRegion> = (0..20)
            .map(|index| tracked(100.0 + index as f32, 50.0, TrackingState::Confident))
            .collect();
        track[10] = tracked(500.0, 500.0, TrackingState::Lost);
        let smoothed = smooth_track(&track, FRAMES_PER_SECOND);
        assert_eq!(smoothed[10], track[10]);
        assert!(
            (smoothed[9].region.center_x - 109.0).abs() < 0.5,
            "not pulled towards the lost frame"
        );
        assert_eq!(runs_of_tracked_frames(&track), vec![0..10, 11..20]);
    }

    #[test]
    fn the_fitted_line_goes_through_aligned_values() {
        assert_eq!(fitted_line(&[3.0, 5.0, 7.0, 9.0]), (3.0, 2.0));
        assert_eq!(fitted_line(&[4.0]), (4.0, 0.0));
    }

    #[test]
    fn short_runs_and_invalid_rates_are_left_alone() {
        let track = vec![tracked(1.0, 2.0, TrackingState::Uncertain)];
        assert_eq!(smooth_track(&track, FRAMES_PER_SECOND), track);
        let two = vec![
            tracked(1.0, 2.0, TrackingState::Confident),
            tracked(3.0, 2.0, TrackingState::Confident),
        ];
        assert_eq!(smooth_track(&two, 0.0), two);
    }
}
