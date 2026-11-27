//! Video stabilisation: measuring a clip's camera path, and the correction
//! that steadies each of its frames.
//!
//! [`analyse_camera_path`] reads the part of the file a clip shows, plus a
//! second on each side, follows the picture from one frame to the next
//! (`kadr_vision::camera_motion`) and returns the camera's path. The path is
//! stored as measured in the clip's [`Stabilization`] effect. It is smoothed
//! only when a frame is drawn ([`stabilization_correction_at`]), so cutting,
//! trimming or changing the speed of the clip needs no new analysis.

mod correction;

use std::path::PathBuf;
use std::sync::Arc;

use kadr_core::clip::ClipContent;
use kadr_core::effect::{CameraPathSample, Stabilization};
use kadr_core::error::EditError;
use kadr_core::ids::ClipId;
use kadr_core::media_asset::PictureSize;
use kadr_core::project::Project;
use kadr_core::time::{FrameRate, TimeSpan, Timestamp};
use kadr_core::timeline::TrackKind;
use kadr_media::{MediaBackend, MediaError};
use kadr_vision::camera_motion::CameraMotionEstimator;
use kadr_vision::camera_path::CameraPose;
use kadr_vision::{Point, shrink_factor_for_tracking};

use crate::frame_reading::{FrameReader, FrameReadingError};
use crate::region_tracking::{TrackingCancellation, TrackingProgress};

pub(crate) use correction::applied_stabilization_of;
pub use correction::{
    FrameCorrection, StabilizationSummary, stabilization_correction_at, stabilization_summary,
    stabilization_with_smoothing,
};

/// Stored shifts are rounded to this many pixels.
const SHIFT_ROUNDING_IN_PIXELS: f64 = 0.01;
/// Stored rotations are rounded to this many degrees.
const ROTATION_ROUNDING_IN_DEGREES: f64 = 0.001;
/// Stored sizes are rounded to this fraction.
const SCALE_ROUNDING: f64 = 0.000_01;
/// The path starts again from "no motion" when the picture has grown or shrunk
/// by more than this since the path's start (a long zoom): the stored size must
/// stay well inside the range the file format accepts.
const LARGEST_SCALE_BEFORE_RESTART: f64 = 3.0;
/// The path also starts again when the picture has moved this far (a very long
/// pan), so that stored shifts keep their hundredth of a pixel.
const LARGEST_SHIFT_BEFORE_RESTART_IN_PIXELS: f64 = 50_000.0;

/// What to analyse.
#[derive(Debug, Clone, PartialEq)]
pub struct StabilizationRequest {
    /// The video file.
    pub video_path: PathBuf,
    /// Its frame rate.
    pub frame_rate: FrameRate,
    /// The size of its pictures.
    pub picture_size: PictureSize,
    /// The first moment to analyse, in the file's time (margin included).
    pub source_start: Timestamp,
    /// The end of the part to analyse (excluded), in the file's time (margin
    /// included).
    pub source_end: Timestamp,
}

impl StabilizationRequest {
    /// How much of the file is analysed before and after the part the clip
    /// shows. The clip's first and last frames are then steadied like the
    /// others, transitions that show frames past a cut stay covered, and so
    /// does a clip made a little longer afterwards.
    pub const ANALYSIS_MARGIN: TimeSpan = TimeSpan::from_whole_seconds(1);

    /// The request to analyse the part of the file clip `clip_id` shows, and a
    /// margin around it.
    ///
    /// # Errors
    ///
    /// The clip does not exist, or does not show a video on a video track.
    pub fn for_clip(project: &Project, clip_id: ClipId) -> Result<Self, StabilizationError> {
        let (track, clip) = project.timeline().find_clip(clip_id).ok_or_else(|| {
            StabilizationError::Edit(EditError::ClipNotFound {
                clip_id,
                existing_clip_ids: project.timeline().clip_ids(),
            })
        })?;
        // A video placed on an audio track only gives its sound: it is never drawn.
        let ClipContent::Media(segment) = &clip.content else {
            return Err(StabilizationError::NotAVideoClip { clip_id });
        };
        if track.kind != TrackKind::Video {
            return Err(StabilizationError::NotAVideoClip { clip_id });
        }
        let description = project
            .media_asset(segment.media_asset_id)
            .map(|asset| &asset.description)
            .ok_or(StabilizationError::NotAVideoClip { clip_id })?;
        // A still image has a picture size but no frame rate: nothing moves in it.
        let (Some(picture_size), Some(frame_rate)) =
            (description.picture_size, description.frame_rate)
        else {
            return Err(StabilizationError::NotAVideoClip { clip_id });
        };
        let source_start = segment
            .source_start
            .checked_sub(Self::ANALYSIS_MARGIN)
            .unwrap_or(Timestamp::ZERO);
        // Past the end of the file there is nothing to measure. A file of unknown
        // length gets no margin at its end.
        let source_end = description.duration.map_or(segment.source_end, |duration| {
            let end_of_file = Timestamp::from_ticks(duration.ticks());
            (segment.source_end + Self::ANALYSIS_MARGIN)
                .min(end_of_file)
                .max(segment.source_end)
        });
        Ok(Self {
            video_path: description.file_path.clone(),
            frame_rate,
            picture_size,
            source_start,
            source_end,
        })
    }

