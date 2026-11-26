//! Preview frames show the exact frame the export would, after edits and jumps,
//! at the requested size; thumbnails and waveforms describe the right files.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use kadr_core::commands::MoveClip;
use kadr_core::history::EditSession;
use kadr_core::media_asset::PictureSize;
use kadr_core::project::{Project, ProjectSettings};
use kadr_core::time::{FrameRate, TimeSpan, Timestamp};
use kadr_engine::RendererPreference;
use kadr_engine::media_overview::{ThumbnailMaker, compute_waveform_peaks};
use kadr_engine::preview::PreviewRenderer;
use kadr_engine::quick_assembly::assemble_project_from_files;
use kadr_media::test_patterns::{
    BeepPattern, TestVideoDescription, read_frame_index_barcode, write_test_video,
};
use kadr_media::{FfmpegBackend, MediaBackend, VideoFrame};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;

fn size(width_in_pixels: u32, height_in_pixels: u32) -> PictureSize {
    PictureSize {
        width_in_pixels,
        height_in_pixels,
    }
}

/// A 2-second, 30 fps barcode video with a beep at every whole second.
fn barcode_video(directory: &Path) -> PathBuf {
    let path = directory.join("barcode.mp4");
    let duration = TimeSpan::from_whole_seconds(2);
    write_test_video(
        &FfmpegBackend::new().unwrap(),
        &TestVideoDescription {
            output_path: path.clone(),
            width_in_pixels: WIDTH,
            height_in_pixels: HEIGHT,
            frame_rate: FrameRate::FPS_30,
            duration,
            audio_sample_rate_in_hertz: 48_000,
            sound: BeepPattern::every_second(
                1_000.0,
                0.5,
                duration,
                TimeSpan::from_milliseconds(200),
            ),
        },
    )
    .unwrap();
    path
}

fn project_with_canvas(backend: &FfmpegBackend, video: &Path, canvas: PictureSize) -> Project {
    assemble_project_from_files(
        backend,
        &[video.to_path_buf()],
        ProjectSettings {
            canvas_size: canvas,
            ..ProjectSettings::landscape_1080p()
        },
        TimeSpan::from_whole_seconds(3),
    )
    .unwrap()
}

fn frame_time(frame_index: u64) -> Timestamp {
    FrameRate::FPS_30.timestamp_of_frame(frame_index)
}

fn frame_number_shown(
    renderer: &mut PreviewRenderer,
    project: &Project,
    backend: &FfmpegBackend,
    time: Timestamp,
) -> u32 {
    let picture = renderer
        .render_frame(project, backend, time, size(WIDTH, HEIGHT))
        .unwrap();
    read_frame_index_barcode(&VideoFrame::Yuv420(picture))
}

#[test]
fn preview_shows_the_exact_frame_forwards_backwards_and_after_an_edit() {
    let directory = tempfile::tempdir().unwrap();
    let backend = FfmpegBackend::new().unwrap();
    let video = barcode_video(directory.path());
    let project = project_with_canvas(&backend, &video, size(WIDTH, HEIGHT));
    let mut renderer = PreviewRenderer::new(RendererPreference::CpuOnly).unwrap();

    for frame_index in [0, 1, 2, 45, 10, 59, 3] {
        let shown = frame_number_shown(&mut renderer, &project, &backend, frame_time(frame_index));
        assert_eq!(u64::from(shown), frame_index);
    }

    // Moving the clip one second later: the same timeline time shows an earlier frame.
    let mut session = EditSession::new(project).unwrap();
    let clip_id = session.project().timeline().clip_ids()[0];
    session
        .execute(MoveClip::along_track(
            clip_id,
            Timestamp::from_whole_seconds(1),
        ))
        .unwrap();
    let shown = frame_number_shown(&mut renderer, session.project(), &backend, frame_time(45));
    assert_eq!(shown, 15);
}

#[test]
fn preview_frames_fit_the_requested_size() {
    let directory = tempfile::tempdir().unwrap();
    let backend = FfmpegBackend::new().unwrap();
    let video = barcode_video(directory.path());
    // A canvas twice the video size: the picture is drawn, not passed through.
    let project = project_with_canvas(&backend, &video, size(WIDTH * 2, HEIGHT * 2));
    let mut renderer = PreviewRenderer::new(RendererPreference::CpuOnly).unwrap();

    let picture = renderer
        .render_frame(&project, &backend, frame_time(37), size(WIDTH, 1_000))
        .unwrap();
    assert_eq!((picture.width(), picture.height()), (WIDTH, HEIGHT));
    assert_eq!(read_frame_index_barcode(&VideoFrame::Yuv420(picture)), 37);

    let small = renderer
        .render_frame(&project, &backend, frame_time(37), size(100, 100))
        .unwrap();
    assert_eq!((small.width(), small.height()), (100, 74));
}

#[test]
fn thumbnails_fit_their_size_and_follow_the_requested_time() {
    let directory = tempfile::tempdir().unwrap();
    let backend = FfmpegBackend::new().unwrap();
    let video = barcode_video(directory.path());
    let mut thumbnails = ThumbnailMaker::new();

    let thumbnail = thumbnails
        .thumbnail_at(&backend, &video, frame_time(5), size(160, 160))
        .unwrap();
    assert_eq!(thumbnail.dimensions(), (160, 120));

    // Full size so the barcode stays readable; out of order on purpose.
    for frame_index in [50, 12] {
        let picture = thumbnails
            .thumbnail_at(
                &backend,
                &video,
                frame_time(frame_index),
                size(WIDTH, HEIGHT),
            )
            .unwrap();
        assert_eq!(
            u64::from(read_frame_index_barcode(&VideoFrame::Rgba(picture))),
            frame_index
        );
    }
}

#[test]
fn waveform_peaks_follow_the_beeps() {
    let directory = tempfile::tempdir().unwrap();
    let backend = FfmpegBackend::new().unwrap();
    let video = barcode_video(directory.path());
    let description = backend.probe(&video).unwrap();

    let waveform = compute_waveform_peaks(&backend, &description, 100).unwrap();

    assert_eq!(waveform.peaks_per_second, 100);
    assert!(
        (200..=202).contains(&waveform.peaks.len()),
        "{}",
        waveform.peaks.len()
    );
    // Beeps of amplitude 0.5 (≈ 128) during the first 200 ms of each second.
    let loud = |second: usize, hundredth: usize| waveform.peaks[second * 100 + hundredth];
    for second in 0..2 {
        assert!((110..=145).contains(&loud(second, 10)), "{waveform:?}");
        assert!(loud(second, 50) < 10, "{waveform:?}");
    }
}

#[test]
fn files_without_sound_have_no_waveform() {
    let directory = tempfile::tempdir().unwrap();
    let backend = FfmpegBackend::new().unwrap();
    let video = barcode_video(directory.path());
    let mut description = backend.probe(&video).unwrap();
    description.audio_format = None;

    assert!(compute_waveform_peaks(&backend, &description, 100).is_err());
}
