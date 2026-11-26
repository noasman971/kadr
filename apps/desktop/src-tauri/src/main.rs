//! The `kadr` executable: starts the desktop app.

// No console window behind the app in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    match kadr_desktop::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Kadr could not start: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
