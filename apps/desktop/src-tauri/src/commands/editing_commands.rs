//! Commands that change the open project: edits, imports, undo and redo.

use std::path::PathBuf;
use std::sync::Arc;

use kadr_core::ids::{ClipId, MediaAssetId, TrackId};
use kadr_core::media_asset::MediaAssetDescription;
use kadr_core::timeline::TrackKind;
use kadr_engine::color_grading::load_color_lookup_table;
use kadr_engine::subtitle_files::{
    export_subtitle_track, read_subtitle_file, subtitle_track_name_for,
};
use kadr_media::MediaBackend;
use serde::Serialize;
use tauri::State;

use crate::app_state::AppState;
use crate::command_error::{CommandError, CommandResult, ErrorCode};
use crate::editing::{
    EditOutcome, EditRequest, LutChanges, apply_edit_request, import_media_files, import_subtitles,
};
use crate::project_view::ProjectView;

/// The answer to an edit.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditResponse {
    /// The project after the edit.
    pub view: ProjectView,
    /// Created clips and messages for the user.
    pub outcome: EditOutcome,
}

/// A file that could not be imported.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportFailure {
    /// The path given.
    pub file_path: String,
    /// The file name, for messages.
    pub file_name: String,
    /// Why, in the engine's words.
    pub details: String,
}

/// The answer to an import.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResponse {
    /// The project after the import.
    pub view: ProjectView,
    /// The imported (or already present) media, in the order of the readable files.
    pub media_ids: Vec<MediaAssetId>,
    /// The files that could not be read.
    pub failures: Vec<ImportFailure>,
}

/// Applies one edit as one undo step.
#[tauri::command]
pub async fn apply_edit(
    state: State<'_, AppState>,
    request: EditRequest,
) -> CommandResult<EditResponse> {
    state.with_open_project(|open_project| {
        let outcome = open_project
            .edit(|session, adopts_format| apply_edit_request(session, adopts_format, &request))?;
        Ok(EditResponse {
            view: open_project.view(),
            outcome,
        })
    })
}

/// Undoes the last step.
#[tauri::command]
pub async fn undo(state: State<'_, AppState>) -> CommandResult<ProjectView> {
    state.with_open_project(|open_project| {
        open_project.undo()?;
        Ok(open_project.view())
    })
}

/// Redoes the last undone step.
#[tauri::command]
pub async fn redo(state: State<'_, AppState>) -> CommandResult<ProjectView> {
    state.with_open_project(|open_project| {
        open_project.redo()?;
        Ok(open_project.view())
    })
}

/// Reads a `.cube` file (in the background), then gives the clip its look as one
/// undo step: the clip's LUT takes this file and keeps its strength, or a LUT is
/// added at full strength.
#[tauri::command]
pub async fn choose_clip_lut_file(
    state: State<'_, AppState>,
    clip_id: ClipId,
    file_path: String,
) -> CommandResult<EditResponse> {
    if state.lock_open_project().is_none() {
        return Err(CommandError::no_project_open());
    }
    let file_path = PathBuf::from(file_path);
    let path_to_read = file_path.clone();
    tauri::async_runtime::spawn_blocking(move || load_color_lookup_table(&path_to_read))
        .await
        .map_err(CommandError::internal)?
        .map_err(|error| {
            CommandError::new(ErrorCode::LutUnreadable, error).about_file(&file_path)
        })?;
    let request = EditRequest::SetClipLut {
        clip_id,
        changes: LutChanges {
            file_path: Some(file_path),
            strength: None,
        },
    };
    state.with_open_project(|open_project| {
        let outcome = open_project
            .edit(|session, adopts_format| apply_edit_request(session, adopts_format, &request))?;
        Ok(EditResponse {
            view: open_project.view(),
            outcome,
        })
    })
}

