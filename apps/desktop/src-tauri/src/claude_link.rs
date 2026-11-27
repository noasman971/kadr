//! Claude's calls, run on the project the window shows (decision record 0030).
//!
//! `kadr-mcp`, started by Claude Desktop, passes each tool call through a local
//! pipe (`kadr-app-link`). The app runs it with its own [`KadrToolbox`] on its
//! open project, then tells the front end what changed (the event
//! [`CLAUDE_CHANGED_EVENT`]), so Claude's work shows live, in the same undo
//! history.
//!
//! - Tools that only read get a **snapshot** of the project: a slow picture
//!   never blocks the window.
//! - Tools that change it hold the project's lock for the call: a user gesture
//!   and a Claude call never interleave.
//! - `open_project` and `create_project` open the project in the window.
//! - Analyses started by `blur_region` are added as soon as they end (a timer).
//!
//! Locks are always taken in the same order: the toolbox, then the project.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use kadr_app_link::{LinkRequest, LinkResponse};
use kadr_core::ids::ClipId;
use kadr_core::project::Project;
use kadr_engine::project_files::save_project_file;
use kadr_tools::{
    AllowedFolders, CLAUDE_ACCESS_FILE_NAME, CLAUDE_UNDO_LABEL_PREFIX, EditableProject,
    KadrToolbox, ProjectHost, ToolError, ToolSteps, combined_allowed_folders,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::app_state::{AppState, lock_shared};
use crate::open_project::OpenProject;
use crate::project_view::ProjectView;
use crate::recent_projects::remember_recent_project;

/// The event telling the front end that Claude changed or opened the project.
pub const CLAUDE_CHANGED_EVENT: &str = "claude-changed-project";

/// How often finished analyses are looked for.
const BACKGROUND_WORK_INTERVAL: Duration = Duration::from_secs(1);

/// What Claude did, for the front end.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeChange {
    /// The project after the change.
    pub view: ProjectView,
    /// Claude opened (or created) this project: the window shows it now.
    pub opened_project: bool,
    /// The step Claude added to the history, without the "Claude: " prefix.
    pub step_label: Option<String>,
    /// The clips Claude added or changed, to outline for a moment.
    pub changed_clip_ids: Vec<ClipId>,
}

/// The toolbox running Claude's calls.
pub struct ClaudeToolbox {
    toolbox: Mutex<KadrToolbox>,
}

impl ClaudeToolbox {
    /// A toolbox using the app's media backend; the allowed folders come with
    /// each call.
    pub fn new(backend: Arc<dyn kadr_media::MediaBackend>) -> Self {
        Self {
            toolbox: Mutex::new(KadrToolbox::new(AllowedFolders::default(), backend)),
        }
    }
}

/// Starts listening for `kadr-mcp` and adding finished analyses. Without the
/// pipe (another Kadr app already has it), the app works as before and says
/// why in its log.
pub fn start(app: AppHandle) {
    let handler_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let handler = Arc::new(move |request: LinkRequest| run_call(&handler_app, request));
        if let Err(error) = kadr_app_link::server::serve(handler).await {
            eprintln!("Kadr: Claude cannot reach this window ({error}); is another Kadr open?");
        }
    });
    let spawned = thread::Builder::new()
        .name("claude-background-work".to_string())
        .spawn(move || {
            loop {
                thread::sleep(BACKGROUND_WORK_INTERVAL);
                add_finished_background_work(&app);
            }
        });
    if let Err(error) = spawned {
        eprintln!("Kadr: finished analyses for Claude will wait for its next call ({error})");
    }
}

/// Runs one call from `kadr-mcp`.
fn run_call(app: &AppHandle, request: LinkRequest) -> LinkResponse {
    let state = app.state::<AppState>();
    let mut toolbox = lock_shared(&state.claude.toolbox);
    // The folders kadr-mcp was started with, and those of Kadr's settings.
    toolbox.set_allowed_folders(AllowedFolders::new(combined_allowed_folders(
        &request.allowed_folders,
        Some(&state.configuration_folder.join(CLAUDE_ACCESS_FILE_NAME)),
    )));
    if KadrToolbox::only_reads_the_open_project(&request.tool_name) {
        let mut host = SnapshotHost {
            project: state.lock_open_project().as_ref().map(SnapshotProject::of),
        };
        let output = toolbox.call_tool_on(&mut host, &request.tool_name, request.arguments);
        let project_file = host.project.map(|project| project.file_path);
        return LinkResponse::from_tool_output(&output, project_file);
    }
    let mut slot = state.lock_open_project();
    let before = slot.as_ref().map(|open| (open.identity(), open.revision()));
    let mut host = AppProjectHost {
        slot: &mut slot,
        has_opened_a_project: false,
    };
    let output = toolbox.call_tool_on(&mut host, &request.tool_name, request.arguments);
    let has_opened_a_project = host.has_opened_a_project;
    drop(toolbox);
    let change = change_of(&mut slot, before, has_opened_a_project);
    let project_file = slot.as_ref().map(|open| open.file_path().to_path_buf());
    drop(slot);
    if let (Some(change), Some(project_file)) = (change, &project_file) {
        if change.opened_project {
            let _ = remember_recent_project(&state.configuration_folder, project_file, None);
        }
        let _ = app.emit(CLAUDE_CHANGED_EVENT, change);
        // Claude may have imported heavy videos, or opened another project.
        state.notice_media_of_open_project();
    }
    LinkResponse::from_tool_output(&output, project_file)
}

