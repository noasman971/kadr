//! Text clips are drawn into the export: white words on a black canvas must show
//! up as bright pixels in the middle and nowhere else.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use kadr_core::commands::{AddClip, AddTrack, CreatedObjectsExt, TrackPosition};
use kadr_core::history::EditSession;
use kadr_core::media_asset::PictureSize;
use kadr_core::project::{Project, ProjectSettings};
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_core::timeline::TrackKind;
use kadr_engine::{ExportCancellation, ExportRequest, RendererPreference, export_project};
use kadr_media::{EncodingSpeed, FfmpegBackend, MediaBackend, VideoEncoderPreference, VideoFrame};

const CANVAS_WIDTH: u32 = 320;
const CANVAS_HEIGHT: u32 = 240;

/// A black 320×240 project showing "KADR" in the default style for one second.
fn title_project() -> Project {
    let settings = ProjectSettings {
        canvas_size: PictureSize {
            width_in_pixels: CANVAS_WIDTH,
            height_in_pixels: CANVAS_HEIGHT,
        },
        ..ProjectSettings::landscape_1080p()
    };
    let mut session = EditSession::new(Project::new(settings)).unwrap();
    let track_id = session
        .execute(AddTrack::new(TrackKind::Video, TrackPosition::Top))
        .unwrap()
        .first_track_id()
        .unwrap();
    session
        .execute(AddClip::text(
            track_id,
            "KADR",
            TimeSpan::from_whole_seconds(1),
            Timestamp::ZERO,
        ))
        .unwrap();
    session.into_project()
}

/// The brightness (Y) values of a region of the picture.
fn brightness_values(
    frame: &VideoFrame,
    columns: std::ops::Range<u32>,
    rows: std::ops::Range<u32>,
) -> Vec<u8> {
    let VideoFrame::Yuv420(picture) = frame else {
        panic!("decoded videos are YUV");
    };
    rows.flat_map(|row| columns.clone().map(move |column| (row, column)))
        .map(|(row, column)| picture.y_plane()[(row * picture.width() + column) as usize])
        .collect()
}

fn export_title(renderer_preference: RendererPreference) {
    let directory = tempfile::tempdir().unwrap();
    let backend = FfmpegBackend::new().unwrap();
    let output_path = directory.path().join("title.mp4");
    let report = export_project(
        &title_project(),
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

    let frame = backend
        .open_video_source(&output_path)
        .unwrap()
        .frame_at(Timestamp::from_milliseconds(500))
        .unwrap();
    // The 64-pixel title is centred: its strokes are near-white (Y close to 235).
    let middle = brightness_values(&frame, 100..220, 95..145);
    let bright_pixel_count = middle
        .iter()
        .filter(|&&brightness| brightness > 200)
        .count();
    assert!(
        bright_pixel_count > 400,
        "{} renderer: only {bright_pixel_count} bright pixels",
        report.renderer_description
    );
    // Corners stay black (Y close to 16).
    for corner in [
        brightness_values(&frame, 0..30, 0..30),
        brightness_values(&frame, 290..320, 210..240),
    ] {
        assert!(
            corner.iter().all(|&brightness| brightness < 30),
            "{} renderer",
            report.renderer_description
        );
    }
}

#[test]
fn a_title_is_drawn_by_the_cpu_renderer() {
    export_title(RendererPreference::CpuOnly);
}

#[test]
fn a_title_is_drawn_by_the_default_renderer() {
    export_title(RendererPreference::GpuIfAvailable);
}
