//! Blur tools: hide a face (or a plate, a screen) through a whole clip from one
//! rectangle, follow the analysis, and check and correct the result.
//!
//! `blur_region` starts a background analysis (`kadr_engine::region_tracking`)
//! and answers with a task identifier. When the analysis ends, the toolbox adds
//! the tracked blur to the project as one undo step (see `blur_tasks.rs`).
//! `get_blur_status` then lists the ranges where the tracker was unsure, so that
//! Claude can look at them with `get_frame` and correct them with `blur_region`
//! and an `effect_id`.

use kadr_core::clip::{Clip, ClipContent};
use kadr_core::commands::{AddEffect, CreatedObjectsExt, ReplaceEffect};
use kadr_core::effect::{
    BlurShape, BlurStyle, EffectKind, PixelRegion, TrackedBlur, TrackingConfidence, UnsureRange,
};
use kadr_core::history::EditSession;
use kadr_core::ids::{ClipId, EffectId};
use kadr_core::project::Project;
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_engine::{RegionTrackingRequest, corrected_tracked_blur, find_tracked_blur};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::blur_tasks::{AnalysedBlur, BlurTaskPurpose, BlurTaskState, BlurTaskStatus};
use crate::catalog::{ToolEffect, ToolEntry, declare_tool};
use crate::output::{ToolError, ToolOutput};
use crate::time_input::{seconds_in, seconds_of, timestamp_from_seconds};
use crate::toolbox::KadrToolbox;
use crate::tools::common::parse_color;
use crate::tools::picture_tools::ensure_project_media_readable;
use crate::tools::timeline_tools::missing_created_object;

/// The blur tools.
pub(crate) fn blur_tools() -> Vec<ToolEntry> {
    vec![
        declare_tool(
            "blur_region",
            "Blur a face through a clip",
            BLUR_REGION_DESCRIPTION,
            ToolEffect::ChangesProject,
            blur_region,
        ),
        declare_tool(
            BLUR_STATUS_TOOL_NAME,
            "Follow and check a blur",
            GET_BLUR_STATUS_DESCRIPTION,
            ToolEffect::ReadsOnly,
            get_blur_status,
        ),
        declare_tool(
            "cancel_blur",
            "Cancel a blur analysis",
            CANCEL_BLUR_DESCRIPTION,
            ToolEffect::ChangesProject,
            cancel_blur,
        ),
    ]
}

/// The name of the status tool, whose answers need no background notice.
pub(crate) const BLUR_STATUS_TOOL_NAME: &str = "get_blur_status";

/// Blur strength when none is given.
const DEFAULT_BLUR_RADIUS_IN_PIXELS: f32 = 30.0;
/// Pixel block size when none is given.
const DEFAULT_PIXEL_BLOCK_SIZE_IN_PIXELS: u32 = 24;

// ----- blur_region -----

const BLUR_REGION_DESCRIPTION: &str = "\
Hides a face (or a licence plate, a screen, a logo) through a whole video clip from ONE \
rectangle: Kadr's tracker (classic computer vision, no AI) follows the rectangle forwards and \
backwards through the clip, then blurs it on every frame.

1. Call get_frame with the clip_id and a time_in_seconds where the face is clearly visible. \
Measure the rectangle around the whole face (forehead to chin, ear to ear) on that picture and \
convert it to the video's original pixels with the factor get_frame gives. Pass the same \
clip_id and time_in_seconds here. A margin is added around it automatically.
2. The analysis runs in the background: this tool answers at once with a task_id (\"blur-1\"). \
Call get_blur_status with it every few seconds (a minute of 1080p takes about 10 s).
3. When finished, the blur is in the project (one undo step) and get_blur_status lists the \
ranges where the tracker was unsure. Look at each with get_frame (clip_id, a time inside it). \
If the face shows, call blur_region again with effect_id (the blur to correct), a time in that \
range and the face's rectangle there: tracking restarts from that frame and only replaces what \
it does better.

Coordinates: pixels of the clip's source video, as on get_frame with clip_id (not canvas \
pixels), origin at the top-left corner, x to the right, y down. One blur_region per face. With \
effect_id and the same rectangle the style can change too.

Example: {\"clip_id\": 4, \"time_in_seconds\": 2.5, \"left_in_pixels\": 420, \"top_in_pixels\": \
180, \"width_in_pixels\": 160, \"height_in_pixels\": 200}";

/// How the region is hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum HidingStyle {
    /// A strong soft blur.
    GaussianBlur,
    /// Big square pixels.
    Pixelate,
    /// A plain colour.
    SolidFill,
}

/// Shape of the hidden area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum HiddenShape {
    /// An ellipse inside the rectangle: suits faces.
    Ellipse,
    /// The rectangle itself: suits plates and screens.
    Rectangle,
}