/// Reads an `.srt` file (in the background), then adds its cues on a new
/// subtitle track on top as one undo step ("importSubtitles"). The first cue is
/// selected; the notice gives the count and what was changed to read the file.
#[tauri::command]
pub async fn import_subtitle_file(
    state: State<'_, AppState>,
    file_path: String,
) -> CommandResult<EditResponse> {
    if state.lock_open_project().is_none() {
        return Err(CommandError::no_project_open());
    }
    let file_path = PathBuf::from(file_path);
    let path_to_read = file_path.clone();
    let parsed = tauri::async_runtime::spawn_blocking(move || read_subtitle_file(&path_to_read))
        .await
        .map_err(CommandError::internal)?
        .map_err(|error| {
            CommandError::new(ErrorCode::SubtitlesUnreadable, error).about_file(&file_path)
        })?;
    let track_name = subtitle_track_name_for(&file_path);
    state.with_open_project(|open_project| {
        let outcome = open_project.edit(|session, _| {
            import_subtitles(session, &track_name, &parsed.cues, parsed.warnings.clone())
        })?;
        Ok(EditResponse {
            view: open_project.view(),
            outcome,
        })
    })
}

/// Writes the cues of a subtitle track to an `.srt` file (replacing it: the save
/// dialog already asked). Answers how many cues were written.
#[tauri::command]
pub async fn export_subtitle_file(
    state: State<'_, AppState>,
    track_id: TrackId,
    file_path: String,
) -> CommandResult<usize> {
    let track = {
        let guard = state.lock_open_project();
        let open_project = guard.as_ref().ok_or_else(CommandError::no_project_open)?;
        let project = open_project.project();
        let track = project
            .require_track(track_id)
            .map_err(CommandError::from)?;
        if track.kind != TrackKind::Subtitle {
            return Err(CommandError::new(
                ErrorCode::Internal,
                format!("track {track_id} is not a subtitle track"),
            ));
        }
        track.clone()
    };
    let file_path = PathBuf::from(file_path);
    let path_to_write = file_path.clone();
    tauri::async_runtime::spawn_blocking(move || export_subtitle_track(&track, &path_to_write))
        .await
        .map_err(CommandError::internal)?
        .map_err(|error| {
            CommandError::new(ErrorCode::SubtitlesNotSaved, error).about_file(&file_path)
        })
}

/// Reads the files (in the background) and imports the readable ones as one undo step.
#[tauri::command]
pub async fn import_media(
    state: State<'_, AppState>,
    file_paths: Vec<String>,
) -> CommandResult<ImportResponse> {
    if state.lock_open_project().is_none() {
        return Err(CommandError::no_project_open());
    }
    let backend = Arc::clone(&state.backend);
    let (descriptions, failures) =
        tauri::async_runtime::spawn_blocking(move || probe_files(backend.as_ref(), &file_paths))
            .await
            .map_err(CommandError::internal)?;
    state.with_open_project(|open_project| {
        let media_ids = if descriptions.is_empty() {
            Vec::new()
        } else {
            open_project.edit(|session, _| import_media_files(session, descriptions))?
        };
        Ok(ImportResponse {
            view: open_project.view(),
            media_ids,
            failures,
        })
    })
}

/// Probes every file, keeping the readable ones and explaining the others.
fn probe_files(
    backend: &dyn MediaBackend,
    file_paths: &[String],
) -> (Vec<MediaAssetDescription>, Vec<ImportFailure>) {
    let mut descriptions = Vec::new();
    let mut failures = Vec::new();
    for file_path in file_paths {
        let path = PathBuf::from(file_path);
        match backend.probe(&path) {
            Ok(description) => descriptions.push(description),
            Err(error) => failures.push(ImportFailure {
                file_path: file_path.clone(),
                file_name: CommandError::new(ErrorCode::MediaUnreadable, "")
                    .about_file(&path)
                    .file_name
                    .unwrap_or_else(|| file_path.clone()),
                details: error.to_string(),
            }),
        }
    }
    (descriptions, failures)
}
