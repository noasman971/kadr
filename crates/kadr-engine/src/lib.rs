//! # kadr-engine — the single entry point for user interfaces
//!
//! The engine ties the other crates together. User interfaces (desktop app,
//! Android app, command line, MCP server) only talk to the engine:
//!
//! - open, save and auto-save projects;
//! - apply commands and expose undo / redo;
//! - provide preview frames for the player;
//! - run long tasks (export, face tracking, proxy generation) in the background,
//!   with progress reporting and cancellation.
//!
//! ## Rules for this crate
//!
//! - No user interface code and no dependency on Tauri: the engine is shared
//!   between desktop and mobile.
//! - Long tasks never block the caller's thread.
//!
//! Grows from phase 2 onwards. See `docs/architecture.md`.
