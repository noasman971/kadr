//! Exact time for the timeline.
//!
//! Video editing needs exact time: a cut placed on frame 1 234 must stay on frame
//! 1 234 after saving, loading, splitting and moving clips. Floating-point seconds
//! drift, so Kadr counts time in integer **ticks**.
//!
//! One tick is a *flick* (1 / 705 600 000 of a second), a unit designed so that
//! the frame durations of all common frame rates (24, 25, 30, 48, 50, 60, 90, 100,
//! 120 fps and their NTSC variants such as 30000/1001) and the sample durations of
//! all common audio sample rates (8 kHz to 192 kHz) are whole numbers of ticks.
//! A `u64` of ticks covers more than 800 years.
//!
//! - [`Timestamp`] is a point in time (on the timeline or inside a source file).
//! - [`TimeSpan`] is a length of time.
//! - [`FrameRate`] converts between ticks and frame numbers.
//!
//! Floating-point seconds are only used at the boundaries: reading user or
//! Claude input ([`Timestamp::from_seconds`]) and displaying values.

use std::fmt;
use std::ops::Add;

use serde::{Deserialize, Serialize};

/// Number of ticks in one second. One tick is one *flick* (1 / 705 600 000 s).
pub const TICKS_PER_SECOND: u64 = 705_600_000;

/// Number of ticks in one millisecond (exact: 705 600).
const TICKS_PER_MILLISECOND: u64 = TICKS_PER_SECOND / 1_000;

/// Errors raised when building time values from outside input.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TimeError {
    /// The number of seconds is negative, infinite or not a number.
    #[error(
        "{value_in_seconds} is not a valid time: it must be a finite number of seconds, zero or more"
    )]
    InvalidSeconds {
        /// The rejected value.
        value_in_seconds: f64,
    },
    /// The number of seconds is too large to be represented.
    #[error("{value_in_seconds} seconds is too large")]
    SecondsTooLarge {
        /// The rejected value.
        value_in_seconds: f64,
    },
    /// A frame rate with a zero numerator or denominator.
    #[error(
        "{numerator}/{denominator} is not a valid frame rate: both numbers must be greater than zero"
    )]
    InvalidFrameRate {
        /// Frames counted.
        numerator: u32,
        /// Seconds during which they are counted.
        denominator: u32,
    },
}

/// Converts a number of seconds coming from outside (user, Claude) into ticks,
/// rounded to the nearest tick.
fn convert_seconds_to_ticks(value_in_seconds: f64) -> Result<u64, TimeError> {
    if !value_in_seconds.is_finite() || value_in_seconds < 0.0 {
        return Err(TimeError::InvalidSeconds { value_in_seconds });
    }
    let ticks = (value_in_seconds * TICKS_PER_SECOND as f64).round();
    // `u64::MAX as f64` rounds up to 2^64, so `>=` rejects every value that does not fit.
    if ticks >= u64::MAX as f64 {
        return Err(TimeError::SecondsTooLarge { value_in_seconds });
    }
    Ok(ticks as u64)
}

/// Writes a number of ticks as seconds with millisecond precision, e.g. `12.345 s`.
fn format_ticks_as_seconds(ticks: u64, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    let whole_seconds = ticks / TICKS_PER_SECOND;
    let milliseconds = (ticks % TICKS_PER_SECOND) / TICKS_PER_MILLISECOND;
    write!(formatter, "{whole_seconds}.{milliseconds:03} s")
}

/// A point in time, in ticks, measured from the start of the timeline or from the
/// start of a source file.
///
/// Serialized as a plain integer number of ticks.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Timestamp {
    ticks: u64,
}

impl Timestamp {
    /// The very beginning.
    pub const ZERO: Self = Self { ticks: 0 };

    /// Creates a timestamp from a number of ticks.
    pub const fn from_ticks(ticks: u64) -> Self {
        Self { ticks }
    }

