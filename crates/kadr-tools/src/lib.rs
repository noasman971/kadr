//! # kadr-tools — the editing tools Claude can call
//!
//! Each tool is defined **once** here: its name, a description written for a
//! language model (with explicit units and examples), the JSON schema of its
//! parameters, and the function that turns a call into Kadr commands.
//!
//! Examples of tools: `get_timeline`, `get_frame`, `split_clip`, `add_text`,
//! `blur_region`, `export`.
//!
//! ## Rules for this crate
//!
//! - Tools are independent of the transport: `kadr-mcp` exposes them over MCP,
//!   and other transports can reuse them unchanged.
//! - One tool call is one undo group, shown in the history as "Claude: …".
//! - Errors are written for humans and language models
//!   ("clip 12 does not exist, existing clips: 3, 5, 8").
//!
//! Implemented in phase 5. See `docs/architecture.md`.
