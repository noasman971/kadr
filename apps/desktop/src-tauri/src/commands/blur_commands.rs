//! Commands of the blur tool: what to draw on the preview, and following a box
//! drawn there through the clip, for a new blur or to correct one.

use std::sync::Arc;

use kadr_core::effect::TrackedBlur;
use kadr_core::ids::{ClipId, EffectId};
use kadr_core::time::Timestamp;
use kadr_engine::{RegionTrackingReport, RegionTrackingRequest};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use crate::app_state::AppState;
use crate::blur_overlay::{ClipOverlay, SourceRegion, clip_overlay_at};
use crate::blur_results::{TrackingPurpose, add_analysis_to_project};
use crate::command_error::{CommandError, CommandResult, ErrorCode};
use crate::editing::BlurShapeChoice;
use crate::tracking_job::{TrackingEvent, TrackingStarted};

/// Where clip `clip_id`'s picture is on the canvas at `time_ticks`, and its
/// blurs' boxes then; `None` when the clip shows no picture at that time.
#[tauri::command]
pub async fn get_clip_overlay(
    state: State<'_, AppState>,
    clip_id: ClipId,
    time_ticks: u64,
) -> CommandResult<Option<ClipOverlay>> {
    let project = state.with_open_project(|open_project| Ok(open_project.snapshot()))?;
    clip_overlay_at(&project, clip_id, Timestamp::from_ticks(time_ticks))
}

/// Starts following `region` (drawn on clip `clip_id` at `time_ticks`, in the
/// source video's pixels) through the clip. Without `effect_id`, a new tracked
/// blur of `shape` is added when it ends; with it, that blur is corrected from
/// the redrawn box (only up to its neighbouring anchors). Either is one undo
/// step; progress and the result come through `on_event`.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // The arguments are the front end's, by name.
pub async fn start_region_tracking(
    app: AppHandle,
    state: State<'_, AppState>,
    clip_id: ClipId,
    time_ticks: u64,
    region: SourceRegion,
    shape: BlurShapeChoice,
    effect_id: Option<EffectId>,
    on_event: Channel<TrackingEvent>,
) -> CommandResult<TrackingStarted> {
    let (project, project_identity) = state
        .with_open_project(|open_project| Ok((open_project.snapshot(), open_project.identity())))?;
    let time = Timestamp::from_ticks(time_ticks);
    let (request, purpose) = match effect_id {
        None => (
            RegionTrackingRequest::for_clip(&project, clip_id, time, region.into())?,
            TrackingPurpose::NewBlur {
                style: TrackedBlur {
                    shape: shape.into(),
                    ..TrackedBlur::with_default_style()
                },
            },
        ),
        Some(effect_id) => (
            RegionTrackingRequest::for_correction(
                &project,
                clip_id,
                effect_id,
                time,
                region.into(),
            )?,
            TrackingPurpose::Correction { effect_id },
        ),
    };
    request.check_selection()?;
    let backend = Arc::clone(&state.backend);
    let analysed_request = request.clone();
    state.tracking.start(
        backend,
        request,
        move |event| {
            // The window may have closed meanwhile: nobody is listening then.
            let _ = on_event.send(event);
        },
        move |report| {
            add_result(
                &app.state::<AppState>(),
                project_identity,
                clip_id,
                &purpose,
                &analysed_request,
                &report,
            )
            .unwrap_or_else(TrackingEvent::failed)
        },
    )?;
    Ok(TrackingStarted { clip_id })
}

/// Stops the running analysis, if any; nothing is added.
#[tauri::command]
pub async fn cancel_region_tracking(state: State<'_, AppState>) -> CommandResult<()> {
    state.tracking.cancel();
    Ok(())
}

/// Adds the analysis to the project, if it is still the one it started on.
fn add_result(
    state: &AppState,
    project_identity: u64,
    clip_id: ClipId,
    purpose: &TrackingPurpose,
    request: &RegionTrackingRequest,
    report: &RegionTrackingReport,
) -> CommandResult<TrackingEvent> {
    state
        .with_open_project(|open_project| {
            if open_project.identity() != project_identity {
                return Err(project_closed());
            }
            add_analysis_to_project(open_project, clip_id, purpose, request, report)
        })
        .map_err(|error| match error.code {
            ErrorCode::NoProjectOpen => project_closed(),
            _ => error,
        })
}

fn project_closed() -> CommandError {
    CommandError::new(
        ErrorCode::TrackingProjectClosed,
        "the project was closed before the region was followed to the end",
    )
}
