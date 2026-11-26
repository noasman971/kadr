//! Conversions between FFmpeg timestamps and Kadr time.
//!
//! FFmpeg counts time in "time base" units, a fraction of a second that differs
//! per stream (for example 1/15360 or 1/48000). Kadr counts in ticks. All
//! conversions use 128-bit integers, so they are exact up to rounding down.

use ffmpeg_next::Rational;
use kadr_core::time::{TICKS_PER_SECOND, Timestamp};

/// FFmpeg's own time base for container-level timestamps: microseconds.
const FFMPEG_TICKS_PER_SECOND: i128 = 1_000_000;

/// Converts `value` (counted in `time_base` units) into a Kadr timestamp.
/// Negative values become zero.
pub(crate) fn timestamp_from_stream_units(value: i64, time_base: Rational) -> Timestamp {
    let (numerator, denominator) = (
        i128::from(time_base.numerator()),
        i128::from(time_base.denominator()),
    );
    if value <= 0 || numerator <= 0 || denominator <= 0 {
        return Timestamp::ZERO;
    }
    let ticks = i128::from(value) * numerator * i128::from(TICKS_PER_SECOND) / denominator;
    Timestamp::from_ticks(u64::try_from(ticks).unwrap_or(u64::MAX))
}

/// Converts a Kadr timestamp into microseconds, FFmpeg's container time unit.
pub(crate) fn microseconds_from_timestamp(time: Timestamp) -> i64 {
    let microseconds =
        i128::from(time.ticks()) * FFMPEG_TICKS_PER_SECOND / i128::from(TICKS_PER_SECOND);
    i64::try_from(microseconds).unwrap_or(i64::MAX)
}

/// Writes a timestamp as decimal seconds for FFmpeg filter arguments
/// (microsecond precision).
pub(crate) fn seconds_for_filter_argument(time: Timestamp) -> String {
    let microseconds = microseconds_from_timestamp(time);
    format!(
        "{}.{:06}",
        microseconds / 1_000_000,
        microseconds % 1_000_000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_units_convert_exactly() {
        let time_base = Rational::new(1, 15_360);
        assert_eq!(
            timestamp_from_stream_units(23_040, time_base),
            Timestamp::from_milliseconds(1_500)
        );
    }

    #[test]
    fn negative_or_invalid_values_become_zero() {
        assert_eq!(
            timestamp_from_stream_units(-5, Rational::new(1, 1_000)),
            Timestamp::ZERO
        );
        assert_eq!(
            timestamp_from_stream_units(5, Rational::new(0, 0)),
            Timestamp::ZERO
        );
    }

    #[test]
    fn filter_arguments_use_microseconds() {
        assert_eq!(
            seconds_for_filter_argument(Timestamp::from_milliseconds(2_005)),
            "2.005000"
        );
        assert_eq!(
            microseconds_from_timestamp(Timestamp::from_whole_seconds(3)),
            3_000_000
        );
    }
}
