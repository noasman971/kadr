//! Tracking analyses running in the background, followed by a task identifier.
//!
//! Following a region through a clip decodes the whole clip: a few seconds for a
//! short clip, about ten for a minute of 1080p. The `blur_region` tool starts
//! one of these tasks and answers at once with its identifier (`"blur-1"`);
//! `get_blur_status` follows it and `cancel_blur` stops it.
//!
//! The analysis runs on its own thread, on its own copy of what it needs (the
//! video file and the selection), so editing meanwhile is fine. Its result is
//! **added to the project by the toolbox**, not by the thread: the project
//! belongs to whoever runs the tool calls. The toolbox does it as soon as it
//! can: at the start of the next tool call, or when its host asks
//! ([`crate::KadrToolbox::apply_finished_background_work`]).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use kadr_core::effect::TrackedBlur;
use kadr_core::ids::{ClipId, EffectId};
use kadr_engine::{
    MediaBackend, RegionTrackingError, RegionTrackingReport, RegionTrackingRequest,
    TrackingCancellation, TrackingProgress, track_region,
};

use crate::output::ToolError;

/// What the analysis is for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BlurTaskPurpose {
    /// A new tracked blur on the clip, with this style (its samples are empty).
    NewBlur {
        /// Style, shape and margin of the new blur.
        style: TrackedBlur,
    },
    /// A correction of an existing tracked blur, from a redrawn region.
    Correction {
        /// The blur being corrected.
        effect_id: EffectId,
        /// The style, shape and margin to give it, if they change too.
        new_style: Option<TrackedBlur>,
    },
}

/// Where an analysis stands.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BlurTaskState {
    /// Still analysing.
    Running {
        /// Frames analysed so far, and in total.
        progress: TrackingProgress,
    },
    /// Analysed; waiting to be added to the project by the toolbox.
    Analysed {
        /// What the tracker found.
        report: Arc<RegionTrackingReport>,
        /// How long the analysis took.
        duration: Duration,
    },
    /// Added to the project.
    Applied {
        /// The tracked blur created or corrected.
        effect_id: EffectId,
        /// Frames the tracker was confident, uncertain and lost on.
        confidence_counts: (usize, usize, usize),
        /// How long the analysis took.
        duration: Duration,
    },
    /// The analysis failed, or its result could not be added; nothing changed.
    Failed {
        /// Why, in plain words.
        reason: String,
    },
    /// The analysis was cancelled; nothing changed.
    Cancelled,
}

/// One analysis, as seen by the tools.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BlurTaskStatus {
    /// For example `"blur-1"`.
    pub(crate) task_id: String,
    /// The clip being analysed.
    pub(crate) clip_id: ClipId,
    /// New blur or correction.
    pub(crate) purpose: BlurTaskPurpose,
    /// Time since the analysis started (its duration once finished).
    pub(crate) elapsed: Duration,
    /// Where it stands.
    pub(crate) state: BlurTaskState,
}

/// A finished analysis to add to the project.
#[derive(Debug, Clone)]
pub(crate) struct AnalysedBlur {
    /// The task's number.
    pub(crate) task_number: u64,
    /// For example `"blur-1"`.
    pub(crate) task_id: String,
    /// The project file the clip belongs to.
    pub(crate) project_file: PathBuf,
    /// The clip analysed.
    pub(crate) clip_id: ClipId,
    /// New blur or correction.
    pub(crate) purpose: BlurTaskPurpose,
    /// What was analysed.
    pub(crate) request: RegionTrackingRequest,
    /// What the tracker found.
    pub(crate) report: Arc<RegionTrackingReport>,
    /// How long the analysis took.
    pub(crate) duration: Duration,
}

#[derive(Debug)]
struct BlurTask {
    project_file: PathBuf,
    clip_id: ClipId,
    purpose: BlurTaskPurpose,
    request: RegionTrackingRequest,
    started_at: Instant,
    state: Arc<Mutex<BlurTaskState>>,
    cancellation: TrackingCancellation,
}