/// Parameters of `blur_region`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct BlurRegionParameters {
    /// The video clip, from get_timeline. Example: 4.
    clip_id: u64,
    /// The moment ON THE TIMELINE the rectangle was measured at, in seconds (the same time given
    /// to get_frame with this clip_id). The clip must be on screen then.
    time_in_seconds: f64,
    /// Left edge of the rectangle, in pixels of the clip's source video.
    left_in_pixels: f32,
    /// Top edge of the rectangle, in pixels of the clip's source video.
    top_in_pixels: f32,
    /// Width of the rectangle, in pixels of the source video (at least 24 in 1080p).
    width_in_pixels: f32,
    /// Height of the rectangle, in pixels of the source video (at least 24 in 1080p).
    height_in_pixels: f32,
    /// To correct an existing tracked blur of this clip (from get_blur_status or get_clip)
    /// instead of adding a new one: the rectangle is where the region really is at
    /// time_in_seconds.
    effect_id: Option<u64>,
    /// "gaussian_blur" (default), "pixelate" or "solid_fill".
    style: Option<HidingStyle>,
    /// Blur radius (gaussian_blur, default 30, 1 to 500) or size of the squares (pixelate,
    /// default 24, 2 to 512), in pixels of the source video.
    strength_in_pixels: Option<f32>,
    /// With solid_fill: the colour, "#RRGGBB". Default black.
    fill_color: Option<String>,
    /// "ellipse" (default, for faces) or "rectangle" (plates, screens).
    shape: Option<HiddenShape>,
    /// Extra space hidden around the tracked rectangle, in percent of its size, 0 to 200.
    /// Default 20; it grows by half where the tracker is unsure.
    margin_in_percent: Option<f32>,
}

fn blur_region(
    toolbox: &mut KadrToolbox,
    parameters: BlurRegionParameters,
) -> Result<ToolOutput, ToolError> {
    let clip_id = ClipId::from_raw(parameters.clip_id);
    let time = timestamp_from_seconds("time_in_seconds", parameters.time_in_seconds)?;
    let rectangle = PixelRegion {
        left_in_pixels: parameters.left_in_pixels,
        top_in_pixels: parameters.top_in_pixels,
        width_in_pixels: parameters.width_in_pixels,
        height_in_pixels: parameters.height_in_pixels,
    };
    let project = toolbox.require_project()?;
    ensure_project_media_readable(toolbox, project)?;
    let (request, purpose) = match parameters.effect_id.map(EffectId::from_raw) {
        None => {
            let style = style_from(&parameters, &TrackedBlur::with_default_style())?;
            check_style_is_valid(project, clip_id, None, &style)?;
            (
                RegionTrackingRequest::for_clip(project, clip_id, time, rectangle),
                BlurTaskPurpose::NewBlur { style },
            )
        }
        Some(effect_id) => {
            let current = find_tracked_blur(project, clip_id, effect_id)
                .map_err(|error| ToolError::new(error.to_string()))?;
            let new_style = if changes_style(&parameters) {
                let style = style_from(&parameters, current)?;
                check_style_is_valid(project, clip_id, Some(effect_id), &style)?;
                Some(style)
            } else {
                None
            };
            (
                RegionTrackingRequest::for_correction(project, clip_id, effect_id, time, rectangle),
                BlurTaskPurpose::Correction {
                    effect_id,
                    new_style,
                },
            )
        }
    };
    let request = request.map_err(|error| tracking_error_for_claude(&error.to_string()))?;
    request
        .check_selection()
        .map_err(|error| tracking_error_for_claude(&error.to_string()))?;
    let project_file = toolbox.require_project_session()?.file_path().to_path_buf();
    let analysed_seconds = request
        .source_end
        .span_since(request.source_start)
        .map_or(0.0, seconds_in);
    let backend = toolbox.shared_backend();
    let task_id =
        toolbox
            .blur_tasks_mut()
            .start(clip_id, project_file, purpose.clone(), request, backend)?;
    let what = match purpose {
        BlurTaskPurpose::NewBlur { .. } => format!("Tracking started on clip {clip_id}"),
        BlurTaskPurpose::Correction { effect_id, .. } => {
            format!("Correction of blur {effect_id} on clip {clip_id} started")
        }
    };
    Ok(ToolOutput::text(format!(
        "{what} as task {task_id}: {analysed_seconds} s of video to analyse. Call \
         get_blur_status with {{\"task_id\": \"{task_id}\"}} every few seconds until it is \
         finished; the blur is added to the project then."
    )))
}

