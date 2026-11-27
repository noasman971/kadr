//! Export tools: start an export in the background, follow it, cancel it.

use std::path::PathBuf;

use kadr_core::media_asset::PictureSize;
use kadr_core::project::Project;
use kadr_engine::export_presets::{
    ExportPreset as EnginePreset, ExportSizeError, project_at_export_size,
};
use kadr_engine::{EncodingSpeed, ExportRequest, RendererPreference, VideoEncoderPreference};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{ToolEffect, ToolEntry, declare_tool};
use crate::export_tasks::{ExportTaskState, ExportTaskStatus};
use crate::output::{ToolError, ToolOutput};
use crate::time_input::seconds_in;
use crate::toolbox::ToolCall;
use crate::tools::common::with_extension;
use crate::tools::picture_tools::{
    ensure_project_color_luts_readable, ensure_project_media_readable,
};

/// The export tools.
pub(crate) fn export_tools() -> Vec<ToolEntry> {
    vec![
        declare_tool(
            "export",
            "Export the video",
            EXPORT_DESCRIPTION,
            ToolEffect::WritesFiles,
            export,
        ),
        declare_tool(
            "get_export_status",
            "Follow an export",
            GET_EXPORT_STATUS_DESCRIPTION,
            ToolEffect::ReadsOnly,
            get_export_status,
        ),
        declare_tool(
            "cancel_export",
            "Cancel an export",
            CANCEL_EXPORT_DESCRIPTION,
            ToolEffect::ChangesProject,
            cancel_export,
        ),
    ]
}

// ----- export -----

const EXPORT_DESCRIPTION: &str = "\
Exports the open project to an MP4 video file (H.264 video, AAC sound) on the user's computer. \
The export runs in the background: this tool answers at once with a task_id; then call \
get_export_status every few seconds until it says finished, and tell the user where the file is. \
The export renders the project as it is now; later edits do not change it.

Presets: \"project\" (default) uses the project's own size; \"tiktok_reels_shorts\" is \
1080×1920, \"youtube_1080p\" 1920×1080, \"youtube_4k\" 3840×2160, \"custom\" takes \
width_in_pixels and height_in_pixels. A preset of the same shape as the project but another size \
scales everything (texts included). A preset of another shape is refused: first change the \
project with set_project_settings (for example canvas vertical_1080x1920), check the framing \
with get_frame, then export. Never replaces an existing file unless \"overwrite\": true (ask the \
user first).

Example: {\"output_path\": \"C:\\\\Users\\\\sam\\\\Videos\\\\holiday-tiktok.mp4\", \"preset\": \
\"tiktok_reels_shorts\"}";

/// The export sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ExportPreset {
    /// The project's own canvas size.
    #[default]
    Project,
    /// 1080 × 1920 vertical: TikTok, Instagram Reels, YouTube Shorts.
    TiktokReelsShorts,
    /// 1920 × 1080 landscape.
    #[serde(rename = "youtube_1080p")]
    Youtube1080p,
    /// 3840 × 2160 landscape.
    #[serde(rename = "youtube_4k")]
    Youtube4k,
    /// width_in_pixels × height_in_pixels.
    Custom,
}

/// Speed / quality trade-off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ExportQuality {
    /// Good quality for the final video.
    #[default]
    Balanced,
    /// Faster, larger file: for a quick draft.
    Fast,
}

/// Parameters of `export`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExportParameters {
    /// Full path of the MP4 file to create, inside a folder the user allowed (".mp4" added if
    /// missing). Example: "C:\\Users\\sam\\Videos\\holiday.mp4".
    output_path: PathBuf,
    /// Video size preset. Default "project".
    #[serde(default)]
    preset: ExportPreset,
    /// With preset "custom": width in pixels (even number).
    width_in_pixels: Option<u32>,
    /// With preset "custom": height in pixels (even number).
    height_in_pixels: Option<u32>,
    /// "balanced" (default) or "fast".
    #[serde(default)]
    quality: ExportQuality,
    /// Replace output_path if it already exists. Only true if the user agreed. Default false.
    #[serde(default)]
    overwrite: bool,
}

