//! Commands of the export dialog: start, cancel, and open what was exported.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::State;
use tauri::ipc::Channel;
use tauri_plugin_opener::OpenerExt;

use crate::app_state::AppState;
use crate::command_error::{CommandError, CommandResult, ErrorCode};
use crate::export_job::{ExportEvent, ExportSettings, ExportStarted};

/// Starts exporting the open project as it is now. Progress and the result come
/// through `on_event`.
#[tauri::command]
pub async fn start_export(
    state: State<'_, AppState>,
    settings: ExportSettings,
    on_event: Channel<ExportEvent>,
) -> CommandResult<ExportStarted> {
    let project = state.with_open_project(|open_project| Ok(open_project.snapshot()))?;
    let backend = Arc::clone(&state.backend);
    state
        .exports
        .start(&project, backend, &settings, move |event| {
            // The window may have closed meanwhile: nobody is listening then.
            let _ = on_event.send(event);
        })
}

/// Stops the running export, if any.
#[tauri::command]
pub async fn cancel_export(state: State<'_, AppState>) -> CommandResult<()> {
    state.exports.cancel();
    Ok(())
}

/// Shows an exported file in the system's file explorer.
#[tauri::command]
pub async fn reveal_exported_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    file_path: PathBuf,
) -> CommandResult<()> {
    require_exported(&state, &file_path)?;
    app.opener()
        .reveal_item_in_dir(&file_path)
        .map_err(|error| CommandError::new(ErrorCode::Internal, error).about_file(&file_path))
}

/// Opens an exported file in the system's video player.
#[tauri::command]
pub async fn open_exported_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    file_path: PathBuf,
) -> CommandResult<()> {
    require_exported(&state, &file_path)?;
    app.opener()
        .open_path(file_path.display().to_string(), None::<&str>)
        .map_err(|error| CommandError::new(ErrorCode::Internal, error).about_file(&file_path))
}

/// Only files an export wrote since the app started can be opened from here.
fn require_exported(state: &AppState, file_path: &std::path::Path) -> CommandResult<()> {
    if state.exports.has_written(file_path) {
        Ok(())
    } else {
        Err(CommandError::internal(format!(
            "\"{}\" was not exported by Kadr in this session",
            file_path.display()
        )))
    }
}