/// Whether the call asks for another style, shape or margin.
fn changes_style(parameters: &BlurRegionParameters) -> bool {
    parameters.style.is_some()
        || parameters.strength_in_pixels.is_some()
        || parameters.fill_color.is_some()
        || parameters.shape.is_some()
        || parameters.margin_in_percent.is_some()
}

/// `base` (style, shape, margin; no samples) with the changes the call asks for.
fn style_from(
    parameters: &BlurRegionParameters,
    base: &TrackedBlur,
) -> Result<TrackedBlur, ToolError> {
    let current_style = match base.style {
        BlurStyle::GaussianBlur { .. } => HidingStyle::GaussianBlur,
        BlurStyle::Pixelate { .. } => HidingStyle::Pixelate,
        BlurStyle::SolidFill { .. } => HidingStyle::SolidFill,
    };
    let chosen_style = parameters.style.unwrap_or(current_style);
    let keeps_kind = chosen_style == current_style;
    let style = match chosen_style {
        HidingStyle::GaussianBlur => BlurStyle::GaussianBlur {
            radius_in_pixels: match (parameters.strength_in_pixels, base.style) {
                (Some(strength), _) => strength,
                (None, BlurStyle::GaussianBlur { radius_in_pixels }) if keeps_kind => {
                    radius_in_pixels
                }
                (None, _) => DEFAULT_BLUR_RADIUS_IN_PIXELS,
            },
        },
        HidingStyle::Pixelate => BlurStyle::Pixelate {
            block_size_in_pixels: match (parameters.strength_in_pixels, base.style) {
                (Some(strength), _) => whole_pixels(strength)?,
                (
                    None,
                    BlurStyle::Pixelate {
                        block_size_in_pixels,
                    },
                ) if keeps_kind => block_size_in_pixels,
                (None, _) => DEFAULT_PIXEL_BLOCK_SIZE_IN_PIXELS,
            },
        },
        HidingStyle::SolidFill => BlurStyle::SolidFill {
            color: match (&parameters.fill_color, base.style) {
                (Some(color_text), _) => parse_color("fill_color", color_text)?,
                (None, BlurStyle::SolidFill { color }) => color,
                (None, _) => kadr_core::color::Color::BLACK,
            },
        },
    };
    Ok(TrackedBlur {
        style,
        shape: match parameters.shape {
            Some(HiddenShape::Ellipse) => BlurShape::Ellipse,
            Some(HiddenShape::Rectangle) => BlurShape::Rectangle,
            None => base.shape,
        },
        margin_in_percent: parameters
            .margin_in_percent
            .unwrap_or(base.margin_in_percent),
        region_samples: Vec::new(),
        ..base.clone()
    })
}

fn whole_pixels(strength: f32) -> Result<u32, ToolError> {
    if !strength.is_finite() || strength < 0.0 {
        return Err(ToolError::new(format!(
            "strength_in_pixels must be a positive number of pixels, not {strength}"
        )));
    }
    Ok(strength.round() as u32)
}

