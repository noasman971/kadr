//! The project open in the window: its edit history, its file, and automatic saving.
//!
//! Every change goes through [`OpenProject::edit`], which applies it as one undo
//! step, bumps the revision, saves the file and refreshes the snapshot the preview
//! threads read. Nothing else changes the project.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use kadr_core::error::EditError;
use kadr_core::history::EditSession;
use kadr_core::project::{Project, ProjectSettings};
use kadr_engine::project_files::{load_project_file, save_project_file};

use crate::command_error::{CommandError, CommandResult, ErrorCode};
use crate::editing::ADD_FIRST_VIDEO_UNDO_LABEL;
use crate::project_view::{ProjectView, ViewContext, build_project_view};

/// Extension of Kadr project files.
pub const PROJECT_FILE_EXTENSION: &str = "kadr";

/// Numbers the projects opened since the app started (see [`OpenProject::identity`]).
static LAST_OPENING_NUMBER: AtomicU64 = AtomicU64::new(0);

/// The project being edited.
pub struct OpenProject {
    session: EditSession,
    /// Which opening this is, unchanged by renaming or "save as".
    identity: u64,
    file_path: PathBuf,
    revision: u64,
    /// The format follows the first video placed on the timeline ("Automatic").
    adopts_first_video_format: bool,
    /// Why the last save failed, if it did.
    save_failure: Option<String>,
    /// The current project, shared with the preview and sound threads.
    snapshot: Arc<Project>,
}

impl OpenProject {
    /// Creates a new, empty project and saves it to `file_path` (which must not exist).
    ///
    /// # Errors
    ///
    /// Fails when a file already exists there or the file cannot be written.
    pub fn create(
        file_path: PathBuf,
        settings: ProjectSettings,
        adopts_first_video_format: bool,
    ) -> CommandResult<Self> {
        if file_path.exists() {
            return Err(CommandError::new(
                ErrorCode::ProjectNameTaken,
                format!("\"{}\" already exists", file_path.display()),
            )
            .about_file(&file_path));
        }
        if let Some(folder) = file_path.parent() {
            fs::create_dir_all(folder).map_err(|error| {
                CommandError::new(ErrorCode::ProjectFileNotSaved, error).about_file(&file_path)
            })?;
        }
        let project = Project::new(settings);
        save_project_file(&project, &file_path).map_err(|error| {
            CommandError::new(ErrorCode::ProjectFileNotSaved, error).about_file(&file_path)
        })?;
        Self::with_project(project, file_path, adopts_first_video_format)
    }

    /// Opens the project saved in `file_path`.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read or is not a valid project.
    pub fn open(file_path: PathBuf) -> CommandResult<Self> {
        let project = load_project_file(&file_path).map_err(|error| {
            CommandError::new(ErrorCode::ProjectFileUnreadable, error).about_file(&file_path)
        })?;
        Self::with_project(project, file_path, false)
    }

    fn with_project(
        project: Project,
        file_path: PathBuf,
        adopts_first_video_format: bool,
    ) -> CommandResult<Self> {
        let snapshot = Arc::new(project.clone());
        let session = EditSession::new(project).map_err(|error| {
            CommandError::new(ErrorCode::ProjectFileUnreadable, error).about_file(&file_path)
        })?;
        Ok(Self {
            session,
            identity: LAST_OPENING_NUMBER.fetch_add(1, Ordering::Relaxed) + 1,
            file_path,
            revision: 1,
            adopts_first_video_format,
            save_failure: None,
            snapshot,
        })
    }

    /// The project as it is now.
    pub fn project(&self) -> &Project {
        self.session.project()
    }

    /// A cheap, shareable copy of the project as it is now.
    pub fn snapshot(&self) -> Arc<Project> {
        Arc::clone(&self.snapshot)
    }

    /// The `.kadr` file.
    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    /// A number naming this opening of the project: background work started on
    /// it (a tracking analysis) checks it is still the open project before
    /// changing it. Renaming or "save as" keeps it; closing and reopening does not.
    pub fn identity(&self) -> u64 {
        self.identity
    }