impl BlurTask {
    fn state(&self) -> BlurTaskState {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set_state(&self, new_state: BlurTaskState) {
        *self.state.lock().unwrap_or_else(PoisonError::into_inner) = new_state;
    }
}

/// The analyses started since the toolbox was created, by identifier.
#[derive(Debug, Default)]
pub(crate) struct BlurTasks {
    tasks: BTreeMap<u64, BlurTask>,
    last_task_number: u64,
}

impl BlurTasks {
    /// Starts analysing `request` on a background thread and returns the task
    /// identifier at once.
    ///
    /// # Errors
    ///
    /// Fails when a correction of the same blur is still running, or the thread
    /// cannot be started.
    pub(crate) fn start(
        &mut self,
        clip_id: ClipId,
        project_file: PathBuf,
        purpose: BlurTaskPurpose,
        request: RegionTrackingRequest,
        backend: Arc<dyn MediaBackend>,
    ) -> Result<String, ToolError> {
        if let BlurTaskPurpose::Correction { effect_id, .. } = &purpose
            && let Some(running) = self.unfinished_task_correcting(clip_id, *effect_id)
        {
            return Err(ToolError::new(format!(
                "{running} is still correcting blur {effect_id} of clip {clip_id}: wait for it \
                 (get_blur_status) or cancel it (cancel_blur) before correcting it again"
            )));
        }
        let state = Arc::new(Mutex::new(BlurTaskState::Running {
            progress: TrackingProgress {
                analysed_frame_count: 0,
                total_frame_count: 0,
            },
        }));
        let cancellation = TrackingCancellation::new();
        let started_at = Instant::now();
        spawn_analysis_thread(
            request.clone(),
            backend,
            Arc::clone(&state),
            cancellation.clone(),
            started_at,
        )?;
        self.last_task_number += 1;
        self.tasks.insert(
            self.last_task_number,
            BlurTask {
                project_file,
                clip_id,
                purpose,
                request,
                started_at,
                state,
                cancellation,
            },
        );
        Ok(task_identifier(self.last_task_number))
    }

    /// The status of one task.
    ///
    /// # Errors
    ///
    /// Fails when no task has this identifier; the message lists the existing ones.
    pub(crate) fn status(&self, task_id: &str) -> Result<BlurTaskStatus, ToolError> {
        let task_number = self.require_task_number(task_id)?;
        Ok(self.status_of(task_number))
    }

    /// The status of every task, oldest first.
    pub(crate) fn all_statuses(&self) -> Vec<BlurTaskStatus> {
        self.tasks
            .keys()
            .map(|task_number| self.status_of(*task_number))
            .collect()
    }

    /// Asks a running analysis to stop (within one frame). A finished analysis
    /// is not undone: use `undo` or `remove_effect` for that.
    ///
    /// # Errors
    ///
    /// Fails when no task has this identifier.
    pub(crate) fn cancel(&self, task_id: &str) -> Result<BlurTaskStatus, ToolError> {
        let task_number = self.require_task_number(task_id)?;
        if let Some(task) = self.tasks.get(&task_number) {
            task.cancellation.cancel();
            // An analysis that finished but was not added yet is dropped too.
            if matches!(task.state(), BlurTaskState::Analysed { .. }) {
                task.set_state(BlurTaskState::Cancelled);
            }
        }
        Ok(self.status_of(task_number))
    }

    /// The finished analyses not yet added to the project, oldest first. Each
    /// must then get its outcome with [`Self::record_outcome`].
    pub(crate) fn analysed_blurs(&self) -> Vec<AnalysedBlur> {
        self.tasks
            .iter()
            .filter_map(|(task_number, task)| match task.state() {
                BlurTaskState::Analysed { report, duration } => Some(AnalysedBlur {
                    task_number: *task_number,
                    task_id: task_identifier(*task_number),
                    project_file: task.project_file.clone(),
                    clip_id: task.clip_id,
                    purpose: task.purpose.clone(),
                    request: task.request.clone(),
                    report,
                    duration,
                }),
                _ => None,
            })
            .collect()
    }

