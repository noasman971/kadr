//! Subtitles are burned into the export: on a black vertical canvas, a cue shows
//! as bright letters just above the bottom margin, nowhere else, the same way
//! with the CPU and the GPU renderer.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use kadr_core::commands::{AddSubtitleCues, AddTrack, CreatedObjectsExt, TrackPosition};
use kadr_core::history::EditSession;
use kadr_core::media_asset::PictureSize;
use kadr_core::project::{Project, ProjectSettings};
use kadr_core::subtitle::{SubtitleStyle, TimedSubtitleCue};
use kadr_core::time::Timestamp;
use kadr_core::timeline::TrackKind;
use kadr_engine::{ExportCancellation, ExportRequest, RendererPreference, export_project};
use kadr_media::{EncodingSpeed, FfmpegBackend, MediaBackend, VideoEncoderPreference, VideoFrame};

const CANVAS: PictureSize = PictureSize {
    width_in_pixels: 360,
    height_in_pixels: 640,
};

/// A black vertical project with two cues: 0–1 s and 1–2 s.
fn subtitled_project() -> Project {
    let settings = ProjectSettings {
        canvas_size: CANVAS,
        ..ProjectSettings::vertical_1080x1920()
    };
    let mut session = EditSession::new(Project::new(settings)).unwrap();
    let track_id = session
        .execute(AddTrack::new(TrackKind::Subtitle, TrackPosition::Top))
        .unwrap()
        .first_track_id()
        .unwrap();
    let cue = |start: u64, end: u64, text: &str| TimedSubtitleCue {
        start: Timestamp::from_milliseconds(start),
        end: Timestamp::from_milliseconds(end),
        text: text.to_string(),
    };
    session
        .execute(AddSubtitleCues::new(
            track_id,
            vec![
                cue(0, 1_000, "Hello Kadr"),
                cue(1_000, 2_000, "<i>Second</i> cue\non two lines"),
            ],
        ))
        .unwrap();
    session.into_project()
}

/// The rows holding bright pixels (white letters) in a frame.
fn bright_rows(frame: &VideoFrame) -> Vec<u32> {
    let VideoFrame::Yuv420(picture) = frame else {
        panic!("decoded videos are YUV");
    };
    (0..picture.height())
        .filter(|row| {
            let start = (row * picture.width()) as usize;
            picture.y_plane()[start..start + picture.width() as usize]
                .iter()
                .any(|&brightness| brightness > 200)
        })
        .collect()
}

fn export_and_read_frames(renderer_preference: RendererPreference) -> (VideoFrame, VideoFrame) {
    let directory = tempfile::tempdir().unwrap();
    let backend = FfmpegBackend::new().unwrap();
    let output_path = directory.path().join("subtitled.mp4");
    let report = export_project(
        &subtitled_project(),
        &backend,
        &ExportRequest {
            output_path: output_path.clone(),
            video_encoder_preference: VideoEncoderPreference::SoftwareOnly,
            encoding_speed: EncodingSpeed::Fast,
            renderer_preference,
        },
        &mut |_| {},
        &ExportCancellation::new(),
    )
    .unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let mut source = backend.open_video_source(&output_path).unwrap();
    let first = source.frame_at(Timestamp::from_milliseconds(500)).unwrap();
    let second = source
        .frame_at(Timestamp::from_milliseconds(1_500))
        .unwrap();
    ((*first).clone(), (*second).clone())
}

fn check_cues_sit_above_the_bottom_margin(renderer_preference: RendererPreference) {
    let (first, second) = export_and_read_frames(renderer_preference);
    let style = SubtitleStyle::for_canvas(CANVAS);
    let lines_bottom = CANVAS.height_in_pixels - style.margin_in_pixels as u32;
    let line_height = (style.font_size_in_pixels * 1.2).ceil() as u32;

    let first_rows = bright_rows(&first);
    assert!(!first_rows.is_empty(), "the first cue is drawn");
    assert!(
        first_rows
            .iter()
            .all(|row| (lines_bottom - line_height..lines_bottom).contains(row)),
        "one line just above the margin, at rows {lines_bottom} and above: {first_rows:?}"
    );
    let second_rows = bright_rows(&second);
    assert!(
        second_rows
            .iter()
            .all(|row| (lines_bottom - 2 * line_height..lines_bottom).contains(row)),
        "two lines, still above the margin: {second_rows:?}"
    );
    assert!(
        second_rows
            .iter()
            .any(|row| *row < lines_bottom - line_height),
        "the second cue has two lines: {second_rows:?}"
    );
}

#[test]
fn cues_are_burned_in_by_the_cpu_renderer() {
    check_cues_sit_above_the_bottom_margin(RendererPreference::CpuOnly);
}

#[test]
fn cues_are_burned_in_by_the_default_renderer() {
    check_cues_sit_above_the_bottom_margin(RendererPreference::GpuIfAvailable);
}

#[test]
fn both_renderers_burn_in_the_same_subtitles() {
    let (cpu_first, _) = export_and_read_frames(RendererPreference::CpuOnly);
    let (gpu_first, _) = export_and_read_frames(RendererPreference::GpuIfAvailable);
    let (VideoFrame::Yuv420(cpu), VideoFrame::Yuv420(gpu)) = (&cpu_first, &gpu_first) else {
        panic!("decoded videos are YUV");
    };
    let differences: Vec<u8> = cpu
        .y_plane()
        .iter()
        .zip(gpu.y_plane())
        .map(|(cpu_value, gpu_value)| cpu_value.abs_diff(*gpu_value))
        .collect();
    let mean = differences
        .iter()
        .map(|&value| f64::from(value))
        .sum::<f64>()
        / differences.len() as f64;
    assert!(mean < 1.0, "mean brightness difference {mean:.3}");
}
