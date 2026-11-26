//! The `blur` command: follow a region through a video and hide it.
//!
//! 1. A project is built around the video, with a canvas of the video's size
//!    (an odd last column or row is cropped: encoders need even sizes).
//! 2. The region selected at `--at` is tracked forwards and backwards.
//! 3. A tracked blur holding the result is added to the clip.
//! 4. The project is exported (and saved with `--save-project`).
//! 5. With `--debug`, a second video shows the tracked box and the tracker's
//!    confidence on every frame.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Instant;

use kadr_core::clip::CropInsets;
use kadr_core::color::Color;
use kadr_core::commands::{AddEffect, SetClipCrop};
use kadr_core::effect::{BlurStyle, EffectKind, PixelRegion, TrackedBlur};
use kadr_core::history::EditSession;
use kadr_core::ids::ClipId;
use kadr_core::media_asset::{MediaKind, PictureSize};
use kadr_core::project::ProjectSettings;
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_engine::project_files::save_project_file;
use kadr_engine::quick_assembly::assemble_project_from_files;
use kadr_engine::region_tracking::{DebugVideoRequest, export_tracking_debug_video};
use kadr_engine::{
    ExportCancellation, ExportRequest, RegionTrackingError, RegionTrackingRequest,
    RendererPreference, TrackingCancellation, export_project, track_region,
};
use kadr_media::{EncodingSpeed, FfmpegBackend, MediaBackend, VideoEncoderPreference};

use crate::arguments::{HidingStyle, RectangleArgument};
use crate::reports;

/// Strength of the gaussian blur, in pixels.
const BLUR_RADIUS_IN_PIXELS: f32 = 30.0;

/// Size of the squares of the pixelation, in pixels.
const PIXELATION_BLOCK_SIZE_IN_PIXELS: u32 = 24;

/// Still images are not tracked, but the assembly needs a duration for them.
const STILL_IMAGE_DURATION: TimeSpan = TimeSpan::from_whole_seconds(5);

/// Everything the `blur` command was asked to do.
#[derive(Debug, Clone)]
pub struct BlurCommand {
    /// The video file.
    pub video: PathBuf,
    /// The moment of the selection, in seconds.
    pub at_seconds: f64,
    /// The selected region.
    pub rectangle: RectangleArgument,
    /// The MP4 file to create.
    pub output: PathBuf,
    /// How the region is hidden.
    pub style: HidingStyle,
    /// Whether to write the debug video too.
    pub write_debug_video: bool,
    /// Where to save the project, if asked.
    pub project_output: Option<PathBuf>,
    /// Never use a hardware encoder.
    pub software_encoder: bool,
    /// Encode faster.
    pub fast: bool,
    /// Render on the CPU.
    pub cpu: bool,
}

/// Runs the `blur` command.
pub fn blur(backend: &FfmpegBackend, command: &BlurCommand) -> Result<(), Box<dyn Error>> {
    refuse_outputs_replacing_the_video(command)?;
    let description = backend.probe(&command.video)?;
    let video_name = command.video.display();
    let Some(picture_size) = description.picture_size else {
        return Err(format!("{video_name} is not a video").into());
    };
    let Some(frame_rate) = description.frame_rate else {
        return Err(match description.kind {
            MediaKind::Image => format!("{video_name} is a still image, not a video"),
            _ => format!("the frame rate of {video_name} could not be read"),
        }
        .into());
    };
    let selection_time = Timestamp::from_seconds(command.at_seconds)?;
    if let Some(duration) = description.duration
        && selection_time
            .span_since(Timestamp::ZERO)
            .is_some_and(|elapsed| elapsed >= duration)
    {
        return Err(format!(
            "--at {selection_time} is not inside {video_name}, which lasts {duration}"
        )
        .into());
    }
    let settings = ProjectSettings {
        canvas_size: even_size(picture_size),
        frame_rate,
        ..ProjectSettings::landscape_1080p()
    };
    let project = assemble_project_from_files(
        backend,
        std::slice::from_ref(&command.video),
        settings,
        STILL_IMAGE_DURATION,
    )?;
    let clip_id = *project
        .timeline()
        .clip_ids()
        .first()
        .ok_or("the video could not be placed on the timeline")?;
    let selection = PixelRegion {
        left_in_pixels: command.rectangle.left,
        top_in_pixels: command.rectangle.top,
        width_in_pixels: command.rectangle.width,
        height_in_pixels: command.rectangle.height,
    };
    let tracking_request =
        RegionTrackingRequest::for_clip(&project, clip_id, selection_time, selection)?;

    let tracking_cancellation = TrackingCancellation::new();
    let export_cancellation = ExportCancellation::new();
    {
        let (tracking, export) = (tracking_cancellation.clone(), export_cancellation.clone());
        // Ctrl+C stops cleanly instead of leaving a half-written file.
        ctrlc::set_handler(move || {
            tracking.cancel();
            export.cancel();
        })?;
    }

    let started_at = Instant::now();
    let mut progress_line = reports::ProgressLine::default();
    let report = track_region(
        backend,
        &tracking_request,
        &mut |progress| progress_line.update_tracking("Tracking", progress),
        &tracking_cancellation,
    );
    progress_line.finish();
    let report = report?;
    println!(
        "{}",
        reports::describe_tracking(&report, started_at.elapsed(), frame_rate)
    );

    let tracked_blur = report.tracked_blur_like(&TrackedBlur {
        style: blur_style_for(command.style),
        ..TrackedBlur::with_default_style()
    });
    let mut session = EditSession::new(project)?;
    crop_odd_edges(&mut session, clip_id, picture_size)?;
    session.execute(AddEffect::new(
        clip_id,
        EffectKind::TrackedBlur(tracked_blur),
    ))?;
    if let Some(project_path) = &command.project_output {
        save_project_file(session.project(), project_path)?;
        println!("Saved {}", project_path.display());
    }

    let (video_encoder_preference, encoding_speed) = encoding_choices(command);
    let export_request = ExportRequest {
        output_path: command.output.clone(),
        video_encoder_preference,
        encoding_speed,
        renderer_preference: if command.cpu {
            RendererPreference::CpuOnly
        } else {
            RendererPreference::GpuIfAvailable
        },
    };
    let started_at = Instant::now();
    let mut progress_line = reports::ProgressLine::default();
    let exported = export_project(
        session.project(),
        backend,
        &export_request,
        &mut |progress| progress_line.update(progress),
        &export_cancellation,
    );
    progress_line.finish();
    println!(
        "{}",
        reports::describe_export(&exported?, started_at.elapsed())
    );

    if command.write_debug_video {
        let debug_path = debug_video_path(&command.output);
        let mut progress_line = reports::ProgressLine::default();
        let written = export_tracking_debug_video(
            backend,
            &tracking_request,
            &report,
            &DebugVideoRequest {
                output_path: debug_path.clone(),
                video_encoder_preference,
                encoding_speed,
            },
            &mut |progress| progress_line.update_tracking("Drawing the debug video", progress),
            &tracking_cancellation,
        );
        progress_line.finish();
        written.map_err(|error| match error {
            RegionTrackingError::DebugVideoCancelled => format!(
                "the debug video was cancelled ({} was exported)",
                command.output.display()
            ),
            other => format!(
                "could not write the debug video ({} was exported): {other}",
                command.output.display()
            ),
        })?;
        println!("Debug video: {}", debug_path.display());
    }
    Ok(())
}

