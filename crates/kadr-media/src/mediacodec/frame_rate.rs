//! Finding a video's exact frame rate on Android.
//!
//! Android's file reader announces the frame rate as a whole number (`frame-rate`
//! = 30 for a 29.97 fps phone video), which would make every frame of a long
//! video drift. So the rate is measured instead, from the presentation times of
//! the first frames, and snapped to the standard rate it matches (29.97 is really
//! 30000/1001). The announced rate is the fallback when there are too few frames.

use kadr_core::time::FrameRate;

/// How many frames are measured. Two seconds at 30 fps: enough to average out
/// the rounding of microsecond timestamps, few enough to read instantly.
pub(crate) const FRAMES_MEASURED_FOR_FRAME_RATE: usize = 60;

/// The rates cameras, phones and editors produce, as exact fractions.
const STANDARD_FRAME_RATES: [(u32, u32); 14] = [
    (24_000, 1_001),
    (24, 1),
    (25, 1),
    (30_000, 1_001),
    (30, 1),
    (48, 1),
    (50, 1),
    (60_000, 1_001),
    (60, 1),
    (90, 1),
    (100, 1),
    (120_000, 1_001),
    (120, 1),
    (240, 1),
];

/// A measured rate within this relative distance of a standard rate is that rate.
/// 29.97 and 30 are 0.1 % apart, so this stays well below.
const STANDARD_RATE_RELATIVE_TOLERANCE: f64 = 0.000_3;

/// Rates that are not standard are kept to this precision (1/1000 fps).
const NON_STANDARD_RATE_DENOMINATOR: u32 = 1_000;

/// Highest believable rate, like the FFmpeg backend.
const MAXIMUM_PLAUSIBLE_FRAMES_PER_SECOND: f64 = 240.0;

/// The frame rate of a video whose first frames start at
/// `frame_starts_in_microseconds` (in any order: files store frames in decoding
/// order). `announced_frames_per_second` is the file's own claim, used when fewer
/// than two frames are known.
pub(crate) fn measure_frame_rate(
    frame_starts_in_microseconds: &[i64],
    announced_frames_per_second: Option<f64>,
) -> Option<FrameRate> {
    let mut sorted_starts = frame_starts_in_microseconds.to_vec();
    sorted_starts.sort_unstable();
    sorted_starts.dedup();
    let measured_frames_per_second = match (sorted_starts.first(), sorted_starts.last()) {
        (Some(first), Some(last)) if sorted_starts.len() >= 2 && last > first => {
            let interval_count = (sorted_starts.len() - 1) as f64;
            Some(interval_count * 1_000_000.0 / (last - first) as f64)
        }
        _ => None,
    };
    let frames_per_second = measured_frames_per_second.or(announced_frames_per_second)?;
    frame_rate_closest_to(frames_per_second)
}

/// The standard rate within tolerance of `frames_per_second`, or that rate to the
/// nearest thousandth. `None` when it is not a believable video rate.
fn frame_rate_closest_to(frames_per_second: f64) -> Option<FrameRate> {
    if !frames_per_second.is_finite()
        || frames_per_second <= 0.0
        || frames_per_second > MAXIMUM_PLAUSIBLE_FRAMES_PER_SECOND
    {
        return None;
    }
    let standard_rate = STANDARD_FRAME_RATES
        .iter()
        .find(|(numerator, denominator)| {
            let standard = f64::from(*numerator) / f64::from(*denominator);
            ((frames_per_second - standard) / standard).abs() < STANDARD_RATE_RELATIVE_TOLERANCE
        });
    let (numerator, denominator) = match standard_rate {
        Some(&rate) => rate,
        None => (
            (frames_per_second * f64::from(NON_STANDARD_RATE_DENOMINATOR)).round() as u32,
            NON_STANDARD_RATE_DENOMINATOR,
        ),
    };
    FrameRate::new(numerator, denominator).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mediacodec::media_time::frame_start_in_microseconds;

    fn starts(frame_rate: FrameRate, count: u64) -> Vec<i64> {
        (0..count)
            .map(|index| frame_start_in_microseconds(index, frame_rate))
            .collect()
    }

    #[test]
    fn ntsc_rates_are_told_apart_from_whole_ones() {
        assert_eq!(
            measure_frame_rate(&starts(FrameRate::FPS_29_97, 60), Some(30.0)),
            Some(FrameRate::FPS_29_97)
        );
        assert_eq!(
            measure_frame_rate(&starts(FrameRate::FPS_30, 60), Some(30.0)),
            Some(FrameRate::FPS_30)
        );
    }

    #[test]
    fn decoding_order_does_not_matter() {
        // B-frames: stored as 0, 3, 1, 2, 6, 4, 5…
        let mut shuffled = starts(FrameRate::FPS_25, 30);
        shuffled.swap(1, 3);
        shuffled.swap(4, 6);

        assert_eq!(measure_frame_rate(&shuffled, None), Some(FrameRate::FPS_25));
    }

    #[test]
    fn unusual_rates_keep_three_decimals() {
        let rate = measure_frame_rate(&[0, 80_000, 160_000], None).unwrap();
        assert_eq!(rate, FrameRate::new(12_500, 1_000).unwrap());
    }

    #[test]
    fn the_announced_rate_is_used_when_frames_are_missing() {
        assert_eq!(
            measure_frame_rate(&[0], Some(24.0)),
            Some(FrameRate::FPS_24)
        );
        assert_eq!(measure_frame_rate(&[], None), None);
    }

    #[test]
    fn unbelievable_rates_are_ignored() {
        assert_eq!(measure_frame_rate(&[], Some(90_000.0)), None);
        assert_eq!(measure_frame_rate(&[], Some(0.0)), None);
    }
}