    /// Creates a timestamp from a whole number of seconds (saturates instead of overflowing).
    pub const fn from_whole_seconds(seconds: u64) -> Self {
        Self::from_ticks(seconds.saturating_mul(TICKS_PER_SECOND))
    }

    /// Creates a timestamp from a whole number of milliseconds (saturates instead of overflowing).
    pub const fn from_milliseconds(milliseconds: u64) -> Self {
        Self::from_ticks(milliseconds.saturating_mul(TICKS_PER_MILLISECOND))
    }

    /// Creates a timestamp from a number of seconds given by a person or by Claude,
    /// rounded to the nearest tick.
    ///
    /// # Errors
    ///
    /// Fails when the value is negative, not finite, or too large.
    ///
    /// # Example
    ///
    /// ```
    /// use kadr_core::time::Timestamp;
    ///
    /// let two_and_a_half_seconds = Timestamp::from_seconds(2.5).unwrap();
    /// assert_eq!(two_and_a_half_seconds, Timestamp::from_milliseconds(2_500));
    /// ```
    pub fn from_seconds(value_in_seconds: f64) -> Result<Self, TimeError> {
        convert_seconds_to_ticks(value_in_seconds).map(Self::from_ticks)
    }

    /// The number of ticks since the origin.
    pub const fn ticks(self) -> u64 {
        self.ticks
    }

    /// The time in seconds, **for display only**: never compute with it.
    pub fn as_seconds(self) -> f64 {
        self.ticks as f64 / TICKS_PER_SECOND as f64
    }

    /// The time `span` later, or `None` if it would overflow.
    pub fn checked_add(self, span: TimeSpan) -> Option<Self> {
        self.ticks.checked_add(span.ticks()).map(Self::from_ticks)
    }

    /// The time `span` earlier, or `None` if it would be before zero.
    pub fn checked_sub(self, span: TimeSpan) -> Option<Self> {
        self.ticks.checked_sub(span.ticks()).map(Self::from_ticks)
    }

    /// The time elapsed since `earlier`, or `None` if `earlier` is after `self`.
    pub fn span_since(self, earlier: Timestamp) -> Option<TimeSpan> {
        self.ticks
            .checked_sub(earlier.ticks)
            .map(TimeSpan::from_ticks)
    }
}

/// Moving a timestamp later. Saturates at the largest representable time
/// (more than 800 years) instead of overflowing.
impl Add<TimeSpan> for Timestamp {
    type Output = Timestamp;

    fn add(self, span: TimeSpan) -> Timestamp {
        Timestamp::from_ticks(self.ticks.saturating_add(span.ticks()))
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        format_ticks_as_seconds(self.ticks, formatter)
    }
}

/// A length of time, in ticks. Never negative.
///
/// Serialized as a plain integer number of ticks.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct TimeSpan {
    ticks: u64,
}

impl TimeSpan {
    /// No time at all.
    pub const ZERO: Self = Self { ticks: 0 };

    /// Creates a span from a number of ticks.
    pub const fn from_ticks(ticks: u64) -> Self {
        Self { ticks }
    }

    /// Creates a span from a whole number of seconds (saturates instead of overflowing).
    pub const fn from_whole_seconds(seconds: u64) -> Self {
        Self::from_ticks(seconds.saturating_mul(TICKS_PER_SECOND))
    }

    /// Creates a span from a whole number of milliseconds (saturates instead of overflowing).
    pub const fn from_milliseconds(milliseconds: u64) -> Self {
        Self::from_ticks(milliseconds.saturating_mul(TICKS_PER_MILLISECOND))
    }

    /// Creates a span from a number of seconds given by a person or by Claude,
    /// rounded to the nearest tick.
    ///
    /// # Errors
    ///
    /// Fails when the value is negative, not finite, or too large.
    pub fn from_seconds(value_in_seconds: f64) -> Result<Self, TimeError> {
        convert_seconds_to_ticks(value_in_seconds).map(Self::from_ticks)
    }

