//! Transitions and text animations in exports: a dissolve mixes the two clips, a
//! slide pushes one out while the other comes in, a fade starts from the
//! background, and a typed-on title shows its first letters first.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use image::{Rgba, RgbaImage};
use kadr_core::commands::{
    AddClip, AddTrack, ClipEdge, CreatedObjectsExt, ImportMedia, SetClipTransition,
    SetTextAnimation, TrackPosition,
};
use kadr_core::history::EditSession;
use kadr_core::ids::{ClipId, TrackId};
use kadr_core::media_asset::PictureSize;
use kadr_core::project::{Project, ProjectSettings};
use kadr_core::text_animation::{TextAnimation, TextAnimationStyle};
use kadr_core::time::{FrameRate, TimeSpan, Timestamp};
use kadr_core::timeline::TrackKind;
use kadr_core::transition::{SlideDirection, Transition, TransitionStyle};
use kadr_engine::{ExportCancellation, ExportRequest, RendererPreference, export_project};
use kadr_media::{
    EncodingSpeed, ExportSinkSettings, FfmpegBackend, MediaBackend, VideoEncoderPreference,
    VideoFrame,
};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;
const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];
/// Colour tolerance after the H.264 round trip.
const COLOUR_TOLERANCE: u8 = 40;

/// A one-second video entirely of one colour.
fn solid_colour_video(directory: &Path, name: &str, color: [u8; 3]) -> PathBuf {
    let path = directory.join(name);
    let backend = FfmpegBackend::new().unwrap();
    let mut sink = backend
        .create_export_sink(&ExportSinkSettings {
            output_path: path.clone(),
            width_in_pixels: WIDTH,
            height_in_pixels: HEIGHT,
            frame_rate: FrameRate::FPS_25,
            audio_sample_rate_in_hertz: 48_000,
            video_encoder_preference: VideoEncoderPreference::SoftwareOnly,
            encoding_speed: EncodingSpeed::Fast,
        })
        .unwrap();
    let [red, green, blue] = color;
    let picture = VideoFrame::Rgba(RgbaImage::from_pixel(
        WIDTH,
        HEIGHT,
        Rgba([red, green, blue, 255]),
    ));
    for _ in 0..25 {
        sink.write_video_frame(&picture).unwrap();
    }
    sink.finish().unwrap();
    path
}

/// A small black project being edited, with one video track.
struct EditedProject {
    session: EditSession,
    track_id: TrackId,
}

impl EditedProject {
    fn new() -> Self {
        let settings = ProjectSettings {
            canvas_size: PictureSize {
                width_in_pixels: WIDTH,
                height_in_pixels: HEIGHT,
            },
            ..ProjectSettings::landscape_1080p()
        };
        let mut session = EditSession::new(Project::new(settings)).unwrap();
        let track_id = session
            .execute(AddTrack::new(TrackKind::Video, TrackPosition::Top))
            .unwrap()
            .first_track_id()
            .unwrap();
        Self { session, track_id }
    }

    /// Places the whole of a one-second `video` at `start_second`.
    fn add_video(&mut self, video: &Path, start_second: u64) -> ClipId {
        let backend = FfmpegBackend::new().unwrap();
        let media_id = self
            .session
            .execute(ImportMedia::new(backend.probe(video).unwrap()))
            .unwrap()
            .first_media_asset_id()
            .unwrap();
        self.session
            .execute(AddClip::media(
                self.track_id,
                media_id,
                Timestamp::ZERO,
                Timestamp::from_whole_seconds(1),
                Timestamp::from_whole_seconds(start_second),
            ))
            .unwrap()
            .first_clip_id()
            .unwrap()
    }

    fn set_entry_transition(&mut self, clip_id: ClipId, style: TransitionStyle, milliseconds: u64) {
        self.session
            .execute(SetClipTransition::new(
                clip_id,
                ClipEdge::Start,
                Some(Transition {
                    style,
                    duration: TimeSpan::from_milliseconds(milliseconds),
                }),
            ))
            .unwrap();
    }

    /// Exports the project and returns the colour of the pixels at the given
    /// columns (middle row) at each of the given moments.
    fn export_and_read(
        &self,
        directory: &Path,
        renderer_preference: RendererPreference,
        moments_in_milliseconds: &[u64],
        columns: &[u32],
    ) -> Vec<Vec<[u8; 3]>> {
        let output = export(self.session.project(), directory, renderer_preference);
        let backend = FfmpegBackend::new().unwrap();
        let mut source = backend.open_video_source(&output).unwrap();
        moments_in_milliseconds
            .iter()
            .map(|&milliseconds| {
                let frame = source
                    .frame_at(Timestamp::from_milliseconds(milliseconds))
                    .unwrap();
                let picture = frame.to_rgba();
                columns
                    .iter()
                    .map(|&column| {
                        let pixel = picture.get_pixel(column, HEIGHT / 2);
                        [pixel[0], pixel[1], pixel[2]]
                    })
                    .collect()
            })
            .collect()
    }
}

