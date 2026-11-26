//! Passing Claude's calls to the open desktop app (decision record 0030).
//!
//! Before each call, `kadr-mcp` tries the app's pipe:
//!
//! - the app answers: the call ran on the project open in its window, live;
//! - nothing listens (the app is closed): the call runs here, on the project
//!   file (file mode);
//! - the app is there but the exchange fails: the call is **not** run again
//!   here (it may have been applied in the app); Claude is told to check.
//!
//! The first answer after a switch says so, and when the app goes away its last
//! project is reopened from its file.

use std::path::PathBuf;

use kadr_app_link::{
    LinkAddress, LinkRequest, LinkResponse, PROTOCOL_VERSION, client::send_to_app_at,
    tool_output_from,
};
use kadr_tools::{KadrToolbox, ToolContent, ToolError, ToolOutput};
use serde_json::Value;

/// Where calls go, and what `kadr-mcp` remembers of the app between calls.
#[derive(Debug)]
pub struct AppRelay {
    /// Where the app listens; `None` for file mode only (tests).
    address: Option<LinkAddress>,
    /// Whether the previous call ran in the app.
    was_in_app: bool,
    /// The project the app had open after the last call.
    last_app_project: Option<PathBuf>,
}

impl AppRelay {
    /// Calls go to the app listening at `address` when it is open.
    pub fn to_app_at(address: LinkAddress) -> Self {
        Self {
            address: Some(address),
            was_in_app: false,
            last_app_project: None,
        }
    }

    /// Calls always run here, on the project file.
    pub fn file_mode_only() -> Self {
        Self {
            address: None,
            was_in_app: false,
            last_app_project: None,
        }
    }

    /// Runs `tool_name` in the app if it is open, else with `toolbox` on the
    /// project file.
    pub fn run(
        &mut self,
        toolbox: &mut KadrToolbox,
        tool_name: &str,
        arguments: Value,
    ) -> ToolOutput {
        let Some(address) = self.address.clone() else {
            return toolbox.call_tool(tool_name, arguments);
        };
        let request = LinkRequest {
            protocol_version: PROTOCOL_VERSION,
            client: format!("kadr-mcp {}", env!("CARGO_PKG_VERSION")),
            tool_name: tool_name.to_string(),
            arguments: arguments.clone(),
            allowed_folders: toolbox.allowed_folders().folders().to_vec(),
        };
        match send_to_app_at(&address, &request) {
            Ok(Some(LinkResponse::ToolOutput {
                content,
                is_error,
                project_file,
            })) => {
                let output = tool_output_from(&content, is_error);
                let note = (!self.was_in_app).then(|| APP_OPEN_NOTE.to_string());
                self.was_in_app = true;
                self.last_app_project = project_file;
                with_note(output, note)
            }
            Ok(Some(LinkResponse::Refused { reason, .. })) => {
                tracing::warn!("the Kadr app refused the call: {reason}");
                let output = toolbox.call_tool(tool_name, arguments);
                with_note(
                    output,
                    Some(format!(
                        "The Kadr app is open but could not take the call ({reason}), so it ran on \
                         the project file instead: the app does not show it until the project is \
                         opened again there."
                    )),
                )
            }
            Ok(None) => {
                let note = self.leave_app(toolbox);
                with_note(toolbox.call_tool(tool_name, arguments), note)
            }
            Err(error) => {
                tracing::error!("the link with the Kadr app failed: {error}");
                ToolOutput::from_error(&ToolError::new(format!(
                    "{error}. The call may or may not have been applied in the Kadr app: check with \
                     get_timeline or get_project_info before trying again"
                )))
            }
        }
    }

    /// The app is gone: reopens its last project from its file, and says so.
    fn leave_app(&mut self, toolbox: &mut KadrToolbox) -> Option<String> {
        if !self.was_in_app {
            return None;
        }
        self.was_in_app = false;
        let Some(project_file) = self.last_app_project.take() else {
            return Some(APP_CLOSED_NOTE.to_string());
        };
        let reopened = toolbox.call_tool(
            "open_project",
            serde_json::json!({ "file_path": project_file }),
        );
        Some(if reopened.is_error {
            format!(
                "{APP_CLOSED_NOTE} Its project \"{}\" could not be reopened: {}",
                project_file.display(),
                reopened.all_text()
            )
        } else {
            format!(
                "{APP_CLOSED_NOTE} Its project \"{}\" was reopened from its file.",
                project_file.display()
            )
        })
    }
}

const APP_OPEN_NOTE: &str = "Note: the Kadr app is open, so the tools now work on the project open \
in it, live: the user sees each change, and undo only undoes your own changes.";

const APP_CLOSED_NOTE: &str = "Note: the Kadr app was closed, so the tools now work directly on the \
project file (saved after every change).";

/// `output` with `note` as its first text.
fn with_note(mut output: ToolOutput, note: Option<String>) -> ToolOutput {
    if let Some(note) = note {
        output.content.insert(0, ToolContent::Text(note));
    }
    output
}
