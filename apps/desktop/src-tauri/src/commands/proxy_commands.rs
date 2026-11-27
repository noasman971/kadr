//! Commands of proxies (decision record 0055): their states, stopping or making
//! one again, the automatic setting and clearing the cache.

use std::path::PathBuf;

use kadr_core::ids::MediaAssetId;
use serde::Serialize;
use tauri::{Manager, State};

use crate::app_state::AppState;
use crate::command_error::{CommandError, CommandResult, ErrorCode};
use crate::proxy_jobs::ProxyOverview;

/// The setting, the cache's size and the state of every video seen.
#[tauri::command]
pub async fn get_proxy_overview(state: State<'_, AppState>) -> CommandResult<ProxyOverview> {
    Ok(state.proxies.overview())
}

/// Stops making the proxy of a video of the open project.
#[tauri::command]
pub async fn stop_proxy(state: State<'_, AppState>, media_id: MediaAssetId) -> CommandResult<()> {
    let file_path = media_file_path(&state, media_id)?;
    state.proxies.stop(&file_path);
    Ok(())
}

/// Makes the proxy of a video of the open project again (after it was stopped
/// or failed).
#[tauri::command]
pub async fn make_proxy_again(
    state: State<'_, AppState>,
    media_id: MediaAssetId,
) -> CommandResult<()> {
    let description = state.with_open_project(|open_project| {
        open_project
            .project()
            .media_asset(media_id)
            .map(|media_asset| media_asset.description.clone())
            .ok_or_else(|| unknown_media(media_id))
    })?;
    state.proxies.make_again(description)
}

/// Turns automatic proxies on or off.
#[tauri::command]
pub async fn set_automatic_proxies(
    state: State<'_, AppState>,
    is_automatic: bool,
) -> CommandResult<ProxyOverview> {
    let project = state
        .lock_open_project()
        .as_ref()
        .map(crate::open_project::OpenProject::snapshot);
    state
        .proxies
        .set_automatic(is_automatic, project.as_deref())?;
    Ok(state.proxies.overview())
}

/// What clearing the cache freed.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyCacheCleared {
    /// Bytes deleted.
    pub freed_bytes: u64,
    /// The setting and states afterwards.
    pub overview: ProxyOverview,
}

/// Stops every proxy and deletes the cache's files. The preview reads the
/// originals until the videos get proxies again (next opening or import).
#[tauri::command]
pub async fn clear_proxy_cache(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<ProxyCacheCleared> {
    state.proxies.forget_every_proxy();
    let freed_bytes = tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        // Files open in a decoder cannot be deleted on Windows.
        state.preview.close_files();
        state.thumbnails.close_files();
        state.proxies.delete_cache_files()
    })
    .await
    .map_err(CommandError::internal)?;
    Ok(ProxyCacheCleared {
        freed_bytes,
        overview: state.proxies.overview(),
    })
}

/// The file of a media of the open project.
fn media_file_path(state: &AppState, media_id: MediaAssetId) -> CommandResult<PathBuf> {
    state.with_open_project(|open_project| {
        open_project
            .project()
            .media_asset(media_id)
            .map(|media_asset| media_asset.description.file_path.clone())
            .ok_or_else(|| unknown_media(media_id))
    })
}

fn unknown_media(media_id: MediaAssetId) -> CommandError {
    CommandError::new(
        ErrorCode::Internal,
        format!("media {media_id} does not exist"),
    )
}