    /// Number of frames [`analyse_camera_path`] reads for this request.
    pub fn frame_count(&self) -> u64 {
        let (first, last) = self.frame_indices();
        last - first + 1
    }

    /// First and last frame to analyse.
    fn frame_indices(&self) -> (u64, u64) {
        let first = self.frame_rate.frame_index_at(self.source_start);
        let frames_before_end = self
            .frame_rate
            .frame_count_covering(TimeSpan::from_ticks(self.source_end.ticks()));
        (first, frames_before_end.saturating_sub(1).max(first))
    }
}

/// Why a stabilisation analysis failed.
#[derive(Debug, thiserror::Error)]
pub enum StabilizationError {
    /// The clip does not exist.
    #[error(transparent)]
    Edit(#[from] EditError),
    /// The clip shows a text, a sound or a still image, or is on an audio track.
    #[error(
        "clip {clip_id} does not show a video: only video clips on a video track can be stabilised"
    )]
    NotAVideoClip {
        /// The clip asked for.
        clip_id: ClipId,
    },
    /// The video's pictures changed size in the middle of the clip.
    #[error(
        "the video's pictures change size in the middle of the clip, which stabilisation does not \
         support"
    )]
    VideoSizeChanged,
    /// The analysis was cancelled.
    #[error("the analysis was cancelled")]
    Cancelled,
    /// Reading the video failed.
    #[error(transparent)]
    Media(#[from] MediaError),
}

impl From<FrameReadingError> for StabilizationError {
    fn from(error: FrameReadingError) -> Self {
        match error {
            FrameReadingError::SizeChanged => Self::VideoSizeChanged,
            FrameReadingError::Media(error) => Self::Media(error),
        }
    }
}

/// The camera's measured path through the analysed part of a video.
#[derive(Debug, Clone, PartialEq)]
pub struct CameraPathReport {
    /// One sample per analysed frame, in the file's pixels. Shared, not copied,
    /// with the effects made from this report.
    pub camera_path: Arc<[CameraPathSample]>,
    /// The size of the analysed pictures.
    pub analysed_picture_size: PictureSize,
    /// Frames, after the first, whose motion could not be measured (cuts,
    /// uniform or very blurred pictures).
    pub unmeasured_frame_count: usize,
    /// By how much the pictures were shrunk for the analysis.
    pub shrink_factor: usize,
}

impl CameraPathReport {
    /// The effect that steadies the analysed video with this smoothing, its
    /// enlargement fitted to the path.
    pub fn stabilization_with(&self, smoothing_in_seconds: f32) -> Stabilization {
        let unfitted = Stabilization {
            smoothing_in_seconds,
            zoom_factor: Stabilization::MINIMUM_ZOOM_FACTOR,
            analysed_picture_size: self.analysed_picture_size,
            camera_path: Arc::clone(&self.camera_path),
        };
        stabilization_with_smoothing(&unfitted, smoothing_in_seconds)
    }
}