/// Adds finished analyses to the open project, and tells the front end.
fn add_finished_background_work(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut toolbox = lock_shared(&state.claude.toolbox);
    if !toolbox.has_finished_background_work() {
        return;
    }
    let mut slot = state.lock_open_project();
    let before = slot.as_ref().map(|open| (open.identity(), open.revision()));
    let mut host = AppProjectHost {
        slot: &mut slot,
        has_opened_a_project: false,
    };
    toolbox.apply_finished_background_work_on(&mut host);
    drop(toolbox);
    if let Some(change) = change_of(&mut slot, before, false) {
        drop(slot);
        let _ = app.emit(CLAUDE_CHANGED_EVENT, change);
    }
}

/// What changed in the open project since `before` (its identity and revision).
fn change_of(
    slot: &mut Option<OpenProject>,
    before: Option<(u64, u64)>,
    has_opened_a_project: bool,
) -> Option<ClaudeChange> {
    let open = slot.as_mut()?;
    if has_opened_a_project {
        return Some(ClaudeChange {
            view: open.view(),
            opened_project: true,
            step_label: None,
            changed_clip_ids: Vec::new(),
        });
    }
    if before == Some((open.identity(), open.revision())) {
        return None;
    }
    let view = open.view();
    let step_label = view
        .undo_label
        .as_deref()
        .and_then(|label| label.strip_prefix(CLAUDE_UNDO_LABEL_PREFIX))
        .map(str::to_string);
    Some(ClaudeChange {
        view,
        opened_project: false,
        step_label,
        changed_clip_ids: open.take_clips_changed_by_claude(),
    })
}

/// The window's project, for tools that change it.
struct AppProjectHost<'slot> {
    slot: &'slot mut Option<OpenProject>,
    has_opened_a_project: bool,
}

impl ProjectHost for AppProjectHost<'_> {
    fn open_project(&self) -> Option<&dyn EditableProject> {
        self.slot.as_ref().map(|open| open as &dyn EditableProject)
    }

    fn open_project_mut(&mut self) -> Option<&mut dyn EditableProject> {
        self.slot
            .as_mut()
            .map(|open| open as &mut dyn EditableProject)
    }

    fn create_project(&mut self, project: Project, file_path: PathBuf) -> Result<(), ToolError> {
        save_project_file(&project, &file_path)?;
        self.open_project_file(file_path)
    }

    fn open_project_file(&mut self, file_path: PathBuf) -> Result<(), ToolError> {
        let opened = OpenProject::open(file_path).map_err(|error| ToolError::new(error.details))?;
        if let Some(previous) = self.slot.as_mut() {
            previous.save();
        }
        *self.slot = Some(opened);
        self.has_opened_a_project = true;
        Ok(())
    }
}

/// A copy of the window's project, for tools that only read.
struct SnapshotHost {
    project: Option<SnapshotProject>,
}

struct SnapshotProject {
    project: Arc<Project>,
    file_path: PathBuf,
    undo_history_labels: Vec<String>,
    next_undo_label: Option<String>,
    next_redo_label: Option<String>,
}

impl SnapshotProject {
    fn of(open: &OpenProject) -> Self {
        Self {
            project: open.snapshot(),
            file_path: open.file_path().to_path_buf(),
            undo_history_labels: EditableProject::undo_history_labels(open),
            next_undo_label: EditableProject::next_undo_label(open),
            next_redo_label: EditableProject::next_redo_label(open),
        }
    }
}

fn read_only() -> ToolError {
    ToolError::new("internal error: a tool that only reads tried to change the project")
}

impl ProjectHost for SnapshotHost {
    fn open_project(&self) -> Option<&dyn EditableProject> {
        self.project
            .as_ref()
            .map(|project| project as &dyn EditableProject)
    }

    fn open_project_mut(&mut self) -> Option<&mut dyn EditableProject> {
        None
    }

    fn create_project(&mut self, _: Project, _: PathBuf) -> Result<(), ToolError> {
        Err(read_only())
    }

    fn open_project_file(&mut self, _: PathBuf) -> Result<(), ToolError> {
        Err(read_only())
    }
}

impl EditableProject for SnapshotProject {
    fn project(&self) -> &Project {
        &self.project
    }

    fn file_path(&self) -> &Path {
        &self.file_path
    }

    fn undo_history_labels(&self) -> Vec<String> {
        self.undo_history_labels.clone()
    }

    fn next_undo_label(&self) -> Option<String> {
        self.next_undo_label.clone()
    }

    fn next_redo_label(&self) -> Option<String> {
        self.next_redo_label.clone()
    }

    fn change_in_one_step(&mut self, _: &str, _: &mut ToolSteps<'_>) -> Result<(), ToolError> {
        Err(read_only())
    }

    fn undo(&mut self) -> Result<String, ToolError> {
        Err(read_only())
    }

    fn redo(&mut self) -> Result<String, ToolError> {
        Err(read_only())
    }

    fn save(&mut self) -> Result<(), ToolError> {
        Err(read_only())
    }

    fn save_as(&mut self, _: PathBuf) -> Result<(), ToolError> {
        Err(read_only())
    }
}
