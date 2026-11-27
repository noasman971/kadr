//! The keyframe gestures of the inspector and the timeline (docs/ux.md, "Animate
//! a clip"): add a keyframe at the playhead, move it, change its curve, remove
//! it. Each function is one undo step.
//!
//! Like the other inspector changes, each request names only what it changes,
//! and reads everything else from the clip as the project has it when the
//! request is applied: the interface's copy of the project can be behind.

use kadr_core::animation::{BezierCurve, BezierHandle, Interpolation, Keyframe};
use kadr_core::clip::{AnimatableProperty, Clip};
use kadr_core::commands::{MoveKeyframe, RemoveKeyframe, SetKeyframe};
use kadr_core::error::EditError;
use kadr_core::history::EditSession;
use kadr_core::ids::ClipId;
use kadr_core::time::{TimeSpan, Timestamp};
use serde::{Deserialize, Serialize};

use super::EditOutcome;
use super::clip_settings::{ClipValue, find_clip};

/// The curve from a keyframe to the next one, as the interface names it.
///
/// `bezier` takes the two handles of a CSS-like `cubic-bezier(x1, y1, x2, y2)`,
/// each from 0 to 1 (the project rules refuse others).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum KeyframeCurve {
    /// Constant speed.
    Linear,
    /// Starts slowly.
    EaseIn,
    /// Slows down as it arrives.
    EaseOut,
    /// Starts and ends slowly.
    EaseInOut,
    /// Keeps the value, then jumps.
    Hold,
    /// A curve shaped by two handles.
    Bezier {
        /// Time fraction of the first handle.
        x1: f32,
        /// Progress fraction of the first handle.
        y1: f32,
        /// Time fraction of the second handle.
        x2: f32,
        /// Progress fraction of the second handle.
        y2: f32,
    },
}

impl From<Interpolation> for KeyframeCurve {
    fn from(interpolation: Interpolation) -> Self {
        match interpolation {
            Interpolation::Linear => Self::Linear,
            Interpolation::EaseIn => Self::EaseIn,
            Interpolation::EaseOut => Self::EaseOut,
            Interpolation::EaseInOut => Self::EaseInOut,
            Interpolation::Hold => Self::Hold,
            Interpolation::CubicBezier(curve) => Self::Bezier {
                x1: curve.start_handle.time_fraction,
                y1: curve.start_handle.progress_fraction,
                x2: curve.end_handle.time_fraction,
                y2: curve.end_handle.progress_fraction,
            },
        }
    }
}

impl From<KeyframeCurve> for Interpolation {
    fn from(curve: KeyframeCurve) -> Self {
        match curve {
            KeyframeCurve::Linear => Self::Linear,
            KeyframeCurve::EaseIn => Self::EaseIn,
            KeyframeCurve::EaseOut => Self::EaseOut,
            KeyframeCurve::EaseInOut => Self::EaseInOut,
            KeyframeCurve::Hold => Self::Hold,
            KeyframeCurve::Bezier { x1, y1, x2, y2 } => Self::CubicBezier(BezierCurve {
                start_handle: BezierHandle {
                    time_fraction: x1,
                    progress_fraction: y1,
                },
                end_handle: BezierHandle {
                    time_fraction: x2,
                    progress_fraction: y2,
                },
            }),
        }
    }
}

/// The time from the clip's start to `time` on the timeline, for a keyframe:
/// from the clip's first frame to before its end.
///
/// # Errors
///
/// When `time` is not on the clip.
pub(super) fn offset_on_clip(clip: &Clip, time: Timestamp) -> Result<TimeSpan, EditError> {
    time.span_since(clip.timeline_start)
        .filter(|_| time < clip.timeline_end())
        .ok_or(EditError::TimeOutsideClip {
            clip_id: clip.id,
            requested_time: time,
            clip_start: clip.timeline_start,
            clip_end: clip.timeline_end(),
        })
}

/// A keyframe at `offset` with `value`: the one already there with its value
/// changed, or a new one taking the curve of the stretch it falls in.
pub(super) fn keyframe_with_value(
    clip: &Clip,
    property: AnimatableProperty,
    offset: TimeSpan,
    value: f32,
) -> Keyframe {
    let animation = clip.animated_property(property);
    match animation.keyframe_at(offset) {
        Some(existing) => Keyframe { value, ..*existing },
        None => Keyframe {
            time_offset: offset,
            value,
            interpolation_to_next: animation.interpolation_in_effect_at(offset),
        },
    }
}

