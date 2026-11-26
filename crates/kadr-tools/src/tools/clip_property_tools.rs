//! Clip property tools: speed, volume, position, scale, rotation, opacity, crop.

use kadr_core::animation::Keyframe;
use kadr_core::clip::{AnimatableProperty, Clip, ClipContent, CropInsets, PlaybackSpeed};
use kadr_core::commands::{MoveClip, SetClipCrop, SetClipPropertyValue, SetClipSpeed, SetKeyframe};
use kadr_core::history::Transaction;
use kadr_core::ids::ClipId;
use kadr_core::project::Project;
use kadr_core::time::{TimeSpan, Timestamp};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::catalog::{ToolEffect, ToolEntry, declare_tool};
use crate::output::{ToolError, ToolOutput};
use crate::time_input::half_frame;
use crate::toolbox::ToolCall;
use crate::tools::timeline_tools::clip_details;

/// The clip property tools.
pub(crate) fn clip_property_tools() -> Vec<ToolEntry> {
    vec![
        declare_tool(
            "set_clip_speed",
            "Change a clip's speed",
            SET_CLIP_SPEED_DESCRIPTION,
            ToolEffect::ChangesProject,
            set_clip_speed,
        ),
        declare_tool(
            "set_clip_volume",
            "Change a clip's volume",
            SET_CLIP_VOLUME_DESCRIPTION,
            ToolEffect::ChangesProject,
            set_clip_volume,
        ),
        declare_tool(
            "set_transform",
            "Place, scale, rotate or crop a clip",
            SET_TRANSFORM_DESCRIPTION,
            ToolEffect::ChangesProject,
            set_transform,
        ),
    ]
}

// ----- set_clip_speed -----

const SET_CLIP_SPEED_DESCRIPTION: &str = "\
Changes how fast a video or sound clip plays: 2 is twice as fast (the clip becomes half as \
long), 0.5 is slow motion (twice as long). From 0.1 to 16. Sound keeps its pitch. The clip's \
start stays in place and its end moves; if the longer clip would overlap the next one, set \
\"shift_later_clips\": true to move the later clips of the same track along. \
Example: {\"clip_id\": 4, \"speed\": 1.5}";

/// Parameters of `set_clip_speed`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetClipSpeedParameters {
    /// The media clip, from get_timeline. Example: 4.
    clip_id: u64,
    /// Speed multiplier from 0.1 to 16: 1 normal, 2 twice as fast, 0.5 half speed.
    speed: f64,
    /// Move the later clips of the same track by the change in length, so they stay right
    /// after this clip. Default false.
    #[serde(default)]
    shift_later_clips: bool,
}

fn set_clip_speed(
    toolbox: &mut ToolCall<'_>,
    parameters: SetClipSpeedParameters,
) -> Result<ToolOutput, ToolError> {
    let clip_id = ClipId::from_raw(parameters.clip_id);
    let speed = PlaybackSpeed::from_multiplier(parameters.speed)?;
    let project = toolbox.require_project()?;
    let (track, clip) = project.require_clip(clip_id)?;
    let ClipContent::Media(segment) = &clip.content else {
        return Err(ToolError::new(format!(
            "clip {clip_id} is a text: only video and sound clips have a speed"
        )));
    };
    let old_end = clip.timeline_end();
    let new_end = clip.timeline_start + speed.timeline_span_for_source_span(segment.source_span());
    let tolerance = half_frame(project.settings().frame_rate);
    let mut later_clips: Vec<(ClipId, Timestamp)> = if parameters.shift_later_clips {
        track
            .clips()
            .iter()
            .filter(|other| other.id != clip_id && other.timeline_start + tolerance >= old_end)
            .map(|other| (other.id, other.timeline_start))
            .collect()
    } else {
        Vec::new()
    };
    let is_getting_longer = new_end > old_end;
    // Make room from the last clip when growing, close up from the first when shrinking.
    if is_getting_longer {
        later_clips.reverse();
    }
    let label = format!("Set the speed of clip {clip_id} to {speed}");
    toolbox.change_project(&label, |transaction| {
        if !is_getting_longer {
            transaction.execute(SetClipSpeed::new(clip_id, speed))?;
        }
        for (later_clip_id, later_start) in &later_clips {
            let new_start = shifted_start(*later_start, old_end, new_end);
            transaction.execute(MoveClip::along_track(*later_clip_id, new_start))?;
        }
        if is_getting_longer {
            transaction.execute(SetClipSpeed::new(clip_id, speed))?;
        }
        Ok(())
    })?;
    let view = clip_details(toolbox.require_project()?, clip_id)?;
    let mut message = format!(
        "Clip {clip_id} now plays at {}× and runs from {} s to {} s.",
        parameters.speed,
        view.summary.timeline_start_in_seconds,
        view.summary.timeline_end_in_seconds
    );
    if !later_clips.is_empty() {
        message.push_str(&format!(
            " {} later clip(s) moved along.",
            later_clips.len()
        ));
    }
    Ok(ToolOutput::message_with_details(message, &view))
}

