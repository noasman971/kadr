//! The export running in the background (docs/ux.md, "Export").
//!
//! One export at a time. It renders the project **as it was when it started**
//! (a snapshot), on its own thread, and reports to the front end through a
//! channel: progress a few times a second, then the result. The files it wrote
//! are remembered: only those can be shown in the explorer or played from the
//! app.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use kadr_core::media_asset::PictureSize;
use kadr_core::project::Project;
use kadr_engine::export_presets::{ExportPreset, ExportSizeError, project_at_export_size};
use kadr_engine::{
    EncodingSpeed, ExportCancellation, ExportError, ExportProgress, ExportRequest,
    RendererPreference, VideoEncoderPreference, export_project,
};
use kadr_media::MediaBackend;
use serde::{Deserialize, Serialize};

use crate::app_state::lock_shared;
use crate::command_error::{CommandError, CommandResult, ErrorCode};

/// Progress is sent at most this often.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);

/// The size of the exported video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportSize {
    /// The project's own canvas.
    Project,
    /// 1920 × 1080.
    Youtube1080p,
    /// 3840 × 2160.
    Youtube4k,
    /// 1080 × 1920.
    VerticalReelsShorts,
    /// 1080 × 1080.
    Square1080,
}

impl ExportSize {
    /// The picture size, for a project whose canvas is `canvas_size`.
    pub fn picture_size(self, canvas_size: PictureSize) -> PictureSize {
        match self {
            Self::Project => canvas_size,
            Self::Youtube1080p => ExportPreset::Youtube1080p.picture_size(),
            Self::Youtube4k => ExportPreset::Youtube4k.picture_size(),
            Self::VerticalReelsShorts => ExportPreset::VerticalReelsShorts.picture_size(),
            Self::Square1080 => ExportPreset::Square1080.picture_size(),
        }
    }
}

/// Speed / quality trade-off, in the dialog's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportQuality {
    /// Good quality for the final video.
    High,
    /// Faster, larger file.
    Fast,
}

/// What the export dialog asks for.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSettings {
    /// The MP4 file to write.
    pub file_path: PathBuf,
    /// Its size.
    pub size: ExportSize,
    /// Speed / quality.
    pub quality: ExportQuality,
    /// `false` to use the software encoder even when a hardware one exists.
    pub uses_hardware_encoder: bool,
    /// Replace the file if it exists (the user confirmed).
    pub replaces_existing_file: bool,
}

/// What the front end hears about a running export.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ExportEvent {
    /// Frames written so far.
    Progress {
        /// Frames written.
        encoded_frame_count: u64,
        /// Frames in the whole video.
        total_frame_count: u64,
        /// Seconds since the export started.
        elapsed_seconds: f64,
    },
    /// The file is written.
    Finished {
        /// The file.
        file_path: String,
        /// Its size on disk.
        file_size_in_bytes: u64,
        /// Length of the video.
        duration_seconds: f64,
        /// Frames written.
        frame_count: u64,
        /// Seconds the export took.
        elapsed_seconds: f64,
        /// The encoder used (`h264_nvenc`, `libx264`…).
        video_encoder_name: String,
        /// What drew the frames.
        renderer_description: String,
        /// Parts of the project not rendered yet, in English.
        warnings: Vec<String>,
    },
    /// The export stopped on an error; the previous file, if any, is untouched.
    Failed {
        /// The engine's explanation, in English.
        details: String,
    },
    /// The user cancelled it.
    Cancelled,
}

/// What `start` answers at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportStarted {
    /// The file being written.
    pub file_path: String,
    /// Width of the video.
    pub width: u32,
    /// Height of the video.
    pub height: u32,
}

/// The export slot and the files exports wrote.
#[derive(Default)]
pub struct ExportJobs {
    running: Arc<Mutex<Option<ExportCancellation>>>,
    written_files: Arc<Mutex<HashSet<PathBuf>>>,
}

