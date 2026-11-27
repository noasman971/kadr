//! # kadr-desktop — the Kadr desktop app
//!
//! A Tauri 2 application: this crate is its Rust back end, the Svelte front end
//! lives in `apps/desktop/src/`. The back end **owns the project** (its edit
//! history and its file) and answers the front end with project views, pictures
//! and sound; the front end draws them and sends user gestures back as edit
//! requests. See `docs/desktop-app.md` and decision record 0022.
//!
//! - [`open_project`]: the project being edited, saved after every change;
//! - [`editing`]: the edit requests and the rules each one follows;
//! - [`project_view`]: the description of the project the front end draws;
//! - [`media_workers`]: the threads owning decoders (preview, thumbnails);
//! - [`binary_messages`]: the byte layouts of pictures and sound;
//! - [`recent_projects`]: the home screen's list;
//! - [`commands`]: the Tauri commands themselves.
//!
//! The engine crates never depend on this one: everything reusable (preview
//! frames, thumbnails, playback sound) lives in `kadr-engine`.

pub mod app_state;
pub mod binary_messages;
pub mod command_error;
pub mod commands;
pub mod editing;
pub mod export_job;
pub mod media_workers;
pub mod open_project;
pub mod project_view;
pub mod recent_projects;
pub mod silence_search;

use tauri::Manager;

use crate::app_state::AppState;
use crate::commands::{
    editing_commands, export_commands, picture_and_sound_commands, project_commands,
    silence_commands,
};

/// Name of the folder, in the user's documents, where new projects are created.
const PROJECTS_FOLDER_NAME: &str = "Kadr";

/// Starts the app and returns when its window is closed.
///
/// # Errors
///
/// When the window, FFmpeg or a worker thread cannot be started.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> Result<(), tauri::Error> {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let paths = app.path();
            let configuration_folder = paths.app_config_dir()?;
            let projects_folder = paths.document_dir()?.join(PROJECTS_FOLDER_NAME);
            app.manage(AppState::new(configuration_folder, projects_folder)?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            project_commands::list_recent_projects,
            project_commands::forget_recent,
            project_commands::create_project,
            project_commands::open_project,
            project_commands::get_project_view,
            project_commands::close_project,
            project_commands::rename_project,
            project_commands::save_project_as,
            project_commands::save_project,
            editing_commands::apply_edit,
            editing_commands::undo,
            editing_commands::redo,
            editing_commands::import_media,
            picture_and_sound_commands::render_preview_frame,
            picture_and_sound_commands::get_thumbnail,
            picture_and_sound_commands::get_waveform,
            picture_and_sound_commands::get_timeline_sound,
            export_commands::start_export,
            export_commands::cancel_export,
            export_commands::reveal_exported_file,
            export_commands::open_exported_file,
            silence_commands::find_clip_silences,
            silence_commands::stop_silence_search,
        ])
        .run(tauri::generate_context!())
}