fn export(
    toolbox: &mut ToolCall<'_>,
    parameters: ExportParameters,
) -> Result<ToolOutput, ToolError> {
    let project = toolbox.require_project()?;
    if project.timeline().duration().is_zero() {
        return Err(ToolError::new(
            "the timeline is empty: add clips before exporting",
        ));
    }
    let output_path = with_extension(&parameters.output_path, "mp4")?;
    let output_path = toolbox
        .allowed_folders()
        .check_file_to_write(&output_path, parameters.overwrite)?;
    ensure_project_media_readable(toolbox, project)?;
    ensure_project_color_luts_readable(toolbox, project)?;
    let target_size = target_size_of(&parameters, project.settings().canvas_size)?;
    let exported_project = project_at_size(project, target_size)?;
    let request = ExportRequest {
        output_path: output_path.clone(),
        video_encoder_preference: VideoEncoderPreference::HardwareFirst,
        encoding_speed: match parameters.quality {
            ExportQuality::Balanced => EncodingSpeed::Balanced,
            ExportQuality::Fast => EncodingSpeed::Fast,
        },
        renderer_preference: RendererPreference::GpuIfAvailable,
    };
    let duration = exported_project.timeline().duration();
    let backend = toolbox.shared_backend();
    let task_id = toolbox
        .export_tasks()
        .start(exported_project, backend, request)?;
    Ok(ToolOutput::text(format!(
        "Export {task_id} started: {} s of video at {}×{} pixels into \"{}\". Call \
         get_export_status with {{\"task_id\": \"{task_id}\"}} every few seconds until it is \
         finished.",
        seconds_in(duration),
        target_size.width_in_pixels,
        target_size.height_in_pixels,
        output_path.display()
    )))
}

/// The picture size the preset asks for.
fn target_size_of(
    parameters: &ExportParameters,
    canvas_size: PictureSize,
) -> Result<PictureSize, ToolError> {
    let size = |width_in_pixels, height_in_pixels| PictureSize {
        width_in_pixels,
        height_in_pixels,
    };
    Ok(match parameters.preset {
        ExportPreset::Project => canvas_size,
        ExportPreset::TiktokReelsShorts => EnginePreset::VerticalReelsShorts.picture_size(),
        ExportPreset::Youtube1080p => EnginePreset::Youtube1080p.picture_size(),
        ExportPreset::Youtube4k => EnginePreset::Youtube4k.picture_size(),
        ExportPreset::Custom => match (parameters.width_in_pixels, parameters.height_in_pixels) {
            (Some(width), Some(height)) => size(width, height),
            _ => {
                return Err(ToolError::new(
                    "the custom preset needs width_in_pixels and height_in_pixels",
                ));
            }
        },
    })
}

/// The project to export at `target_size`: the project itself at its own size,
/// or a scaled copy for another size of the same shape (`kadr_engine::export_presets`).
fn project_at_size(project: &Project, target_size: PictureSize) -> Result<Project, ToolError> {
    project_at_export_size(project, target_size).map_err(|error| match error {
        ExportSizeError::OtherShape {
            project_size,
            export_size,
        } => ToolError::new(format!(
            "the project is {}×{} but this export is {}×{}, another shape: the picture would be              distorted or badly framed. First call set_project_settings with the new size (for              example canvas vertical_1080x1920), check the framing with get_frame (set_transform              with fit \"fill_canvas\" fills the canvas), then export with preset \"project\"",
            project_size.width_in_pixels,
            project_size.height_in_pixels,
            export_size.width_in_pixels,
            export_size.height_in_pixels
        )),
        ExportSizeError::Edit(edit_error) => ToolError::from(edit_error),
    })
}

// ----- get_export_status -----

const GET_EXPORT_STATUS_DESCRIPTION: &str = "\
Tells how an export is going: running (with the percentage done and the estimated time left), \
finished (with the file path, duration and any warning), failed (with the reason) or \
cancelled. Without task_id, lists every export started since Kadr started. \
Example: {\"task_id\": \"export-1\"}";

/// Parameters of `get_export_status`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetExportStatusParameters {
    /// The task_id returned by export, for example "export-1". Omit to list every export.
    task_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct ExportStatusView {
    task_id: String,
    output_path: String,
    state: &'static str,
    elapsed_seconds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    percent_done: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    estimated_seconds_left: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    video_duration_in_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    video_encoder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    renderer: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_reason: Option<String>,
}