/// Refuses a style the project rules would refuse, before spending time on the
/// analysis: the style is tried on a copy of the project.
fn check_style_is_valid(
    project: &Project,
    clip_id: ClipId,
    replaced_effect: Option<EffectId>,
    style: &TrackedBlur,
) -> Result<(), ToolError> {
    let mut trial = EditSession::new(project.clone())?;
    let kind = EffectKind::TrackedBlur(style.clone());
    match replaced_effect {
        Some(effect_id) => trial.execute(ReplaceEffect::new(clip_id, effect_id, kind))?,
        None => trial.execute(AddEffect::new(clip_id, kind))?,
    };
    Ok(())
}

/// The tracking error, with what to do about it.
fn tracking_error_for_claude(message: &str) -> ToolError {
    ToolError::new(format!(
        "{message}. Check the clip with get_timeline and the rectangle on a fresh get_frame (with \
         this clip_id and time), in the source video's pixels"
    ))
}

// ----- get_blur_status -----

const GET_BLUR_STATUS_DESCRIPTION: &str = "\
Follows blur_region analyses and shows what is left to check. With task_id: that analysis \
(running: percent done and time left; finished: the blur's effect_id and the ranges to check; \
failed or cancelled: why). With clip_id: every tracked blur of that clip and its ranges to \
check. With neither: every analysis and every tracked blur of the project.

A range to check is \"uncertain\" (the tracker probably followed the region; the hidden area was \
widened) or \"lost\" (the region was hidden or left the picture; the blur stays where it was last \
seen). Times are seconds on the timeline. Look at look_at_time_in_seconds with get_frame and the \
clip_id: if the face shows, correct it with blur_region and the blur's effect_id.

Example: {\"task_id\": \"blur-1\"}";

/// Parameters of `get_blur_status`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetBlurStatusParameters {
    /// The task given by blur_region, for example "blur-1".
    task_id: Option<String>,
    /// A clip whose tracked blurs to show, for example 4.
    clip_id: Option<u64>,
}

fn get_blur_status(
    toolbox: &mut KadrToolbox,
    parameters: GetBlurStatusParameters,
) -> Result<ToolOutput, ToolError> {
    match (&parameters.task_id, parameters.clip_id) {
        (Some(_), Some(_)) => Err(ToolError::new(
            "give task_id or clip_id, not both (or neither for everything)",
        )),
        (Some(task_id), None) => {
            let status = toolbox.blur_tasks().status(task_id)?;
            let view = task_view(toolbox.require_project().ok(), &status);
            Ok(ToolOutput::message_with_details(task_message(&view), &view))
        }
        (None, Some(raw_clip_id)) => {
            let project = toolbox.require_project()?;
            let clip_id = ClipId::from_raw(raw_clip_id);
            let (_, clip) = project.require_clip(clip_id)?;
            let blurs = blur_views_of_clip(clip);
            let message = match blurs.len() {
                0 => format!("Clip {clip_id} has no tracked blur. Add one with blur_region."),
                count => format!(
                    "Clip {clip_id} has {count} tracked blur(s); {} range(s) to check.",
                    blurs
                        .iter()
                        .map(|blur| blur.ranges_to_check.len())
                        .sum::<usize>()
                ),
            };
            Ok(ToolOutput::message_with_details(message, &blurs))
        }
        (None, None) => {
            let project = toolbox.require_project().ok();
            let tasks: Vec<BlurTaskView> = toolbox
                .blur_tasks()
                .all_statuses()
                .iter()
                .map(|status| task_view(project, status))
                .collect();
            let blurs: Vec<TrackedBlurView> = project
                .map(|project| {
                    project
                        .timeline()
                        .tracks()
                        .iter()
                        .flat_map(|track| track.clips())
                        .flat_map(blur_views_of_clip)
                        .collect()
                })
                .unwrap_or_default();
            let message = format!(
                "{} blur analysis task(s) since Kadr started; {} tracked blur(s) in the project.",
                tasks.len(),
                blurs.len()
            );
            Ok(ToolOutput::message_with_details(
                message,
                &serde_json::json!({ "tasks": tasks, "tracked_blurs": blurs }),
            ))
        }
    }
}

/// One analysis, as shown to Claude.
#[derive(Debug, Serialize)]
struct BlurTaskView {
    task_id: String,
    clip_id: u64,
    /// "new_blur" or "correction".
    purpose: &'static str,
    /// "running", "finished", "failed" or "cancelled".
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    percent_done: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    estimated_seconds_left: Option<f64>,
    elapsed_seconds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_counts: Option<FrameCountsView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    /// The blur as it is in the project now.
    #[serde(skip_serializing_if = "Option::is_none")]
    tracked_blur: Option<TrackedBlurView>,
}

