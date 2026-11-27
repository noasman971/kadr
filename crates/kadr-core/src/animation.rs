//! Animated values: numbers that can change over the duration of a clip.
//!
//! An [`AnimatedValue`] is either a constant (no keyframes) or a curve defined by
//! [`Keyframe`]s. Keyframe times are measured **from the start of the clip**, so
//! moving a clip on the timeline moves its animation with it.
//!
//! Between two keyframes, the value follows the first keyframe's
//! [`Interpolation`]: a straight line, an eased curve, a hold, or a cubic Bézier
//! curve ([`BezierCurve`]). Before the first keyframe the value is the first
//! keyframe's value; after the last one, the last keyframe's value. Every curve
//! stays between the values of its two keyframes, so the rules on keyframe values
//! cover every moment of the animation.

use serde::{Deserialize, Serialize};

use crate::time::TimeSpan;

/// How the value travels from one keyframe to the next.
///
/// Saved as `"linear"`, `"ease_in"`, `"ease_out"`, `"ease_in_out"`, `"hold"`, or
/// `{"cubic_bezier": {…}}`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    /// Straight line, constant speed.
    #[default]
    Linear,
    /// Starts slowly and speeds up until the next keyframe (p²).
    EaseIn,
    /// Starts fast and slows down before the next keyframe (1 − (1 − p)²).
    EaseOut,
    /// Starts slowly, speeds up, slows down before the next keyframe (smoothstep,
    /// 3p² − 2p³).
    EaseInOut,
    /// Keeps the value, then jumps to the next keyframe's value.
    Hold,
    /// A curve shaped by two handles, like CSS `cubic-bezier()`.
    CubicBezier(BezierCurve),
}

impl Interpolation {
    /// The same curve as a cubic Bézier curve, or `None` for [`Interpolation::Hold`]
    /// (a jump is not a curve).
    ///
    /// The presets are cubic Bézier curves whose handles sit at a third and two
    /// thirds of the time, so a curve editor can start from them without changing
    /// the animation.
    pub fn as_bezier_curve(self) -> Option<BezierCurve> {
        let curve = |start_progress, end_progress| {
            Some(BezierCurve {
                start_handle: BezierHandle {
                    time_fraction: ONE_THIRD,
                    progress_fraction: start_progress,
                },
                end_handle: BezierHandle {
                    time_fraction: TWO_THIRDS,
                    progress_fraction: end_progress,
                },
            })
        };
        match self {
            Interpolation::Linear => curve(ONE_THIRD, TWO_THIRDS),
            Interpolation::EaseIn => curve(0.0, ONE_THIRD),
            Interpolation::EaseOut => curve(TWO_THIRDS, 1.0),
            Interpolation::EaseInOut => curve(0.0, 1.0),
            Interpolation::Hold => None,
            Interpolation::CubicBezier(curve) => Some(curve),
        }
    }

    /// Maps the elapsed proportion between two keyframes (0 to 1) to the
    /// proportion of the value change already done (0 to 1).
    fn apply_to_progress(self, linear_progress: f64) -> f64 {
        let progress = linear_progress.clamp(0.0, 1.0);
        let eased_progress = match self {
            Interpolation::Linear => progress,
            Interpolation::EaseIn => progress * progress,
            Interpolation::EaseOut => 1.0 - (1.0 - progress) * (1.0 - progress),
            Interpolation::EaseInOut => progress * progress * (3.0 - 2.0 * progress),
            Interpolation::Hold => 0.0,
            Interpolation::CubicBezier(curve) => curve.progress_at(progress),
        };
        eased_progress.clamp(0.0, 1.0)
    }
}

const ONE_THIRD: f32 = 1.0 / 3.0;
const TWO_THIRDS: f32 = 2.0 / 3.0;

/// One handle of a [`BezierCurve`], as fractions of the stretch between two
/// keyframes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BezierHandle {
    /// Along the time between the two keyframes: 0 at the first, 1 at the next.
    pub time_fraction: f32,
    /// Along the value change between them: 0 at the first keyframe's value, 1 at
    /// the next one's.
    pub progress_fraction: f32,
}

