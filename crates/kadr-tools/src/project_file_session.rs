//! The project the tools are editing, kept in sync with its `.kadr` file.
//!
//! In file mode (decision record 0012) the tools work directly on a project
//! file:
//!
//! - the project is loaded into an [`EditSession`], which keeps the undo
//!   history for as long as the server runs;
//! - **every successful change is saved at once** (atomically, by
//!   `kadr_engine::project_files`), so nothing is lost if the conversation ends;
//! - before each change, the file on disk is compared with what Kadr last read
//!   or wrote. If another program changed it meanwhile (for example the desktop
//!   app), the change is refused instead of silently overwriting the other
//!   program's work;
//! - if saving fails, the change is undone, so the project in memory and the
//!   file never disagree.

use std::fs;
use std::path::{Path, PathBuf};

use kadr_core::error::EditError;
use kadr_core::history::{EditSession, Transaction};
use kadr_core::project::Project;
use kadr_core::project_file::save_project_to_json;
use kadr_engine::project_files::{load_project_file, save_project_file};

use crate::output::ToolError;

/// The prefix of every undo step made by a tool call, so the history shows what
/// Claude did.
pub const CLAUDE_UNDO_LABEL_PREFIX: &str = "Claude: ";

/// The largest number of undo steps kept.
const MAXIMUM_UNDO_STEPS: usize = 200;

/// A project open for editing, with its file.
#[derive(Debug)]
pub struct ProjectFileSession {
    session: EditSession,
    file_path: PathBuf,
    /// The file content Kadr last read or wrote, to notice changes by others.
    last_known_file_content: String,
}

impl ProjectFileSession {
    /// Starts editing a new project, saved at once to `file_path`.
    ///
    /// # Errors
    ///
    /// Fails when the project is invalid or the file cannot be written.
    pub fn create(project: Project, file_path: PathBuf) -> Result<Self, ToolError> {
        let session = EditSession::new(project)?.with_maximum_undo_steps(MAXIMUM_UNDO_STEPS);
        let mut created = Self {
            session,
            file_path,
            last_known_file_content: String::new(),
        };
        created.save()?;
        Ok(created)
    }

    /// Opens the project saved in `file_path`.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read or is not a valid project.
    pub fn open(file_path: PathBuf) -> Result<Self, ToolError> {
        let file_content = read_file_text(&file_path)?;
        let project = load_project_file(&file_path)?;
        let session = EditSession::new(project)?.with_maximum_undo_steps(MAXIMUM_UNDO_STEPS);
        Ok(Self {
            session,
            file_path,
            last_known_file_content: file_content,
        })
    }

    /// The project as it is now.
    pub fn project(&self) -> &Project {
        self.session.project()
    }

    /// The edit session (for reading the history).
    pub fn edit_session(&self) -> &EditSession {
        &self.session
    }

