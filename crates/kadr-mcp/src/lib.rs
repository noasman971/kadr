//! # kadr-mcp — the local MCP server
//!
//! This crate exposes the tools of `kadr-tools` through the Model Context
//! Protocol, over **stdio**. The user adds Kadr to Claude Desktop or Claude Code,
//! describes the edit in their own conversation, and Claude calls these tools on
//! their computer.
//!
//! ## Rules for this crate
//!
//! - **No Claude API call and no API key**: Kadr never talks to Anthropic. A
//!   regular Claude subscription is enough.
//! - Over stdio, **nothing but the protocol is written to stdout**. Logs go to
//!   stderr or to a log file.
//! - File access is limited to the folders the user allowed in Kadr's settings.
//!
//! Implemented in phase 5, connected to the running desktop app in phase 7.
//! See `docs/decisions/0004-claude-through-local-mcp-only.md`.
