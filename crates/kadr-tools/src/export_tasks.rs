//! Exports running in the background, followed by a task identifier.
//!
//! An export can take minutes, far longer than a tool call should. The `export`
//! tool starts one of these tasks and answers at once with its identifier
//! (`"export-1"`); `get_export_status` reads its progress, and `cancel_export`
//! stops it. Each export renders a **copy** of the project taken when it
//! started, so editing meanwhile does not change it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use kadr_core::project::Project;
use kadr_engine::{
    ExportCancellation, ExportError, ExportProgress, ExportReport, ExportRequest, MediaBackend,
    export_project,
};

use crate::output::ToolError;

/// Where an export stands.
#[derive(Debug, Clone, PartialEq)]
pub enum ExportTaskState {
    /// Still rendering.
    Running {
        /// Frames written so far, and in total.
        progress: ExportProgress,
    },
    /// The file is complete.
    Finished {
        /// What was written.
        report: ExportReport,
        /// How long the export took.
        duration: Duration,
    },
    /// The export failed; the output file was not created or replaced.
    Failed {
        /// Why, in plain words.
        reason: String,
    },
    /// The export was cancelled; the output file was not created or replaced.
    Cancelled,
}

/// One export task, as seen by the tools.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportTaskStatus {
    /// For example `"export-1"`.
    pub task_id: String,
    /// The file being written.
    pub output_path: PathBuf,
    /// Time since the export started.
    pub elapsed: Duration,
    /// Where it stands.
    pub state: ExportTaskState,
}

#[derive(Debug)]
struct ExportTask {
    output_path: PathBuf,
    started_at: Instant,
    state: Arc<Mutex<ExportTaskState>>,
    cancellation: ExportCancellation,
}

/// The exports started since the server started, by identifier.
#[derive(Debug, Default)]
pub struct ExportTasks {
    tasks: BTreeMap<u64, ExportTask>,
    last_task_number: u64,
}

impl ExportTasks {
    /// Starts exporting `project` on a background thread and returns the task
    /// identifier at once.
    ///
    /// # Errors
    ///
    /// Fails when another running export writes to the same file, or the thread
    /// cannot be started.
    pub fn start(
        &mut self,
        project: Project,
        backend: Arc<dyn MediaBackend>,
        request: ExportRequest,
    ) -> Result<String, ToolError> {
        if let Some(task_id) = self.running_task_writing(&request.output_path) {
            return Err(ToolError::new(format!(
                "{task_id} is already exporting to \"{}\": wait for it (get_export_status) or \
                 cancel it (cancel_export) first",
                request.output_path.display()
            )));
        }
        let state = Arc::new(Mutex::new(ExportTaskState::Running {
            progress: ExportProgress {
                encoded_frame_count: 0,
                total_frame_count: project
                    .settings()
                    .frame_rate
                    .frame_count_covering(project.timeline().duration()),
            },
        }));
        let cancellation = ExportCancellation::new();
        let output_path = request.output_path.clone();
        let started_at = Instant::now();
        spawn_export_thread(
            project,
            backend,
            request,
            Arc::clone(&state),
            cancellation.clone(),
            started_at,
        )?;

        self.last_task_number += 1;
        self.tasks.insert(
            self.last_task_number,
            ExportTask {
                output_path,
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
    pub fn status(&self, task_id: &str) -> Result<ExportTaskStatus, ToolError> {
        let task_number = self.require_task_number(task_id)?;
        Ok(self.status_of(task_number))
    }

    /// The status of every task, oldest first.
    pub fn all_statuses(&self) -> Vec<ExportTaskStatus> {
        self.tasks
            .keys()
            .map(|task_number| self.status_of(*task_number))
            .collect()
    }

    /// Asks a running export to stop. Stopping takes at most one frame.
    ///
    /// # Errors
    ///
    /// Fails when no task has this identifier.
    pub fn cancel(&self, task_id: &str) -> Result<ExportTaskStatus, ToolError> {
        let task_number = self.require_task_number(task_id)?;
        if let Some(task) = self.tasks.get(&task_number) {
            task.cancellation.cancel();
        }
        Ok(self.status_of(task_number))
    }

    fn status_of(&self, task_number: u64) -> ExportTaskStatus {
        let task = &self.tasks[&task_number];
        let state = task
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let elapsed = match &state {
            ExportTaskState::Finished { duration, .. } => *duration,
            _ => task.started_at.elapsed(),
        };
        ExportTaskStatus {
            task_id: task_identifier(task_number),
            output_path: task.output_path.clone(),
            elapsed,
            state,
        }
    }

    fn running_task_writing(&self, output_path: &Path) -> Option<String> {
        self.all_statuses()
            .into_iter()
            .find(|status| {
                status.output_path == output_path
                    && matches!(status.state, ExportTaskState::Running { .. })
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
                "none (no export was started since Kadr started)".to_string()
            } else {
                existing.join(", ")
            };
            ToolError::new(format!(
                "there is no export task \"{task_id}\"; existing tasks: {existing}"
            ))
        })
    }
}

const TASK_IDENTIFIER_PREFIX: &str = "export-";

fn task_identifier(task_number: u64) -> String {
    format!("{TASK_IDENTIFIER_PREFIX}{task_number}")
}

/// Runs the export on its own thread, recording progress and the outcome in `state`.
fn spawn_export_thread(
    project: Project,
    backend: Arc<dyn MediaBackend>,
    request: ExportRequest,
    state: Arc<Mutex<ExportTaskState>>,
    cancellation: ExportCancellation,
    started_at: Instant,
) -> Result<(), ToolError> {
    thread::Builder::new()
        .name("kadr-export".to_string())
        .spawn(move || {
            let progress_state = Arc::clone(&state);
            let mut record_progress = move |progress: ExportProgress| {
                *progress_state
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) =
                    ExportTaskState::Running { progress };
            };
            let result = export_project(
                &project,
                backend.as_ref(),
                &request,
                &mut record_progress,
                &cancellation,
            );
            let final_state = match result {
                Ok(report) => ExportTaskState::Finished {
                    report,
                    duration: started_at.elapsed(),
                },
                Err(ExportError::Cancelled) => ExportTaskState::Cancelled,
                Err(error) => ExportTaskState::Failed {
                    reason: error.to_string(),
                },
            };
            *state.lock().unwrap_or_else(PoisonError::into_inner) = final_state;
        })
        .map(|_| ())
        .map_err(|error| ToolError::new(format!("the export could not be started: {error}")))
}