impl BezierHandle {
    /// Whether both fractions are finite numbers from 0 to 1, as the project
    /// rules require.
    pub fn is_inside_the_unit_square(self) -> bool {
        [self.time_fraction, self.progress_fraction]
            .into_iter()
            .all(|fraction| fraction.is_finite() && (0.0..=1.0).contains(&fraction))
    }
}

/// A cubic Bézier timing curve between two keyframes, like CSS
/// `cubic-bezier(x1, y1, x2, y2)`.
///
/// It is drawn in a unit square: time goes from 0 (this keyframe) to 1 (the next
/// one) along x, and the value change done so far from 0 to 1 along y. The curve
/// starts at (0, 0), ends at (1, 1), and leans towards its two handles.
///
/// The project rules keep every handle coordinate from 0 to 1 (see
/// [`BezierHandle::is_inside_the_unit_square`]). Then the curve never goes back in
/// time, and the value always stays between the values of the two keyframes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BezierCurve {
    /// The handle leaving this keyframe (x1, y1).
    pub start_handle: BezierHandle,
    /// The handle arriving at the next keyframe (x2, y2).
    pub end_handle: BezierHandle,
}

/// How close the curve's x must come to the asked time.
const BEZIER_TIME_TOLERANCE: f64 = 1e-9;
/// Newton's method usually settles in two to four steps.
const BEZIER_NEWTON_STEPS: usize = 8;
/// Bisection, when Newton's method does not settle (flat stretches of x).
const BEZIER_BISECTION_STEPS: usize = 64;

impl BezierCurve {
    /// Whether both handles are inside the unit square, as the project rules
    /// require.
    pub fn has_handles_inside_the_unit_square(self) -> bool {
        self.start_handle.is_inside_the_unit_square() && self.end_handle.is_inside_the_unit_square()
    }

    /// The proportion of the value change done when `time_progress` (0 to 1) of
    /// the time between the keyframes has passed.
    ///
    /// Finds the curve parameter whose x is `time_progress` (Newton's method, then
    /// bisection if it does not settle), and returns its y. The preview and the
    /// export run these same steps, so both show the same value.
    pub fn progress_at(self, time_progress: f64) -> f64 {
        let x1 = f64::from(self.start_handle.time_fraction);
        let x2 = f64::from(self.end_handle.time_fraction);
        let y1 = f64::from(self.start_handle.progress_fraction);
        let y2 = f64::from(self.end_handle.progress_fraction);
        let parameter = curve_parameter_at(x1, x2, time_progress.clamp(0.0, 1.0));
        cubic_bezier_coordinate(y1, y2, parameter)
    }
}

/// One coordinate of a cubic Bézier curve going from 0 to 1, whose two middle
/// control points have the coordinates `first` and `second`, at parameter `t`.
fn cubic_bezier_coordinate(first: f64, second: f64, t: f64) -> f64 {
    let rest = 1.0 - t;
    3.0 * rest * rest * t * first + 3.0 * rest * t * t * second + t * t * t
}

/// The derivative of [`cubic_bezier_coordinate`] with respect to `t`.
fn cubic_bezier_slope(first: f64, second: f64, t: f64) -> f64 {
    let rest = 1.0 - t;
    3.0 * rest * rest * first + 6.0 * rest * t * (second - first) + 3.0 * t * t * (1.0 - second)
}

