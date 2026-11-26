//! MediaCodec time (microseconds) and Kadr time (ticks), and which decoded frame
//! is on screen at a given time.
//!
//! Android gives every sample and frame a presentation time in whole
//! microseconds, rounded down from the file's own clock. Frames rarely fall on
//! whole microseconds (one frame at 30 fps lasts 33 333.3 µs), so a frame may
//! appear to start up to a microsecond early, or, with another file writer, late.
//! [`FRAME_START_TOLERANCE`] absorbs that, so that asking for the exact time of a
//! frame always gives that frame.

use kadr_core::time::{FrameRate, TICKS_PER_SECOND, TimeSpan, Timestamp};

/// MediaCodec's time unit.
const MICROSECONDS_PER_SECOND: u128 = 1_000_000;

/// One microsecond, in ticks, rounded up (705.6 → 706).
pub(crate) const FRAME_START_TOLERANCE: TimeSpan =
    TimeSpan::from_ticks((TICKS_PER_SECOND as u128).div_ceil(MICROSECONDS_PER_SECOND) as u64);

/// Beyond this distance ahead, jumping to a keyframe is faster than decoding forward.
pub(crate) const MAXIMUM_FORWARD_DECODING_DISTANCE: TimeSpan = TimeSpan::from_milliseconds(2_000);

/// A MediaCodec time as a Kadr timestamp. Negative times become zero.
pub(crate) fn timestamp_from_microseconds(microseconds: i64) -> Timestamp {
    let Ok(microseconds) = u128::try_from(microseconds) else {
        return Timestamp::ZERO;
    };
    let ticks = microseconds * u128::from(TICKS_PER_SECOND) / MICROSECONDS_PER_SECOND;
    Timestamp::from_ticks(u64::try_from(ticks).unwrap_or(u64::MAX))
}

/// A Kadr timestamp as a MediaCodec time, rounded down.
pub(crate) fn microseconds_from_timestamp(time: Timestamp) -> i64 {
    let microseconds =
        u128::from(time.ticks()) * MICROSECONDS_PER_SECOND / u128::from(TICKS_PER_SECOND);
    i64::try_from(microseconds).unwrap_or(i64::MAX)
}

/// When frame `frame_index` of a video at `frame_rate` starts, in microseconds
/// (rounded down, like Android's own file readers).
pub(crate) fn frame_start_in_microseconds(frame_index: u64, frame_rate: FrameRate) -> i64 {
    let microseconds =
        u128::from(frame_index) * MICROSECONDS_PER_SECOND * u128::from(frame_rate.denominator())
            / u128::from(frame_rate.numerator());
    i64::try_from(microseconds).unwrap_or(i64::MAX)
}

/// When sample `sample_index` of sound at `sample_rate_in_hertz` starts, in microseconds.
pub(crate) fn sample_start_in_microseconds(sample_index: u64, sample_rate_in_hertz: u32) -> i64 {
    let microseconds = u128::from(sample_index) * MICROSECONDS_PER_SECOND
        / u128::from(sample_rate_in_hertz.max(1));
    i64::try_from(microseconds).unwrap_or(i64::MAX)
}

/// Whether a frame starting at `frame_start` is the one on screen at
/// `requested_time`, knowing when the next frame starts (`None`: not decoded yet,
/// or there is none when `is_last_frame`).
pub(crate) fn frame_is_on_screen_at(
    frame_start: Timestamp,
    next_frame_start: Option<Timestamp>,
    is_last_frame: bool,
    requested_time: Timestamp,
) -> bool {
    let tolerant_time = requested_time + FRAME_START_TOLERANCE;
    let has_started = frame_start <= tolerant_time;
    let next_has_not_started = match next_frame_start {
        Some(next_frame_start) => tolerant_time < next_frame_start,
        None => is_last_frame,
    };
    has_started && next_has_not_started
}