/// How many frames the tracker was confident, uncertain and lost on.
#[derive(Debug, Serialize)]
struct FrameCountsView {
    confident: usize,
    uncertain: usize,
    lost: usize,
}

/// A tracked blur and the ranges to check.
#[derive(Debug, Serialize)]
struct TrackedBlurView {
    clip_id: u64,
    effect_id: u64,
    is_enabled: bool,
    style: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    strength_in_pixels: Option<f32>,
    shape: &'static str,
    margin_in_percent: f32,
    /// How many times it was corrected.
    correction_count: usize,
    ranges_to_check: Vec<RangeToCheckView>,
}

/// A stretch where the tracker was unsure, in timeline seconds.
#[derive(Debug, Serialize)]
struct RangeToCheckView {
    start_time_in_seconds: f64,
    /// The start of the last unsure frame.
    end_time_in_seconds: f64,
    /// A good time to look at with get_frame (the middle of the range).
    look_at_time_in_seconds: f64,
    /// "uncertain" or "lost".
    confidence: &'static str,
}

fn task_view(project: Option<&Project>, status: &BlurTaskStatus) -> BlurTaskView {
    let (purpose, corrected_effect) = match &status.purpose {
        BlurTaskPurpose::NewBlur { .. } => ("new_blur", None),
        BlurTaskPurpose::Correction { effect_id, .. } => ("correction", Some(*effect_id)),
    };
    let mut view = BlurTaskView {
        task_id: status.task_id.clone(),
        clip_id: status.clip_id.raw(),
        purpose,
        state: "running",
        percent_done: None,
        estimated_seconds_left: None,
        elapsed_seconds: rounded_to_tenths(status.elapsed.as_secs_f64()),
        frame_counts: None,
        reason: None,
        tracked_blur: None,
    };
    match &status.state {
        BlurTaskState::Running { progress } => {
            if progress.total_frame_count > 0 {
                let fraction = progress.fraction();
                view.percent_done = Some(rounded_to_tenths(fraction * 100.0));
                if fraction > 0.0 {
                    let elapsed = status.elapsed.as_secs_f64();
                    view.estimated_seconds_left =
                        Some(rounded_to_tenths(elapsed / fraction - elapsed));
                }
            }
        }
        // Finished, being added to the project right now.
        BlurTaskState::Analysed { .. } => view.percent_done = Some(100.0),
        BlurTaskState::Applied {
            effect_id,
            confidence_counts: (confident, uncertain, lost),
            ..
        } => {
            view.state = "finished";
            view.frame_counts = Some(FrameCountsView {
                confident: *confident,
                uncertain: *uncertain,
                lost: *lost,
            });
            view.tracked_blur = project
                .and_then(|project| project.require_clip(status.clip_id).ok())
                .and_then(|(_, clip)| {
                    blur_views_of_clip(clip)
                        .into_iter()
                        .find(|blur| blur.effect_id == effect_id.raw())
                });
        }
        BlurTaskState::Failed { reason } => {
            view.state = "failed";
            view.reason = Some(reason.clone());
        }
        BlurTaskState::Cancelled => view.state = "cancelled",
    }
    if view.tracked_blur.is_none()
        && let Some(effect_id) = corrected_effect
        && view.state == "finished"
    {
        view.reason = Some(format!(
            "blur {effect_id} is no longer on clip {}",
            status.clip_id
        ));
    }
    view
}