/// The parameter where the curve's x is `target`. With both x controls from 0 to
/// 1, x never decreases, so bisection always finds it.
fn curve_parameter_at(x1: f64, x2: f64, target: f64) -> f64 {
    let mut parameter = target;
    for _ in 0..BEZIER_NEWTON_STEPS {
        let error = cubic_bezier_coordinate(x1, x2, parameter) - target;
        if error.abs() < BEZIER_TIME_TOLERANCE {
            return parameter;
        }
        let slope = cubic_bezier_slope(x1, x2, parameter);
        if slope.abs() < BEZIER_TIME_TOLERANCE {
            break;
        }
        parameter -= error / slope;
        if !(0.0..=1.0).contains(&parameter) {
            break;
        }
    }
    let (mut low, mut high) = (0.0_f64, 1.0_f64);
    parameter = target;
    for _ in 0..BEZIER_BISECTION_STEPS {
        let error = cubic_bezier_coordinate(x1, x2, parameter) - target;
        if error.abs() < BEZIER_TIME_TOLERANCE {
            break;
        }
        if error < 0.0 {
            low = parameter;
        } else {
            high = parameter;
        }
        parameter = (low + high) / 2.0;
    }
    parameter
}

/// A value fixed at a given moment of a clip.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    /// When, measured from the start of the clip.
    pub time_offset: TimeSpan,
    /// The value at that moment.
    pub value: f32,
    /// How to reach the next keyframe.
    pub interpolation_to_next: Interpolation,
}

/// A number that is either constant or animated by keyframes.
///
/// Keyframes are always sorted by time, with at most one keyframe per moment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimatedValue {
    /// The value used when there are no keyframes.
    pub constant_value: f32,
    keyframes: Vec<Keyframe>,
}

impl AnimatedValue {
    /// A value that never changes.
    pub const fn constant(value: f32) -> Self {
        Self {
            constant_value: value,
            keyframes: Vec::new(),
        }
    }

    /// The keyframes, sorted by time. Empty when the value is constant.
    pub fn keyframes(&self) -> &[Keyframe] {
        &self.keyframes
    }

    /// Whether the value changes over time.
    pub fn is_animated(&self) -> bool {
        !self.keyframes.is_empty()
    }