    /// The number of ticks.
    pub const fn ticks(self) -> u64 {
        self.ticks
    }

    /// Whether the span is empty.
    pub const fn is_zero(self) -> bool {
        self.ticks == 0
    }

    /// The length in seconds, **for display only**: never compute with it.
    pub fn as_seconds(self) -> f64 {
        self.ticks as f64 / TICKS_PER_SECOND as f64
    }

    /// The span shortened by `other`, or `None` if `other` is longer.
    pub fn checked_sub(self, other: TimeSpan) -> Option<Self> {
        self.ticks.checked_sub(other.ticks).map(Self::from_ticks)
    }

    /// The span multiplied by `numerator / denominator`, rounded down to a whole tick.
    ///
    /// Used to convert between source time and timeline time when a clip plays
    /// faster or slower. Computed with 128-bit integers, so it never overflows
    /// for realistic ratios. Saturates at the largest span. A zero denominator
    /// gives the largest span.
    pub fn scaled_by_ratio(self, numerator: u64, denominator: u64) -> Self {
        if denominator == 0 {
            return Self::from_ticks(u64::MAX);
        }
        let scaled_ticks = u128::from(self.ticks) * u128::from(numerator) / u128::from(denominator);
        Self::from_ticks(u64::try_from(scaled_ticks).unwrap_or(u64::MAX))
    }

    /// The proportion `self / whole`, between 0 and 1 when `self <= whole`.
    /// Returns 0 when `whole` is empty. For interpolation, not for storage.
    pub fn fraction_of(self, whole: TimeSpan) -> f64 {
        if whole.is_zero() {
            return 0.0;
        }
        self.ticks as f64 / whole.ticks as f64
    }
}

/// Adding two spans. Saturates at the largest representable span instead of overflowing.
impl Add for TimeSpan {
    type Output = TimeSpan;

    fn add(self, other: TimeSpan) -> TimeSpan {
        TimeSpan::from_ticks(self.ticks.saturating_add(other.ticks))
    }
}

impl fmt::Display for TimeSpan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        format_ticks_as_seconds(self.ticks, formatter)
    }
}

/// A frame rate, as an exact fraction: `numerator` frames every `denominator` seconds.
///
/// Integer rates use a denominator of 1 (`30/1`). NTSC rates use 1001
/// (`30000/1001` ≈ 29.97 fps).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "FrameRateFields", into = "FrameRateFields")]
pub struct FrameRate {
    numerator: u32,
    denominator: u32,
}

/// The serialized form of [`FrameRate`], checked on load by [`FrameRate::new`].
#[derive(Serialize, Deserialize)]
struct FrameRateFields {
    numerator: u32,
    denominator: u32,
}

impl TryFrom<FrameRateFields> for FrameRate {
    type Error = TimeError;

    fn try_from(fields: FrameRateFields) -> Result<Self, Self::Error> {
        FrameRate::new(fields.numerator, fields.denominator)
    }
}

impl From<FrameRate> for FrameRateFields {
    fn from(frame_rate: FrameRate) -> Self {
        Self {
            numerator: frame_rate.numerator,
            denominator: frame_rate.denominator,
        }
    }
}

impl FrameRate {
    /// 24 frames per second (cinema).
    pub const FPS_24: Self = Self::from_valid_fraction(24, 1);
    /// 25 frames per second (PAL).
    pub const FPS_25: Self = Self::from_valid_fraction(25, 1);
    /// 30 frames per second.
    pub const FPS_30: Self = Self::from_valid_fraction(30, 1);
    /// 30000/1001 ≈ 29.97 frames per second (NTSC).
    pub const FPS_29_97: Self = Self::from_valid_fraction(30_000, 1_001);
    /// 50 frames per second.
    pub const FPS_50: Self = Self::from_valid_fraction(50, 1);
    /// 60 frames per second.
    pub const FPS_60: Self = Self::from_valid_fraction(60, 1);

