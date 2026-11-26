//! Content tools: texts, fades, colour adjustments, removing effects.

use kadr_core::animation::{Interpolation, Keyframe};
use kadr_core::clip::{
    AnimatableProperty, Clip, ClipContent, TextAlignment, TextContent, TextStyle,
};
use kadr_core::commands::{
    AddClip, AddEffect, CreatedObjectsExt, RemoveEffect, RemoveKeyframe, ReplaceEffect,
    SetClipPropertyValue, SetClipText, SetKeyframe,
};
use kadr_core::effect::{ColorAdjustment, EffectKind};
use kadr_core::history::Transaction;
use kadr_core::ids::{ClipId, EffectId, TrackId};
use kadr_core::project::Project;
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_core::timeline::{Track, TrackKind};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::catalog::{ToolEffect, ToolEntry, declare_tool};
use crate::output::{ToolError, ToolOutput};
use crate::time_input::{
    half_frame, positive_span_from_seconds, seconds_in, timestamp_from_seconds,
};
use crate::toolbox::KadrToolbox;
use crate::tools::common::parse_color;
use crate::tools::timeline_tools::{clip_details, is_free, missing_created_object};

/// The content tools.
pub(crate) fn content_tools() -> Vec<ToolEntry> {
    vec![
        declare_tool(
            "add_text",
            "Add a text or title",
            ADD_TEXT_DESCRIPTION,
            ToolEffect::ChangesProject,
            add_text,
        ),
        declare_tool(
            "edit_text",
            "Change a text",
            EDIT_TEXT_DESCRIPTION,
            ToolEffect::ChangesProject,
            edit_text,
        ),
        declare_tool(
            "add_transition",
            "Add a fade",
            ADD_TRANSITION_DESCRIPTION,
            ToolEffect::ChangesProject,
            add_transition,
        ),
        declare_tool(
            "remove_transitions",
            "Remove a clip's fades",
            REMOVE_TRANSITIONS_DESCRIPTION,
            ToolEffect::ChangesProject,
            remove_transitions,
        ),
        declare_tool(
            "add_color_adjustment",
            "Adjust colours",
            ADD_COLOR_ADJUSTMENT_DESCRIPTION,
            ToolEffect::ChangesProject,
            add_color_adjustment,
        ),
        declare_tool(
            "remove_effect",
            "Remove an effect",
            REMOVE_EFFECT_DESCRIPTION,
            ToolEffect::ChangesProject,
            remove_effect,
        ),
    ]
}

// ----- add_text -----

const ADD_TEXT_DESCRIPTION: &str = "\
Adds a text (a title, a caption) shown over the video from start_time_in_seconds for \
duration_in_seconds. It goes on a video track above the videos (created if needed), centred \
unless center_x/center_y are given: canvas pixels from the top-left corner, so on a 1080×1920 \
canvas a title near the top is center_y_in_pixels 300. Line breaks (\\n) start new lines. The \
default size is one sixteenth of the canvas height. Check the result with get_frame.

Example: {\"text\": \"Summer 2026\", \"start_time_in_seconds\": 0, \"duration_in_seconds\": 3, \
\"font_size_in_pixels\": 120, \"color\": \"#FFFFFF\", \"center_y_in_pixels\": 300}";

/// Parameters of `add_text`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddTextParameters {
    /// The words to show. "\n" starts a new line.
    text: String,
    /// When the text appears on the timeline, in seconds. Default 0.
    start_time_in_seconds: Option<f64>,
    /// How long it stays, in seconds. Default 3.
    duration_in_seconds: Option<f64>,
    /// The video track to put it on. Default: a text track above the videos.
    track_id: Option<u64>,
    /// Text height in canvas pixels, 1 to 2000. Default: canvas height ÷ 16.
    font_size_in_pixels: Option<f32>,
    /// "#RRGGBB" or "#RRGGBBAA". Default white "#FFFFFF".
    color: Option<String>,
    /// A font installed on the computer. Default "Inter" (bundled with Kadr).
    font_family: Option<String>,
    /// Alignment of the lines: "left", "center" (default) or "right".
    alignment: Option<AlignmentParameter>,
    /// Horizontal position of the text's centre, in canvas pixels from the left. Default: centre.
    center_x_in_pixels: Option<f32>,
    /// Vertical position of the text's centre, in canvas pixels from the top. Default: centre.
    center_y_in_pixels: Option<f32>,
}