    /// Every value the property can take: the constant value and each keyframe value.
    pub fn all_values(&self) -> impl Iterator<Item = f32> + '_ {
        std::iter::once(self.constant_value)
            .chain(self.keyframes.iter().map(|keyframe| keyframe.value))
    }

    /// Whether the keyframes are sorted with strictly increasing times.
    /// Always true for values built through this API; checked when loading files.
    pub fn has_strictly_increasing_keyframe_times(&self) -> bool {
        self.keyframes
            .windows(2)
            .all(|pair| pair[0].time_offset < pair[1].time_offset)
    }

    /// The value at `time_offset` from the start of the clip.
    pub fn value_at(&self, time_offset: TimeSpan) -> f32 {
        let (Some(first_keyframe), Some(last_keyframe)) =
            (self.keyframes.first(), self.keyframes.last())
        else {
            return self.constant_value;
        };
        if time_offset <= first_keyframe.time_offset {
            return first_keyframe.value;
        }
        if time_offset >= last_keyframe.time_offset {
            return last_keyframe.value;
        }
        let index_of_next_keyframe = self
            .keyframes
            .partition_point(|keyframe| keyframe.time_offset <= time_offset);
        let previous_keyframe = self.keyframes[index_of_next_keyframe - 1];
        let next_keyframe = self.keyframes[index_of_next_keyframe];
        interpolate_between(previous_keyframe, next_keyframe, time_offset)
    }

    /// Adds a keyframe, or replaces the one already at the same moment.
    pub fn set_keyframe(&mut self, keyframe: Keyframe) {
        match self
            .keyframes
            .binary_search_by(|existing| existing.time_offset.cmp(&keyframe.time_offset))
        {
            Ok(existing_index) => self.keyframes[existing_index] = keyframe,
            Err(insertion_index) => self.keyframes.insert(insertion_index, keyframe),
        }
    }

    /// Removes the keyframe at exactly `time_offset` and returns it, if there is one.
    pub fn remove_keyframe_at(&mut self, time_offset: TimeSpan) -> Option<Keyframe> {
        let index = self
            .keyframes
            .binary_search_by(|existing| existing.time_offset.cmp(&time_offset))
            .ok()?;
        Some(self.keyframes.remove(index))
    }

    /// The keyframe at exactly `time_offset`, if there is one.
    pub fn keyframe_at(&self, time_offset: TimeSpan) -> Option<&Keyframe> {
        self.keyframes
            .iter()
            .find(|keyframe| keyframe.time_offset == time_offset)
    }

    /// Cuts the animation in two at `split_offset`, for splitting a clip.
    ///
    /// The first part ends with a keyframe holding the value at the cut, and the
    /// second part starts with one. With linear and hold interpolation the result
    /// is exactly the original animation. With the eased and Bézier curves, the
    /// stretch containing the cut follows the same curve again over each shorter
    /// part, so it changes slightly.
    pub fn split_at(&self, split_offset: TimeSpan) -> (AnimatedValue, AnimatedValue) {
        if !self.is_animated() {
            return (self.clone(), self.clone());
        }
        let value_at_cut = self.value_at(split_offset);
        let interpolation_at_cut = self.interpolation_in_effect_at(split_offset);

        let mut first_part = AnimatedValue {
            constant_value: self.constant_value,
            keyframes: self
                .keyframes
                .iter()
                .copied()
                .filter(|keyframe| keyframe.time_offset < split_offset)
                .collect(),
        };
        first_part.set_keyframe(Keyframe {
            time_offset: split_offset,
            value: value_at_cut,
            interpolation_to_next: interpolation_at_cut,
        });

        let mut second_part = self.without_keyframes_before(split_offset);
        if second_part.keyframe_at(TimeSpan::ZERO).is_none() {
            second_part.set_keyframe(Keyframe {
                time_offset: TimeSpan::ZERO,
                value: value_at_cut,
                interpolation_to_next: interpolation_at_cut,
            });
        }
        (first_part, second_part)
    }

    /// The animation as seen by a clip whose start moved `removed_span` later
    /// (trimming its beginning): keyframes move earlier by `removed_span`, and those
    /// that fall before the new start are replaced by one keyframe at the new start.
    pub fn after_removing_start(&self, removed_span: TimeSpan) -> AnimatedValue {
        self.split_at(removed_span).1
    }

    /// The animation as seen by a clip whose start moved `added_span` earlier
    /// (extending its beginning): every keyframe moves later by `added_span`.
    pub fn after_extending_start(&self, added_span: TimeSpan) -> AnimatedValue {
        AnimatedValue {
            constant_value: self.constant_value,
            keyframes: self
                .keyframes
                .iter()
                .map(|keyframe| Keyframe {
                    time_offset: keyframe.time_offset + added_span,
                    ..*keyframe
                })
                .collect(),
        }
    }

    /// Keyframes at or after `offset`, moved earlier by `offset`.
    fn without_keyframes_before(&self, offset: TimeSpan) -> AnimatedValue {
        AnimatedValue {
            constant_value: self.constant_value,
            keyframes: self
                .keyframes
                .iter()
                .filter_map(|keyframe| {
                    keyframe
                        .time_offset
                        .checked_sub(offset)
                        .map(|shifted_offset| Keyframe {
                            time_offset: shifted_offset,
                            ..*keyframe
                        })
                })
                .collect(),
        }
    }

    /// The interpolation of the stretch containing `time_offset`: the one of the
    /// last keyframe at or before it, or of the first keyframe before the
    /// animation starts. Linear when there are no keyframes. A keyframe added in
    /// the middle of a stretch takes it, so both halves keep the same kind of
    /// curve.
    pub fn interpolation_in_effect_at(&self, time_offset: TimeSpan) -> Interpolation {
        self.keyframes
            .iter()
            .rev()
            .find(|keyframe| keyframe.time_offset <= time_offset)
            .or(self.keyframes.first())
            .map_or(Interpolation::Linear, |keyframe| {
                keyframe.interpolation_to_next
            })
    }
}