/// Refuses outputs that would replace the video being blurred, or each other.
fn refuse_outputs_replacing_the_video(command: &BlurCommand) -> Result<(), Box<dyn Error>> {
    let video = std::fs::canonicalize(&command.video)?;
    let mut outputs = vec![command.output.clone()];
    if command.write_debug_video {
        outputs.push(debug_video_path(&command.output));
    }
    outputs.extend(command.project_output.clone());
    for (index, output) in outputs.iter().enumerate() {
        // A file that does not exist yet cannot be the video.
        if std::fs::canonicalize(output).is_ok_and(|path| path == video) {
            return Err(format!(
                "{} is the video being blurred: choose another output",
                output.display()
            )
            .into());
        }
        if outputs[..index].contains(output) {
            return Err(format!("{} is given twice as an output", output.display()).into());
        }
    }
    Ok(())
}

/// Encoders need even sizes: an odd last column or row is cropped, so that the
/// rest of the picture is not resampled to fit the even canvas.
fn crop_odd_edges(
    session: &mut EditSession,
    clip_id: ClipId,
    picture_size: PictureSize,
) -> Result<(), Box<dyn Error>> {
    let even = even_size(picture_size);
    if even == picture_size {
        return Ok(());
    }
    let cropped_fraction = |side: u32, even_side: u32| (side - even_side) as f32 / side as f32;
    session.execute(SetClipCrop::new(
        clip_id,
        CropInsets {
            left_fraction: 0.0,
            top_fraction: 0.0,
            right_fraction: cropped_fraction(picture_size.width_in_pixels, even.width_in_pixels),
            bottom_fraction: cropped_fraction(picture_size.height_in_pixels, even.height_in_pixels),
        },
    ))?;
    Ok(())
}

fn blur_style_for(style: HidingStyle) -> BlurStyle {
    match style {
        HidingStyle::Blur => BlurStyle::GaussianBlur {
            radius_in_pixels: BLUR_RADIUS_IN_PIXELS,
        },
        HidingStyle::Pixelate => BlurStyle::Pixelate {
            block_size_in_pixels: PIXELATION_BLOCK_SIZE_IN_PIXELS,
        },
        HidingStyle::Fill => BlurStyle::SolidFill {
            color: Color::BLACK,
        },
    }
}

fn encoding_choices(command: &BlurCommand) -> (VideoEncoderPreference, EncodingSpeed) {
    (
        if command.software_encoder {
            VideoEncoderPreference::SoftwareOnly
        } else {
            VideoEncoderPreference::HardwareFirst
        },
        if command.fast {
            EncodingSpeed::Fast
        } else {
            EncodingSpeed::Balanced
        },
    )
}

/// Encoders need even sizes.
fn even_size(size: PictureSize) -> PictureSize {
    PictureSize {
        width_in_pixels: size.width_in_pixels & !1,
        height_in_pixels: size.height_in_pixels & !1,
    }
}

/// `blurred.mp4` → `blurred.debug.mp4`.
fn debug_video_path(output: &Path) -> PathBuf {
    output.with_extension("debug.mp4")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_debug_video_sits_next_to_the_output() {
        assert_eq!(
            debug_video_path(Path::new("out/blurred.mp4")),
            PathBuf::from("out/blurred.debug.mp4")
        );
    }

    #[test]
    fn odd_sizes_are_rounded_down_to_even() {
        let size = even_size(PictureSize {
            width_in_pixels: 1281,
            height_in_pixels: 721,
        });
        assert_eq!((size.width_in_pixels, size.height_in_pixels), (1280, 720));
    }
}
