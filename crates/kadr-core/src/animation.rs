//! Animated values: numbers that can change over the duration of a clip.
//!
//! An [`AnimatedValue`] is either a constant (no keyframes) or a curve defined by
//! [`Keyframe`]s. Keyframe times are measured **from the start of the clip**, so
//! moving a clip on the timeline moves its animation with it.
//!
//! Between two keyframes, the value is interpolated according to the first
//! keyframe's [`Interpolation`]. Before the first keyframe the value is the first
//! keyframe's value; after the last one, the last keyframe's value.

use serde::{Deserialize, Serialize};

use crate::time::TimeSpan;

/// How the value travels from one keyframe to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    /// Straight line, constant speed.
    #[default]
    Linear,
    /// Starts slowly, speeds up, slows down before the next keyframe.
    EaseInOut,
    /// Keeps the value, then jumps to the next keyframe's value.
    Hold,
}

impl Interpolation {
    /// Maps the elapsed proportion between two keyframes (0 to 1) to the
    /// proportion of the value change already done (0 to 1).
    fn apply_to_progress(self, linear_progress: f64) -> f64 {
        match self {
            Interpolation::Linear => linear_progress,
            // Smoothstep: 3p² − 2p³, flat at both ends.
            Interpolation::EaseInOut => {
                linear_progress * linear_progress * (3.0 - 2.0 * linear_progress)
            }
            Interpolation::Hold => 0.0,
        }
    }
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
    /// is exactly the original animation. With ease-in-out, the segment containing
    /// the cut is eased again over each shorter half, so it changes slightly.
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

    /// The interpolation of the segment containing `time_offset`.
    fn interpolation_in_effect_at(&self, time_offset: TimeSpan) -> Interpolation {
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
}