/// Whether reaching `requested_time` from the frame at `current_frame_start`
/// (`None`: nothing decoded yet) needs a jump to a keyframe rather than decoding
/// forward.
pub(crate) fn needs_jump_to_keyframe(
    current_frame_start: Option<Timestamp>,
    requested_time: Timestamp,
) -> bool {
    let Some(current_frame_start) = current_frame_start else {
        return requested_time > Timestamp::ZERO;
    };
    let is_backwards = requested_time + FRAME_START_TOLERANCE < current_frame_start;
    let is_far_ahead = requested_time
        .span_since(current_frame_start)
        .is_some_and(|distance| distance > MAXIMUM_FORWARD_DECODING_DISTANCE);
    is_backwards || is_far_ahead
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn microseconds_convert_both_ways() {
        let one_and_a_half_seconds = Timestamp::from_milliseconds(1_500);

        assert_eq!(
            timestamp_from_microseconds(1_500_000),
            one_and_a_half_seconds
        );
        assert_eq!(
            microseconds_from_timestamp(one_and_a_half_seconds),
            1_500_000
        );
        assert_eq!(timestamp_from_microseconds(-40), Timestamp::ZERO);
    }

    #[test]
    fn frame_starts_are_rounded_down() {
        assert_eq!(frame_start_in_microseconds(1, FrameRate::FPS_30), 33_333);
        assert_eq!(frame_start_in_microseconds(2, FrameRate::FPS_30), 66_666);
        assert_eq!(
            frame_start_in_microseconds(30, FrameRate::FPS_30),
            1_000_000
        );
        assert_eq!(frame_start_in_microseconds(1, FrameRate::FPS_29_97), 33_366);
    }

    #[test]
    fn sample_starts_are_in_microseconds() {
        assert_eq!(sample_start_in_microseconds(48_000, 48_000), 1_000_000);
        assert_eq!(sample_start_in_microseconds(1_024, 44_100), 23_219);
    }

    #[test]
    fn exact_frame_times_give_that_frame_even_when_rounded_up() {
        let frame_rate = FrameRate::FPS_30;
        let exact_second_frame = frame_rate.timestamp_of_frame(2);
        // A file writer that rounded 66 666.67 µs up instead of down.
        let second_frame_rounded_up = timestamp_from_microseconds(66_667);
        let third_frame = timestamp_from_microseconds(100_000);

        assert!(frame_is_on_screen_at(
            second_frame_rounded_up,
            Some(third_frame),
            false,
            exact_second_frame
        ));
    }

    #[test]
    fn a_frame_stops_being_on_screen_when_the_next_one_starts() {
        let (first, second) = (Timestamp::ZERO, Timestamp::from_milliseconds(40));

        assert!(frame_is_on_screen_at(
            first,
            Some(second),
            false,
            Timestamp::from_milliseconds(39)
        ));
        assert!(!frame_is_on_screen_at(first, Some(second), false, second));
        assert!(!frame_is_on_screen_at(first, None, false, second));
        assert!(frame_is_on_screen_at(
            first,
            None,
            true,
            Timestamp::from_whole_seconds(9)
        ));
    }

    #[test]
    fn jumps_happen_backwards_or_far_ahead_only() {
        let current = Some(Timestamp::from_whole_seconds(5));

        assert!(needs_jump_to_keyframe(
            current,
            Timestamp::from_whole_seconds(4)
        ));
        assert!(needs_jump_to_keyframe(
            current,
            Timestamp::from_whole_seconds(8)
        ));
        assert!(!needs_jump_to_keyframe(
            current,
            Timestamp::from_milliseconds(6_500)
        ));
        assert!(!needs_jump_to_keyframe(None, Timestamp::ZERO));
        assert!(needs_jump_to_keyframe(
            None,
            Timestamp::from_whole_seconds(3)
        ));
    }

    #[test]
    fn the_tolerance_is_one_microsecond() {
        assert_eq!(FRAME_START_TOLERANCE.ticks(), 706);
    }
}