fn task_message(view: &BlurTaskView) -> String {
    let task_id = &view.task_id;
    let clip_id = view.clip_id;
    match view.state {
        "running" => match (view.percent_done, view.estimated_seconds_left) {
            (Some(percent), Some(seconds_left)) => format!(
                "{task_id} is analysing clip {clip_id}: {percent} % done, about {seconds_left} s \
                 left. Call get_blur_status again in a few seconds."
            ),
            _ => format!(
                "{task_id} is starting on clip {clip_id}. Call get_blur_status again in a few \
                 seconds."
            ),
        },
        "finished" => match &view.tracked_blur {
            Some(blur) if blur.ranges_to_check.is_empty() => format!(
                "{task_id} finished in {} s: tracked blur {} hides the region on clip {clip_id}, \
                 and the tracker was confident on every frame. Look at one or two frames with \
                 get_frame (clip_id {clip_id}) to confirm it is hidden.",
                view.elapsed_seconds, blur.effect_id
            ),
            Some(blur) => format!(
                "{task_id} finished in {} s: tracked blur {effect_id} hides the region on clip \
                 {clip_id}. Check the {} range(s) below with get_frame (clip_id {clip_id}, \
                 look_at_time_in_seconds). Where the face shows, call blur_region with effect_id \
                 {effect_id}, that time and the face's rectangle there.",
                view.elapsed_seconds,
                blur.ranges_to_check.len(),
                effect_id = blur.effect_id
            ),
            None => format!(
                "{task_id} finished, but its blur is no longer in the project (undone or removed)."
            ),
        },
        "failed" => format!(
            "{task_id} failed: {}. Nothing was changed.",
            view.reason.as_deref().unwrap_or("unknown reason")
        ),
        _ => format!("{task_id} was cancelled. Nothing was changed."),
    }
}

/// The tracked blurs of `clip`, with their ranges to check.
fn blur_views_of_clip(clip: &Clip) -> Vec<TrackedBlurView> {
    clip.effects
        .iter()
        .filter_map(|effect| match &effect.kind {
            EffectKind::TrackedBlur(blur) => Some(TrackedBlurView {
                clip_id: clip.id.raw(),
                effect_id: effect.id.raw(),
                is_enabled: effect.is_enabled,
                style: match blur.style {
                    BlurStyle::GaussianBlur { .. } => "gaussian_blur",
                    BlurStyle::Pixelate { .. } => "pixelate",
                    BlurStyle::SolidFill { .. } => "solid_fill",
                },
                strength_in_pixels: match blur.style {
                    BlurStyle::GaussianBlur { radius_in_pixels } => Some(radius_in_pixels),
                    BlurStyle::Pixelate {
                        block_size_in_pixels,
                    } => Some(block_size_in_pixels as f32),
                    BlurStyle::SolidFill { .. } => None,
                },
                shape: match blur.shape {
                    BlurShape::Ellipse => "ellipse",
                    BlurShape::Rectangle => "rectangle",
                },
                margin_in_percent: blur.margin_in_percent,
                correction_count: blur
                    .region_samples
                    .iter()
                    .filter(|sample| {
                        sample.origin == kadr_core::effect::RegionSampleOrigin::ManualCorrection
                    })
                    .count(),
                ranges_to_check: blur
                    .unsure_ranges()
                    .iter()
                    .filter_map(|range| range_on_timeline(clip, range))
                    .collect(),
            }),
            EffectKind::ColorAdjustment(_) => None,
        })
        .collect()
}

/// The part of `range` the clip shows, in timeline seconds; `None` when the
/// clip does not show it (trimmed away).
fn range_on_timeline(clip: &Clip, range: &UnsureRange) -> Option<RangeToCheckView> {
    let ClipContent::Media(segment) = &clip.content else {
        return None;
    };
    let last_shown_source_time = Timestamp::from_ticks(segment.source_end.ticks().checked_sub(1)?);
    let first = range.first_source_time.max(segment.source_start);
    let last = range.last_source_time.min(last_shown_source_time);
    if first > last {
        return None;
    }
    let start = clip.timeline_time_showing(first)?;
    let end = clip.timeline_time_showing(last)?;
    let middle = start + TimeSpan::from_ticks(end.span_since(start)?.ticks() / 2);
    Some(RangeToCheckView {
        start_time_in_seconds: seconds_of(start),
        end_time_in_seconds: seconds_of(end),
        look_at_time_in_seconds: seconds_of(middle),
        confidence: match range.confidence {
            TrackingConfidence::Lost => "lost",
            TrackingConfidence::Uncertain | TrackingConfidence::Confident => "uncertain",
        },
    })
}

