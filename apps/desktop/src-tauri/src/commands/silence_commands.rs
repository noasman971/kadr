//! Commands of the silence dialog: look for the silences of a clip, or stop
//! looking. Removing them is an edit (`removeSilences`).

use std::sync::Arc;

use kadr_core::ids::ClipId;
use tauri::State;
use tauri::ipc::Channel;

use crate::app_state::AppState;
use crate::command_error::{CommandError, CommandResult};
use crate::silence_search::{SilencePlanView, SilenceSearchSettings, search_silences};

/// Finds what removing the silences of a clip would cut, in the project as it
/// is now. Changes nothing. `on_progress` receives the fraction analysed. A
/// newer search, or [`stop_silence_search`], ends this one with a
/// `superseded` error.
#[tauri::command]
pub async fn find_clip_silences(
    state: State<'_, AppState>,
    clip_id: ClipId,
    settings: SilenceSearchSettings,
    on_progress: Channel<f64>,
) -> CommandResult<SilencePlanView> {
    let project = state.with_open_project(|open_project| Ok(open_project.snapshot()))?;
    let backend = Arc::clone(&state.backend);
    let ticket = state.silence_searches.start_new();
    tauri::async_runtime::spawn_blocking(move || {
        search_silences(
            &project,
            backend.as_ref(),
            clip_id,
            settings,
            &ticket,
            |fraction| {
                // The dialog may have closed meanwhile: nobody is listening then.
                let _ = on_progress.send(fraction);
            },
        )
    })
    .await
    .map_err(CommandError::internal)?
}

/// Stops looking for silences (the dialog closed).
#[tauri::command]
pub async fn stop_silence_search(state: State<'_, AppState>) -> CommandResult<()> {
    state.silence_searches.stop_all();
    Ok(())
}