/// Alignment of text lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum AlignmentParameter {
    /// Lines start on the left.
    Left,
    /// Lines are centred.
    Center,
    /// Lines end on the right.
    Right,
}

impl From<AlignmentParameter> for TextAlignment {
    fn from(alignment: AlignmentParameter) -> Self {
        match alignment {
            AlignmentParameter::Left => TextAlignment::Left,
            AlignmentParameter::Center => TextAlignment::Center,
            AlignmentParameter::Right => TextAlignment::Right,
        }
    }
}

const DEFAULT_TEXT_DURATION_IN_SECONDS: f64 = 3.0;
/// The default font size is the canvas height divided by this.
const CANVAS_HEIGHTS_PER_DEFAULT_FONT_SIZE: f32 = 16.0;
/// The name of the track created for texts.
const TEXT_TRACK_NAME: &str = "Texts";

fn add_text(
    toolbox: &mut KadrToolbox,
    parameters: AddTextParameters,
) -> Result<ToolOutput, ToolError> {
    let project = toolbox.require_project()?;
    let start = timestamp_from_seconds(
        "start_time_in_seconds",
        parameters.start_time_in_seconds.unwrap_or(0.0),
    )?;
    let duration = positive_span_from_seconds(
        "duration_in_seconds",
        parameters
            .duration_in_seconds
            .unwrap_or(DEFAULT_TEXT_DURATION_IN_SECONDS),
    )?;
    let canvas = project.settings().canvas_size;
    let mut style = TextStyle {
        font_size_in_pixels: canvas.height_in_pixels as f32 / CANVAS_HEIGHTS_PER_DEFAULT_FONT_SIZE,
        ..TextStyle::default()
    };
    apply_style_changes(&mut style, &StyleChanges::from_add_text(&parameters))?;
    let track = match parameters.track_id {
        Some(raw_track_id) => Some(project.require_track(TrackId::from_raw(raw_track_id))?.id),
        None => text_track_with_room(project, start, start + duration),
    };
    let canvas_centre = (
        canvas.width_in_pixels as f32 / 2.0,
        canvas.height_in_pixels as f32 / 2.0,
    );
    let position_offsets = [
        (
            AnimatableProperty::PositionX,
            parameters.center_x_in_pixels.map(|x| x - canvas_centre.0),
        ),
        (
            AnimatableProperty::PositionY,
            parameters.center_y_in_pixels.map(|y| y - canvas_centre.1),
        ),
    ];
    let content = ClipContent::Text(TextContent {
        text: parameters.text.clone(),
        duration,
        style,
    });
    let label = format!("Add the text \"{}\"", short_text(&parameters.text));
    let clip_id = toolbox.change_project(&label, |transaction| {
        let track_id = match track {
            Some(track_id) => track_id,
            None => create_text_track(transaction)?,
        };
        let created = transaction.execute(AddClip::new(track_id, content, start))?;
        let clip_id = created
            .first_clip_id()
            .ok_or_else(|| missing_created_object("clip"))?;
        for (property, offset) in position_offsets {
            if let Some(offset) = offset {
                transaction.execute(SetClipPropertyValue::new(clip_id, property, offset))?;
            }
        }
        Ok(clip_id)
    })?;
    let view = clip_details(toolbox.require_project()?, clip_id)?;
    Ok(ToolOutput::message_with_details(
        format!(
            "Added text clip {clip_id} on track {}, from {} s to {} s. Look at it with get_frame.",
            view.track_id,
            view.summary.timeline_start_in_seconds,
            view.summary.timeline_end_in_seconds
        ),
        &view,
    ))
}

