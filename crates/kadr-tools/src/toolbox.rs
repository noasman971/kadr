//! [`KadrToolbox`]: the state shared by the tools, and the entry point that runs
//! a tool call by name.
//!
//! The toolbox keeps what lasts between calls (the allowed folders, the media
//! backend, the background exports and analyses). The project comes from a
//! [`ProjectHost`]: the toolbox's own `.kadr` file host in file mode
//! ([`KadrToolbox::call_tool`]), or the host a desktop app passes for each call
//! ([`KadrToolbox::call_tool_on`]). During a call, the tools see both through a
//! [`ToolCall`].

use std::path::Path;
use std::sync::{Arc, LazyLock};

use kadr_core::history::Transaction;
use kadr_core::project::Project;
use kadr_engine::MediaBackend;
use serde_json::Value;

use crate::blur_tasks::BlurTasks;
use crate::catalog::{ToolDefinition, ToolEffect, ToolEntry};
use crate::export_tasks::ExportTasks;
use crate::file_access::AllowedFolders;
use crate::output::{ToolContent, ToolError, ToolOutput};
use crate::project_host::{EditableProject, FileProjectHost, ProjectHost};
use crate::tools::all_tools;
use crate::tools::blur_tools::{BLUR_STATUS_TOOL_NAME, add_analysed_blurs_to_project};

/// Every tool, built once.
static TOOLS: LazyLock<Vec<ToolEntry>> = LazyLock::new(all_tools);

/// Runs Kadr's tools.
///
/// The toolbox owns the folders the user allowed, the media backend, the
/// background exports and tracking analyses, and, for file mode, the open
/// `.kadr` file. Calls are made one after the other (`&mut self`); exports and
/// analyses run on their own threads.
pub struct KadrToolbox {
    services: ToolServices,
    file_host: FileProjectHost,
}

/// What the tools use besides the project.
struct ToolServices {
    allowed_folders: AllowedFolders,
    backend: Arc<dyn MediaBackend>,
    export_tasks: ExportTasks,
    blur_tasks: BlurTasks,
    /// What happened in the background since the last call, told with the next answer.
    background_notices: Vec<String>,
}

impl std::fmt::Debug for KadrToolbox {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KadrToolbox")
            .field("allowed_folders", &self.services.allowed_folders)
            .field("open_project_file", &self.file_host.file_path())
            .finish_non_exhaustive()
    }
}

impl KadrToolbox {
    /// A toolbox with no project open, reading and writing files only inside
    /// `allowed_folders`, decoding and encoding with `backend`.
    pub fn new(allowed_folders: AllowedFolders, backend: Arc<dyn MediaBackend>) -> Self {
        Self {
            services: ToolServices {
                allowed_folders,
                backend,
                export_tasks: ExportTasks::default(),
                blur_tasks: BlurTasks::default(),
                background_notices: Vec::new(),
            },
            file_host: FileProjectHost::default(),
        }
    }

    /// The definitions of every tool, in a stable order.
    pub fn tool_definitions() -> Vec<ToolDefinition> {
        TOOLS.iter().map(|entry| entry.definition.clone()).collect()
    }

    /// What the tool `tool_name` may change, or `None` for an unknown tool.
    pub fn tool_effect(tool_name: &str) -> Option<ToolEffect> {
        TOOLS
            .iter()
            .find(|entry| entry.definition.name == tool_name)
            .map(|entry| entry.definition.effect)
    }

    /// The folders the tools may use.
    pub fn allowed_folders(&self) -> &AllowedFolders {
        &self.services.allowed_folders
    }

    /// Replaces the folders the tools may use (a desktop app applies the
    /// folders of each call's client).
    pub fn set_allowed_folders(&mut self, allowed_folders: AllowedFolders) {
        self.services.allowed_folders = allowed_folders;
    }

    /// In file mode, the open project's file.
    pub fn file_mode_project_file(&self) -> Option<&Path> {
        self.file_host.file_path()
    }

    /// Runs the tool `tool_name` with `arguments` (a JSON object, or null for
    /// none) on the toolbox's own project file (file mode). Failures are
    /// returned as an output with `is_error` set and a message saying what to do.
    ///
    /// Finished tracking analyses are added to the project first (each as its
    /// own undo step), and the answer ends with a note about them, except the
    /// answer of `get_blur_status`, which tells their state itself.
    pub fn call_tool(&mut self, tool_name: &str, arguments: Value) -> ToolOutput {
        self.apply_finished_background_work();
        let Self {
            services,
            file_host,
        } = self;
        services.run_tool(file_host, tool_name, arguments)
    }

    /// Runs the tool `tool_name` on the project of `host` (the desktop app's).
    /// The host adds finished analyses itself
    /// ([`Self::apply_finished_background_work_on`]); notes about them end the
    /// answer as with [`Self::call_tool`].
    pub fn call_tool_on(
        &mut self,
        host: &mut dyn ProjectHost,
        tool_name: &str,
        arguments: Value,
    ) -> ToolOutput {
        self.services.run_tool(host, tool_name, arguments)
    }

