//! Keyframe tools: animating a clip property with keyframes and curves, the way
//! the app's inspector does (decision record 0054).

use kadr_core::animation::{BezierCurve, BezierHandle, Interpolation, Keyframe};
use kadr_core::clip::{AnimatableProperty, Clip, ClipContent};
use kadr_core::commands::{RemoveKeyframe, SetKeyframe};
use kadr_core::ids::ClipId;
use kadr_core::project::Project;
use kadr_core::time::TimeSpan;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::catalog::{ToolEffect, ToolEntry, declare_tool};
use crate::output::{ToolError, ToolOutput};
use crate::time_input::{seconds_in, span_from_seconds};
use crate::toolbox::ToolCall;
use crate::tools::timeline_tools::clip_details;
use crate::views::{claude_value_offset, property_name};

/// The keyframe tools.
pub(crate) fn keyframe_tools() -> Vec<ToolEntry> {
    vec![declare_tool(
        "set_keyframes",
        "Animate a clip property with keyframes",
        SET_KEYFRAMES_DESCRIPTION,
        ToolEffect::ChangesProject,
        set_keyframes,
    )]
}

// ----- set_keyframes -----

const SET_KEYFRAMES_DESCRIPTION: &str = "\
Animates one property of a clip with keyframes, replacing the keyframes it has (get_clip lists \
them under \"animations\"). Each keyframe has a time from the clip's start \
(time_in_clip_in_seconds, rounded to the nearest frame, from 0 to the clip's duration), a \
value, and the curve to the next keyframe: \"linear\" (default), \"ease_in\" (starts slowly), \
\"ease_out\" (slows down as it arrives: good for something sliding or growing in), \
\"ease_in_out\", \"hold\" (keeps the value, then jumps), or \"bezier\" with bezier_handles \
[x1, y1, x2, y2] as in CSS cubic-bezier(), all four from 0 to 1 (so values never overshoot). \
Values use the units of set_transform and set_clip_volume: center_x_in_pixels and \
center_y_in_pixels in canvas pixels from the top-left corner (they may be off the canvas to \
come in from outside), scale (1 fits the canvas), rotation_in_degrees, opacity from 0 to 1, \
volume from 0 to 4. Before the first keyframe the value is the first keyframe's, after the last \
one the last keyframe's. An empty list removes the animation: the property keeps the value of \
its last keyframe. The person can see and edit the keyframes in the app's inspector. Check \
with get_frame at a few times.

Example, a title sliding in from the left over 0.8 s in a 1080×1920 project: {\"clip_id\": 7, \
\"property\": \"center_x_in_pixels\", \"keyframes\": [{\"time_in_clip_in_seconds\": 0, \
\"value\": -300, \"curve\": \"ease_out\"}, {\"time_in_clip_in_seconds\": 0.8, \"value\": 540}]}";

/// The properties `set_keyframes` animates, named as in the other tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum PropertyParameter {
    /// Canvas pixels from the left edge to the clip's centre.
    CenterXInPixels,
    /// Canvas pixels from the top edge to the clip's centre.
    CenterYInPixels,
    /// Size multiplier: 1 fits inside the canvas.
    Scale,
    /// Clockwise rotation in degrees.
    RotationInDegrees,
    /// 0 invisible to 1 opaque.
    Opacity,
    /// Sound level multiplier, 0 silent to 4.
    Volume,
}

impl PropertyParameter {
    fn property(self) -> AnimatableProperty {
        match self {
            Self::CenterXInPixels => AnimatableProperty::PositionX,
            Self::CenterYInPixels => AnimatableProperty::PositionY,
            Self::Scale => AnimatableProperty::Scale,
            Self::RotationInDegrees => AnimatableProperty::Rotation,
            Self::Opacity => AnimatableProperty::Opacity,
            Self::Volume => AnimatableProperty::Volume,
        }
    }
}

/// How the value travels from a keyframe to the next one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum CurveParameter {
    /// Constant speed.
    #[default]
    Linear,
    /// Starts slowly, speeds up.
    EaseIn,
    /// Starts fast, slows down as it arrives.
    EaseOut,
    /// Starts and ends slowly.
    EaseInOut,
    /// Keeps the value, then jumps to the next keyframe's value.
    Hold,
    /// A cubic Bézier curve given by bezier_handles.
    Bezier,
}