/// The highest video track holding only texts with room for `start`..`end`.
fn text_track_with_room(project: &Project, start: Timestamp, end: Timestamp) -> Option<TrackId> {
    let holds_only_texts = |track: &Track| {
        track
            .clips()
            .iter()
            .all(|clip| matches!(clip.content, ClipContent::Text(_)))
    };
    let top_video_track = project
        .timeline()
        .tracks()
        .iter()
        .rev()
        .find(|track| track.kind == TrackKind::Video)?;
    let is_usable = !top_video_track.clips().is_empty()
        && holds_only_texts(top_video_track)
        && is_free(top_video_track, start, end, None);
    is_usable.then_some(top_video_track.id)
}

fn create_text_track(transaction: &mut Transaction<'_>) -> Result<TrackId, ToolError> {
    let created = transaction.execute(
        kadr_core::commands::AddTrack::new(
            TrackKind::Video,
            kadr_core::commands::TrackPosition::Top,
        )
        .named(TEXT_TRACK_NAME),
    )?;
    Ok(created
        .first_track_id()
        .ok_or_else(|| missing_created_object("track"))?)
}

/// The first words of a text, for history labels.
fn short_text(text: &str) -> String {
    const LABEL_CHARACTER_COUNT: usize = 30;
    let first_line = text.lines().next().unwrap_or_default();
    if first_line.chars().count() <= LABEL_CHARACTER_COUNT && !text.contains('\n') {
        return first_line.to_string();
    }
    let shortened: String = first_line.chars().take(LABEL_CHARACTER_COUNT).collect();
    format!("{shortened}…")
}

/// Style values to change; `None` keeps the current one.
struct StyleChanges<'parameters> {
    font_size_in_pixels: Option<f32>,
    color: Option<&'parameters str>,
    font_family: Option<&'parameters str>,
    alignment: Option<AlignmentParameter>,
}

impl<'parameters> StyleChanges<'parameters> {
    fn from_add_text(parameters: &'parameters AddTextParameters) -> Self {
        Self {
            font_size_in_pixels: parameters.font_size_in_pixels,
            color: parameters.color.as_deref(),
            font_family: parameters.font_family.as_deref(),
            alignment: parameters.alignment,
        }
    }

    fn from_edit_text(parameters: &'parameters EditTextParameters) -> Self {
        Self {
            font_size_in_pixels: parameters.font_size_in_pixels,
            color: parameters.color.as_deref(),
            font_family: parameters.font_family.as_deref(),
            alignment: parameters.alignment,
        }
    }

    fn is_empty(&self) -> bool {
        self.font_size_in_pixels.is_none()
            && self.color.is_none()
            && self.font_family.is_none()
            && self.alignment.is_none()
    }
}

fn apply_style_changes(style: &mut TextStyle, changes: &StyleChanges<'_>) -> Result<(), ToolError> {
    if let Some(font_size_in_pixels) = changes.font_size_in_pixels {
        style.font_size_in_pixels = font_size_in_pixels;
    }
    if let Some(color_text) = changes.color {
        style.color = parse_color("color", color_text)?;
    }
    if let Some(font_family) = changes.font_family {
        style.font_family = font_family.to_string();
    }
    if let Some(alignment) = changes.alignment {
        style.alignment = alignment.into();
    }
    Ok(())
}

// ----- edit_text -----

const EDIT_TEXT_DESCRIPTION: &str = "\
Changes the words or the style of a text clip. Omitted values are unchanged. To move it, use \
set_transform; to change when it shows, move_clip or trim_clip. \
Example: {\"clip_id\": 9, \"text\": \"Summer\\n2026\", \"color\": \"#FFD400\"}";

/// Parameters of `edit_text`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EditTextParameters {
    /// The text clip, from get_timeline. Example: 9.
    clip_id: u64,
    /// The new words. "\n" starts a new line.
    text: Option<String>,
    /// Text height in canvas pixels, 1 to 2000.
    font_size_in_pixels: Option<f32>,
    /// "#RRGGBB" or "#RRGGBBAA".
    color: Option<String>,
    /// A font installed on the computer, or "Inter" (bundled).
    font_family: Option<String>,
    /// "left", "center" or "right".
    alignment: Option<AlignmentParameter>,
}

