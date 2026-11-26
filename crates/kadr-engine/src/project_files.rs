//! Saving and opening `.kadr` project files on disk.
//!
//! Saving is **atomic**: the project is written to a temporary file next to the
//! target, then renamed over it. A crash or a full disk during saving never
//! leaves a half-written project behind.

use std::fs;
use std::path::{Path, PathBuf};

use kadr_core::project::Project;
use kadr_core::project_file::{ProjectFileError, load_project_from_json, save_project_to_json};

/// Why a project file could not be saved or opened.
#[derive(Debug, thiserror::Error)]
pub enum ProjectFileIoError {
    /// The file could not be read.
    #[error("could not read \"{}\": {reason}", file_path.display())]
    CannotRead {
        /// The file.
        file_path: PathBuf,
        /// What the system said.
        reason: String,
    },
    /// The file could not be written.
    #[error("could not save \"{}\": {reason}", file_path.display())]
    CannotWrite {
        /// The file.
        file_path: PathBuf,
        /// What the system said.
        reason: String,
    },
    /// The file was read but is not a valid project.
    #[error("\"{}\": {source}", file_path.display())]
    InvalidContent {
        /// The file.
        file_path: PathBuf,
        /// What is wrong with it.
        source: ProjectFileError,
    },
}

/// The temporary file used while saving `file_path`.
fn temporary_path_for(file_path: &Path) -> PathBuf {
    let mut temporary_name = file_path.as_os_str().to_owned();
    temporary_name.push(".saving");
    PathBuf::from(temporary_name)
}

/// Saves `project` to `file_path`, replacing the file if it exists.
///
/// # Errors
///
/// Fails when the file cannot be written.
pub fn save_project_file(project: &Project, file_path: &Path) -> Result<(), ProjectFileIoError> {
    let cannot_write = |reason: String| ProjectFileIoError::CannotWrite {
        file_path: file_path.to_path_buf(),
        reason,
    };
    let json_text =
        save_project_to_json(project).map_err(|error| cannot_write(error.to_string()))?;
    let temporary_path = temporary_path_for(file_path);
    fs::write(&temporary_path, json_text).map_err(|error| cannot_write(error.to_string()))?;
    fs::rename(&temporary_path, file_path).map_err(|error| {
        // Best effort: do not leave the temporary file behind.
        let _ = fs::remove_file(&temporary_path);
        cannot_write(error.to_string())
    })
}

/// Opens the project saved in `file_path`.
///
/// # Errors
///
/// Fails when the file cannot be read or does not contain a valid project.
pub fn load_project_file(file_path: &Path) -> Result<Project, ProjectFileIoError> {
    let json_text =
        fs::read_to_string(file_path).map_err(|error| ProjectFileIoError::CannotRead {
            file_path: file_path.to_path_buf(),
            reason: error.to_string(),
        })?;
    load_project_from_json(&json_text).map_err(|source| ProjectFileIoError::InvalidContent {
        file_path: file_path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kadr_core::project::ProjectSettings;

    #[test]
    fn a_saved_project_opens_identically_and_leaves_no_temporary_file() {
        let directory = tempfile::tempdir().unwrap();
        let file_path = directory.path().join("my edit.kadr");
        let project = Project::new(ProjectSettings::vertical_1080x1920());

        save_project_file(&project, &file_path).unwrap();
        save_project_file(&project, &file_path).unwrap(); // overwriting works too

        assert_eq!(load_project_file(&file_path).unwrap(), project);
        assert!(!temporary_path_for(&file_path).exists());
    }

    #[test]
    fn errors_name_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing.kadr");
        assert!(
            load_project_file(&missing)
                .unwrap_err()
                .to_string()
                .contains("missing.kadr")
        );

        let damaged = directory.path().join("damaged.kadr");
        fs::write(&damaged, "{ not json").unwrap();
        let error = load_project_file(&damaged).unwrap_err();
        assert!(matches!(error, ProjectFileIoError::InvalidContent { .. }));
        assert!(error.to_string().contains("damaged.kadr"));
    }
}