    /// Builds a constant whose numbers are known to be valid.
    const fn from_valid_fraction(numerator: u32, denominator: u32) -> Self {
        Self {
            numerator,
            denominator,
        }
    }

    /// Creates a frame rate of `numerator / denominator` frames per second.
    ///
    /// # Errors
    ///
    /// Fails when either number is zero.
    pub fn new(numerator: u32, denominator: u32) -> Result<Self, TimeError> {
        if numerator == 0 || denominator == 0 {
            return Err(TimeError::InvalidFrameRate {
                numerator,
                denominator,
            });
        }
        Ok(Self::from_valid_fraction(numerator, denominator))
    }

    /// Frames counted (the top of the fraction).
    pub const fn numerator(self) -> u32 {
        self.numerator
    }

    /// Seconds during which they are counted (the bottom of the fraction).
    pub const fn denominator(self) -> u32 {
        self.denominator
    }

    /// Frames per second, **for display only**.
    pub fn as_frames_per_second(self) -> f64 {
        f64::from(self.numerator) / f64::from(self.denominator)
    }

    /// The moment frame number `frame_index` starts (frame 0 starts at zero).
    ///
    /// Rounded **up** to a whole tick, so that [`FrameRate::frame_index_at`] of
    /// the result always gives `frame_index` back, even for unusual rates whose
    /// frame duration is not a whole number of ticks.
    pub fn timestamp_of_frame(self, frame_index: u64) -> Timestamp {
        let dividend =
            u128::from(frame_index) * u128::from(self.denominator) * u128::from(TICKS_PER_SECOND);
        let ticks = dividend.div_ceil(u128::from(self.numerator));
        Timestamp::from_ticks(u64::try_from(ticks).unwrap_or(u64::MAX))
    }

    /// The number of the frame being shown at `timestamp` (rounded down).
    pub fn frame_index_at(self, timestamp: Timestamp) -> u64 {
        let frames = u128::from(timestamp.ticks()) * u128::from(self.numerator)
            / (u128::from(self.denominator) * u128::from(TICKS_PER_SECOND));
        u64::try_from(frames).unwrap_or(u64::MAX)
    }
}