fn export(project: &Project, directory: &Path, renderer_preference: RendererPreference) -> PathBuf {
    let output_path = directory.join(format!("export-{renderer_preference:?}.mp4"));
    export_project(
        project,
        &FfmpegBackend::new().unwrap(),
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
    output_path
}

fn assert_colour_close(found: [u8; 3], expected: [u8; 3], context: &str) {
    let is_close = found
        .iter()
        .zip(expected)
        .all(|(found, expected)| found.abs_diff(expected) <= COLOUR_TOLERANCE);
    assert!(
        is_close,
        "{context}: {found:?} instead of about {expected:?}"
    );
}

#[test]
fn a_fade_between_two_clips_dissolves_one_into_the_other() {
    let directory = tempfile::tempdir().unwrap();
    let mut edited = EditedProject::new();
    edited.add_video(&solid_colour_video(directory.path(), "red.mp4", RED), 0);
    let blue_clip_id = edited.add_video(&solid_colour_video(directory.path(), "blue.mp4", BLUE), 1);
    edited.set_entry_transition(blue_clip_id, TransitionStyle::Fade, 600);

    for renderer_preference in [
        RendererPreference::CpuOnly,
        RendererPreference::GpuIfAvailable,
    ] {
        let colours = edited.export_and_read(
            directory.path(),
            renderer_preference,
            &[900, 1_300, 1_700],
            &[WIDTH / 2],
        );
        // The red file ends at 1 s: its last frame is held under the dissolve.
        assert_colour_close(colours[0][0], RED, "before the cut");
        assert_colour_close(colours[1][0], [128, 0, 128], "halfway through the dissolve");
        assert_colour_close(colours[2][0], BLUE, "after the dissolve");
    }
}

#[test]
fn a_slide_pushes_the_previous_clip_out() {
    let directory = tempfile::tempdir().unwrap();
    let mut edited = EditedProject::new();
    edited.add_video(&solid_colour_video(directory.path(), "red.mp4", RED), 0);
    let blue_clip_id = edited.add_video(&solid_colour_video(directory.path(), "blue.mp4", BLUE), 1);
    let slide_left = TransitionStyle::Slide {
        direction: SlideDirection::Left,
    };
    edited.set_entry_transition(blue_clip_id, slide_left, 1_000);

    let colours = edited.export_and_read(
        directory.path(),
        RendererPreference::GpuIfAvailable,
        &[1_500],
        &[WIDTH / 4, WIDTH * 3 / 4],
    );
    // Halfway: the red clip fills the left half, the blue one the right half.
    assert_colour_close(colours[0][0], RED, "left half");
    assert_colour_close(colours[0][1], BLUE, "right half");
}

#[test]
fn a_fade_with_nothing_before_starts_from_the_background() {
    let directory = tempfile::tempdir().unwrap();
    let mut edited = EditedProject::new();
    let red_clip_id = edited.add_video(&solid_colour_video(directory.path(), "red.mp4", RED), 0);
    edited.set_entry_transition(red_clip_id, TransitionStyle::Fade, 800);

    let colours = edited.export_and_read(
        directory.path(),
        RendererPreference::GpuIfAvailable,
        &[0, 400, 900],
        &[WIDTH / 2],
    );
    assert_colour_close(colours[0][0], [0, 0, 0], "first frame");
    assert_colour_close(colours[1][0], [128, 0, 0], "halfway");
    assert_colour_close(colours[2][0], RED, "after the fade");
}

#[test]
fn a_typed_on_title_shows_its_first_letters_first() {
    let directory = tempfile::tempdir().unwrap();
    let mut edited = EditedProject::new();
    let title_clip_id = edited
        .session
        .execute(AddClip::text(
            edited.track_id,
            "KADR",
            TimeSpan::from_whole_seconds(2),
            Timestamp::ZERO,
        ))
        .unwrap()
        .first_clip_id()
        .unwrap();
    edited
        .session
        .execute(SetTextAnimation::new(
            title_clip_id,
            ClipEdge::Start,
            Some(TextAnimation {
                style: TextAnimationStyle::TypeOn,
                duration: TimeSpan::from_whole_seconds(1),
            }),
        ))
        .unwrap();
    let output = export(
        edited.session.project(),
        directory.path(),
        RendererPreference::GpuIfAvailable,
    );

    let backend = FfmpegBackend::new().unwrap();
    let mut source = backend.open_video_source(&output).unwrap();
    let mut bright_columns_at = |milliseconds| {
        let frame = source
            .frame_at(Timestamp::from_milliseconds(milliseconds))
            .unwrap();
        let VideoFrame::Yuv420(picture) = frame.as_ref() else {
            panic!("decoded videos are YUV");
        };
        let is_bright_column = |column: u32| {
            (0..HEIGHT).any(|row| picture.y_plane()[(row * WIDTH + column) as usize] > 128)
        };
        let bright: Vec<u32> = (0..WIDTH)
            .filter(|&column| is_bright_column(column))
            .collect();
        bright.first().copied().zip(bright.last().copied())
    };
    let (whole_left, whole_right) = bright_columns_at(1_500).expect("the whole title");
    // Halfway through typing "KADR": "KA" only, starting where the title starts.
    let (half_left, half_right) = bright_columns_at(500).expect("the first letters");
    assert!(
        half_left.abs_diff(whole_left) <= 2,
        "{half_left} vs {whole_left}"
    );
    let whole_width = whole_right - whole_left;
    assert!(
        half_right < whole_left + whole_width * 6 / 10,
        "{half_right}: more than half the title is shown ({whole_left}–{whole_right})"
    );
    assert!(
        bright_columns_at(0).is_none(),
        "nothing is typed yet at the start"
    );
}