/// The value between two consecutive keyframes.
fn interpolate_between(
    previous_keyframe: Keyframe,
    next_keyframe: Keyframe,
    time_offset: TimeSpan,
) -> f32 {
    let elapsed = time_offset
        .checked_sub(previous_keyframe.time_offset)
        .unwrap_or(TimeSpan::ZERO);
    let segment_length = next_keyframe
        .time_offset
        .checked_sub(previous_keyframe.time_offset)
        .unwrap_or(TimeSpan::ZERO);
    let linear_progress = elapsed.fraction_of(segment_length).clamp(0.0, 1.0);
    let eased_progress = previous_keyframe
        .interpolation_to_next
        .apply_to_progress(linear_progress);
    let value_change = f64::from(next_keyframe.value) - f64::from(previous_keyframe.value);
    (f64::from(previous_keyframe.value) + value_change * eased_progress) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seconds(value: u64) -> TimeSpan {
        TimeSpan::from_whole_seconds(value)
    }

    fn keyframe(at_second: u64, value: f32, interpolation_to_next: Interpolation) -> Keyframe {
        Keyframe {
            time_offset: seconds(at_second),
            value,
            interpolation_to_next,
        }
    }

    /// Fades from 0 at 2 s to 1 at 4 s, linearly.
    fn linear_fade_in() -> AnimatedValue {
        let mut fade = AnimatedValue::constant(1.0);
        fade.set_keyframe(keyframe(4, 1.0, Interpolation::Linear));
        fade.set_keyframe(keyframe(2, 0.0, Interpolation::Linear));
        fade
    }

    #[test]
    fn constant_value_is_used_without_keyframes() {
        let opacity = AnimatedValue::constant(0.5);
        assert!(!opacity.is_animated());
        assert_eq!(opacity.value_at(seconds(100)), 0.5);
    }

    #[test]
    fn keyframes_are_kept_sorted_and_unique() {
        let mut fade = linear_fade_in();
        fade.set_keyframe(keyframe(2, 0.25, Interpolation::Hold));

        let times: Vec<TimeSpan> = fade.keyframes().iter().map(|k| k.time_offset).collect();
        assert_eq!(times, vec![seconds(2), seconds(4)]);
        assert_eq!(fade.keyframes()[0].value, 0.25);
        assert!(fade.has_strictly_increasing_keyframe_times());
    }

    #[test]
    fn linear_interpolation_between_keyframes() {
        let fade = linear_fade_in();
        assert_eq!(fade.value_at(seconds(0)), 0.0);
        assert_eq!(fade.value_at(seconds(3)), 0.5);
        assert_eq!(fade.value_at(seconds(10)), 1.0);
    }

    #[test]
    fn ease_in_out_is_symmetric_and_flat_at_the_ends() {
        let mut eased = AnimatedValue::constant(0.0);
        eased.set_keyframe(keyframe(0, 0.0, Interpolation::EaseInOut));
        eased.set_keyframe(keyframe(10, 100.0, Interpolation::Linear));

        assert_eq!(eased.value_at(seconds(5)), 50.0);
        assert!(eased.value_at(seconds(1)) < 10.0, "slow start");
        assert!(eased.value_at(seconds(9)) > 90.0, "slow end");
    }

    #[test]
    fn hold_keeps_the_value_until_the_next_keyframe() {
        let mut steps = AnimatedValue::constant(0.0);
        steps.set_keyframe(keyframe(0, 1.0, Interpolation::Hold));
        steps.set_keyframe(keyframe(2, 3.0, Interpolation::Hold));

        assert_eq!(steps.value_at(TimeSpan::from_milliseconds(1_999)), 1.0);
        assert_eq!(steps.value_at(seconds(2)), 3.0);
    }

    #[test]
    fn removing_a_keyframe_returns_it() {
        let mut fade = linear_fade_in();
        let removed = fade.remove_keyframe_at(seconds(2)).unwrap();
        assert_eq!(removed.value, 0.0);
        assert_eq!(fade.remove_keyframe_at(seconds(3)), None);
        assert_eq!(fade.keyframes().len(), 1);
    }

    #[test]
    fn splitting_preserves_the_animation_on_both_sides() {
        let fade = linear_fade_in();
        let (first_part, second_part) = fade.split_at(seconds(3));

        for offset_in_milliseconds in [0, 1_000, 2_000, 2_500, 3_000] {
            let offset = TimeSpan::from_milliseconds(offset_in_milliseconds);
            assert_eq!(first_part.value_at(offset), fade.value_at(offset));
        }
        for offset_in_milliseconds in [0, 500, 1_000, 5_000] {
            let offset_in_second_part = TimeSpan::from_milliseconds(offset_in_milliseconds);
            let offset_in_original = seconds(3) + offset_in_second_part;
            assert_eq!(
                second_part.value_at(offset_in_second_part),
                fade.value_at(offset_in_original)
            );
        }
    }

    #[test]
    fn splitting_a_constant_value_gives_two_constants() {
        let (first_part, second_part) = AnimatedValue::constant(0.7).split_at(seconds(1));
        assert_eq!(first_part, AnimatedValue::constant(0.7));
        assert_eq!(second_part, AnimatedValue::constant(0.7));
    }

    #[test]
    fn trimming_and_extending_the_start_shift_keyframes() {
        let fade = linear_fade_in();

        let trimmed = fade.after_removing_start(seconds(1));
        assert_eq!(trimmed.value_at(seconds(2)), fade.value_at(seconds(3)));

        let extended = fade.after_extending_start(seconds(5));
        assert_eq!(extended.keyframes()[0].time_offset, seconds(7));
        assert_eq!(extended.value_at(seconds(8)), fade.value_at(seconds(3)));
    }

    /// From 0 at 0 s to 100 at 10 s with `interpolation`.
    fn ramp(interpolation: Interpolation) -> AnimatedValue {
        let mut ramp = AnimatedValue::constant(0.0);
        ramp.set_keyframe(keyframe(0, 0.0, interpolation));
        ramp.set_keyframe(keyframe(10, 100.0, Interpolation::Linear));
        ramp
    }

    fn bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> Interpolation {
        Interpolation::CubicBezier(BezierCurve {
            start_handle: BezierHandle {
                time_fraction: x1,
                progress_fraction: y1,
            },
            end_handle: BezierHandle {
                time_fraction: x2,
                progress_fraction: y2,
            },
        })
    }

    #[test]
    fn ease_in_starts_slowly_and_ease_out_ends_slowly() {
        let ease_in = ramp(Interpolation::EaseIn);
        assert_eq!(ease_in.value_at(seconds(5)), 25.0);
        assert!(ease_in.value_at(seconds(1)) < 2.0, "slow start");

        let ease_out = ramp(Interpolation::EaseOut);
        assert_eq!(ease_out.value_at(seconds(5)), 75.0);
        assert!(ease_out.value_at(seconds(9)) > 98.0, "slow end");
    }

    #[test]
    fn every_curve_starts_and_ends_on_its_keyframes() {
        for interpolation in [
            Interpolation::Linear,
            Interpolation::EaseIn,
            Interpolation::EaseOut,
            Interpolation::EaseInOut,
            bezier(0.9, 0.0, 0.1, 1.0),
            bezier(0.0, 1.0, 1.0, 0.0),
        ] {
            let animation = ramp(interpolation);
            assert_eq!(animation.value_at(seconds(0)), 0.0, "{interpolation:?}");
            assert_eq!(animation.value_at(seconds(10)), 100.0, "{interpolation:?}");
        }
    }

    #[test]
    fn the_presets_are_the_same_as_their_bezier_curves() {
        for preset in [
            Interpolation::Linear,
            Interpolation::EaseIn,
            Interpolation::EaseOut,
            Interpolation::EaseInOut,
        ] {
            let curve = preset.as_bezier_curve().unwrap();
            assert!(curve.has_handles_inside_the_unit_square());
            let as_preset = ramp(preset);
            let as_curve = ramp(Interpolation::CubicBezier(curve));
            for millisecond in (0..=10_000).step_by(250) {
                let offset = TimeSpan::from_milliseconds(millisecond);
                let difference = (as_preset.value_at(offset) - as_curve.value_at(offset)).abs();
                assert!(
                    difference < 1e-3,
                    "{preset:?} at {millisecond} ms: {difference}"
                );
            }
        }
        assert_eq!(Interpolation::Hold.as_bezier_curve(), None);
    }

    #[test]
    fn a_bezier_curve_follows_its_handles() {
        // CSS "ease-out": quick at first, then slow.
        let css_ease_out = ramp(bezier(0.0, 0.0, 0.58, 1.0));
        let at_quarter = css_ease_out.value_at(TimeSpan::from_milliseconds(2_500));
        // Solved by hand: x(t) = 0.25 at t ≈ 0.41803, where y ≈ 0.37810.
        assert!((at_quarter - 37.81).abs() < 0.01, "{at_quarter}");
        // Values only go up, and stay between the keyframes' values.
        let mut previous = 0.0;
        for millisecond in (0..=10_000).step_by(100) {
            let value = css_ease_out.value_at(TimeSpan::from_milliseconds(millisecond));
            assert!(
                value >= previous && value <= 100.0,
                "{value} at {millisecond} ms"
            );
            previous = value;
        }
    }

    #[test]
    fn a_bezier_curve_with_vertical_stretches_is_still_solved() {
        // Both handles on the left edge: x is nearly flat at the start, where
        // Newton's method has no slope to follow.
        let steep = BezierCurve {
            start_handle: BezierHandle {
                time_fraction: 0.0,
                progress_fraction: 1.0,
            },
            end_handle: BezierHandle {
                time_fraction: 0.0,
                progress_fraction: 1.0,
            },
        };
        for step in 0..=100 {
            let time_progress = f64::from(step) / 100.0;
            let progress = steep.progress_at(time_progress);
            assert!((0.0..=1.0).contains(&progress), "{progress}");
        }
        assert!(steep.progress_at(0.01) > 0.5, "a fast start");
    }

    #[test]
    fn handles_outside_the_unit_square_are_detected() {
        let Interpolation::CubicBezier(mut curve) = bezier(0.2, 0.0, 0.8, 1.0) else {
            unreachable!()
        };
        assert!(curve.has_handles_inside_the_unit_square());
        curve.end_handle.progress_fraction = 1.5;
        assert!(!curve.has_handles_inside_the_unit_square());
        curve.end_handle.progress_fraction = f32::NAN;
        assert!(!curve.has_handles_inside_the_unit_square());
    }

    #[test]
    fn curves_are_saved_with_readable_names() {
        let saved = |interpolation| serde_json::to_string(&interpolation).unwrap();
        assert_eq!(saved(Interpolation::EaseIn), "\"ease_in\"");
        assert_eq!(saved(Interpolation::EaseInOut), "\"ease_in_out\"");
        assert_eq!(
            saved(bezier(0.25, 0.0, 0.75, 1.0)),
            "{\"cubic_bezier\":{\"start_handle\":{\"time_fraction\":0.25,\"progress_fraction\":0.0},\
             \"end_handle\":{\"time_fraction\":0.75,\"progress_fraction\":1.0}}}"
        );
    }

    #[test]
    fn a_keyframe_in_the_middle_takes_the_curve_of_its_stretch() {
        let mut animation = ramp(Interpolation::EaseOut);
        animation.set_keyframe(keyframe(10, 100.0, Interpolation::Hold));
        assert_eq!(
            animation.interpolation_in_effect_at(seconds(4)),
            Interpolation::EaseOut
        );
        assert_eq!(
            animation.interpolation_in_effect_at(seconds(12)),
            Interpolation::Hold
        );
        assert_eq!(
            AnimatedValue::constant(1.0).interpolation_in_effect_at(seconds(1)),
            Interpolation::Linear
        );
    }
}
