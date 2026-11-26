//! Commands of the blur tool: what to draw on the preview, and following a box
//! drawn there through the clip.

use std::sync::Arc;

use kadr_core::commands::{AddEffect, CreatedObjectsExt};
use kadr_core::effect::{EffectKind, TrackedBlur};
use kadr_core::error::EditError;
use kadr_core::ids::ClipId;
use kadr_core::time::Timestamp;
use kadr_engine::{RegionTrackingReport, RegionTrackingRequest};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use crate::app_state::AppState;
use crate::blur_overlay::{ClipOverlay, SourceRegion, clip_overlay_at};
use crate::command_error::{CommandError, CommandResult, ErrorCode};
use crate::editing::BlurShapeChoice;
use crate::tracking_job::{TrackingEvent, TrackingStarted};

/// Undo label of a blur added by following a box.
pub const BLUR_REGION_UNDO_LABEL: &str = "blurRegion";

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
/// source video's pixels) through the whole clip. When it ends, a tracked blur of
/// `shape` is added as one undo step; progress and the result come through
/// `on_event`.
#[tauri::command]
pub async fn start_region_tracking(
    app: AppHandle,
    state: State<'_, AppState>,
    clip_id: ClipId,
    time_ticks: u64,
    region: SourceRegion,
    shape: BlurShapeChoice,
    on_event: Channel<TrackingEvent>,
) -> CommandResult<TrackingStarted> {
    let (project, project_identity) = state
        .with_open_project(|open_project| Ok((open_project.snapshot(), open_project.identity())))?;
    let request = RegionTrackingRequest::for_clip(
        &project,
        clip_id,
        Timestamp::from_ticks(time_ticks),
        region.into(),
    )?;
    request.check_selection()?;
    let style = TrackedBlur {
        shape: shape.into(),
        ..TrackedBlur::with_default_style()
    };
    let backend = Arc::clone(&state.backend);
    state.tracking.start(
        backend,
        request,
        move |event| {
            // The window may have closed meanwhile: nobody is listening then.
            let _ = on_event.send(event);
        },
        move |report| {
            add_tracked_blur(
                &app.state::<AppState>(),
                project_identity,
                clip_id,
                &style,
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

/// Adds the analysed blur to the project, if it is still the one the analysis
/// started on.
fn add_tracked_blur(
    state: &AppState,
    project_identity: u64,
    clip_id: ClipId,
    style: &TrackedBlur,
    report: &RegionTrackingReport,
) -> CommandResult<TrackingEvent> {
    state
        .with_open_project(|open_project| {
            if open_project.identity() != project_identity {
                return Err(project_closed());
            }
            let blur = EffectKind::TrackedBlur(report.tracked_blur_like(style));
            let effect_id = open_project.edit(|session, _| {
                session.transaction(BLUR_REGION_UNDO_LABEL, |transaction| {
                    transaction
                        .execute(AddEffect::new(clip_id, blur))?
                        .first_effect_id()
                        .ok_or_else(|| EditError::InternalInconsistency {
                            details: "the blur created was not reported".to_string(),
                        })
                })
            })?;
            Ok(TrackingEvent::finished(
                open_project.view(),
                clip_id,
                effect_id,
                &report.unsure_ranges,
            ))
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
