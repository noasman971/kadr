//! Tracking a region through a real (encoded) video file: forwards and
//! backwards from the selection, through an obstacle, with progress,
//! cancellation and the debug video.
//!
//! The videos are synthetic scenes from `kadr-vision`, encoded to H.264, so the
//! true position of the object is known in every frame.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use image::{Rgba, RgbaImage};
use kadr_core::effect::{PixelRegion, RegionSampleOrigin, TrackedBlur, TrackingConfidence};
use kadr_core::error::EditError;
use kadr_core::ids::ClipId;
use kadr_core::media_asset::PictureSize;
use kadr_core::project::{Project, ProjectSettings};
use kadr_core::time::{FrameRate, TimeSpan, Timestamp};
use kadr_engine::quick_assembly::assemble_project_from_files;
use kadr_engine::region_tracking::{DebugVideoRequest, export_tracking_debug_video};
use kadr_engine::{
    RegionTrackingError, RegionTrackingReport, RegionTrackingRequest, TrackingCancellation,
    TrackingProgress, track_region,
};
use kadr_media::{
    EncodingSpeed, ExportSinkSettings, FfmpegBackend, MediaBackend, VideoEncoderPreference,
    VideoFrame,
};
use kadr_vision::Region;
use kadr_vision::synthetic_scenes::{SceneFrame, SceneObject};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;
const FRAME_RATE: FrameRate = FrameRate::FPS_25;
const FRAME_COUNT: u64 = 75;

/// Where the object truly is in frame `index`.
fn true_region(index: u64) -> Region {
    let time = index as f32;
    Region::from_top_left(60.0 + 2.4 * time, 80.0 + 0.6 * time, 44.0, 52.0)
}

/// Writes the scene as a 3-second H.264 video. `hidden_frames` are covered by
/// an obstacle.
fn write_scene_video(directory: &Path, name: &str, hidden_frames: std::ops::Range<u64>) -> PathBuf {
    let path = directory.join(name);
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
    let obstacle = true_region(hidden_frames.start.midpoint(hidden_frames.end)).scaled_by(1.8);
    for index in 0..FRAME_COUNT {
        let is_hidden = hidden_frames.contains(&index);
        let grey = SceneFrame {
            width: WIDTH as usize,
            height: HEIGHT as usize,
            background_seed: 7,
            background_offset_in_pixels: 0.0,
            object: Some(SceneObject {
                region: true_region(index),
                texture_seed: 8,
            }),
            obstacle: is_hidden.then_some(obstacle),
        }
        .render();
        let picture = RgbaImage::from_fn(WIDTH, HEIGHT, |x, y| {
            let value = grey.value(x as usize, y as usize).round().clamp(0.0, 255.0) as u8;
            Rgba([value, value, value, 255])
        });
        sink.write_video_frame(&VideoFrame::Rgba(picture)).unwrap();
    }
    sink.finish().unwrap();
    path
}

fn project_with(video: &Path) -> Project {
    let settings = ProjectSettings {
        canvas_size: PictureSize {
            width_in_pixels: WIDTH,
            height_in_pixels: HEIGHT,
        },
        frame_rate: FRAME_RATE,
        ..ProjectSettings::landscape_1080p()
    };
    assemble_project_from_files(
        &FfmpegBackend::new().unwrap(),
        &[video.to_path_buf()],
        settings,
        TimeSpan::from_whole_seconds(5),
    )
    .unwrap()
}

fn pixels_of(region: Region) -> PixelRegion {
    PixelRegion {
        left_in_pixels: region.left(),
        top_in_pixels: region.top(),
        width_in_pixels: region.width,
        height_in_pixels: region.height,
    }
}

fn center_error(tracked: &PixelRegion, truth: Region) -> f32 {
    let center_x = tracked.left_in_pixels + tracked.width_in_pixels / 2.0;
    let center_y = tracked.top_in_pixels + tracked.height_in_pixels / 2.0;
    (center_x - truth.center_x).hypot(center_y - truth.center_y)
}

