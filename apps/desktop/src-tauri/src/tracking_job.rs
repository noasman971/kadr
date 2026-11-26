//! Following a region drawn on the preview (docs/ux.md, "Blurring a face").
//!
//! One analysis at a time, on its own thread, with its own copy of what it
//! needs (the video file and the selection), so editing goes on meanwhile.
//! Progress goes to the front end through a channel a few times a second. When
//! the analysis ends, `finish` adds its result to the project the window shows
//! (as one undo step) and says how it went; this module knows nothing of Tauri.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use kadr_core::effect::{TrackingConfidence, UnsureRange};
use kadr_core::ids::{ClipId, EffectId};
use kadr_engine::{
    MediaBackend, RegionTrackingError, RegionTrackingReport, RegionTrackingRequest,
    TrackingCancellation, TrackingProgress, track_region,
};
use serde::Serialize;

use crate::app_state::lock_shared;
use crate::command_error::{CommandError, CommandResult, ErrorCode};
use crate::project_view::ProjectView;

/// Progress is sent at most this often.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(150);

/// What the front end hears about a running analysis.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TrackingEvent {
    /// Frames analysed so far.
    Progress {
        /// Frames analysed.
        analysed_frame_count: u64,
        /// Frames to analyse in all.
        total_frame_count: u64,
        /// Seconds since the analysis started.
        elapsed_seconds: f64,
    },
    /// The blur is in the project.
    Finished {
        /// The project with the blur.
        view: Box<ProjectView>,
        /// The clip blurred.
        clip_id: ClipId,
        /// The blur created or corrected.
        effect_id: EffectId,
        /// Stretches where the tracker was uncertain.
        uncertain_range_count: usize,
        /// Stretches where the tracker lost the region.
        lost_range_count: usize,
    },
    /// The analysis failed, or its result could not be added; nothing changed.
    Failed {
        /// What went wrong, for the interface to explain.
        code: ErrorCode,
        /// The engine's explanation, in English.
        details: String,
    },
    /// The user cancelled it.
    Cancelled,
}

impl TrackingEvent {
    /// The event of a result that was added to the project.
    pub fn finished(
        view: ProjectView,
        clip_id: ClipId,
        effect_id: EffectId,
        unsure_ranges: &[UnsureRange],
    ) -> Self {
        let count_of = |confidence| {
            unsure_ranges
                .iter()
                .filter(|range| range.confidence == confidence)
                .count()
        };
        Self::Finished {
            view: Box::new(view),
            clip_id,
            effect_id,
            uncertain_range_count: count_of(TrackingConfidence::Uncertain),
            lost_range_count: count_of(TrackingConfidence::Lost),
        }
    }

    /// The event of an error.
    pub fn failed(error: CommandError) -> Self {
        Self::Failed {
            code: error.code,
            details: error.details,
        }
    }
}

/// What `start` answers at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackingStarted {
    /// The clip analysed.
    pub clip_id: ClipId,
}

/// The analysis slot.
#[derive(Default)]
pub struct TrackingJobs {
    running: Arc<Mutex<Option<TrackingCancellation>>>,
}

impl TrackingJobs {
    /// Starts analysing `request` on its own thread. `report` receives the
    /// progress, then the event `finish` makes of the result (or the failure).
    ///
    /// # Errors
    ///
    /// When an analysis is already running or the thread cannot start.
    pub fn start(
        &self,
        backend: Arc<dyn MediaBackend>,
        request: RegionTrackingRequest,
        report: impl Fn(TrackingEvent) + Send + 'static,
        finish: impl FnOnce(RegionTrackingReport) -> TrackingEvent + Send + 'static,
    ) -> CommandResult<()> {
        let mut running = lock_shared(&self.running);
        if running.is_some() {
            return Err(CommandError::new(
                ErrorCode::TrackingAlreadyRunning,
                "a region is already being followed",
            ));
        }
        let cancellation = TrackingCancellation::new();
        *running = Some(cancellation.clone());
        drop(running);

        let running_slot = Arc::clone(&self.running);
        let spawned = thread::Builder::new()
            .name("region-tracking".to_string())
            .spawn(move || {
                let event = match run_analysis(backend.as_ref(), &request, &cancellation, &report) {
                    Ok(analysis) if !cancellation.is_cancelled() => finish(analysis),
                    Ok(_) | Err(RegionTrackingError::Cancelled) => TrackingEvent::Cancelled,
                    Err(error) => TrackingEvent::failed(CommandError::from(error)),
                };
                *lock_shared(&running_slot) = None;
                report(event);
            });
        if let Err(error) = spawned {
            *lock_shared(&self.running) = None;
            return Err(CommandError::internal(error));
        }
        Ok(())
    }

    /// Asks the running analysis to stop; nothing happens when none runs.
    pub fn cancel(&self) {
        if let Some(cancellation) = lock_shared(&self.running).as_ref() {
            cancellation.cancel();
        }
    }

    /// Whether an analysis is running.
    pub fn is_running(&self) -> bool {
        lock_shared(&self.running).is_some()
    }
}

/// Runs the analysis, reporting progress at most every [`PROGRESS_INTERVAL`].
fn run_analysis(
    backend: &dyn MediaBackend,
    request: &RegionTrackingRequest,
    cancellation: &TrackingCancellation,
    report: &impl Fn(TrackingEvent),
) -> Result<RegionTrackingReport, RegionTrackingError> {
    let started_at = Instant::now();
    let mut last_report_at: Option<Instant> = None;
    let mut on_progress = |progress: TrackingProgress| {
        let is_due = last_report_at.is_none_or(|last| last.elapsed() >= PROGRESS_INTERVAL);
        if is_due {
            last_report_at = Some(Instant::now());
            report(TrackingEvent::Progress {
                analysed_frame_count: progress.analysed_frame_count,
                total_frame_count: progress.total_frame_count,
                elapsed_seconds: started_at.elapsed().as_secs_f64(),
            });
        }
    };
    track_region(backend, request, &mut on_progress, cancellation)
}