impl fmt::Display for FrameRate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.denominator == 1 {
            write!(formatter, "{} fps", self.numerator)
        } else {
            write!(
                formatter,
                "{}/{} fps (≈{:.3})",
                self.numerator,
                self.denominator,
                self.as_frames_per_second()
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_frame_and_sample_durations_are_whole_numbers_of_ticks() {
        for frames_per_second in [24, 25, 30, 48, 50, 60, 90, 100, 120] {
            assert_eq!(
                TICKS_PER_SECOND % frames_per_second,
                0,
                "{frames_per_second} fps"
            );
        }
        // NTSC: one frame lasts 1001/30000 s.
        assert_eq!((TICKS_PER_SECOND * 1_001) % 30_000, 0);
        assert_eq!((TICKS_PER_SECOND * 1_001) % 24_000, 0);
        for sample_rate in [
            8_000, 16_000, 22_050, 32_000, 44_100, 48_000, 96_000, 192_000,
        ] {
            assert_eq!(TICKS_PER_SECOND % sample_rate, 0, "{sample_rate} Hz");
        }
    }

    #[test]
    fn seconds_are_rounded_to_the_nearest_tick() {
        assert_eq!(
            Timestamp::from_seconds(1.5).unwrap(),
            Timestamp::from_milliseconds(1_500)
        );
        assert_eq!(Timestamp::from_seconds(0.0).unwrap(), Timestamp::ZERO);
        assert_eq!(
            TimeSpan::from_seconds(2.0).unwrap(),
            TimeSpan::from_whole_seconds(2)
        );
    }

    #[test]
    fn invalid_seconds_are_rejected() {
        for invalid_value in [-0.5, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(
                Timestamp::from_seconds(invalid_value).is_err(),
                "{invalid_value}"
            );
        }
        assert!(matches!(
            TimeSpan::from_seconds(1e30),
            Err(TimeError::SecondsTooLarge { .. })
        ));
    }

    #[test]
    fn timestamps_and_spans_combine_exactly() {
        let start = Timestamp::from_whole_seconds(10);
        let length = TimeSpan::from_milliseconds(2_500);

        assert_eq!(start + length, Timestamp::from_milliseconds(12_500));
        assert_eq!(
            start.checked_sub(length),
            Some(Timestamp::from_milliseconds(7_500))
        );
        assert_eq!(Timestamp::ZERO.checked_sub(length), None);
        assert_eq!((start + length).span_since(start), Some(length));
        assert_eq!(start.span_since(start + length), None);
    }

    #[test]
    fn spans_scale_by_a_ratio_with_floor_rounding() {
        let three_seconds = TimeSpan::from_whole_seconds(3);
        assert_eq!(
            three_seconds.scaled_by_ratio(1, 2),
            TimeSpan::from_milliseconds(1_500)
        );
        assert_eq!(
            three_seconds.scaled_by_ratio(2, 1),
            TimeSpan::from_whole_seconds(6)
        );
        assert_eq!(
            TimeSpan::from_ticks(10).scaled_by_ratio(1, 3),
            TimeSpan::from_ticks(3)
        );
        assert_eq!(
            three_seconds.scaled_by_ratio(1, 0),
            TimeSpan::from_ticks(u64::MAX)
        );
    }

    #[test]
    fn display_shows_seconds_with_milliseconds() {
        assert_eq!(Timestamp::from_milliseconds(12_345).to_string(), "12.345 s");
        assert_eq!(TimeSpan::from_milliseconds(50).to_string(), "0.050 s");
        assert_eq!(FrameRate::FPS_30.to_string(), "30 fps");
        assert_eq!(FrameRate::FPS_29_97.to_string(), "30000/1001 fps (≈29.970)");
    }

    #[test]
    fn frame_rate_rejects_zero() {
        assert!(FrameRate::new(0, 1).is_err());
        assert!(FrameRate::new(30, 0).is_err());
        assert_eq!(FrameRate::new(25, 1).unwrap(), FrameRate::FPS_25);
    }

    #[test]
    fn frame_numbers_and_timestamps_round_trip() {
        // 11 fps: one frame does not last a whole number of ticks.
        let unusual_rate = FrameRate::new(11, 1).unwrap();
        for frame_rate in [
            FrameRate::FPS_24,
            FrameRate::FPS_29_97,
            FrameRate::FPS_60,
            unusual_rate,
        ] {
            for frame_index in [0, 1, 2, 29, 30, 1_000, 123_456] {
                let frame_start = frame_rate.timestamp_of_frame(frame_index);
                assert_eq!(
                    frame_rate.frame_index_at(frame_start),
                    frame_index,
                    "{frame_rate}"
                );
            }
        }
    }

    #[test]
    fn frame_index_is_rounded_down_inside_a_frame() {
        let frame_rate = FrameRate::FPS_25; // one frame = 40 ms
        assert_eq!(
            frame_rate.frame_index_at(Timestamp::from_milliseconds(39)),
            0
        );
        assert_eq!(
            frame_rate.frame_index_at(Timestamp::from_milliseconds(40)),
            1
        );
        assert_eq!(
            frame_rate.timestamp_of_frame(25),
            Timestamp::from_whole_seconds(1)
        );
    }

    #[test]
    fn frame_rate_serialization_rejects_zero_denominator() {
        let parsed: Result<FrameRate, _> =
            serde_json::from_str(r#"{ "numerator": 30, "denominator": 0 }"#);
        assert!(parsed.is_err());
    }
}