/// Tracks from frame `selected_frame`, with the true region as the selection.
fn track_from(
    video: &Path,
    selected_frame: u64,
) -> (
    RegionTrackingRequest,
    RegionTrackingReport,
    Vec<TrackingProgress>,
) {
    let project = project_with(video);
    let clip_id = project.timeline().clip_ids()[0];
    let request = RegionTrackingRequest::for_clip(
        &project,
        clip_id,
        FRAME_RATE.timestamp_of_frame(selected_frame),
        pixels_of(true_region(selected_frame)),
    )
    .unwrap();
    let mut progress = Vec::new();
    let report = track_region(
        &FfmpegBackend::new().unwrap(),
        &request,
        &mut |step| progress.push(step),
        &TrackingCancellation::new(),
    )
    .unwrap();
    (request, report, progress)
}

#[test]
fn a_region_is_followed_forwards_and_backwards_from_the_selection() {
    let directory = tempfile::tempdir().unwrap();
    let video = write_scene_video(directory.path(), "moving.mp4", 0..0);
    let (_, report, progress) = track_from(&video, 37);

    assert_eq!(report.frames.len() as u64, FRAME_COUNT);
    let largest_error = report
        .frames
        .iter()
        .enumerate()
        .map(|(index, frame)| center_error(&frame.region, true_region(index as u64)))
        .fold(0.0, f32::max);
    eprintln!(
        "largest centre error {largest_error:.2} px, {} samples",
        report.region_samples.len()
    );
    assert!(largest_error < 3.0, "largest centre error {largest_error}");
    assert_eq!(report.confidence_counts(), (FRAME_COUNT as usize, 0, 0));
    assert!(report.unsure_ranges.is_empty());

    // The selection is stored exactly, marked as the user's.
    let selection = &report.frames[37];
    assert_eq!(selection.origin, RegionSampleOrigin::UserSelection);
    assert_eq!(selection.region, pixels_of(true_region(37)));
    assert!(
        report
            .region_samples
            .iter()
            .any(|sample| sample.origin == RegionSampleOrigin::UserSelection)
    );

    // Fewer samples than frames, and the blur still finds every frame's region.
    assert!(report.region_samples.len() < report.frames.len() / 2);
    let blur = report.tracked_blur_like(&TrackedBlur::with_default_style());
    for frame in &report.frames {
        let (region, _) = blur.region_at(frame.source_time).unwrap();
        assert!(
            center_error(
                &region,
                Region::from_top_left(
                    frame.region.left_in_pixels,
                    frame.region.top_in_pixels,
                    frame.region.width_in_pixels,
                    frame.region.height_in_pixels,
                )
            ) < 1.5
        );
    }

    // Progress went from 1 to every frame.
    assert_eq!(progress.first().unwrap().analysed_frame_count, 1);
    assert_eq!(progress.last().unwrap().analysed_frame_count, FRAME_COUNT);
    assert_eq!(progress.last().unwrap().fraction(), 1.0);
}

#[test]
fn a_hidden_stretch_is_reported_as_unsure() {
    let directory = tempfile::tempdir().unwrap();
    let video = write_scene_video(directory.path(), "hidden.mp4", 30..42);
    let (_, report, _) = track_from(&video, 5);
    let states: String = report
        .frames
        .iter()
        .map(|frame| match frame.confidence {
            TrackingConfidence::Confident => 'C',
            TrackingConfidence::Uncertain => 'u',
            TrackingConfidence::Lost => '.',
        })
        .collect();
    eprintln!("{states}");
    let [range] = report.unsure_ranges[..] else {
        panic!(
            "expected one unsure range: {:?}\n{states}",
            report.unsure_ranges
        );
    };
    assert!(
        range.first_source_time <= FRAME_RATE.timestamp_of_frame(31),
        "{states}"
    );
    assert!(
        range.last_source_time >= FRAME_RATE.timestamp_of_frame(40),
        "{states}"
    );
    assert!(
        range.last_source_time <= FRAME_RATE.timestamp_of_frame(46),
        "{states}"
    );
    assert_eq!(range.confidence, TrackingConfidence::Lost);
    let last = report.frames.last().unwrap();
    assert_eq!(last.confidence, TrackingConfidence::Confident, "{states}");
    assert!(center_error(&last.region, true_region(FRAME_COUNT - 1)) < 3.0);
}