impl ExportJobs {
    /// Starts exporting `project` on its own thread; `report` receives the events.
    ///
    /// # Errors
    ///
    /// When an export is already running, the file exists and may not be
    /// replaced, the size has another shape than the project, or the folder
    /// cannot be created.
    pub fn start(
        &self,
        project: &Project,
        backend: Arc<dyn MediaBackend>,
        settings: &ExportSettings,
        report: impl Fn(ExportEvent) + Send + 'static,
    ) -> CommandResult<ExportStarted> {
        let mut running = lock_shared(&self.running);
        if running.is_some() {
            return Err(CommandError::new(
                ErrorCode::ExportAlreadyRunning,
                "an export is already running",
            ));
        }
        let file_path = settings.file_path.clone();
        if file_path.exists() && !settings.replaces_existing_file {
            return Err(CommandError::new(
                ErrorCode::ExportFileExists,
                format!("\"{}\" already exists", file_path.display()),
            )
            .about_file(&file_path));
        }
        let size = settings.size.picture_size(project.settings().canvas_size);
        let exported_project =
            project_at_export_size(project, size).map_err(|error| match error {
                ExportSizeError::OtherShape { .. } => {
                    CommandError::new(ErrorCode::ExportOtherShape, error)
                }
                ExportSizeError::Edit(edit_error) => CommandError::from(edit_error),
            })?;
        if let Some(folder) = file_path.parent() {
            fs::create_dir_all(folder).map_err(|error| {
                CommandError::new(ErrorCode::ExportFailed, error).about_file(&file_path)
            })?;
        }
        let request = export_request(settings);
        let cancellation = ExportCancellation::new();
        *running = Some(cancellation.clone());
        drop(running);

        let running_slot = Arc::clone(&self.running);
        let written_files = Arc::clone(&self.written_files);
        let spawned = thread::Builder::new()
            .name("export".to_string())
            .spawn(move || {
                let event = run_export(
                    &exported_project,
                    backend.as_ref(),
                    &request,
                    &cancellation,
                    &report,
                );
                if let ExportEvent::Finished { .. } = event {
                    lock_shared(&written_files).insert(request.output_path.clone());
                }
                *lock_shared(&running_slot) = None;
                report(event);
            });
        if let Err(error) = spawned {
            *lock_shared(&self.running) = None;
            return Err(CommandError::internal(error));
        }
        Ok(ExportStarted {
            file_path: file_path.display().to_string(),
            width: size.width_in_pixels,
            height: size.height_in_pixels,
        })
    }

    /// Asks the running export to stop; nothing happens when none runs.
    pub fn cancel(&self) {
        if let Some(cancellation) = lock_shared(&self.running).as_ref() {
            cancellation.cancel();
        }
    }

    /// Whether an export wrote `file_path` since the app started.
    pub fn has_written(&self, file_path: &Path) -> bool {
        lock_shared(&self.written_files).contains(file_path)
    }
}

/// The engine's request for the dialog's settings.
fn export_request(settings: &ExportSettings) -> ExportRequest {
    ExportRequest {
        output_path: settings.file_path.clone(),
        video_encoder_preference: if settings.uses_hardware_encoder {
            VideoEncoderPreference::HardwareFirst
        } else {
            VideoEncoderPreference::SoftwareOnly
        },
        encoding_speed: match settings.quality {
            ExportQuality::High => EncodingSpeed::Balanced,
            ExportQuality::Fast => EncodingSpeed::Fast,
        },
        renderer_preference: RendererPreference::GpuIfAvailable,
    }
}

/// Runs the export to the end and says how it ended.
fn run_export(
    project: &Project,
    backend: &dyn MediaBackend,
    request: &ExportRequest,
    cancellation: &ExportCancellation,
    report: &impl Fn(ExportEvent),
) -> ExportEvent {
    let started_at = Instant::now();
    let mut last_report_at: Option<Instant> = None;
    let mut on_progress = |progress: ExportProgress| {
        let is_due = last_report_at.is_none_or(|last| last.elapsed() >= PROGRESS_INTERVAL);
        if is_due {
            last_report_at = Some(Instant::now());
            report(ExportEvent::Progress {
                encoded_frame_count: progress.encoded_frame_count,
                total_frame_count: progress.total_frame_count,
                elapsed_seconds: started_at.elapsed().as_secs_f64(),
            });
        }
    };
    match export_project(project, backend, request, &mut on_progress, cancellation) {
        Ok(export_report) => ExportEvent::Finished {
            file_path: export_report.output_path.display().to_string(),
            file_size_in_bytes: fs::metadata(&export_report.output_path)
                .map(|metadata| metadata.len())
                .unwrap_or(0),
            duration_seconds: export_report.duration.as_seconds(),
            frame_count: export_report.frame_count,
            elapsed_seconds: started_at.elapsed().as_secs_f64(),
            video_encoder_name: export_report.video_encoder_name,
            renderer_description: export_report.renderer_description,
            warnings: export_report.warnings,
        },
        Err(ExportError::Cancelled) => ExportEvent::Cancelled,
        Err(error) => ExportEvent::Failed {
            details: error.to_string(),
        },
    }
}
