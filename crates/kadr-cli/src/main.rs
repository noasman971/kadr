//! # kadr-cli — Kadr from the command line
//!
//! A small binary to exercise the engine without the desktop app: rendering a
//! project, probing a file, testing the face-blur tracker.
//!
//! In phase 0 it only knows `--version` and `--help`. Real commands arrive with
//! the features they test (`probe` and `render` in phase 2, `blur` in phase 4).

use std::process::ExitCode;

/// Name of the program, as typed by the user.
const PROGRAM_NAME: &str = "kadr-cli";

/// Version of the program, taken from the workspace `Cargo.toml`.
const PROGRAM_VERSION: &str = env!("CARGO_PKG_VERSION");

/// What the program prints and how it exits, decided from the command-line arguments.
#[derive(Debug, PartialEq, Eq)]
struct CommandLineResponse {
    /// Text printed to the terminal.
    message: String,
    /// Whether the arguments were understood.
    is_success: bool,
}

fn main() -> ExitCode {
    let arguments_after_program_name: Vec<String> = std::env::args().skip(1).collect();
    let response = respond_to_arguments(&arguments_after_program_name);

    if response.is_success {
        println!("{}", response.message);
        ExitCode::SUCCESS
    } else {
        eprintln!("{}", response.message);
        ExitCode::FAILURE
    }
}

/// Decides what to print for the given arguments (the program name excluded).
fn respond_to_arguments(arguments: &[String]) -> CommandLineResponse {
    match arguments.first().map(String::as_str) {
        Some("--version" | "-V") => CommandLineResponse {
            message: build_version_text(),
            is_success: true,
        },
        None | Some("--help" | "-h") => CommandLineResponse {
            message: build_help_text(),
            is_success: true,
        },
        Some(unknown_argument) => CommandLineResponse {
            message: format!(
                "Unknown argument: {unknown_argument}\n\n{}",
                build_help_text()
            ),
            is_success: false,
        },
    }
}

/// Returns the text printed by `--version`, for example `kadr-cli 0.1.0`.
fn build_version_text() -> String {
    format!("{PROGRAM_NAME} {PROGRAM_VERSION}")
}

/// Returns the text printed by `--help`.
fn build_help_text() -> String {
    format!(
        "{PROGRAM_NAME} {PROGRAM_VERSION} — Kadr from the command line\n\
         \n\
         Usage: {PROGRAM_NAME} [OPTION]\n\
         \n\
         Options:\n  \
         -h, --help       Show this help\n  \
         -V, --version    Show the version"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn version_flag_prints_program_name_and_version() {
        for flag in ["--version", "-V"] {
            let response = respond_to_arguments(&to_arguments(&[flag]));

            assert!(response.is_success);
            assert_eq!(response.message, format!("kadr-cli {PROGRAM_VERSION}"));
        }
    }

    #[test]
    fn help_is_shown_without_arguments_and_with_help_flags() {
        for arguments in [vec![], to_arguments(&["--help"]), to_arguments(&["-h"])] {
            let response = respond_to_arguments(&arguments);

            assert!(response.is_success);
            assert!(response.message.contains("Usage: kadr-cli"));
        }
    }

    #[test]
    fn unknown_argument_fails_and_names_the_argument() {
        let response = respond_to_arguments(&to_arguments(&["--explode"]));

        assert!(!response.is_success);
        assert!(response.message.contains("Unknown argument: --explode"));
        assert!(response.message.contains("Usage: kadr-cli"));
    }
}