/// Measures the camera's path through the requested part of the video.
///
/// `on_progress` is called after every frame; `cancellation` is checked before
/// every frame. The video is opened on the calling thread.
///
/// # Errors
///
/// See [`StabilizationError`].
pub fn analyse_camera_path(
    backend: &dyn MediaBackend,
    request: &StabilizationRequest,
    on_progress: &mut dyn FnMut(TrackingProgress),
    cancellation: &TrackingCancellation,
) -> Result<CameraPathReport, StabilizationError> {
    let (first_frame, last_frame) = request.frame_indices();
    let picture_size = request.picture_size;
    let shrink_factor = shrink_factor_for_tracking(picture_size.width_in_pixels as usize);
    let picture_center = Point {
        x: picture_size.width_in_pixels as f32 / 2.0,
        y: picture_size.height_in_pixels as f32 / 2.0,
    };
    let mut reader = FrameReader::open(
        backend,
        &request.video_path,
        request.frame_rate,
        shrink_factor,
    )?;
    let mut estimator = CameraMotionEstimator::new();
    let mut progress = TrackingProgress {
        analysed_frame_count: 0,
        total_frame_count: request.frame_count(),
    };
    let mut pose = CameraPose::IDENTITY;
    let mut camera_path = Vec::with_capacity(progress.total_frame_count as usize);
    let mut unmeasured_frame_count = 0;

    for frame_index in first_frame..=last_frame {
        if cancellation.is_cancelled() {
            return Err(StabilizationError::Cancelled);
        }
        let image = reader.grey_frame(frame_index)?;
        let motion = estimator
            .push_frame(image)
            .map_err(|_| StabilizationError::VideoSizeChanged)?;
        let mut motion_was_measured = false;
        if let Some(motion) = motion {
            let moved = pose.followed_by(&motion.in_full_size_image(shrink_factor), picture_center);
            // A path that has drifted very far starts again: nothing is lost,
            // since each stretch between two breaks is smoothed on its own.
            if has_drifted_too_far(&moved) {
                pose = CameraPose::IDENTITY;
            } else {
                pose = moved;
                motion_was_measured = true;
            }
        }
        if !motion_was_measured && frame_index != first_frame {
            unmeasured_frame_count += 1;
        }
        camera_path.push(CameraPathSample {
            source_time: request.frame_rate.timestamp_of_frame(frame_index),
            shift_x_in_pixels: rounded_to(pose.shift_x_in_pixels, SHIFT_ROUNDING_IN_PIXELS),
            shift_y_in_pixels: rounded_to(pose.shift_y_in_pixels, SHIFT_ROUNDING_IN_PIXELS),
            rotation_in_degrees: rounded_to(pose.rotation_in_degrees, ROTATION_ROUNDING_IN_DEGREES),
            scale_factor: rounded_to(pose.scale_factor, SCALE_ROUNDING),
            motion_was_measured,
        });
        progress.analysed_frame_count += 1;
        on_progress(progress);
    }

    Ok(CameraPathReport {
        camera_path: camera_path.into(),
        analysed_picture_size: picture_size,
        unmeasured_frame_count,
        shrink_factor,
    })
}

/// Whether `pose` is too far from the path's start to be stored precisely.
fn has_drifted_too_far(pose: &CameraPose) -> bool {
    let is_finite = [
        pose.shift_x_in_pixels,
        pose.shift_y_in_pixels,
        pose.rotation_in_degrees,
        pose.scale_factor,
    ]
    .iter()
    .all(|value| value.is_finite());
    !is_finite
        || pose.scale_factor > LARGEST_SCALE_BEFORE_RESTART
        || pose.scale_factor < 1.0 / LARGEST_SCALE_BEFORE_RESTART
        || pose.shift_x_in_pixels.abs() > LARGEST_SHIFT_BEFORE_RESTART_IN_PIXELS
        || pose.shift_y_in_pixels.abs() > LARGEST_SHIFT_BEFORE_RESTART_IN_PIXELS
}

/// `value` rounded to a multiple of `step`, as stored in the project file.
fn rounded_to(value: f64, step: f64) -> f32 {
    ((value / step).round() * step) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_values_are_rounded() {
        assert_eq!(rounded_to(12.3449, SHIFT_ROUNDING_IN_PIXELS), 12.34);
        assert_eq!(rounded_to(-0.12351, ROTATION_ROUNDING_IN_DEGREES), -0.124);
        assert_eq!(rounded_to(1.000_014, SCALE_ROUNDING), 1.000_01);
    }

    #[test]
    fn a_path_far_from_its_start_begins_again() {
        assert!(!has_drifted_too_far(&CameraPose::IDENTITY));
        let zoomed = CameraPose {
            scale_factor: 3.5,
            ..CameraPose::IDENTITY
        };
        assert!(has_drifted_too_far(&zoomed));
        let panned = CameraPose {
            shift_x_in_pixels: -60_000.0,
            ..CameraPose::IDENTITY
        };
        assert!(has_drifted_too_far(&panned));
        let broken = CameraPose {
            rotation_in_degrees: f64::NAN,
            ..CameraPose::IDENTITY
        };
        assert!(has_drifted_too_far(&broken));
    }

    #[test]
    fn the_zoom_limit_is_the_same_in_the_file_format_and_the_algorithm() {
        assert_eq!(
            Stabilization::MAXIMUM_ZOOM_FACTOR,
            kadr_vision::camera_path::MAXIMUM_ZOOM_FACTOR as f32
        );
    }
}