    /// Records what happened to an analysed blur: applied, or failed.
    pub(crate) fn record_outcome(&self, task_number: u64, outcome: BlurTaskState) {
        if let Some(task) = self.tasks.get(&task_number) {
            task.set_state(outcome);
        }
    }

    fn status_of(&self, task_number: u64) -> BlurTaskStatus {
        let task = &self.tasks[&task_number];
        let state = task.state();
        let elapsed = match &state {
            BlurTaskState::Analysed { duration, .. } | BlurTaskState::Applied { duration, .. } => {
                *duration
            }
            _ => task.started_at.elapsed(),
        };
        BlurTaskStatus {
            task_id: task_identifier(task_number),
            clip_id: task.clip_id,
            purpose: task.purpose.clone(),
            elapsed,
            state,
        }
    }

    fn unfinished_task_correcting(&self, clip_id: ClipId, effect_id: EffectId) -> Option<String> {
        self.all_statuses()
            .into_iter()
            .find(|status| {
                status.clip_id == clip_id
                    && matches!(
                        status.purpose,
                        BlurTaskPurpose::Correction { effect_id: corrected, .. } if corrected == effect_id
                    )
                    && matches!(
                        status.state,
                        BlurTaskState::Running { .. } | BlurTaskState::Analysed { .. }
                    )
            })
            .map(|status| status.task_id)
    }

    fn require_task_number(&self, task_id: &str) -> Result<u64, ToolError> {
        let task_number = task_id
            .trim()
            .strip_prefix(TASK_IDENTIFIER_PREFIX)
            .and_then(|number| number.parse::<u64>().ok())
            .filter(|number| self.tasks.contains_key(number));
        task_number.ok_or_else(|| {
            let existing: Vec<String> = self
                .tasks
                .keys()
                .map(|number| task_identifier(*number))
                .collect();
            let existing = if existing.is_empty() {
                "none (blur_region was not called since Kadr started)".to_string()
            } else {
                existing.join(", ")
            };
            ToolError::new(format!(
                "there is no blur task \"{task_id}\"; existing tasks: {existing}"
            ))
        })
    }
}

const TASK_IDENTIFIER_PREFIX: &str = "blur-";

fn task_identifier(task_number: u64) -> String {
    format!("{TASK_IDENTIFIER_PREFIX}{task_number}")
}

/// Runs the analysis on its own thread, recording progress and the outcome in `state`.
fn spawn_analysis_thread(
    request: RegionTrackingRequest,
    backend: Arc<dyn MediaBackend>,
    state: Arc<Mutex<BlurTaskState>>,
    cancellation: TrackingCancellation,
    started_at: Instant,
) -> Result<(), ToolError> {
    thread::Builder::new()
        .name("kadr-tracking".to_string())
        .spawn(move || {
            let progress_state = Arc::clone(&state);
            let mut record_progress = move |progress: TrackingProgress| {
                *progress_state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = BlurTaskState::Running { progress };
            };
            let result = track_region(
                backend.as_ref(),
                &request,
                &mut record_progress,
                &cancellation,
            );
            let final_state = match result {
                Ok(report) => BlurTaskState::Analysed {
                    report: Arc::new(report),
                    duration: started_at.elapsed(),
                },
                Err(RegionTrackingError::Cancelled) => BlurTaskState::Cancelled,
                Err(error) => BlurTaskState::Failed {
                    reason: error.to_string(),
                },
            };
            let mut current = state.lock().unwrap_or_else(PoisonError::into_inner);
            // A cancellation that arrived after the last frame still wins.
            *current = if cancellation.is_cancelled() {
                BlurTaskState::Cancelled
            } else {
                final_state
            };
        })
        .map(|_| ())
        .map_err(|error| ToolError::new(format!("the analysis could not be started: {error}")))
}