fn edit_text(
    toolbox: &mut KadrToolbox,
    parameters: EditTextParameters,
) -> Result<ToolOutput, ToolError> {
    let clip_id = ClipId::from_raw(parameters.clip_id);
    let (_, clip) = toolbox.require_project()?.require_clip(clip_id)?;
    let ClipContent::Text(current) = &clip.content else {
        return Err(ToolError::new(format!(
            "clip {clip_id} is not a text clip; add a text with add_text"
        )));
    };
    let changes = StyleChanges::from_edit_text(&parameters);
    if parameters.text.is_none() && changes.is_empty() {
        return Err(ToolError::new(
            "nothing to change: give text, font_size_in_pixels, color, font_family or alignment",
        ));
    }
    let mut style = current.style.clone();
    apply_style_changes(&mut style, &changes)?;
    let new_text = parameters
        .text
        .clone()
        .unwrap_or_else(|| current.text.clone());
    toolbox.change_project(&format!("Edit text clip {clip_id}"), |transaction| {
        transaction.execute(SetClipText::new(clip_id, new_text).with_style(style))?;
        Ok(())
    })?;
    let view = clip_details(toolbox.require_project()?, clip_id)?;
    Ok(ToolOutput::message_with_details(
        format!("Text clip {clip_id} changed."),
        &view,
    ))
}

// ----- add_transition -----

const ADD_TRANSITION_DESCRIPTION: &str = "\
Adds a fade to a clip, with an animation of its opacity (and of its volume when \
include_sound is true): \"fade_in\" appears from the background colour (black by default) at \
the clip's start, \"fade_out\" disappears into it at the clip's end, \"dip_to_black\" fades this \
clip out and the next clip of the same track in. duration_in_seconds is the length of each \
fade, at most the clip's length. Cross-dissolves between overlapping clips arrive in a later \
version. Remove fades with remove_transitions. \
Example: {\"clip_id\": 4, \"kind\": \"fade_out\", \"duration_in_seconds\": 1.0}";

/// The kinds of transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum TransitionKind {
    /// The clip appears progressively at its start.
    FadeIn,
    /// The clip disappears progressively at its end.
    FadeOut,
    /// This clip fades out, then the next clip of the same track fades in.
    DipToBlack,
}

impl TransitionKind {
    /// The name used in history labels.
    fn label_name(self) -> &'static str {
        match self {
            Self::FadeIn => "fade in",
            Self::FadeOut => "fade out",
            Self::DipToBlack => "dip to black",
        }
    }
}

/// Parameters of `add_transition`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddTransitionParameters {
    /// The clip, from get_timeline. Example: 4.
    clip_id: u64,
    /// "fade_in", "fade_out" or "dip_to_black".
    kind: TransitionKind,
    /// Length of each fade, in seconds. Default 0.5.
    duration_in_seconds: Option<f64>,
    /// Also fade the sound. Default true.
    #[serde(default = "default_true")]
    include_sound: bool,
}

fn default_true() -> bool {
    true
}

const DEFAULT_FADE_LENGTH_IN_SECONDS: f64 = 0.5;

/// Which end of a clip a fade is at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FadeEnd {
    Start,
    End,
}

