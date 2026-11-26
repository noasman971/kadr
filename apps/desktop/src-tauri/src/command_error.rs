//! Errors sent to the front end.
//!
//! Each error carries a stable **code** the front end translates into a human
//! sentence (in French or English), and the engine's own **details** in English,
//! shown under it and copied with *Copy the details*.

use serde::Serialize;

/// What went wrong, in terms the interface can explain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    /// A command needs an open project and none is open.
    NoProjectOpen,
    /// A project file could not be read or is not a valid project.
    ProjectFileUnreadable,
    /// The project could not be written to disk.
    ProjectFileNotSaved,
    /// A project with this name already exists where it would be created.
    ProjectNameTaken,
    /// A media file is not a video, picture or sound Kadr can read.
    MediaUnreadable,
    /// The engine refused an edit (overlap, out of range…).
    EditRefused,
    /// There is nothing to undo.
    NothingToUndo,
    /// There is nothing to redo.
    NothingToRedo,
    /// A preview frame, thumbnail or sound could not be produced.
    PreviewFailed,
    /// A newer request replaced this one before it was handled.
    Superseded,
    /// The export failed.
    ExportFailed,
    /// The export was cancelled.
    ExportCancelled,
    /// An export is already running.
    ExportAlreadyRunning,
    /// The export file exists and the user has not agreed to replace it.
    ExportFileExists,
    /// The export size has another shape than the project (change the project first).
    ExportOtherShape,
    /// Something that should not happen; the details say what.
    Internal,
}

/// An error answered by a Tauri command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{details}")]
pub struct CommandError {
    /// What went wrong, for the interface to explain.
    pub code: ErrorCode,
    /// The engine's explanation, in English.
    pub details: String,
    /// The file concerned, when there is one (its name only, for messages).
    pub file_name: Option<String>,
}

impl CommandError {
    /// An error with its details.
    pub fn new(code: ErrorCode, details: impl ToString) -> Self {
        Self {
            code,
            details: details.to_string(),
            file_name: None,
        }
    }

    /// The same error, naming the file it is about.
    pub fn about_file(mut self, file_path: &std::path::Path) -> Self {
        self.file_name = file_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        self
    }

    /// No project is open.
    pub fn no_project_open() -> Self {
        Self::new(ErrorCode::NoProjectOpen, "no project is open")
    }

    /// An unexpected failure.
    pub fn internal(details: impl ToString) -> Self {
        Self::new(ErrorCode::Internal, details)
    }
}

impl From<kadr_core::error::EditError> for CommandError {
    fn from(error: kadr_core::error::EditError) -> Self {
        use kadr_core::error::EditError;
        let code = match error {
            EditError::NothingToUndo => ErrorCode::NothingToUndo,
            EditError::NothingToRedo => ErrorCode::NothingToRedo,
            EditError::InternalInconsistency { .. } => ErrorCode::Internal,
            _ => ErrorCode::EditRefused,
        };
        Self::new(code, error)
    }
}

/// The result of a Tauri command.
pub type CommandResult<Output> = Result<Output, CommandError>;
