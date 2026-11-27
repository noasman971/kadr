//! The Tauri commands the front end calls, grouped by subject.
//!
//! Each command is a thin call into the engine or into this crate's editing
//! rules. Long work runs in blocking tasks or on the worker threads, never on the
//! interface's thread. Errors are [`crate::command_error::CommandError`]s the
//! front end translates.

pub mod blur_commands;
pub mod editing_commands;
pub mod export_commands;
pub mod picture_and_sound_commands;
pub mod project_commands;
pub mod silence_commands;
