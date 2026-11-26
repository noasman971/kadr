//! Where the project the tools edit lives (decision record 0030).
//!
//! The tools never hold the project themselves: they reach it through a
//! [`ProjectHost`], which may have a project open ([`EditableProject`]).
//!
//! - In **file mode** (`kadr-mcp` alone, decision record 0012) the host is a
//!   [`FileProjectHost`]: the project lives in a `.kadr` file, saved after every
//!   change.
//! - When the **desktop app** is open, `kadr-mcp` passes the calls to it, and the
//!   app's own host gives the tools the project its window shows, with its undo
//!   history.
//!
//! Every change goes through [`EditableProject::change_in_one_step`]: one call,
//! one undo step labelled `"Claude: …"`.

use std::path::{Path, PathBuf};

use kadr_core::error::EditError;
use kadr_core::history::{EditSession, Transaction};
use kadr_core::project::Project;

use crate::output::ToolError;
use crate::project_file_session::{CLAUDE_UNDO_LABEL_PREFIX, ProjectFileSession};

/// The steps of one tool call, applied in one transaction.
pub type ToolSteps<'steps> = dyn FnMut(&mut Transaction<'_>) -> Result<(), ToolError> + 'steps;

/// A project open for the tools.
pub trait EditableProject {
    /// The project as it is now.
    fn project(&self) -> &Project;

    /// Its `.kadr` file.
    fn file_path(&self) -> &Path;

    /// The labels of the steps undo can revert, oldest first.
    fn undo_history_labels(&self) -> Vec<String>;

    /// What undo would revert.
    fn next_undo_label(&self) -> Option<String>;

    /// What redo would apply again.
    fn next_redo_label(&self) -> Option<String>;

    /// Applies `steps` as **one** undo step labelled `"Claude: " + label`, and
    /// saves. Nothing changes if a step fails.
    ///
    /// # Errors
    ///
    /// The first failing step's error, or why the change could not be kept.
    fn change_in_one_step(
        &mut self,
        label: &str,
        steps: &mut ToolSteps<'_>,
    ) -> Result<(), ToolError>;

    /// Undoes the last step and returns its label.
    ///
    /// # Errors
    ///
    /// When there is nothing (or nothing of Claude's) to undo.
    fn undo(&mut self) -> Result<String, ToolError>;

    /// Redoes the last undone step and returns its label.
    ///
    /// # Errors
    ///
    /// When there is nothing to redo.
    fn redo(&mut self) -> Result<String, ToolError>;

    /// Saves to its file.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    fn save(&mut self) -> Result<(), ToolError>;

    /// Saves under `file_path`, which becomes its file.
    ///
    /// # Errors
    ///
    /// When the file cannot be written; the project keeps its old file then.
    fn save_as(&mut self, file_path: PathBuf) -> Result<(), ToolError>;
}

/// What holds the open project: the tools ask it to open or create one.
pub trait ProjectHost {
    /// The open project, if any.
    fn open_project(&self) -> Option<&dyn EditableProject>;

    /// The open project, to change it.
    fn open_project_mut(&mut self) -> Option<&mut dyn EditableProject>;

    /// Saves `project` as a new file at `file_path` (already checked) and opens it.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    fn create_project(&mut self, project: Project, file_path: PathBuf) -> Result<(), ToolError>;

    /// Opens the project saved in `file_path` (already checked).
    ///
    /// # Errors
    ///
    /// When the file cannot be read or is not a valid project.
    fn open_project_file(&mut self, file_path: PathBuf) -> Result<(), ToolError>;
}

/// File mode: the project lives in a `.kadr` file the tools save after every change.
#[derive(Debug, Default)]
pub struct FileProjectHost {
    open: Option<ProjectFileSession>,
}

impl FileProjectHost {
    /// The open project's file, if any.
    pub fn file_path(&self) -> Option<&Path> {
        self.open.as_ref().map(ProjectFileSession::file_path)
    }
}

impl ProjectHost for FileProjectHost {
    fn open_project(&self) -> Option<&dyn EditableProject> {
        self.open
            .as_ref()
            .map(|session| session as &dyn EditableProject)
    }

    fn open_project_mut(&mut self) -> Option<&mut dyn EditableProject> {
        self.open
            .as_mut()
            .map(|session| session as &mut dyn EditableProject)
    }

    fn create_project(&mut self, project: Project, file_path: PathBuf) -> Result<(), ToolError> {
        self.open = Some(ProjectFileSession::create(project, file_path)?);
        Ok(())
    }

    fn open_project_file(&mut self, file_path: PathBuf) -> Result<(), ToolError> {
        self.open = Some(ProjectFileSession::open(file_path)?);
        Ok(())
    }
}

impl EditableProject for ProjectFileSession {
    fn project(&self) -> &Project {
        ProjectFileSession::project(self)
    }

    fn file_path(&self) -> &Path {
        ProjectFileSession::file_path(self)
    }

    fn undo_history_labels(&self) -> Vec<String> {
        self.edit_session()
            .undo_history_labels()
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    fn next_undo_label(&self) -> Option<String> {
        self.edit_session().next_undo_label().map(str::to_string)
    }

    fn next_redo_label(&self) -> Option<String> {
        self.edit_session().next_redo_label().map(str::to_string)
    }

    fn change_in_one_step(
        &mut self,
        label: &str,
        steps: &mut ToolSteps<'_>,
    ) -> Result<(), ToolError> {
        self.change(label, |transaction| steps(transaction))
    }

    fn undo(&mut self) -> Result<String, ToolError> {
        ProjectFileSession::undo(self)
    }

    fn redo(&mut self) -> Result<String, ToolError> {
        ProjectFileSession::redo(self)
    }

    fn save(&mut self) -> Result<(), ToolError> {
        ProjectFileSession::save(self)
    }

    fn save_as(&mut self, file_path: PathBuf) -> Result<(), ToolError> {
        ProjectFileSession::save_as(self, file_path)
    }
}

/// Runs a tool's `steps` on `session` as one transaction labelled
/// `"Claude: " + label`. A failing step's own error is returned (rather than
/// the transaction's), and the transaction is then undone entirely. Hosts
/// keeping an `EditSession` (the desktop app) use it to implement
/// [`EditableProject::change_in_one_step`].
///
/// # Errors
///
/// The first failing step's error, or the engine's refusal.
pub fn run_tool_steps(
    session: &mut EditSession,
    label: &str,
    steps: &mut ToolSteps<'_>,
) -> Result<(), ToolError> {
    let mut step_error: Option<ToolError> = None;
    session
        .transaction(
            format!("{CLAUDE_UNDO_LABEL_PREFIX}{label}"),
            |transaction| {
                steps(transaction).map_err(|error| {
                    step_error = Some(error);
                    EditError::InternalInconsistency {
                        details: "a tool step failed".to_string(),
                    }
                })
            },
        )
        .map_err(|edit_error| step_error.unwrap_or_else(|| edit_error.into()))
}
