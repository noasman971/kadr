//! # kadr-core — the Kadr data model
//!
//! This crate describes **what a video project is**, independently of how it is
//! decoded, rendered or displayed:
//!
//! - the project settings (resolution, frame rate, audio format, background colour);
//! - the imported media assets;
//! - the timeline, its video and audio tracks, and the clips placed on them;
//! - the effects applied to clips (text, colour, transitions, tracked blur…) and
//!   their keyframes;
//! - the **commands** that modify a project, with undo and redo.
//!
//! ## Rules for this crate
//!
//! - No video decoding, no GPU, no user interface, no platform-specific API.
//!   Everything here must compile for Windows, macOS, Linux and Android unchanged.
//! - Every change to a project goes through a command that can be undone,
//!   whether it comes from the mouse, the keyboard or Claude through MCP.
//! - Timeline positions use exact integer or rational time, never `f64`.
//!
//! The data model is implemented in phase 1. See `docs/architecture.md`.