/// One keyframe of `set_keyframes`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct KeyframeParameters {
    /// Seconds from the clip's start (0 is its first frame), rounded to the nearest frame.
    time_in_clip_in_seconds: f64,
    /// The value at that moment, in the property's unit.
    value: f32,
    /// The curve to the next keyframe. Default "linear". Unused on the last keyframe.
    #[serde(default)]
    curve: CurveParameter,
    /// With "curve": "bezier" only: [x1, y1, x2, y2], each from 0 to 1, as in CSS
    /// cubic-bezier(). Example: [0.2, 0, 0, 1] (a strong ease out).
    bezier_handles: Option<[f32; 4]>,
}

/// Parameters of `set_keyframes`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetKeyframesParameters {
    /// The clip, from get_timeline. Example: 7.
    clip_id: u64,
    /// The property to animate.
    property: PropertyParameter,
    /// The keyframes, in any order. Empty removes the animation.
    keyframes: Vec<KeyframeParameters>,
}

fn set_keyframes(
    toolbox: &mut ToolCall<'_>,
    parameters: SetKeyframesParameters,
) -> Result<ToolOutput, ToolError> {
    let clip_id = ClipId::from_raw(parameters.clip_id);
    let property = parameters.property.property();
    let project = toolbox.require_project()?;
    let clip = project.require_clip(clip_id)?.1;
    ensure_property_applies(project, clip, property)?;
    let new_keyframes = keyframes_from_parameters(project, clip, property, &parameters.keyframes)?;
    let old_offsets: Vec<TimeSpan> = clip
        .animated_property(property)
        .keyframes()
        .iter()
        .map(|keyframe| keyframe.time_offset)
        .collect();
    let name = property_name(property);
    if old_offsets.is_empty() && new_keyframes.is_empty() {
        return Ok(ToolOutput::text(format!(
            "The {name} of clip {clip_id} has no keyframes: nothing to remove."
        )));
    }
    toolbox.change_project(
        &format!("Animate the {name} of clip {clip_id}"),
        |transaction| {
            let last_index = old_offsets.len().saturating_sub(1);
            for (index, time_offset) in old_offsets.iter().enumerate() {
                let removal = RemoveKeyframe::new(clip_id, property, *time_offset);
                // Only an empty list makes the property constant: it keeps the
                // value of its last keyframe, as in the app.
                if index == last_index && new_keyframes.is_empty() {
                    transaction.execute(removal.keeping_the_value_when_last())?;
                } else {
                    transaction.execute(removal)?;
                }
            }
            for keyframe in &new_keyframes {
                transaction.execute(SetKeyframe::new(clip_id, property, *keyframe))?;
            }
            Ok(())
        },
    )?;
    let view = clip_details(toolbox.require_project()?, clip_id)?;
    let message = if new_keyframes.is_empty() {
        format!("The {name} of clip {clip_id} is no longer animated.")
    } else {
        format!(
            "The {name} of clip {clip_id} now has {} keyframe(s). Check with get_frame.",
            new_keyframes.len()
        )
    };
    Ok(ToolOutput::message_with_details(message, &view))
}

/// Refuses animating what a clip does not have: sound on a title, a picture
/// setting on a sound.
fn ensure_property_applies(
    project: &Project,
    clip: &Clip,
    property: AnimatableProperty,
) -> Result<(), ToolError> {
    let (has_picture, has_sound) = match &clip.content {
        ClipContent::Text(_) => (true, false),
        ClipContent::Media(segment) => {
            project
                .media_asset(segment.media_asset_id)
                .map_or((false, false), |asset| {
                    (
                        asset.description.has_picture(),
                        asset.description.has_audio(),
                    )
                })
        }
    };
    let applies = match property {
        AnimatableProperty::Volume => has_sound,
        _ => has_picture,
    };
    if applies {
        Ok(())
    } else {
        Err(ToolError::new(format!(
            "clip {} has no {}: its {} cannot be animated",
            clip.id,
            if has_picture { "sound" } else { "picture" },
            property_name(property)
        )))
    }
}

