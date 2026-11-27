//! Commands of the *Connect to Claude* screen (decision record 0031).
//!
//! `config_path` is the Claude Desktop configuration file the person chose,
//! when it is not the one Kadr found.

use std::path::PathBuf;

use tauri::State;

use crate::app_state::AppState;
use crate::claude_setup::{
    ClaudeSetupPaths, ClaudeSetupView, add_allowed_folder, claude_setup_view,
    connect_claude_desktop as write_claude_desktop_config, remove_allowed_folder,
};
use crate::command_error::CommandResult;

fn paths_of(state: &AppState, config_path: Option<String>) -> ClaudeSetupPaths {
    ClaudeSetupPaths::of_this_app(
        state.configuration_folder.clone(),
        config_path.map(PathBuf::from),
    )
}

/// What the screen shows.
#[tauri::command]
pub async fn get_claude_setup(
    state: State<'_, AppState>,
    config_path: Option<String>,
) -> CommandResult<ClaudeSetupView> {
    Ok(claude_setup_view(&paths_of(&state, config_path)))
}

/// Allows Claude to use `folder` (applies to its next call).
#[tauri::command]
pub async fn add_claude_folder(
    state: State<'_, AppState>,
    folder: String,
    config_path: Option<String>,
) -> CommandResult<ClaudeSetupView> {
    let paths = paths_of(&state, config_path);
    add_allowed_folder(&paths, PathBuf::from(folder))?;
    Ok(claude_setup_view(&paths))
}

/// Stops allowing Claude to use `folder`.
#[tauri::command]
pub async fn remove_claude_folder(
    state: State<'_, AppState>,
    folder: String,
    config_path: Option<String>,
) -> CommandResult<ClaudeSetupView> {
    let paths = paths_of(&state, config_path);
    remove_allowed_folder(&paths, &PathBuf::from(folder))?;
    Ok(claude_setup_view(&paths))
}

/// Writes Kadr's entry into Claude Desktop's configuration.
#[tauri::command]
pub async fn connect_claude_desktop(
    state: State<'_, AppState>,
    config_path: Option<String>,
) -> CommandResult<ClaudeSetupView> {
    let paths = paths_of(&state, config_path);
    write_claude_desktop_config(&paths)?;
    Ok(claude_setup_view(&paths))
}