    /// The file the project is saved to.
    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    /// Applies `steps` as **one** undo step labelled `"Claude: " + label`, then
    /// saves the file. Nothing changes if any step fails, if the file was
    /// changed by another program, or if saving fails.
    ///
    /// # Errors
    ///
    /// The first failing step's error, or why the file could not be saved.
    pub fn change<Output>(
        &mut self,
        label: &str,
        steps: impl FnOnce(&mut Transaction<'_>) -> Result<Output, ToolError>,
    ) -> Result<Output, ToolError> {
        self.ensure_file_unchanged_by_others()?;
        let project_before = self.session.project().clone();
        let mut step_error: Option<ToolError> = None;
        let transaction_result = self.session.transaction(
            format!("{CLAUDE_UNDO_LABEL_PREFIX}{label}"),
            |transaction| {
                steps(transaction).map_err(|error| {
                    step_error = Some(error);
                    EditError::InternalInconsistency {
                        details: "a tool step failed".to_string(),
                    }
                })
            },
        );
        let output = match transaction_result {
            Ok(output) => output,
            Err(edit_error) => return Err(step_error.unwrap_or_else(|| edit_error.into())),
        };
        // A transaction that applied no command records no undo step: then there
        // is nothing to save (and nothing to undo if saving failed).
        if self.session.project() != &project_before {
            self.save_or_undo()?;
        }
        Ok(output)
    }

    /// Undoes the last step and saves. Returns the label of the undone step.
    ///
    /// # Errors
    ///
    /// Fails when there is nothing to undo, the file was changed by another
    /// program, or saving fails (the step is then redone).
    pub fn undo(&mut self) -> Result<String, ToolError> {
        self.ensure_file_unchanged_by_others()?;
        let label = self.session.undo()?;
        if let Err(save_error) = self.save() {
            self.session.redo()?;
            return Err(save_error);
        }
        Ok(label)
    }

    /// Redoes the last undone step and saves. Returns its label.
    ///
    /// # Errors
    ///
    /// Fails when there is nothing to redo, the file was changed by another
    /// program, or saving fails (the step is then undone again).
    pub fn redo(&mut self) -> Result<String, ToolError> {
        self.ensure_file_unchanged_by_others()?;
        let label = self.session.redo()?;
        if let Err(save_error) = self.save() {
            self.session.undo()?;
            return Err(save_error);
        }
        Ok(label)
    }

    /// Saves under a new path, which becomes the project's file from now on.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be written; the project then keeps its old file.
    pub fn save_as(&mut self, new_file_path: PathBuf) -> Result<(), ToolError> {
        let previous_path = std::mem::replace(&mut self.file_path, new_file_path);
        if let Err(save_error) = self.save() {
            self.file_path = previous_path;
            return Err(save_error);
        }
        Ok(())
    }

    /// Writes the project to its file.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be written.
    pub fn save(&mut self) -> Result<(), ToolError> {
        let file_content = save_project_to_json(self.session.project())
            .map_err(|error| ToolError::new(format!("the project cannot be saved: {error}")))?;
        save_project_file(self.session.project(), &self.file_path)?;
        self.last_known_file_content = file_content;
        Ok(())
    }

    /// Saves after a change; if saving fails, undoes the change.
    fn save_or_undo(&mut self) -> Result<(), ToolError> {
        let Err(save_error) = self.save() else {
            return Ok(());
        };
        self.session.undo()?;
        Err(ToolError::new(format!(
            "{save_error}. The change was undone so that the project and its file stay identical"
        )))
    }

    /// Refuses to go on if the file was modified by someone else since Kadr
    /// last read or wrote it.
    fn ensure_file_unchanged_by_others(&self) -> Result<(), ToolError> {
        let current_content = read_file_text(&self.file_path)?;
        if current_content == self.last_known_file_content {
            return Ok(());
        }
        Err(ToolError::new(format!(
            "the project file \"{}\" was changed by another program since Kadr opened it (perhaps \
             the Kadr app). To avoid losing those changes, nothing was done. Call open_project \
             with the same path to load the new version, then try again",
            self.file_path.display()
        )))
    }
}

fn read_file_text(file_path: &Path) -> Result<String, ToolError> {
    fs::read_to_string(file_path).map_err(|error| {
        ToolError::new(format!(
            "the project file \"{}\" cannot be read: {error}",
            file_path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use kadr_core::commands::{AddTrack, TrackPosition};
    use kadr_core::project::ProjectSettings;
    use kadr_core::timeline::TrackKind;

    use super::*;

    fn new_session(folder: &Path) -> ProjectFileSession {
        ProjectFileSession::create(
            Project::new(ProjectSettings::landscape_1080p()),
            folder.join("edit.kadr"),
        )
        .unwrap()
    }

    fn add_video_track(session: &mut ProjectFileSession) -> Result<(), ToolError> {
        session.change("Add a video track", |transaction| {
            transaction.execute(AddTrack::new(TrackKind::Video, TrackPosition::Top))?;
            Ok(())
        })
    }

    #[test]
    fn every_change_is_saved_and_labelled_for_claude() {
        let folder = tempfile::tempdir().unwrap();
        let mut session = new_session(folder.path());
        add_video_track(&mut session).unwrap();

        let reopened = load_project_file(session.file_path()).unwrap();
        assert_eq!(reopened.timeline().tracks().len(), 1);
        assert_eq!(
            session.edit_session().undo_history_labels(),
            vec!["Claude: Add a video track"]
        );

        session.undo().unwrap();
        let reopened = load_project_file(session.file_path()).unwrap();
        assert!(reopened.timeline().tracks().is_empty());
        session.redo().unwrap();
        assert_eq!(
            load_project_file(session.file_path())
                .unwrap()
                .timeline()
                .tracks()
                .len(),
            1
        );
    }

    #[test]
    fn a_failing_step_changes_nothing() {
        let folder = tempfile::tempdir().unwrap();
        let mut session = new_session(folder.path());
        let result = session.change("Two tracks then a failure", |transaction| {
            transaction.execute(AddTrack::new(TrackKind::Video, TrackPosition::Top))?;
            transaction.execute(AddTrack::new(TrackKind::Audio, TrackPosition::Top))?;
            Err::<(), _>(ToolError::new("the third step is impossible"))
        });
        assert_eq!(
            result.unwrap_err().to_string(),
            "the third step is impossible"
        );
        assert!(session.project().timeline().tracks().is_empty());
        assert!(!session.edit_session().can_undo());
    }

    #[test]
    fn a_file_changed_by_another_program_is_not_overwritten() {
        let folder = tempfile::tempdir().unwrap();
        let mut session = new_session(folder.path());
        fs::write(session.file_path(), "{ \"changed\": \"by the app\" }").unwrap();

        let error = add_video_track(&mut session).unwrap_err();
        assert!(
            error.to_string().contains("changed by another program"),
            "{error}"
        );
        assert_eq!(
            fs::read_to_string(session.file_path()).unwrap(),
            "{ \"changed\": \"by the app\" }"
        );
    }
}
