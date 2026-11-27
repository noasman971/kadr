//! Stabilisation on a real encoded video: a synthetic scene filmed by a camera
//! that pans and shakes in a known way is written as H.264, then analysed. The
//! measured camera path must follow the true one.

// Integration tests use helper functions outside `#[test]` functions, which
// clippy's `allow-unwrap-in-tests` does not cover.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use image::{Rgba, RgbaImage};
use kadr_core::commands::{
    AddClip, AddEffect, AddTrack, CreatedObjectsExt, ImportMedia, SplitClip, TrackPosition,
};
use kadr_core::effect::{EffectKind, Stabilization};
use kadr_core::error::EditError;
use kadr_core::history::EditSession;
use kadr_core::ids::ClipId;
use kadr_core::media_asset::{MediaAssetDescription, MediaKind, PictureSize};
use kadr_core::project::{Project, ProjectSettings};
use kadr_core::time::{FrameRate, TimeSpan, Timestamp};
use kadr_core::timeline::TrackKind;
use kadr_engine::quick_assembly::assemble_project_from_files;
use kadr_engine::{
    StabilizationError, StabilizationRequest, TrackingCancellation, analyse_camera_path,
    stabilization_summary,
};
use kadr_media::{
    EncodingSpeed, ExportSinkSettings, FfmpegBackend, MediaBackend, VideoEncoderPreference,
    VideoFrame,
};
use kadr_vision::synthetic_scenes::ShakenCameraFrame;
use kadr_vision::{Point, SimilarityTransform};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 360;
const FRAME_RATE: FrameRate = FrameRate::FPS_30;
/// Four seconds.
const FRAME_COUNT: u64 = 120;

/// Where the camera is at each frame: a steady pan to the right with a shake
/// in both directions and a little roll. Frame 0 is the reference (no motion).
fn camera_at(frame_index: u64) -> SimilarityTransform {
    let frame = frame_index as f32;
    SimilarityTransform {
        center: Point {
            x: WIDTH as f32 / 2.0,
            y: HEIGHT as f32 / 2.0,
        },
        shift_x_in_pixels: 1.5 * frame + 3.0 * (0.9 * frame).sin(),
        shift_y_in_pixels: 2.0 * (1.3 * frame).sin(),
        rotation_in_degrees: 0.3 * (0.7 * frame).sin(),
        scale_factor: 1.0,
    }
}

/// The shaky video, encoded once for all the tests of this file.
fn shaky_video() -> &'static Path {
    static VIDEO: OnceLock<(tempfile::TempDir, PathBuf)> = OnceLock::new();
    let (_, path) = VIDEO.get_or_init(|| {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shaky.mp4");
        let backend = FfmpegBackend::new().unwrap();
        let mut sink = backend
            .create_export_sink(&ExportSinkSettings {
                output_path: path.clone(),
                width_in_pixels: WIDTH,
                height_in_pixels: HEIGHT,
                frame_rate: FRAME_RATE,
                audio_sample_rate_in_hertz: 48_000,
                video_encoder_preference: VideoEncoderPreference::SoftwareOnly,
                encoding_speed: EncodingSpeed::Balanced,
            })
            .unwrap();
        for frame_index in 0..FRAME_COUNT {
            let grey = ShakenCameraFrame {
                width: WIDTH as usize,
                height: HEIGHT as usize,
                background_seed: 7,
                camera: camera_at(frame_index),
                moving_object: None,
                noise_amplitude: 0.0,
                noise_seed: 0,
            }
            .render();
            let picture = RgbaImage::from_fn(WIDTH, HEIGHT, |x, y| {
                let value = grey.value(x as usize, y as usize).round().clamp(0.0, 255.0) as u8;
                Rgba([value, value, value, 255])
            });
            sink.write_video_frame(&VideoFrame::Rgba(picture)).unwrap();
        }
        sink.finish().unwrap();
        (directory, path)
    });
    path
}

fn project_settings() -> ProjectSettings {
    ProjectSettings {
        canvas_size: PictureSize {
            width_in_pixels: WIDTH,
            height_in_pixels: HEIGHT,
        },
        frame_rate: FRAME_RATE,
        ..ProjectSettings::landscape_1080p()
    }
}

