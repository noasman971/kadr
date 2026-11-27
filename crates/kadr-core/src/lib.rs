//! # kadr-core — the Kadr data model
//!
//! This crate describes **what a video project is**, independently of how it is
//! decoded, rendered or displayed:
//!
//! - [`time`]: exact time in integer ticks, frame rates;
//! - [`project`]: the project settings, its media and its timeline;
//! - [`media_asset`]: the imported video, audio and image files;
//! - [`timeline`]: tracks stacked from bottom to top;
//! - [`clip`]: media or text placed on a track, with volume, opacity, position…;
//! - [`animation`]: keyframes that animate clip properties;
//! - [`effect`]: colour adjustment, colour LUT, tracked blur and stabilisation;
//! - [`transition`]: how videos and images enter and leave the picture;
//! - [`text_animation`]: how texts appear and disappear;
//! - [`commands`]: every change, as a reversible command;
//! - [`history`]: the [`EditSession`](history::EditSession) applying commands
//!   with undo and redo;
//! - [`validation`]: the rules a project always respects;
//! - [`project_file`]: the versioned JSON `.kadr` file format.
//!
//! ## Rules for this crate
//!
//! - No video decoding, no GPU, no user interface, no platform-specific API.
//!   Everything here must compile for Windows, macOS, Linux and Android unchanged.
//! - Every change to a project goes through a command that can be undone,
//!   whether it comes from the mouse, the keyboard or Claude through MCP.
//! - Timeline positions use exact integer time, never `f64`.
//!
//! See `docs/data-model.md` for a guided tour.

pub mod animation;
pub mod clip;
pub mod color;
pub mod commands;
pub mod effect;
pub mod error;
pub mod history;
pub mod ids;
pub mod media_asset;
pub mod project;
pub mod project_file;
pub mod text_animation;
pub mod time;
pub mod timeline;
pub mod transition;
pub mod validation;