/// Adds a keyframe at `time` (the playhead) holding the value the property has
/// there, so nothing changes on screen until the value is changed. The first
/// keyframe turns the animation on. Nothing happens if one is already there.
///
/// # Errors
///
/// When the clip does not exist or `time` is not on it.
pub fn add_keyframe(
    session: &mut EditSession,
    clip_id: ClipId,
    value: ClipValue,
    time: Timestamp,
) -> Result<EditOutcome, EditError> {
    let property = value.property();
    session.transaction("addKeyframe", |transaction| {
        let keyframe = {
            let (_, clip) = find_clip(transaction.project(), clip_id)?;
            let offset = offset_on_clip(clip, time)?;
            let animation = clip.animated_property(property);
            if animation.keyframe_at(offset).is_some() {
                return Ok(EditOutcome::default());
            }
            keyframe_with_value(clip, property, offset, animation.value_at(offset))
        };
        transaction.execute(SetKeyframe::new(clip_id, property, keyframe))?;
        Ok(EditOutcome::default())
    })
}

/// Moves the keyframes at `from_offset` to `to_offset` (both from the clip's
/// start): those of `values`, or of every property with a keyframe there when
/// `values` is empty (a keyframe mark on the timeline stands for all of them).
///
/// # Errors
///
/// When the clip does not exist, `to_offset` is past its end, a keyframe named
/// is not there, or another one is already at `to_offset`.
pub fn move_keyframes(
    session: &mut EditSession,
    clip_id: ClipId,
    values: &[ClipValue],
    from_offset: TimeSpan,
    to_offset: TimeSpan,
) -> Result<EditOutcome, EditError> {
    session.transaction("moveKeyframe", |transaction| {
        let properties = {
            let (_, clip) = find_clip(transaction.project(), clip_id)?;
            if to_offset > clip.timeline_duration() {
                return Err(EditError::KeyframeOutsideClip {
                    clip_id,
                    time_offset: to_offset,
                    clip_duration: clip.timeline_duration(),
                });
            }
            properties_at(clip, values, from_offset)
        };
        if from_offset != to_offset {
            for property in properties {
                transaction.execute(MoveKeyframe::new(
                    clip_id,
                    property,
                    from_offset,
                    to_offset,
                ))?;
            }
        }
        Ok(EditOutcome::default())
    })
}

/// Changes the curve from the keyframe at `offset` to the next one.
///
/// # Errors
///
/// When the clip or the keyframe does not exist, or the curve is refused (a
/// Bézier handle outside 0–1).
pub fn set_keyframe_curve(
    session: &mut EditSession,
    clip_id: ClipId,
    value: ClipValue,
    offset: TimeSpan,
    curve: KeyframeCurve,
) -> Result<EditOutcome, EditError> {
    let property = value.property();
    session.transaction("changeKeyframeCurve", |transaction| {
        let keyframe = {
            let (_, clip) = find_clip(transaction.project(), clip_id)?;
            *clip.animated_property(property).keyframe_at(offset).ok_or(
                EditError::KeyframeNotFound {
                    clip_id,
                    property,
                    time_offset: offset,
                },
            )?
        };
        transaction.execute(SetKeyframe::new(
            clip_id,
            property,
            Keyframe {
                interpolation_to_next: curve.into(),
                ..keyframe
            },
        ))?;
        Ok(EditOutcome::default())
    })
}

/// Removes the keyframes at `offset`: those of `values`, or of every property
/// with a keyframe there when `values` is empty. A property losing its last
/// keyframe keeps that keyframe's value.
///
/// # Errors
///
/// When the clip does not exist or a keyframe named is not there.
pub fn remove_keyframes(
    session: &mut EditSession,
    clip_id: ClipId,
    values: &[ClipValue],
    offset: TimeSpan,
) -> Result<EditOutcome, EditError> {
    session.transaction("removeKeyframe", |transaction| {
        let properties = {
            let (_, clip) = find_clip(transaction.project(), clip_id)?;
            properties_at(clip, values, offset)
        };
        for property in properties {
            transaction.execute(
                RemoveKeyframe::new(clip_id, property, offset).keeping_the_value_when_last(),
            )?;
        }
        Ok(EditOutcome::default())
    })
}

/// The properties of `values`, or every property with a keyframe at `offset`
/// when `values` is empty.
fn properties_at(clip: &Clip, values: &[ClipValue], offset: TimeSpan) -> Vec<AnimatableProperty> {
    if values.is_empty() {
        AnimatableProperty::ALL
            .into_iter()
            .filter(|property| {
                clip.animated_property(*property)
                    .keyframe_at(offset)
                    .is_some()
            })
            .collect()
    } else {
        values.iter().map(|value| value.property()).collect()
    }
}