#[test]
fn requests_say_what_is_wrong() {
    let directory = tempfile::tempdir().unwrap();
    let video = write_scene_video(directory.path(), "short.mp4", 0..0);
    let project = project_with(&video);
    let clip_id = project.timeline().clip_ids()[0];
    let selection = pixels_of(true_region(0));
    let unknown = RegionTrackingRequest::for_clip(
        &project,
        ClipId::from_raw(999),
        Timestamp::ZERO,
        selection,
    );
    assert!(matches!(
        unknown,
        Err(RegionTrackingError::Edit(EditError::ClipNotFound { .. }))
    ));
    let too_late = RegionTrackingRequest::for_clip(
        &project,
        clip_id,
        Timestamp::from_whole_seconds(10),
        selection,
    );
    assert!(matches!(
        too_late,
        Err(RegionTrackingError::TimeOutsideClip { .. })
    ));
    let message = too_late.unwrap_err().to_string();
    assert!(message.contains("not on screen"), "{message}");
}

#[test]
fn cancelling_stops_the_analysis() {
    let directory = tempfile::tempdir().unwrap();
    let video = write_scene_video(directory.path(), "cancelled.mp4", 0..0);
    let project = project_with(&video);
    let request = RegionTrackingRequest::for_clip(
        &project,
        project.timeline().clip_ids()[0],
        Timestamp::ZERO,
        pixels_of(true_region(0)),
    )
    .unwrap();
    let cancellation = TrackingCancellation::new();
    cancellation.cancel();
    let result = track_region(
        &FfmpegBackend::new().unwrap(),
        &request,
        &mut |_| {},
        &cancellation,
    );
    assert!(matches!(result, Err(RegionTrackingError::Cancelled)));
}

/// The average colour of a rectangle of a frame.
fn average_color(frame: &RgbaImage, left: u32, top: u32, width: u32, height: u32) -> [f32; 3] {
    let mut sums = [0.0; 3];
    for y in top..top + height {
        for x in left..left + width {
            for (sum, value) in sums.iter_mut().zip(frame.get_pixel(x, y).0) {
                *sum += f32::from(value);
            }
        }
    }
    sums.map(|sum| sum / (width * height) as f32)
}

#[test]
fn the_debug_video_shows_the_box_and_its_state() {
    let directory = tempfile::tempdir().unwrap();
    let video = write_scene_video(directory.path(), "debugged.mp4", 0..0);
    let (request, report, _) = track_from(&video, 37);
    let debug_path = directory.path().join("debugged.debug.mp4");
    let backend = FfmpegBackend::new().unwrap();
    export_tracking_debug_video(
        &backend,
        &request,
        &report,
        &DebugVideoRequest {
            output_path: debug_path.clone(),
            video_encoder_preference: VideoEncoderPreference::SoftwareOnly,
            encoding_speed: EncodingSpeed::Fast,
        },
        &mut |_| {},
        &TrackingCancellation::new(),
    )
    .unwrap();
    let description = backend.probe(&debug_path).unwrap();
    let frames_in_file =
        description.duration.unwrap().as_seconds() * FRAME_RATE.as_frames_per_second();
    assert!(
        (frames_in_file - FRAME_COUNT as f64).abs() <= 1.0,
        "{frames_in_file} frames"
    );
    // The label sits just above the box: blue on the selected frame, green elsewhere.
    let mut source = backend.open_video_source(&debug_path).unwrap();
    let mut label_color_at = |index: u64| {
        let frame = source
            .frame_at(FRAME_RATE.timestamp_of_frame(index))
            .unwrap()
            .to_rgba()
            .into_owned();
        let region = &report.frames[index as usize].region;
        average_color(
            &frame,
            region.left_in_pixels as u32 + 2,
            region.top_in_pixels as u32 - 9,
            24,
            6,
        )
    };
    let [red, green, blue] = label_color_at(37);
    assert!(
        blue > red + 40.0 && blue > green,
        "selection label {red} {green} {blue}"
    );
    let [red, green, blue] = label_color_at(60);
    assert!(
        green > red + 40.0 && green > blue + 40.0,
        "confident label {red} {green} {blue}"
    );
}