/// A project showing the video `file` from the start of the timeline.
fn project_with(file: &Path) -> Project {
    assemble_project_from_files(
        &FfmpegBackend::new().unwrap(),
        &[file.to_path_buf()],
        project_settings(),
        TimeSpan::from_whole_seconds(2),
    )
    .unwrap()
}

#[test]
fn the_camera_path_follows_a_known_shake() {
    let project = project_with(shaky_video());
    let clip_id = project.timeline().clip_ids()[0];
    let request = StabilizationRequest::for_clip(&project, clip_id).unwrap();
    assert_eq!(request.frame_count(), FRAME_COUNT);

    let mut last_progress = None;
    let report = analyse_camera_path(
        &FfmpegBackend::new().unwrap(),
        &request,
        &mut |progress| last_progress = Some(progress),
        &TrackingCancellation::new(),
    )
    .unwrap();

    assert_eq!(last_progress.unwrap().fraction(), 1.0);
    let path = &report.camera_path;
    assert_eq!(path.len() as u64, FRAME_COUNT);
    assert_eq!(report.unmeasured_frame_count, 0);
    assert_eq!(path[0].source_time, Timestamp::ZERO);
    assert!(!path[0].motion_was_measured);
    for frame_index in 1..FRAME_COUNT {
        let (previous, current) = (&path[frame_index as usize - 1], &path[frame_index as usize]);
        let (true_previous, true_current) = (camera_at(frame_index - 1), camera_at(frame_index));
        assert_eq!(
            current.source_time,
            FRAME_RATE.timestamp_of_frame(frame_index)
        );
        assert!(current.motion_was_measured, "frame {frame_index}");
        let step_error_x = (current.shift_x_in_pixels - previous.shift_x_in_pixels)
            - (true_current.shift_x_in_pixels - true_previous.shift_x_in_pixels);
        let step_error_y = (current.shift_y_in_pixels - previous.shift_y_in_pixels)
            - (true_current.shift_y_in_pixels - true_previous.shift_y_in_pixels);
        assert!(
            step_error_x.hypot(step_error_y) <= 0.3,
            "frame {frame_index}: the step is off by ({step_error_x}, {step_error_y}) pixels"
        );
        let roll_error = (current.rotation_in_degrees - previous.rotation_in_degrees)
            - (true_current.rotation_in_degrees - true_previous.rotation_in_degrees);
        assert!(
            roll_error.abs() <= 0.05,
            "frame {frame_index}: the roll step is off by {roll_error}°"
        );
    }
    // Four seconds of steps add up to almost no drift.
    let (last, truth) = (&path[path.len() - 1], camera_at(FRAME_COUNT - 1));
    assert!(
        (last.shift_x_in_pixels - truth.shift_x_in_pixels).abs() < 3.0,
        "{last:?}"
    );
    assert!(
        (last.shift_y_in_pixels - truth.shift_y_in_pixels).abs() < 3.0,
        "{last:?}"
    );
    assert!((last.scale_factor - 1.0).abs() < 0.01, "{last:?}");

    // The effect made from the report is accepted by the project's rules, and a
    // few pixels of shake need only a small enlargement.
    let stabilization = report.stabilization_with(Stabilization::DEFAULT_SMOOTHING_IN_SECONDS);
    let summary = stabilization_summary(&stabilization);
    assert!(
        summary.zoom_factor > 1.005 && summary.zoom_factor < 1.05,
        "{summary:?}"
    );
    assert_eq!(summary.partly_corrected_frame_count, 0);
    assert!(summary.largest_shift_in_pixels > 2.0 && summary.largest_shift_in_pixels < 12.0);
    let mut session = EditSession::new(project).unwrap();
    session
        .execute(AddEffect::new(
            clip_id,
            EffectKind::Stabilization(stabilization),
        ))
        .unwrap();
}