    /// Adds the tracking analyses that finished to the toolbox's own project,
    /// each as one undo step, and saves it. [`Self::call_tool`] does it before
    /// every call; a host calls it regularly so that the project file is up to
    /// date even when Claude makes no further call.
    pub fn apply_finished_background_work(&mut self) {
        let Self {
            services,
            file_host,
        } = self;
        services.apply_finished_background_work(file_host);
    }

    /// Same as [`Self::apply_finished_background_work`], on `host`'s project.
    /// Returns what was done, in sentences (the next answer tells Claude too).
    pub fn apply_finished_background_work_on(&mut self, host: &mut dyn ProjectHost) -> Vec<String> {
        self.services.apply_finished_background_work(host)
    }

    /// Whether an analysis has finished and waits to be added to the project.
    pub fn has_finished_background_work(&self) -> bool {
        !self.services.blur_tasks.analysed_blurs().is_empty()
    }
}

impl ToolServices {
    fn run_tool(
        &mut self,
        host: &mut dyn ProjectHost,
        tool_name: &str,
        arguments: Value,
    ) -> ToolOutput {
        let mut output = ToolCall {
            services: self,
            host,
        }
        .run(tool_name, arguments);
        let notices = std::mem::take(&mut self.background_notices);
        if !notices.is_empty() && tool_name != BLUR_STATUS_TOOL_NAME {
            output.content.push(ToolContent::Text(format!(
                "Meanwhile, in the background: {}",
                notices.join(" ")
            )));
        }
        output
    }

    fn apply_finished_background_work(&mut self, host: &mut dyn ProjectHost) -> Vec<String> {
        let notices = add_analysed_blurs_to_project(&mut ToolCall {
            services: self,
            host,
        });
        self.background_notices.extend(notices.iter().cloned());
        notices
    }
}

/// What a tool sees during a call: the toolbox's services and the project host.
pub(crate) struct ToolCall<'call> {
    services: &'call mut ToolServices,
    host: &'call mut dyn ProjectHost,
}

impl ToolCall<'_> {
    fn run(&mut self, tool_name: &str, arguments: Value) -> ToolOutput {
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

    /// The folders the user allowed.
    pub(crate) fn allowed_folders(&self) -> &AllowedFolders {
        &self.services.allowed_folders
    }

    /// The media backend (FFmpeg on desktop).
    pub(crate) fn backend(&self) -> &dyn MediaBackend {
        self.services.backend.as_ref()
    }

    /// A shared handle on the media backend, for background tasks.
    pub(crate) fn shared_backend(&self) -> Arc<dyn MediaBackend> {
        Arc::clone(&self.services.backend)
    }

    /// The background exports.
    pub(crate) fn export_tasks(&mut self) -> &mut ExportTasks {
        &mut self.services.export_tasks
    }

    /// The background tracking analyses.
    pub(crate) fn blur_tasks(&self) -> &BlurTasks {
        &self.services.blur_tasks
    }

    /// The background tracking analyses, to start one.
    pub(crate) fn blur_tasks_mut(&mut self) -> &mut BlurTasks {
        &mut self.services.blur_tasks
    }

    /// Where the project lives, to open or create one.
    pub(crate) fn host_mut(&mut self) -> &mut dyn ProjectHost {
        self.host
    }

    /// The open project, if any.
    pub(crate) fn open_project(&self) -> Option<&dyn EditableProject> {
        self.host.open_project()
    }

    /// The open project, or an error saying how to open one.
    pub(crate) fn require_project_session(&self) -> Result<&dyn EditableProject, ToolError> {
        self.host.open_project().ok_or_else(no_project_open)
    }

    /// Same as [`Self::require_project_session`], to change it.
    pub(crate) fn require_project_session_mut(
        &mut self,
    ) -> Result<&mut dyn EditableProject, ToolError> {
        self.host.open_project_mut().ok_or_else(no_project_open)
    }

    /// The open project's content, or an error saying how to open one.
    pub(crate) fn require_project(&self) -> Result<&Project, ToolError> {
        Ok(self.require_project_session()?.project())
    }

    /// Applies `steps` to the open project as one undo step labelled
    /// `"Claude: " + label`, and saves it (see [`EditableProject::change_in_one_step`]).
    pub(crate) fn change_project<Output>(
        &mut self,
        label: &str,
        steps: impl FnOnce(&mut Transaction<'_>) -> Result<Output, ToolError>,
    ) -> Result<Output, ToolError> {
        let mut steps = Some(steps);
        let mut output = None;
        self.require_project_session_mut()?
            .change_in_one_step(label, &mut |transaction| {
                let steps = steps
                    .take()
                    .ok_or_else(|| ToolError::new("internal error: a tool's steps ran twice"))?;
                output = Some(steps(transaction)?);
                Ok(())
            })?;
        output.ok_or_else(|| ToolError::new("internal error: a tool's steps did not run"))
    }
}

fn no_project_open() -> ToolError {
    ToolError::new(
        "no project is open. Call open_project with the full path of a .kadr file, or \
         create_project to start a new one (ask the user where to save it)",
    )
}
