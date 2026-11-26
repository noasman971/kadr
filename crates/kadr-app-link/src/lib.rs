//! # kadr-app-link — `kadr-mcp` talking to the open desktop app
//!
//! When the Kadr app is open, `kadr-mcp` passes Claude's tool calls to it, and
//! the app runs them on the project its window shows: Claude's changes appear
//! live, in the same undo history (decision record 0030).
//!
//! ```text
//! Claude Desktop ──stdio──► kadr-mcp ──local pipe──► Kadr app ──► KadrToolbox (its project)
//!                              │
//!                              └── app closed: KadrToolbox on the .kadr file (file mode)
//! ```
//!
//! - [`protocol`]: one request ([`LinkRequest`]: tool name, arguments, allowed
//!   folders), one answer ([`LinkResponse`]: text and pictures), each a single
//!   line of JSON.
//! - [`endpoint`]: where the app listens, `\\.\pipe\kadr-app-<user>` on Windows,
//!   a Unix socket only the user can open elsewhere.
//! - [`send_to_app`]: `kadr-mcp`'s side, blocking; `Ok(None)` when no app listens.
//! - `server::serve` (feature `server`): the app's side, with tokio.
//!
//! ## Rules for this crate
//!
//! - Local only: remote clients are refused, and only the user's own programs
//!   can open the pipe.
//! - No Tauri and no MCP here: the app and `kadr-mcp` convert to their own types.

pub mod client;
pub mod endpoint;
pub mod protocol;
#[cfg(feature = "server")]
pub mod server;

pub use client::{LinkError, send_to_app};
pub use endpoint::{ADDRESS_OVERRIDE_VARIABLE, LinkAddress, link_address};
pub use protocol::{LinkContent, LinkRequest, LinkResponse, PROTOCOL_VERSION, tool_output_from};
