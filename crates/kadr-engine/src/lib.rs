//! # kadr-engine — the single entry point for user interfaces
//!
//! The engine ties the other crates together. User interfaces (desktop app,
//! Android app, command line, MCP server) only talk to the engine:
//!
//! - [`project_files`]: save and open `.kadr` files (atomically);
//! - [`export`]: render a project to an MP4 file, with progress and cancellation;
//! - [`quick_assembly`]: build a simple project from a list of files;
//! - [`still_frames`]: single pictures of a file, a clip or the timeline, and
//!   contact sheets of thumbnails;
//! - [`silence_analysis`]: the silent parts of a file's sound.
//!
//! ## Rules for this crate
//!
//! - No user interface code and no dependency on Tauri: the engine is shared
//!   between desktop and mobile.
//! - Media access goes through a [`kadr_media::MediaBackend`], never directly
//!   through FFmpeg, so the Android backend can replace it.
//! - Long tasks report progress and can be cancelled from another thread.
//!
//! See `docs/architecture.md`.

pub mod export;
pub mod project_files;
pub mod quick_assembly;
pub mod silence_analysis;
pub mod still_frames;

pub use export::{
    ExportCancellation, ExportError, ExportProgress, ExportReport, ExportRequest, export_project,
};
pub use kadr_media::{
    EncodingSpeed, FfmpegBackend, MediaBackend, MediaError, VideoEncoderPreference,
};
pub use kadr_render::RendererPreference;