    /// The change counter.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The view the front end draws.
    pub fn view(&self) -> ProjectView {
        build_project_view(
            self.session.project(),
            &ViewContext {
                revision: self.revision,
                file_path: &self.file_path,
                save_failure: self.save_failure.as_deref(),
                adopts_first_video_format: self.adopts_first_video_format,
                undo_label: self.session.next_undo_label(),
                redo_label: self.session.next_redo_label(),
            },
        )
    }

    /// Applies a change: `change` receives the session and the "Automatic format"
    /// flag. On success the project is saved and the revision goes up; on failure
    /// nothing changed.
    ///
    /// # Errors
    ///
    /// The engine's refusal.
    pub fn edit<Output>(
        &mut self,
        change: impl FnOnce(&mut EditSession, &mut bool) -> Result<Output, EditError>,
    ) -> CommandResult<Output> {
        let output = change(&mut self.session, &mut self.adopts_first_video_format)?;
        self.after_change();
        Ok(output)
    }

    /// Undoes the last step and returns its label. Undoing the step that placed
    /// the first video makes the format automatic again.
    ///
    /// # Errors
    ///
    /// When there is nothing to undo.
    pub fn undo(&mut self) -> CommandResult<String> {
        let label = self.session.undo()?;
        if label == ADD_FIRST_VIDEO_UNDO_LABEL {
            self.adopts_first_video_format = true;
        }
        self.after_change();
        Ok(label)
    }

    /// Redoes the last undone step and returns its label. Redoing the step that
    /// placed the first video fixes the format again.
    ///
    /// # Errors
    ///
    /// When there is nothing to redo.
    pub fn redo(&mut self) -> CommandResult<String> {
        let label = self.session.redo()?;
        if label == ADD_FIRST_VIDEO_UNDO_LABEL {
            self.adopts_first_video_format = false;
        }
        self.after_change();
        Ok(label)
    }

    /// Gives the project a new name: its file is renamed in the same folder.
    ///
    /// # Errors
    ///
    /// When the name is empty or taken, or the file cannot be renamed.
    pub fn rename(&mut self, new_name: &str) -> CommandResult<()> {
        let new_name = new_name.trim();
        let is_valid = !new_name.is_empty()
            && !new_name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']);
        if !is_valid {
            return Err(CommandError::new(
                ErrorCode::ProjectFileNotSaved,
                format!("\"{new_name}\" cannot be used as a file name"),
            ));
        }
        let new_path = self
            .file_path
            .with_file_name(format!("{new_name}.{PROJECT_FILE_EXTENSION}"));
        if new_path == self.file_path {
            return Ok(());
        }
        if new_path.exists() {
            return Err(CommandError::new(
                ErrorCode::ProjectNameTaken,
                format!("\"{}\" already exists", new_path.display()),
            )
            .about_file(&new_path));
        }
        fs::rename(&self.file_path, &new_path).map_err(|error| {
            CommandError::new(ErrorCode::ProjectFileNotSaved, error).about_file(&new_path)
        })?;
        self.file_path = new_path;
        self.revision += 1;
        Ok(())
    }