/// `start` moved by the difference between `new_end` and `old_end`.
fn shifted_start(start: Timestamp, old_end: Timestamp, new_end: Timestamp) -> Timestamp {
    match new_end.span_since(old_end) {
        Some(growth) => start + growth,
        None => {
            let shrink = old_end.span_since(new_end).unwrap_or(TimeSpan::ZERO);
            start.checked_sub(shrink).unwrap_or(Timestamp::ZERO)
        }
    }
}

// ----- set_clip_volume -----

const SET_CLIP_VOLUME_DESCRIPTION: &str = "\
Sets the sound level of a clip as a multiplier: 1 unchanged, 0 silent (mute), 0.5 about half as \
loud (−6 dB), 2 about twice as loud (+6 dB), at most 4. If the clip has a sound fade \
(add_transition), the fade is kept and its level scaled. \
Example: {\"clip_id\": 6, \"volume\": 0.3}";

/// Parameters of `set_clip_volume`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetClipVolumeParameters {
    /// The clip, from get_timeline. Example: 6.
    clip_id: u64,
    /// Volume multiplier from 0 (silent) to 4; 1 leaves the sound unchanged.
    volume: f32,
}

fn set_clip_volume(
    toolbox: &mut ToolCall<'_>,
    parameters: SetClipVolumeParameters,
) -> Result<ToolOutput, ToolError> {
    let clip_id = ClipId::from_raw(parameters.clip_id);
    let clip = toolbox.require_project()?.require_clip(clip_id)?.1.clone();
    let label = format!("Set the volume of clip {clip_id} to {}", parameters.volume);
    toolbox.change_project(&label, |transaction| {
        set_level_keeping_animation(
            transaction,
            &clip,
            AnimatableProperty::Volume,
            parameters.volume,
        )
    })?;
    let view = clip_details(toolbox.require_project()?, clip_id)?;
    Ok(ToolOutput::message_with_details(
        format!("The volume of clip {clip_id} is now {}.", parameters.volume),
        &view,
    ))
}

/// Sets a level property (volume, opacity). If it is animated (by a fade), every
/// keyframe is scaled so the highest one becomes `value`: the fade keeps its shape.
fn set_level_keeping_animation(
    transaction: &mut Transaction<'_>,
    clip: &Clip,
    property: AnimatableProperty,
    value: f32,
) -> Result<(), ToolError> {
    let animated_value = clip.animated_property(property);
    if !animated_value.is_animated() {
        transaction.execute(SetClipPropertyValue::new(clip.id, property, value))?;
        return Ok(());
    }
    let highest = animated_value
        .keyframes()
        .iter()
        .map(|keyframe| keyframe.value)
        .fold(0.0_f32, f32::max);
    for keyframe in animated_value.keyframes() {
        let scaled_value = if highest > 0.0 {
            keyframe.value / highest * value
        } else {
            value
        };
        transaction.execute(SetKeyframe::new(
            clip.id,
            property,
            Keyframe {
                value: scaled_value,
                ..*keyframe
            },
        ))?;
    }
    Ok(())
}

// ----- set_transform -----

const SET_TRANSFORM_DESCRIPTION: &str = "\
Places a clip's picture on the canvas. Coordinates are canvas pixels with the origin at the \
top-left corner, x to the right, y down: the canvas centre of a 1080×1920 project is (540, \
960). center_x/center_y place the clip's centre. scale 1 makes the picture fit entirely inside \
the canvas; \"fit\": \"fill_canvas\" computes the scale that covers the whole canvas (cutting \
the edges), for example a landscape video in a vertical project. Crop values remove a fraction \
(0 to 1) of the source picture from each edge. Omitted values are unchanged. Check the result \
with get_frame (without clip_id).

Examples: {\"clip_id\": 3, \"fit\": \"fill_canvas\"}; picture-in-picture in the top-right corner \
of a 1920×1080 canvas: {\"clip_id\": 8, \"scale\": 0.3, \"center_x_in_pixels\": 1600, \
\"center_y_in_pixels\": 220}";

/// How the picture is sized relative to the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum FitMode {
    /// The whole picture is visible inside the canvas (scale 1), with bars if the shapes differ.
    FitInside,
    /// The picture covers the whole canvas; the parts that stick out are cut.
    FillCanvas,
}

/// Parameters of `set_transform`. Omitted values stay unchanged.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetTransformParameters {
    /// The clip, from get_timeline. Example: 3.
    clip_id: u64,
    /// Horizontal position of the clip's centre, in canvas pixels from the left edge.
    center_x_in_pixels: Option<f32>,
    /// Vertical position of the clip's centre, in canvas pixels from the top edge.
    center_y_in_pixels: Option<f32>,
    /// Size multiplier from 0.01 to 100: 1 fits inside the canvas, 0.5 is half that size.
    scale: Option<f32>,
    /// Sets the scale automatically: "fit_inside" (scale 1) or "fill_canvas". Do not combine with scale.
    fit: Option<FitMode>,
    /// Clockwise rotation in degrees around the clip's centre. Example: 90.
    rotation_in_degrees: Option<f32>,
    /// 0 invisible to 1 opaque.
    opacity: Option<f32>,
    /// Fraction of the source picture removed from the left edge, 0 to below 1.
    crop_left: Option<f32>,
    /// Fraction removed from the top edge, 0 to below 1.
    crop_top: Option<f32>,
    /// Fraction removed from the right edge, 0 to below 1.
    crop_right: Option<f32>,
    /// Fraction removed from the bottom edge, 0 to below 1.
    crop_bottom: Option<f32>,
}

