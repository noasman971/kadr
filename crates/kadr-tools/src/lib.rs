//! # kadr-tools — the editing tools Claude can call
//!
//! Each tool is defined **once** here: its name, a description written for a
//! language model (with explicit units and examples), the JSON schema of its
//! parameters (generated from a Rust struct), and the function that turns a call
//! into Kadr commands.
//!
//! ```text
//! Claude ──► kadr-mcp (MCP over stdio) ──► KadrToolbox::call_tool("split_clip", {…})
//!                                              │
//!                                              ├─► kadr-core commands (one undo step)
//!                                              └─► kadr-engine (pictures, export, files)
//! ```
//!
//! - [`KadrToolbox`]: the allowed folders, the background exports and tracking
//!   analyses; [`KadrToolbox::call_tool`] runs a tool by name on the toolbox's
//!   own project file, [`KadrToolbox::call_tool_on`] on another host's project.
//! - [`ProjectHost`] and [`EditableProject`]: where the project lives, a `.kadr`
//!   file ([`FileProjectHost`]) or the desktop app (decision record 0030).
//! - [`ToolDefinition`]: name, description and JSON schema of a tool, from
//!   [`KadrToolbox::tool_definitions`].
//! - [`ToolOutput`]: text and pictures returned to Claude.
//! - [`AllowedFolders`]: the folders the tools may read and write in.
//!
//! ## Rules for this crate
//!
//! - Tools are independent of the transport: `kadr-mcp` exposes them over MCP,
//!   and the desktop app can reuse them unchanged.
//! - One tool call is one undo group, shown in the history as "Claude: …".
//! - Errors are written for humans and language models
//!   ("clip 12 does not exist; existing clips: 3, 5, 8").
//! - Files are only read and written inside the allowed folders, never replaced
//!   without `"overwrite": true`, and never deleted.
//! - Times are seconds, sizes and positions pixels with the origin at the
//!   top-left corner.
//!
//! See `docs/mcp-server.md` for the list of tools.

mod blur_tasks;
mod catalog;
mod export_tasks;
mod file_access;
mod output;
mod pictures_for_claude;
mod project_file_session;
mod project_host;
mod time_input;
mod toolbox;
mod tools;
mod views;

pub use catalog::{ToolDefinition, ToolEffect};
pub use file_access::AllowedFolders;
pub use output::{ToolContent, ToolError, ToolOutput};
pub use pictures_for_claude::MAXIMUM_PICTURE_SIDE_IN_PIXELS;
pub use project_file_session::CLAUDE_UNDO_LABEL_PREFIX;
pub use project_host::{EditableProject, FileProjectHost, ProjectHost, ToolSteps, run_tool_steps};
pub use toolbox::KadrToolbox;