fn rounded_to_tenths(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

// ----- cancel_blur -----

const CANCEL_BLUR_DESCRIPTION: &str = "\
Stops a blur_region analysis that is still running; nothing is added to the project. A \
finished blur is removed with undo or remove_effect instead. Example: {\"task_id\": \"blur-1\"}";

/// Parameters of `cancel_blur`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CancelBlurParameters {
    /// The task given by blur_region, for example "blur-1".
    task_id: String,
}

fn cancel_blur(
    toolbox: &mut KadrToolbox,
    parameters: CancelBlurParameters,
) -> Result<ToolOutput, ToolError> {
    let status = toolbox.blur_tasks().cancel(&parameters.task_id)?;
    let message = match status.state {
        BlurTaskState::Applied { effect_id, .. } => format!(
            "{} had already finished: blur {effect_id} is in the project. Remove it with undo or \
             remove_effect if it is not wanted.",
            status.task_id
        ),
        BlurTaskState::Failed { reason } => {
            format!("{} had already failed: {reason}", status.task_id)
        }
        _ => format!(
            "{} is cancelled; nothing is added to the project.",
            status.task_id
        ),
    };
    Ok(ToolOutput::text(message))
}

// ----- Adding finished analyses to the project -----

/// Adds every finished analysis to the open project, each as one undo step, and
/// returns a sentence about each for Claude.
pub(crate) fn add_analysed_blurs_to_project(toolbox: &mut KadrToolbox) -> Vec<String> {
    let analysed = toolbox.blur_tasks().analysed_blurs();
    analysed
        .into_iter()
        .map(|analysed_blur| {
            let (outcome, notice) = match add_analysed_blur(toolbox, &analysed_blur) {
                Ok(effect_id) => (
                    BlurTaskState::Applied {
                        effect_id,
                        confidence_counts: analysed_blur.report.confidence_counts(),
                        duration: analysed_blur.duration,
                    },
                    format!(
                        "{} finished: tracked blur {effect_id} is on clip {} (see get_blur_status).",
                        analysed_blur.task_id, analysed_blur.clip_id
                    ),
                ),
                Err(error) => {
                    let reason = format!(
                        "the analysis finished but its result could not be added to the project: \
                         {error}"
                    );
                    let notice = format!("{} failed: {reason}.", analysed_blur.task_id);
                    (BlurTaskState::Failed { reason }, notice)
                }
            };
            toolbox
                .blur_tasks()
                .record_outcome(analysed_blur.task_number, outcome);
            notice
        })
        .collect()
}

fn add_analysed_blur(
    toolbox: &mut KadrToolbox,
    analysed: &AnalysedBlur,
) -> Result<EffectId, ToolError> {
    let session = toolbox.require_project_session()?;
    if session.file_path() != analysed.project_file {
        return Err(ToolError::new(format!(
            "the project \"{}\" was closed before the analysis ended",
            analysed.project_file.display()
        )));
    }
    let clip_id = analysed.clip_id;
    match &analysed.purpose {
        BlurTaskPurpose::NewBlur { style } => {
            let kind = EffectKind::TrackedBlur(analysed.report.tracked_blur_like(style));
            toolbox.change_project(&format!("Blur a region on clip {clip_id}"), |transaction| {
                let created = transaction.execute(AddEffect::new(clip_id, kind))?;
                Ok(created
                    .first_effect_id()
                    .ok_or_else(|| missing_created_object("effect"))?)
            })
        }
        BlurTaskPurpose::Correction {
            effect_id,
            new_style,
        } => {
            let current = find_tracked_blur(toolbox.require_project()?, clip_id, *effect_id)
                .map_err(|error| ToolError::new(error.to_string()))?;
            let corrected = corrected_tracked_blur(current, &analysed.request, &analysed.report);
            let corrected = match new_style {
                Some(style) => TrackedBlur {
                    region_samples: corrected.region_samples,
                    ..style.clone()
                },
                None => corrected,
            };
            let effect_id = *effect_id;
            toolbox.change_project(
                &format!("Correct blur {effect_id} on clip {clip_id}"),
                |transaction| {
                    transaction.execute(ReplaceEffect::new(
                        clip_id,
                        effect_id,
                        EffectKind::TrackedBlur(corrected),
                    ))?;
                    Ok(effect_id)
                },
            )
        }
    }
}