fn add_transition(
    toolbox: &mut KadrToolbox,
    parameters: AddTransitionParameters,
) -> Result<ToolOutput, ToolError> {
    let clip_id = ClipId::from_raw(parameters.clip_id);
    let fade_length = positive_span_from_seconds(
        "duration_in_seconds",
        parameters
            .duration_in_seconds
            .unwrap_or(DEFAULT_FADE_LENGTH_IN_SECONDS),
    )?;
    let project = toolbox.require_project()?;
    let (track, clip) = project.require_clip(clip_id)?;
    let mut fades: Vec<(Clip, FadeEnd)> = Vec::new();
    match parameters.kind {
        TransitionKind::FadeIn => fades.push((clip.clone(), FadeEnd::Start)),
        TransitionKind::FadeOut => fades.push((clip.clone(), FadeEnd::End)),
        TransitionKind::DipToBlack => {
            let next_clip = next_clip_on_track(project, track, clip).ok_or_else(|| {
                ToolError::new(format!(
                    "clip {clip_id} has no clip after it on track {}: use fade_out instead",
                    track.id
                ))
            })?;
            fades.push((clip.clone(), FadeEnd::End));
            fades.push((next_clip.clone(), FadeEnd::Start));
        }
    }
    let one_frame = TimeSpan::from_ticks(half_frame(project.settings().frame_rate).ticks() * 2);
    let plans: Vec<FadePlan> = fades
        .iter()
        .map(|(faded_clip, end)| {
            FadePlan::new(
                project,
                faded_clip,
                *end,
                fade_length,
                one_frame,
                parameters.include_sound,
            )
        })
        .collect::<Result<_, _>>()?;
    let label = format!("Add a {} to clip {clip_id}", parameters.kind.label_name());
    toolbox.change_project(&label, |transaction| {
        for plan in &plans {
            plan.apply(transaction)?;
        }
        Ok(())
    })?;
    let faded_ids: Vec<String> = plans.iter().map(|plan| plan.clip_id.to_string()).collect();
    Ok(ToolOutput::text(format!(
        "Added {} s fade(s) on clip(s) {}. Check with get_frame near the clip edges.",
        seconds_in(fade_length),
        faded_ids.join(" and ")
    )))
}

/// The clip right after `clip` on its track.
fn next_clip_on_track<'project>(
    project: &Project,
    track: &'project Track,
    clip: &Clip,
) -> Option<&'project Clip> {
    let tolerance = half_frame(project.settings().frame_rate);
    track.clips().iter().find(|other| {
        other.id != clip.id && other.timeline_start + tolerance >= clip.timeline_end()
    })
}

/// The keyframes of one fade, computed before changing anything.
struct FadePlan {
    clip_id: ClipId,
    /// For each property: (keyframe at the transparent end, keyframe at the full end).
    keyframes: Vec<(AnimatableProperty, [Keyframe; 2])>,
}

impl FadePlan {
    fn new(
        project: &Project,
        clip: &Clip,
        end: FadeEnd,
        fade_length: TimeSpan,
        one_frame: TimeSpan,
        include_sound: bool,
    ) -> Result<Self, ToolError> {
        let clip_length = clip.timeline_duration();
        if fade_length > clip_length {
            return Err(ToolError::new(format!(
                "the fade ({} s) is longer than clip {} ({} s)",
                seconds_in(fade_length),
                clip.id,
                seconds_in(clip_length)
            )));
        }
        // A fade out ends one frame before the clip's end, so its last frame is
        // fully faded; a fade in starts fully faded on the clip's first frame.
        let (faded_offset, full_offset) = match end {
            FadeEnd::Start => (TimeSpan::ZERO, fade_length),
            FadeEnd::End => {
                let faded_offset = if fade_length > one_frame {
                    clip_length.checked_sub(one_frame).unwrap_or(clip_length)
                } else {
                    clip_length
                };
                (
                    faded_offset,
                    clip_length
                        .checked_sub(fade_length)
                        .unwrap_or(TimeSpan::ZERO),
                )
            }
        };
        let mut properties = vec![AnimatableProperty::Opacity];
        if include_sound && clip_has_sound(project, clip) {
            properties.push(AnimatableProperty::Volume);
        }
        let keyframes = properties
            .into_iter()
            .map(|property| {
                let full_value = clip.animated_property(property).value_at(full_offset);
                let keyframe = |time_offset, value| Keyframe {
                    time_offset,
                    value,
                    interpolation_to_next: Interpolation::Linear,
                };
                (
                    property,
                    [
                        keyframe(faded_offset, 0.0),
                        keyframe(full_offset, full_value),
                    ],
                )
            })
            .collect();
        Ok(Self {
            clip_id: clip.id,
            keyframes,
        })
    }