impl ExportStatusView {
    fn of(status: &ExportTaskStatus) -> Self {
        let elapsed_seconds = round_to_tenths(status.elapsed.as_secs_f64());
        let mut view = Self {
            task_id: status.task_id.clone(),
            output_path: status.output_path.display().to_string(),
            state: "running",
            elapsed_seconds,
            percent_done: None,
            estimated_seconds_left: None,
            video_duration_in_seconds: None,
            video_encoder: None,
            renderer: None,
            warnings: Vec::new(),
            failure_reason: None,
        };
        match &status.state {
            ExportTaskState::Running { progress } => {
                let fraction = progress.fraction();
                view.percent_done = Some(round_to_tenths(fraction * 100.0));
                if fraction > 0.0 {
                    view.estimated_seconds_left = Some(round_to_tenths(
                        elapsed_seconds * (1.0 - fraction) / fraction,
                    ));
                }
            }
            ExportTaskState::Finished { report, .. } => {
                view.state = "finished";
                view.percent_done = Some(100.0);
                view.video_duration_in_seconds = Some(seconds_in(report.duration));
                view.video_encoder = Some(report.video_encoder_name.clone());
                view.renderer = Some(report.renderer_description.clone());
                view.warnings = report.warnings.clone();
            }
            ExportTaskState::Failed { reason } => {
                view.state = "failed";
                view.failure_reason = Some(reason.clone());
            }
            ExportTaskState::Cancelled => view.state = "cancelled",
        }
        view
    }

    fn sentence(&self) -> String {
        match self.state {
            "running" if self.percent_done.unwrap_or(0.0) == 0.0 => format!(
                "{} is preparing: mixing the sound and starting the encoder, before the \
                 first frame. Ask again in a few seconds.",
                self.task_id
            ),
            "running" => format!(
                "{} is running: {}% done.",
                self.task_id,
                self.percent_done.unwrap_or(0.0)
            ),
            "finished" => format!(
                "{} is finished: the video is \"{}\".",
                self.task_id, self.output_path
            ),
            "failed" => format!(
                "{} failed: {}. The output file was not created.",
                self.task_id,
                self.failure_reason.as_deref().unwrap_or("unknown reason")
            ),
            _ => format!(
                "{} was cancelled; the output file was not created.",
                self.task_id
            ),
        }
    }
}

fn round_to_tenths(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn get_export_status(
    toolbox: &mut ToolCall<'_>,
    parameters: GetExportStatusParameters,
) -> Result<ToolOutput, ToolError> {
    let export_tasks = toolbox.export_tasks();
    match parameters.task_id {
        Some(task_id) => {
            let view = ExportStatusView::of(&export_tasks.status(&task_id)?);
            Ok(ToolOutput::message_with_details(view.sentence(), &view))
        }
        None => {
            let views: Vec<ExportStatusView> = export_tasks
                .all_statuses()
                .iter()
                .map(ExportStatusView::of)
                .collect();
            let message = if views.is_empty() {
                "No export was started since Kadr started.".to_string()
            } else {
                format!("{} export(s):", views.len())
            };
            Ok(ToolOutput::message_with_details(message, &views))
        }
    }
}

// ----- cancel_export -----

const CANCEL_EXPORT_DESCRIPTION: &str = "\
Stops a running export. The output file is then not created (and an earlier file with the same \
name is left untouched). Example: {\"task_id\": \"export-1\"}";

/// Parameters of `cancel_export`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CancelExportParameters {
    /// The task_id returned by export, for example "export-1".
    task_id: String,
}

fn cancel_export(
    toolbox: &mut ToolCall<'_>,
    parameters: CancelExportParameters,
) -> Result<ToolOutput, ToolError> {
    let status = toolbox.export_tasks().cancel(&parameters.task_id)?;
    let message = match status.state {
        ExportTaskState::Running { .. } => format!(
            "Asked {} to stop; it stops within a frame (check with get_export_status).",
            status.task_id
        ),
        _ => format!("{} had already ended.", status.task_id),
    };
    Ok(ToolOutput::text(message))
}