impl SetTransformParameters {
    fn changes_crop(&self) -> bool {
        self.crop_left.is_some()
            || self.crop_top.is_some()
            || self.crop_right.is_some()
            || self.crop_bottom.is_some()
    }
}

fn set_transform(
    toolbox: &mut ToolCall<'_>,
    parameters: SetTransformParameters,
) -> Result<ToolOutput, ToolError> {
    let clip_id = ClipId::from_raw(parameters.clip_id);
    let project = toolbox.require_project()?;
    let clip = project.require_clip(clip_id)?.1.clone();
    if parameters.scale.is_some() && parameters.fit.is_some() {
        return Err(ToolError::new("give either scale or fit, not both"));
    }
    let crop = CropInsets {
        left_fraction: parameters
            .crop_left
            .unwrap_or(clip.transform.crop.left_fraction),
        top_fraction: parameters
            .crop_top
            .unwrap_or(clip.transform.crop.top_fraction),
        right_fraction: parameters
            .crop_right
            .unwrap_or(clip.transform.crop.right_fraction),
        bottom_fraction: parameters
            .crop_bottom
            .unwrap_or(clip.transform.crop.bottom_fraction),
    };
    let scale = match parameters.fit {
        Some(FitMode::FitInside) => Some(1.0),
        Some(FitMode::FillCanvas) => Some(scale_filling_canvas(project, &clip, &crop)?),
        None => parameters.scale,
    };
    let canvas = project.settings().canvas_size;
    let centre_x = canvas.width_in_pixels as f32 / 2.0;
    let centre_y = canvas.height_in_pixels as f32 / 2.0;
    let constant_changes = [
        (
            AnimatableProperty::PositionX,
            parameters.center_x_in_pixels.map(|x| x - centre_x),
        ),
        (
            AnimatableProperty::PositionY,
            parameters.center_y_in_pixels.map(|y| y - centre_y),
        ),
        (AnimatableProperty::Scale, scale),
        (AnimatableProperty::Rotation, parameters.rotation_in_degrees),
    ];
    if constant_changes.iter().all(|(_, value)| value.is_none())
        && parameters.opacity.is_none()
        && !parameters.changes_crop()
    {
        return Err(ToolError::new(
            "nothing to change: give at least one of center_x_in_pixels, center_y_in_pixels, \
             scale, fit, rotation_in_degrees, opacity or a crop value",
        ));
    }
    let changes_crop = parameters.changes_crop();
    toolbox.change_project(
        &format!("Change the placement of clip {clip_id}"),
        |transaction| {
            for (property, value) in constant_changes {
                if let Some(value) = value {
                    transaction.execute(SetClipPropertyValue::new(clip_id, property, value))?;
                }
            }
            if let Some(opacity) = parameters.opacity {
                set_level_keeping_animation(
                    transaction,
                    &clip,
                    AnimatableProperty::Opacity,
                    opacity,
                )?;
            }
            if changes_crop {
                transaction.execute(SetClipCrop::new(clip_id, crop))?;
            }
            Ok(())
        },
    )?;
    let view = clip_details(toolbox.require_project()?, clip_id)?;
    Ok(ToolOutput::message_with_details(
        format!("Clip {clip_id} is now placed as follows (check with get_frame):"),
        &view.transform,
    ))
}

/// The scale at which the (cropped) picture covers the whole canvas.
///
/// At scale 1 the picture fits inside the canvas: it is enlarged by the smaller
/// of the two canvas/picture ratios. Covering needs the larger ratio, so the
/// scale is larger ratio ÷ smaller ratio.
fn scale_filling_canvas(
    project: &Project,
    clip: &Clip,
    crop: &CropInsets,
) -> Result<f32, ToolError> {
    let picture_size = match &clip.content {
        ClipContent::Media(segment) => {
            project
                .require_media_asset(segment.media_asset_id)?
                .description
                .picture_size
        }
        ClipContent::Text(_) => None,
    };
    let Some(picture_size) = picture_size else {
        return Err(ToolError::new(format!(
            "clip {} has no picture to fill the canvas with (texts and sounds cannot)",
            clip.id
        )));
    };
    let visible_width =
        picture_size.width_in_pixels as f32 * (1.0 - crop.left_fraction - crop.right_fraction);
    let visible_height =
        picture_size.height_in_pixels as f32 * (1.0 - crop.top_fraction - crop.bottom_fraction);
    if visible_width <= 0.0 || visible_height <= 0.0 {
        return Err(ToolError::new("the crop removes the whole picture"));
    }
    let canvas = project.settings().canvas_size;
    let width_ratio = canvas.width_in_pixels as f32 / visible_width;
    let height_ratio = canvas.height_in_pixels as f32 / visible_height;
    Ok(width_ratio.max(height_ratio) / width_ratio.min(height_ratio))
}