    fn apply(&self, transaction: &mut Transaction<'_>) -> Result<(), ToolError> {
        for (property, keyframes) in &self.keyframes {
            for keyframe in keyframes {
                transaction.execute(SetKeyframe::new(self.clip_id, *property, *keyframe))?;
            }
        }
        Ok(())
    }
}

/// Whether the clip plays sound.
fn clip_has_sound(project: &Project, clip: &Clip) -> bool {
    match &clip.content {
        ClipContent::Media(segment) => project
            .media_asset(segment.media_asset_id)
            .is_some_and(|asset| asset.description.has_audio()),
        ClipContent::Text(_) => false,
    }
}

// ----- remove_transitions -----

const REMOVE_TRANSITIONS_DESCRIPTION: &str = "\
Removes every fade of a clip (all the keyframes of its opacity and volume), giving back its \
steady opacity and volume. Example: {\"clip_id\": 4}";

/// Parameters of `remove_transitions`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RemoveTransitionsParameters {
    /// The clip, from get_timeline. Example: 4.
    clip_id: u64,
}

fn remove_transitions(
    toolbox: &mut KadrToolbox,
    parameters: RemoveTransitionsParameters,
) -> Result<ToolOutput, ToolError> {
    let clip_id = ClipId::from_raw(parameters.clip_id);
    let (_, clip) = toolbox.require_project()?.require_clip(clip_id)?;
    let removals: Vec<(AnimatableProperty, TimeSpan)> =
        [AnimatableProperty::Opacity, AnimatableProperty::Volume]
            .into_iter()
            .flat_map(|property| {
                clip.animated_property(property)
                    .keyframes()
                    .iter()
                    .map(move |keyframe| (property, keyframe.time_offset))
            })
            .collect();
    if removals.is_empty() {
        return Ok(ToolOutput::text(format!("Clip {clip_id} has no fade.")));
    }
    toolbox.change_project(
        &format!("Remove the fades of clip {clip_id}"),
        |transaction| {
            for (property, time_offset) in removals {
                transaction.execute(RemoveKeyframe::new(clip_id, property, time_offset))?;
            }
            Ok(())
        },
    )?;
    let view = clip_details(toolbox.require_project()?, clip_id)?;
    Ok(ToolOutput::message_with_details(
        format!("Removed the fades of clip {clip_id}."),
        &view,
    ))
}

// ----- add_color_adjustment -----

const ADD_COLOR_ADJUSTMENT_DESCRIPTION: &str = "\
Adjusts the colours of a video or image clip: exposure in photographic stops (+1 twice as \
bright, −1 half), contrast and saturation as multipliers (1 unchanged, 0 flat grey / black and \
white, 2 strong), temperature from −1 (cooler, bluer) to 1 (warmer, more orange). Omitted \
values stay neutral. Give effect_id to change an existing colour adjustment instead of adding \
one (see get_clip). Check with get_frame. \
Example: {\"clip_id\": 3, \"exposure_in_stops\": 0.3, \"saturation\": 1.2, \"temperature\": 0.1}";

/// Parameters of `add_color_adjustment`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddColorAdjustmentParameters {
    /// The video or image clip, from get_timeline. Example: 3.
    clip_id: u64,
    /// Brightness change in stops, from −5 to 5. Default 0.
    exposure_in_stops: Option<f32>,
    /// Contrast multiplier from 0 to 4. Default 1.
    contrast: Option<f32>,
    /// Saturation multiplier from 0 to 4. Default 1.
    saturation: Option<f32>,
    /// White balance from −1 (cooler) to 1 (warmer). Default 0.
    temperature: Option<f32>,
    /// An existing colour adjustment of this clip to change instead of adding a new one.
    effect_id: Option<u64>,
}