    /// Saves the project to `new_path`, which becomes its file from now on.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn save_as(&mut self, new_path: PathBuf) -> CommandResult<()> {
        save_project_file(self.session.project(), &new_path).map_err(|error| {
            CommandError::new(ErrorCode::ProjectFileNotSaved, error).about_file(&new_path)
        })?;
        self.file_path = new_path;
        self.save_failure = None;
        self.revision += 1;
        Ok(())
    }

    /// Saves the project to its file now.
    pub fn save(&mut self) {
        self.save_failure = save_project_file(self.session.project(), &self.file_path)
            .err()
            .map(|error| error.to_string());
    }

    fn after_change(&mut self) {
        self.revision += 1;
        self.snapshot = Arc::new(self.session.project().clone());
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editing::{EditRequest, apply_edit_request, import_media_files};
    use kadr_core::commands::{AddTrack, TrackPosition};
    use kadr_core::media_asset::{MediaAssetDescription, MediaKind, PictureSize};
    use kadr_core::time::{FrameRate, TimeSpan};
    use kadr_core::timeline::TrackKind;

    #[test]
    fn every_change_is_saved_and_bumps_the_revision() {
        let directory = tempfile::tempdir().unwrap();
        let file_path = directory.path().join("Holiday.kadr");
        let mut open_project =
            OpenProject::create(file_path.clone(), ProjectSettings::landscape_1080p(), true)
                .unwrap();
        assert_eq!(open_project.view().name, "Holiday");

        open_project
            .edit(|session, _| session.execute(AddTrack::new(TrackKind::Video, TrackPosition::Top)))
            .unwrap();

        assert_eq!(open_project.revision(), 2);
        assert_eq!(open_project.snapshot().timeline().tracks().len(), 1);
        let reopened = OpenProject::open(file_path).unwrap();
        assert_eq!(reopened.project(), open_project.project());

        open_project.undo().unwrap();
        assert_eq!(open_project.revision(), 3);
        assert!(open_project.snapshot().timeline().tracks().is_empty());
    }

    #[test]
    fn undoing_the_first_video_makes_the_format_automatic_again() {
        let directory = tempfile::tempdir().unwrap();
        let mut open_project = OpenProject::create(
            directory.path().join("Auto.kadr"),
            ProjectSettings::landscape_1080p(),
            true,
        )
        .unwrap();
        let video = MediaAssetDescription {
            file_path: PathBuf::from("C:/videos/phone.mp4"),
            display_name: "phone.mp4".to_string(),
            kind: MediaKind::Video,
            duration: Some(TimeSpan::from_whole_seconds(4)),
            picture_size: Some(PictureSize {
                width_in_pixels: 1080,
                height_in_pixels: 1920,
            }),
            frame_rate: Some(FrameRate::FPS_25),
            audio_format: None,
        };
        let media_ids = open_project
            .edit(|session, _| import_media_files(session, vec![video]))
            .unwrap();
        let place_video = EditRequest::AddMediaAtEnd {
            media_id: media_ids[0],
        };
        open_project
            .edit(|session, adopts| apply_edit_request(session, adopts, &place_video))
            .unwrap();
        assert!(!open_project.view().adopts_first_video_format);

        open_project.undo().unwrap();
        assert!(open_project.view().adopts_first_video_format);
        assert_eq!(
            open_project.project().settings(),
            &ProjectSettings::landscape_1080p()
        );

        open_project.redo().unwrap();
        assert!(!open_project.view().adopts_first_video_format);
        assert_eq!(
            open_project
                .project()
                .settings()
                .canvas_size
                .height_in_pixels,
            1920
        );

        // Undoing the import does not touch the flag; the next video decides again.
        open_project.undo().unwrap();
        open_project.undo().unwrap();
        assert!(open_project.view().adopts_first_video_format);
    }

    #[test]
    fn a_refused_change_changes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let mut open_project = OpenProject::create(
            directory.path().join("p.kadr"),
            ProjectSettings::landscape_1080p(),
            false,
        )
        .unwrap();
        let error = open_project.undo().unwrap_err();
        assert_eq!(error.code, ErrorCode::NothingToUndo);
        assert_eq!(open_project.revision(), 1);
    }

    #[test]
    fn renaming_moves_the_file_and_refuses_taken_names() {
        let directory = tempfile::tempdir().unwrap();
        let mut open_project = OpenProject::create(
            directory.path().join("First.kadr"),
            ProjectSettings::landscape_1080p(),
            false,
        )
        .unwrap();
        fs::write(directory.path().join("Taken.kadr"), "{}").unwrap();

        open_project.rename("Second").unwrap();
        assert!(directory.path().join("Second.kadr").exists());
        assert!(!directory.path().join("First.kadr").exists());
        assert_eq!(open_project.view().name, "Second");

        let error = open_project.rename("Taken").unwrap_err();
        assert_eq!(error.code, ErrorCode::ProjectNameTaken);
        assert!(open_project.rename("a/b").is_err());
    }

    #[test]
    fn creating_over_an_existing_project_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let file_path = directory.path().join("Same.kadr");
        OpenProject::create(file_path.clone(), ProjectSettings::square_1080(), false).unwrap();
        let error = OpenProject::create(file_path, ProjectSettings::square_1080(), false)
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::ProjectNameTaken);
    }
}
