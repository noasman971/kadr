//! # kadr-engine — the single entry point for user interfaces
//!
//! The engine ties the other crates together. User interfaces (desktop app,
//! Android app, command line, MCP server) only talk to the engine:
//!
//! - [`project_files`]: save and open `.kadr` files (atomically);
//! - [`export`]: render a project to an MP4 file, with progress and cancellation;
//! - [`clip_geometry`]: where a clip's source picture is on the canvas, for
//!   interfaces drawing on top of the preview;
//! - [`export_presets`]: the export sizes, and a project scaled to another size;
//! - [`quick_assembly`]: build a simple project from a list of files;
//! - [`region_tracking`]: follow a selected region through a clip, forwards and
//!   backwards, to fill a tracked blur (the face blur);
//! - [`preview`]: pictures of the timeline at any time, at an interface's size;
//! - [`timeline_sound`]: the mixed sound of any range of the timeline, for playback;
//! - [`media_overview`]: thumbnails and waveforms of media files;
//! - [`picture_scaling`]: shrinking pictures for display;
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

pub mod clip_geometry;
pub mod export;
pub mod export_presets;
pub mod media_overview;
pub mod picture_scaling;
pub mod preview;
pub mod project_files;
pub mod quick_assembly;
pub mod region_tracking;
pub mod silence_analysis;
pub mod still_frames;
pub mod timeline_sound;

pub use export::{
    ExportCancellation, ExportError, ExportProgress, ExportReport, ExportRequest, export_project,
};
pub use kadr_media::{
    EncodingSpeed, FfmpegBackend, MediaBackend, MediaError, VideoEncoderPreference,
};
pub use kadr_render::RendererPreference;
pub use region_tracking::{
    RegionTrackingError, RegionTrackingReport, RegionTrackingRequest, TrackingCancellation,
    TrackingProgress, corrected_tracked_blur, find_tracked_blur, track_region,
};