/// The keyframes asked for, in Kadr's units, sorted by time, checked before
/// anything changes.
fn keyframes_from_parameters(
    project: &Project,
    clip: &Clip,
    property: AnimatableProperty,
    keyframe_parameters: &[KeyframeParameters],
) -> Result<Vec<Keyframe>, ToolError> {
    let name = property_name(property);
    let offset = claude_value_offset(project.settings(), property);
    let frame_rate = project.settings().frame_rate;
    let clip_length = clip.timeline_duration();
    let mut keyframes = Vec::with_capacity(keyframe_parameters.len());
    for parameters in keyframe_parameters {
        let context = format!("the keyframe at {} s", parameters.time_in_clip_in_seconds);
        let asked = span_from_seconds(
            "time_in_clip_in_seconds",
            parameters.time_in_clip_in_seconds,
        )?;
        // Rounded to the nearest frame of the timeline, where the clip's frames are.
        let asked_moment = clip.timeline_start + asked;
        let earlier_frame = frame_rate.frame_start_at_or_before(asked_moment);
        let later_frame = frame_rate.frame_start_at_or_after(asked_moment);
        let nearest_frame = match (
            asked_moment.span_since(earlier_frame),
            later_frame.span_since(asked_moment),
        ) {
            (Some(after_earlier), Some(before_later)) if before_later < after_earlier => {
                later_frame
            }
            _ => earlier_frame,
        };
        let time_offset = nearest_frame
            .span_since(clip.timeline_start)
            .unwrap_or(TimeSpan::ZERO);
        if time_offset > clip_length {
            return Err(ToolError::new(format!(
                "{context} is after the end of clip {} ({} s long)",
                clip.id,
                seconds_in(clip_length)
            )));
        }
        let value = parameters.value - offset;
        if !property.accepts_value(value) {
            let (minimum, maximum) = property.allowed_range();
            return Err(ToolError::new(format!(
                "{context}: the {name} must be from {} to {}, not {}",
                minimum + offset,
                maximum + offset,
                parameters.value
            )));
        }
        let interpolation_to_next = interpolation_from(parameters, &context)?;
        if keyframes
            .iter()
            .any(|existing: &Keyframe| existing.time_offset == time_offset)
        {
            return Err(ToolError::new(format!(
                "two keyframes fall on the same frame, at {} s from the clip's start",
                seconds_in(time_offset)
            )));
        }
        keyframes.push(Keyframe {
            time_offset,
            value,
            interpolation_to_next,
        });
    }
    keyframes.sort_by_key(|keyframe| keyframe.time_offset);
    Ok(keyframes)
}

/// The curve of one keyframe.
fn interpolation_from(
    parameters: &KeyframeParameters,
    context: &str,
) -> Result<Interpolation, ToolError> {
    let interpolation = match parameters.curve {
        CurveParameter::Linear => Interpolation::Linear,
        CurveParameter::EaseIn => Interpolation::EaseIn,
        CurveParameter::EaseOut => Interpolation::EaseOut,
        CurveParameter::EaseInOut => Interpolation::EaseInOut,
        CurveParameter::Hold => Interpolation::Hold,
        CurveParameter::Bezier => {
            let Some([x1, y1, x2, y2]) = parameters.bezier_handles else {
                return Err(ToolError::new(format!(
                    "{context}: \"curve\": \"bezier\" needs bezier_handles [x1, y1, x2, y2]"
                )));
            };
            let curve = BezierCurve {
                start_handle: BezierHandle {
                    time_fraction: x1,
                    progress_fraction: y1,
                },
                end_handle: BezierHandle {
                    time_fraction: x2,
                    progress_fraction: y2,
                },
            };
            if !curve.has_handles_inside_the_unit_square() {
                return Err(ToolError::new(format!(
                    "{context}: each of bezier_handles must be from 0 to 1"
                )));
            }
            Interpolation::CubicBezier(curve)
        }
    };
    if parameters.bezier_handles.is_some() && parameters.curve != CurveParameter::Bezier {
        return Err(ToolError::new(format!(
            "{context}: bezier_handles only go with \"curve\": \"bezier\""
        )));
    }
    Ok(interpolation)
}