fn add_color_adjustment(
    toolbox: &mut KadrToolbox,
    parameters: AddColorAdjustmentParameters,
) -> Result<ToolOutput, ToolError> {
    let clip_id = ClipId::from_raw(parameters.clip_id);
    let (_, clip) = toolbox.require_project()?.require_clip(clip_id)?;
    let replaced = match parameters.effect_id {
        Some(raw_effect_id) => Some(existing_color_adjustment(
            clip,
            EffectId::from_raw(raw_effect_id),
        )?),
        None => None,
    };
    let base = replaced.map_or(ColorAdjustment::NEUTRAL, |(_, adjustment)| adjustment);
    let adjustment = ColorAdjustment {
        exposure_in_stops: parameters
            .exposure_in_stops
            .unwrap_or(base.exposure_in_stops),
        contrast_multiplier: parameters.contrast.unwrap_or(base.contrast_multiplier),
        saturation_multiplier: parameters.saturation.unwrap_or(base.saturation_multiplier),
        temperature_shift: parameters.temperature.unwrap_or(base.temperature_shift),
    };
    let kind = EffectKind::ColorAdjustment(adjustment);
    let effect_id = toolbox.change_project(
        &format!("Adjust the colours of clip {clip_id}"),
        |transaction| match replaced {
            Some((effect_id, _)) => {
                transaction.execute(ReplaceEffect::new(clip_id, effect_id, kind))?;
                Ok(effect_id)
            }
            None => {
                let created = transaction.execute(AddEffect::new(clip_id, kind))?;
                Ok(created
                    .first_effect_id()
                    .ok_or_else(|| missing_created_object("effect"))?)
            }
        },
    )?;
    let view = clip_details(toolbox.require_project()?, clip_id)?;
    Ok(ToolOutput::message_with_details(
        format!(
            "Colour adjustment {effect_id} applied to clip {clip_id}. Look at it with get_frame."
        ),
        &view.effect_details,
    ))
}

/// The colour adjustment `effect_id` of the clip.
fn existing_color_adjustment(
    clip: &Clip,
    effect_id: EffectId,
) -> Result<(EffectId, ColorAdjustment), ToolError> {
    let effect = clip
        .effects
        .iter()
        .find(|effect| effect.id == effect_id)
        .ok_or_else(|| {
            ToolError::from(kadr_core::error::EditError::EffectNotFound {
                clip_id: clip.id,
                effect_id,
                existing_effect_ids: clip.effects.iter().map(|effect| effect.id).collect(),
            })
        })?;
    match effect.kind {
        EffectKind::ColorAdjustment(adjustment) => Ok((effect_id, adjustment)),
        EffectKind::TrackedBlur(_) => Err(ToolError::new(format!(
            "effect {effect_id} of clip {} is a tracked blur, not a colour adjustment",
            clip.id
        ))),
    }
}

// ----- remove_effect -----

const REMOVE_EFFECT_DESCRIPTION: &str = "\
Removes one effect (colour adjustment or blur) from a clip. Find the effect_id with get_clip. \
Example: {\"clip_id\": 3, \"effect_id\": 12}";

/// Parameters of `remove_effect`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RemoveEffectParameters {
    /// The clip, from get_timeline. Example: 3.
    clip_id: u64,
    /// The effect, from get_clip. Example: 12.
    effect_id: u64,
}

fn remove_effect(
    toolbox: &mut KadrToolbox,
    parameters: RemoveEffectParameters,
) -> Result<ToolOutput, ToolError> {
    let clip_id = ClipId::from_raw(parameters.clip_id);
    let effect_id = EffectId::from_raw(parameters.effect_id);
    toolbox.change_project(
        &format!("Remove effect {effect_id} from clip {clip_id}"),
        |transaction| {
            transaction.execute(RemoveEffect::new(clip_id, effect_id))?;
            Ok(())
        },
    )?;
    Ok(ToolOutput::text(format!(
        "Removed effect {effect_id} from clip {clip_id}."
    )))
}
