//! Exporting a project to an MP4 file.
//!
//! ```text
//! project ──► sound of every clip, mixed ──────────────┐
//!         └─► for each frame: pictures → compositor ───┴─► MP4 (H.264 + AAC)
//! ```
//!
//! The file is written under a temporary name and only renamed to the requested
//! name once complete: a cancelled or failed export never leaves a broken file
//! nor replaces a previous good export.

pub(crate) mod audio_mixer;
mod clip_layers;
pub(crate) mod frame_composer;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use kadr_core::effect::EffectKind;
use kadr_core::project::Project;
use kadr_core::time::TimeSpan;
use kadr_media::audio_buffer::sample_index_at;
use kadr_media::{
    EncodingSpeed, ExportSink, ExportSinkSettings, MediaBackend, MediaError, VideoEncoderPreference,
};
use kadr_render::{
    MAXIMUM_COLOR_ADJUSTMENTS_PER_LAYER, RenderError, RendererPreference, create_renderer,
};

use audio_mixer::mix_timeline_sound;
use frame_composer::FrameComposer;

/// What to export and how.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportRequest {
    /// The MP4 file to create. Replaced only if the export succeeds.
    pub output_path: PathBuf,
    /// Hardware encoder when available, or always software.
    pub video_encoder_preference: VideoEncoderPreference,
    /// Speed / quality trade-off.
    pub encoding_speed: EncodingSpeed,
    /// GPU when available, or always CPU.
    pub renderer_preference: RendererPreference,
}

/// How far the export has gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportProgress {
    /// Frames written so far.
    pub encoded_frame_count: u64,
    /// Frames in the whole export.
    pub total_frame_count: u64,
}

impl ExportProgress {
    /// Progress from 0 to 1.
    pub fn fraction(&self) -> f64 {
        if self.total_frame_count == 0 {
            return 1.0;
        }
        self.encoded_frame_count as f64 / self.total_frame_count as f64
    }
}

/// Lets another thread (the interface) stop an export. Cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct ExportCancellation {
    is_cancelled: Arc<AtomicBool>,
}

impl ExportCancellation {
    /// A cancellation switch that is off.
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks the export to stop as soon as possible.
    pub fn cancel(&self) {
        self.is_cancelled.store(true, Ordering::Relaxed);
    }

    /// Whether [`ExportCancellation::cancel`] was called.
    pub fn is_cancelled(&self) -> bool {
        self.is_cancelled.load(Ordering::Relaxed)
    }
}

/// What happened during a successful export.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportReport {
    /// The file written.
    pub output_path: PathBuf,
    /// Number of video frames written.
    pub frame_count: u64,
    /// Length of the video.
    pub duration: TimeSpan,
    /// The video encoder used, for example `"h264_qsv"` or `"libx264"`.
    pub video_encoder_name: String,
    /// What rendered the frames, for example `"GPU: Intel(R) Arc(TM) Graphics (Vulkan)"` or `"CPU"`.
    pub renderer_description: String,
    /// Parts of the project that were not rendered yet, in plain words.
    pub warnings: Vec<String>,
}

