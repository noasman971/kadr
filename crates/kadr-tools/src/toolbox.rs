//! [`KadrToolbox`]: the state shared by the tools, and the entry point that runs
//! a tool call by name.

use std::sync::{Arc, LazyLock};

use kadr_core::history::Transaction;
use kadr_core::project::Project;
use kadr_engine::MediaBackend;
use serde_json::Value;

use crate::blur_tasks::BlurTasks;
use crate::catalog::{ToolDefinition, ToolEntry};
use crate::export_tasks::ExportTasks;
use crate::file_access::AllowedFolders;
use crate::output::{ToolContent, ToolError, ToolOutput};
use crate::project_file_session::ProjectFileSession;
use crate::tools::all_tools;
use crate::tools::blur_tools::{BLUR_STATUS_TOOL_NAME, add_analysed_blurs_to_project};

/// Every tool, built once.
static TOOLS: LazyLock<Vec<ToolEntry>> = LazyLock::new(all_tools);

/// Runs Kadr's tools on one project at a time.
///
/// The toolbox owns the open project (with its undo history), the folders the
/// user allowed, the media backend, and the background exports and tracking
/// analyses. Calls are made one after the other (`&mut self`); exports and
/// analyses run on their own threads.
pub struct KadrToolbox {
    allowed_folders: AllowedFolders,
    backend: Arc<dyn MediaBackend>,
    open_project: Option<ProjectFileSession>,
    export_tasks: ExportTasks,
    blur_tasks: BlurTasks,
    /// What happened in the background since the last call, told with the next answer.
    background_notices: Vec<String>,
}

impl std::fmt::Debug for KadrToolbox {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KadrToolbox")
            .field("allowed_folders", &self.allowed_folders)
            .field(
                "open_project",
                &self.open_project.as_ref().map(|open| open.file_path()),
            )
            .finish_non_exhaustive()
    }
}

impl KadrToolbox {
    /// A toolbox with no project open, reading and writing files only inside
    /// `allowed_folders`, decoding and encoding with `backend`.
    pub fn new(allowed_folders: AllowedFolders, backend: Arc<dyn MediaBackend>) -> Self {
        Self {
            allowed_folders,
            backend,
            open_project: None,
            export_tasks: ExportTasks::default(),
            blur_tasks: BlurTasks::default(),
            background_notices: Vec::new(),
        }
    }

    /// The definitions of every tool, in a stable order.
    pub fn tool_definitions() -> Vec<ToolDefinition> {
        TOOLS.iter().map(|entry| entry.definition.clone()).collect()
    }

    /// Runs the tool `tool_name` with `arguments` (a JSON object, or null for
    /// none). Failures are returned as an output with `is_error` set and a
    /// message saying what to do.
    ///
    /// Finished tracking analyses are added to the project first (each as its
    /// own undo step), and the answer ends with a note about them, except the
    /// answer of `get_blur_status`, which tells their state itself.
    pub fn call_tool(&mut self, tool_name: &str, arguments: Value) -> ToolOutput {
        self.apply_finished_background_work();
        let mut output = self.run_tool(tool_name, arguments);
        let notices = std::mem::take(&mut self.background_notices);
        if !notices.is_empty() && tool_name != BLUR_STATUS_TOOL_NAME {
            output.content.push(ToolContent::Text(format!(
                "Meanwhile, in the background: {}",
                notices.join(" ")
            )));
        }
        output
    }

    /// Adds the tracking analyses that finished to the project, each as one
    /// undo step, and saves it. [`Self::call_tool`] does it before every call;
    /// a host calls it regularly so that the project file is up to date even
    /// when Claude makes no further call.
    pub fn apply_finished_background_work(&mut self) {
        let notices = add_analysed_blurs_to_project(self);
        self.background_notices.extend(notices);
    }

    fn run_tool(&mut self, tool_name: &str, arguments: Value) -> ToolOutput {
        let Some(entry) = TOOLS
            .iter()
            .find(|entry| entry.definition.name == tool_name)
        else {
            let known_names: Vec<&str> = TOOLS.iter().map(|entry| entry.definition.name).collect();
            return ToolOutput::from_error(&ToolError::new(format!(
                "there is no tool called \"{tool_name}\"; Kadr's tools are: {}",
                known_names.join(", ")
            )));
        };
        match (entry.run)(self, arguments) {
            Ok(output) => output,
            Err(error) => ToolOutput::from_error(&error),
        }
    }

    // ----- Shared by the tools -----

    /// The folders the user allowed.
    pub(crate) fn allowed_folders(&self) -> &AllowedFolders {
        &self.allowed_folders
    }

    /// The media backend (FFmpeg on desktop).
    pub(crate) fn backend(&self) -> &dyn MediaBackend {
        self.backend.as_ref()
    }

    /// A shared handle on the media backend, for background tasks.
    pub(crate) fn shared_backend(&self) -> Arc<dyn MediaBackend> {
        Arc::clone(&self.backend)
    }

    /// The background exports.
    pub(crate) fn export_tasks(&mut self) -> &mut ExportTasks {
        &mut self.export_tasks
    }

    /// The background tracking analyses.
    pub(crate) fn blur_tasks(&self) -> &BlurTasks {
        &self.blur_tasks
    }

    /// The background tracking analyses, to start one.
    pub(crate) fn blur_tasks_mut(&mut self) -> &mut BlurTasks {
        &mut self.blur_tasks
    }

    /// The open project and its file, if any.
    pub(crate) fn open_project(&self) -> Option<&ProjectFileSession> {
        self.open_project.as_ref()
    }

    /// Replaces the open project.
    pub(crate) fn set_open_project(&mut self, project_session: ProjectFileSession) {
        self.open_project = Some(project_session);
    }

    /// The open project and its file, or an error saying how to open one.
    pub(crate) fn require_project_session(&self) -> Result<&ProjectFileSession, ToolError> {
        self.open_project.as_ref().ok_or_else(no_project_open)
    }

    /// Same as [`Self::require_project_session`], to change it.
    pub(crate) fn require_project_session_mut(
        &mut self,
    ) -> Result<&mut ProjectFileSession, ToolError> {
        self.open_project.as_mut().ok_or_else(no_project_open)
    }

    /// The open project, or an error saying how to open one.
    pub(crate) fn require_project(&self) -> Result<&Project, ToolError> {
        Ok(self.require_project_session()?.project())
    }

    /// Applies `steps` to the open project as one undo step labelled
    /// `"Claude: " + label`, and saves it. See [`ProjectFileSession::change`].
    pub(crate) fn change_project<Output>(
        &mut self,
        label: &str,
        steps: impl FnOnce(&mut Transaction<'_>) -> Result<Output, ToolError>,
    ) -> Result<Output, ToolError> {
        self.require_project_session_mut()?.change(label, steps)
    }
}

fn no_project_open() -> ToolError {
    ToolError::new(
        "no project is open. Call open_project with the full path of a .kadr file, or \
         create_project to start a new one (ask the user where to save it)",
    )
}