#[test]
fn the_analysis_covers_the_clip_and_a_second_around_it() {
    let mut session = EditSession::new(project_with(shaky_video())).unwrap();
    let first_clip_id = session.project().timeline().clip_ids()[0];
    let middle_clip_id = session
        .execute(SplitClip::new(
            first_clip_id,
            Timestamp::from_milliseconds(1_500),
        ))
        .unwrap()
        .first_clip_id()
        .unwrap();
    let last_clip_id = session
        .execute(SplitClip::new(
            middle_clip_id,
            Timestamp::from_milliseconds(2_500),
        ))
        .unwrap()
        .first_clip_id()
        .unwrap();
    let project = session.project();

    // The middle clip shows 1.5 s to 2.5 s of the file.
    let middle = StabilizationRequest::for_clip(project, middle_clip_id).unwrap();
    assert_eq!(middle.source_start, Timestamp::from_milliseconds(500));
    assert_eq!(middle.source_end, Timestamp::from_milliseconds(3_500));
    assert_eq!(middle.frame_count(), 90);
    // Nothing is analysed before the start of the file…
    let first = StabilizationRequest::for_clip(project, first_clip_id).unwrap();
    assert_eq!(first.source_start, Timestamp::ZERO);
    assert_eq!(first.source_end, Timestamp::from_milliseconds(2_500));
    // … or after its end.
    let last = StabilizationRequest::for_clip(project, last_clip_id).unwrap();
    assert_eq!(last.source_start, Timestamp::from_milliseconds(1_500));
    assert!(
        last.source_end < Timestamp::from_milliseconds(4_200),
        "{last:?}"
    );
    assert!(last.frame_count() <= FRAME_COUNT - 45, "{last:?}");
}

#[test]
fn cancelling_stops_the_analysis() {
    let project = project_with(shaky_video());
    let request =
        StabilizationRequest::for_clip(&project, project.timeline().clip_ids()[0]).unwrap();
    let cancellation = TrackingCancellation::new();
    cancellation.cancel();
    let result = analyse_camera_path(
        &FfmpegBackend::new().unwrap(),
        &request,
        &mut |_| {},
        &cancellation,
    );
    assert!(matches!(result, Err(StabilizationError::Cancelled)));
}

#[test]
fn images_and_titles_are_refused() {
    let mut session = EditSession::new(Project::new(project_settings())).unwrap();
    let track_id = session
        .execute(AddTrack::new(TrackKind::Video, TrackPosition::Top))
        .unwrap()
        .first_track_id()
        .unwrap();

    // A still image: nothing moves in it. The rules do not look at the disk, so
    // the file does not have to exist.
    let image_asset_id = session
        .execute(ImportMedia::new(MediaAssetDescription {
            file_path: PathBuf::from("images/logo.png"),
            display_name: "logo.png".to_string(),
            kind: MediaKind::Image,
            duration: None,
            picture_size: Some(PictureSize {
                width_in_pixels: WIDTH,
                height_in_pixels: HEIGHT,
            }),
            frame_rate: None,
            audio_format: None,
        }))
        .unwrap()
        .first_media_asset_id()
        .unwrap();
    let image_clip_id = session
        .execute(AddClip::media(
            track_id,
            image_asset_id,
            Timestamp::ZERO,
            Timestamp::from_whole_seconds(2),
            Timestamp::from_whole_seconds(5),
        ))
        .unwrap()
        .first_clip_id()
        .unwrap();
    let refused = StabilizationRequest::for_clip(session.project(), image_clip_id);
    assert!(matches!(
        refused,
        Err(StabilizationError::NotAVideoClip { clip_id }) if clip_id == image_clip_id
    ));
    let message = refused.unwrap_err().to_string();
    assert!(message.contains("does not show a video"), "{message}");

    // A title.
    let title_clip_id = session
        .execute(AddClip::text(
            track_id,
            "KADR",
            TimeSpan::from_whole_seconds(1),
            Timestamp::ZERO,
        ))
        .unwrap()
        .first_clip_id()
        .unwrap();
    assert!(matches!(
        StabilizationRequest::for_clip(session.project(), title_clip_id),
        Err(StabilizationError::NotAVideoClip { .. })
    ));

    // A clip that does not exist.
    assert!(matches!(
        StabilizationRequest::for_clip(session.project(), ClipId::from_raw(999)),
        Err(StabilizationError::Edit(EditError::ClipNotFound { .. }))
    ));
}
