//! Commands that create, open, rename and close projects.

use std::path::{Path, PathBuf};

use kadr_core::project::ProjectSettings;
use serde::Deserialize;
use tauri::State;

use crate::app_state::AppState;
use crate::command_error::{CommandError, CommandResult, ErrorCode};
use crate::open_project::{OpenProject, PROJECT_FILE_EXTENSION};
use crate::project_view::ProjectView;
use crate::recent_projects::{
    RecentProject, forget_recent_project, load_recent_projects, remember_recent_project,
};

/// The format chosen on the home screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectFormatChoice {
    /// Like the first video placed (1920 × 1080 until then).
    Automatic,
    /// 1920 × 1080.
    Landscape,
    /// 1080 × 1920.
    Vertical,
    /// 1080 × 1080.
    Square,
}

impl ProjectFormatChoice {
    fn settings(self) -> ProjectSettings {
        match self {
            Self::Automatic | Self::Landscape => ProjectSettings::landscape_1080p(),
            Self::Vertical => ProjectSettings::vertical_1080x1920(),
            Self::Square => ProjectSettings::square_1080(),
        }
    }
}

/// The recently opened projects, most recent first.
#[tauri::command]
pub async fn list_recent_projects(state: State<'_, AppState>) -> CommandResult<Vec<RecentProject>> {
    Ok(load_recent_projects(&state.configuration_folder))
}

/// Removes a project from the recent list (the file is not touched).
#[tauri::command]
pub async fn forget_recent(state: State<'_, AppState>, file_path: String) -> CommandResult<()> {
    forget_recent_project(&state.configuration_folder, Path::new(&file_path))
        .map_err(CommandError::internal)
}

/// Creates a project named `name` (or `name 2`, `name 3`… if taken) in the
/// projects folder, and opens it.
#[tauri::command]
pub async fn create_project(
    state: State<'_, AppState>,
    name: String,
    format: ProjectFormatChoice,
) -> CommandResult<ProjectView> {
    let file_path = free_project_path(&state.projects_folder, &name);
    let open_project = OpenProject::create(
        file_path,
        format.settings(),
        format == ProjectFormatChoice::Automatic,
    )?;
    Ok(make_current(&state, open_project, None))
}

/// Opens the project saved in `file_path`.
#[tauri::command]
pub async fn open_project(
    state: State<'_, AppState>,
    file_path: String,
) -> CommandResult<ProjectView> {
    let open_project = OpenProject::open(PathBuf::from(file_path))?;
    Ok(make_current(&state, open_project, None))
}

/// The open project's view, or `None` when no project is open.
#[tauri::command]
pub async fn get_project_view(state: State<'_, AppState>) -> CommandResult<Option<ProjectView>> {
    Ok(state.lock_open_project().as_ref().map(OpenProject::view))
}

/// Closes the open project (it is already saved).
#[tauri::command]
pub async fn close_project(state: State<'_, AppState>) -> CommandResult<()> {
    let closed = state.lock_open_project().take();
    if let Some(mut closed) = closed {
        closed.save();
    }
    Ok(())
}

/// Renames the open project (and its file).
#[tauri::command]
pub async fn rename_project(
    state: State<'_, AppState>,
    new_name: String,
) -> CommandResult<ProjectView> {
    state.with_open_project(|open_project| {
        let previous_path = open_project.file_path().to_path_buf();
        open_project.rename(&new_name)?;
        let _ = remember_recent_project(
            &state.configuration_folder,
            open_project.file_path(),
            Some(&previous_path),
        );
        Ok(open_project.view())
    })
}

/// Saves the open project to `file_path`, which becomes its file.
#[tauri::command]
pub async fn save_project_as(
    state: State<'_, AppState>,
    file_path: String,
) -> CommandResult<ProjectView> {
    let mut file_path = PathBuf::from(file_path);
    if file_path.extension().is_none() {
        file_path.set_extension(PROJECT_FILE_EXTENSION);
    }
    state.with_open_project(|open_project| {
        open_project.save_as(file_path)?;
        let _ =
            remember_recent_project(&state.configuration_folder, open_project.file_path(), None);
        Ok(open_project.view())
    })
}

/// Saves the open project now (it is saved after every change anyway; this
/// retries after a failure).
#[tauri::command]
pub async fn save_project(state: State<'_, AppState>) -> CommandResult<ProjectView> {
    state.with_open_project(|open_project| {
        open_project.save();
        match open_project.view().save_failure {
            Some(reason) => Err(CommandError::new(ErrorCode::ProjectFileNotSaved, reason)
                .about_file(open_project.file_path())),
            None => Ok(open_project.view()),
        }
    })
}

/// Makes `open_project` the current project and adds it to the recent list.
fn make_current(
    state: &AppState,
    open_project: OpenProject,
    previous_path: Option<&Path>,
) -> ProjectView {
    let _ = remember_recent_project(
        &state.configuration_folder,
        open_project.file_path(),
        previous_path,
    );
    let view = open_project.view();
    *state.lock_open_project() = Some(open_project);
    state.notice_media_of_open_project();
    view
}

/// `folder/name.kadr`, or `folder/name 2.kadr`… for the first name not taken.
fn free_project_path(folder: &Path, name: &str) -> PathBuf {
    let base_name = sanitized_file_name(name);
    let mut candidate = folder.join(format!("{base_name}.{PROJECT_FILE_EXTENSION}"));
    let mut number = 2;
    while candidate.exists() {
        candidate = folder.join(format!("{base_name} {number}.{PROJECT_FILE_EXTENSION}"));
        number += 1;
    }
    candidate
}

/// `name` without the characters file names cannot hold; "Kadr" if nothing is left.
fn sanitized_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|character| {
            !matches!(
                character,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            )
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "Kadr".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taken_names_get_a_number() {
        let folder = tempfile::tempdir().unwrap();
        let first = free_project_path(folder.path(), "Untitled project");
        assert_eq!(first, folder.path().join("Untitled project.kadr"));
        std::fs::write(&first, "{}").unwrap();
        assert_eq!(
            free_project_path(folder.path(), "Untitled project"),
            folder.path().join("Untitled project 2.kadr")
        );
    }

    #[test]
    fn file_names_lose_forbidden_characters() {
        assert_eq!(sanitized_file_name("a/b:c?"), "abc");
        assert_eq!(sanitized_file_name("  ?? "), "Kadr");
    }
}