/// Why an export failed.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// There is nothing to export.
    #[error("the timeline is empty: add clips before exporting")]
    EmptyTimeline,
    /// The export was cancelled.
    #[error("the export was cancelled")]
    Cancelled,
    /// Reading a source file or writing the video failed.
    #[error(transparent)]
    Media(#[from] MediaError),
    /// Rendering a frame failed.
    #[error(transparent)]
    Render(#[from] RenderError),
    /// The finished file could not be put in place.
    #[error("could not create \"{}\": {reason}", output_path.display())]
    CannotPlaceOutput {
        /// The requested output file.
        output_path: PathBuf,
        /// What the system said.
        reason: String,
    },
}

/// Renders `project` to an MP4 file.
///
/// `on_progress` is called after every frame. `cancellation` is checked before
/// every frame.
///
/// # Errors
///
/// See [`ExportError`]. On error the requested output file is left untouched.
pub fn export_project(
    project: &Project,
    backend: &dyn MediaBackend,
    request: &ExportRequest,
    on_progress: &mut dyn FnMut(ExportProgress),
    cancellation: &ExportCancellation,
) -> Result<ExportReport, ExportError> {
    let duration = project.timeline().duration();
    if duration.is_zero() {
        return Err(ExportError::EmptyTimeline);
    }
    let temporary_path = temporary_path_for(&request.output_path);
    let result = write_export(
        project,
        backend,
        request,
        &temporary_path,
        on_progress,
        cancellation,
    );
    match result {
        Ok(mut report) => {
            fs::rename(&temporary_path, &request.output_path).map_err(|error| {
                let _ = fs::remove_file(&temporary_path);
                ExportError::CannotPlaceOutput {
                    output_path: request.output_path.clone(),
                    reason: error.to_string(),
                }
            })?;
            report.output_path = request.output_path.clone();
            Ok(report)
        }
        Err(error) => {
            // Best effort: the partial file is useless.
            let _ = fs::remove_file(&temporary_path);
            Err(error)
        }
    }
}

/// The file written while the export is in progress.
fn temporary_path_for(output_path: &Path) -> PathBuf {
    let mut temporary_name = output_path.as_os_str().to_owned();
    temporary_name.push(".exporting");
    PathBuf::from(temporary_name)
}

fn write_export(
    project: &Project,
    backend: &dyn MediaBackend,
    request: &ExportRequest,
    temporary_path: &Path,
    on_progress: &mut dyn FnMut(ExportProgress),
    cancellation: &ExportCancellation,
) -> Result<ExportReport, ExportError> {
    let settings = project.settings();
    let duration = project.timeline().duration();
    let total_frame_count = settings.frame_rate.frame_count_covering(duration);
    let sample_rate = settings.audio_sample_rate_in_hertz;

    let timeline_sound = mix_timeline_sound(project, backend, cancellation)?;
    let mut sink = backend.create_export_sink(&ExportSinkSettings {
        output_path: temporary_path.to_path_buf(),
        width_in_pixels: settings.canvas_size.width_in_pixels,
        height_in_pixels: settings.canvas_size.height_in_pixels,
        frame_rate: settings.frame_rate,
        audio_sample_rate_in_hertz: sample_rate,
        video_encoder_preference: request.video_encoder_preference,
        encoding_speed: request.encoding_speed,
    })?;
    let video_encoder_name = sink.video_encoder_name().to_string();
    let mut composer = FrameComposer::new(create_renderer(request.renderer_preference)?);
    let renderer_description = composer.renderer_description();
    let mut written_sample_count = 0;

    for frame_index in 0..total_frame_count {
        if cancellation.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        let frame_start = settings.frame_rate.timestamp_of_frame(frame_index);
        let frame = composer.compose_frame_at::<ExportError>(project, backend, frame_start)?;
        sink.write_video_frame(&frame)?;

        // Keep sound and pictures advancing together in the file.
        let next_frame_start = settings.frame_rate.timestamp_of_frame(frame_index + 1);
        let sound_end_sample = sample_index_at(next_frame_start, sample_rate)
            .min(timeline_sound.samples_per_channel());
        write_sound_range(
            sink.as_mut(),
            &timeline_sound,
            written_sample_count,
            sound_end_sample,
        )?;
        written_sample_count = sound_end_sample.max(written_sample_count);

        on_progress(ExportProgress {
            encoded_frame_count: frame_index + 1,
            total_frame_count,
        });
    }
    write_sound_range(
        sink.as_mut(),
        &timeline_sound,
        written_sample_count,
        timeline_sound.samples_per_channel(),
    )?;
    sink.finish()?;

    Ok(ExportReport {
        output_path: temporary_path.to_path_buf(),
        frame_count: total_frame_count,
        duration,
        video_encoder_name,
        renderer_description,
        warnings: describe_unrendered_parts(project),
    })
}

/// Writes samples `first_sample..end_sample` of the mixed sound.
fn write_sound_range(
    sink: &mut dyn ExportSink,
    timeline_sound: &kadr_media::AudioBuffer,
    first_sample: usize,
    end_sample: usize,
) -> Result<(), MediaError> {
    if end_sample <= first_sample {
        return Ok(());
    }
    let samples = timeline_sound.interleaved_range(first_sample, end_sample - first_sample);
    sink.write_audio(&kadr_media::AudioBuffer::from_interleaved_stereo(
        timeline_sound.sample_rate_in_hertz(),
        samples.to_vec(),
    ))
}

/// Lists, in plain words, what could not be rendered as asked.
fn describe_unrendered_parts(project: &Project) -> Vec<String> {
    let mut warnings = Vec::new();
    for (_, clip) in project.timeline().all_clips() {
        let enabled_effects = clip.effects.iter().filter(|effect| effect.is_enabled);
        let mut color_adjustment_count = 0;
        for effect in enabled_effects {
            match &effect.kind {
                EffectKind::TrackedBlur(tracked_blur) if tracked_blur.region_samples.is_empty() => {
                    warnings.push(format!(
                        "the tracked blur (effect {}) of clip {} hides nothing yet: it has no tracked region",
                        effect.id, clip.id
                    ));
                }
                EffectKind::ColorAdjustment(_) => color_adjustment_count += 1,
                EffectKind::TrackedBlur(_) => {}
            }
        }
        if color_adjustment_count > MAXIMUM_COLOR_ADJUSTMENTS_PER_LAYER {
            warnings.push(format!(
                "clip {} has {color_adjustment_count} colour adjustments; only the first {MAXIMUM_COLOR_ADJUSTMENTS_PER_LAYER} were applied",
                clip.id
            ));
        }
    }
    warnings
}
