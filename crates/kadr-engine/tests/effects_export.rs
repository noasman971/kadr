//! Effects in exports: tracked blurs hide the right area at the right time, colour
//! adjustments and colour LUTs change the colours (in that order), and a blur
//! without tracked regions or a LUT that cannot be read is reported.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use image::{Rgba, RgbaImage};
use kadr_core::color::Color;
use kadr_core::commands::{
    AddClip, AddEffect, AddTrack, CreatedObjectsExt, ImportMedia, SetEffectEnabled, TrackPosition,
};
use kadr_core::effect::{
    BlurShape, BlurStyle, ColorAdjustment, ColorLut, EffectKind, PixelRegion, RegionSample,
    RegionSampleOrigin, TrackedBlur, TrackingConfidence,
};
use kadr_core::history::EditSession;
use kadr_core::media_asset::PictureSize;
use kadr_core::project::{Project, ProjectSettings};
use kadr_core::time::{FrameRate, TimeSpan, Timestamp};
use kadr_core::timeline::TrackKind;
use kadr_engine::preview::PreviewRenderer;
use kadr_engine::{
    ExportCancellation, ExportReport, ExportRequest, RendererPreference, export_project,
};
use kadr_media::test_patterns::{BeepPattern, TestVideoDescription, write_test_video};
use kadr_media::{
    EncodingSpeed, ExportSinkSettings, FfmpegBackend, MediaBackend, VideoEncoderPreference,
    VideoFrame,
};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;

fn small_settings() -> ProjectSettings {
    ProjectSettings {
        canvas_size: PictureSize {
            width_in_pixels: WIDTH,
            height_in_pixels: HEIGHT,
        },
        ..ProjectSettings::landscape_1080p()
    }
}

/// A 2-second barcode video (black and white bars).
fn barcode_video(directory: &Path) -> PathBuf {
    let path = directory.join("barcode.mp4");
    write_test_video(
        &FfmpegBackend::new().unwrap(),
        &TestVideoDescription {
            output_path: path.clone(),
            width_in_pixels: WIDTH,
            height_in_pixels: HEIGHT,
            frame_rate: FrameRate::FPS_30,
            duration: TimeSpan::from_whole_seconds(2),
            audio_sample_rate_in_hertz: 48_000,
            sound: BeepPattern::continuous(440.0, 0.1, TimeSpan::from_whole_seconds(2)),
        },
    )
    .unwrap();
    path
}

/// A 1-second video that is entirely pure red.
fn red_video(directory: &Path) -> PathBuf {
    let path = directory.join("red.mp4");
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
    let red = VideoFrame::Rgba(RgbaImage::from_pixel(WIDTH, HEIGHT, Rgba([255, 0, 0, 255])));
    for _ in 0..25 {
        sink.write_video_frame(&red).unwrap();
    }
    sink.finish().unwrap();
    path
}

/// A project showing `video` from 0 to `seconds`, with `effects` on the clip.
fn project_with_effects(video: &Path, seconds: u64, effects: Vec<EffectKind>) -> Project {
    let backend = FfmpegBackend::new().unwrap();
    let mut session = EditSession::new(Project::new(small_settings())).unwrap();
    let media_id = session
        .execute(ImportMedia::new(backend.probe(video).unwrap()))
        .unwrap()
        .first_media_asset_id()
        .unwrap();
    let track_id = session
        .execute(AddTrack::new(TrackKind::Video, TrackPosition::Top))
        .unwrap()
        .first_track_id()
        .unwrap();
    let seconds_timestamp = Timestamp::from_whole_seconds(seconds);
    let clip_id = session
        .execute(AddClip::media(
            track_id,
            media_id,
            Timestamp::ZERO,
            seconds_timestamp,
            Timestamp::ZERO,
        ))
        .unwrap()
        .first_clip_id()
        .unwrap();
    for effect in effects {
        session.execute(AddEffect::new(clip_id, effect)).unwrap();
    }
    session.into_project()
}

fn export(
    project: &Project,
    directory: &Path,
    renderer_preference: RendererPreference,
) -> (PathBuf, ExportReport) {
    let output_path = directory.join(format!("export-{renderer_preference:?}.mp4"));
    let report = export_project(
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
    (output_path, report)
}

fn frame_at(video: &Path, milliseconds: u64) -> VideoFrame {
    FfmpegBackend::new()
        .unwrap()
        .open_video_source(video)
        .unwrap()
        .frame_at(Timestamp::from_milliseconds(milliseconds))
        .unwrap()
        .as_ref()
        .clone()
}

fn plane_values(
    plane: &[u8],
    plane_width: u32,
    columns: std::ops::Range<u32>,
    rows: std::ops::Range<u32>,
) -> Vec<u8> {
    rows.flat_map(|row| {
        columns
            .clone()
            .map(move |column| plane[(row * plane_width + column) as usize])
    })
    .collect()
}

fn region_sample(at_second: u64, left: f32) -> RegionSample {
    RegionSample {
        source_time: Timestamp::from_whole_seconds(at_second),
        region: PixelRegion {
            left_in_pixels: left,
            top_in_pixels: 60.0,
            width_in_pixels: 80.0,
            height_in_pixels: 120.0,
        },
        confidence: TrackingConfidence::Confident,
        origin: RegionSampleOrigin::Tracker,
    }
}

/// A white rectangle moving from x = 0 (at 0 s) to x = 160 (at 2 s).
fn moving_white_box() -> EffectKind {
    EffectKind::TrackedBlur(TrackedBlur {
        style: BlurStyle::SolidFill {
            color: Color::WHITE,
        },
        shape: BlurShape::Rectangle,
        margin_in_percent: 0.0,
        edge_softness_in_pixels: 0.0,
        region_samples: vec![region_sample(0, 0.0), region_sample(2, 160.0)],
    })
}

#[test]
fn a_tracked_area_is_hidden_where_the_samples_put_it() {
    let directory = tempfile::tempdir().unwrap();
    let project = project_with_effects(
        &barcode_video(directory.path()),
        2,
        vec![moving_white_box()],
    );
    for renderer_preference in [
        RendererPreference::CpuOnly,
        RendererPreference::GpuIfAvailable,
    ] {
        let (output, report) = export(&project, directory.path(), renderer_preference);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        // At 1 s the box is halfway: from x = 80 to 160, y = 60 to 180.
        let VideoFrame::Yuv420(picture) = frame_at(&output, 1_000) else {
            panic!("decoded videos are YUV");
        };
        let inside = plane_values(picture.y_plane(), WIDTH, 90..150, 70..170);
        assert!(
            inside.iter().all(|&luma| luma > 220),
            "{}: inside the box",
            report.renderer_description
        );
        // Left of the box, the barcode's bars are still visible (dark and light).
        let left_of_box = plane_values(picture.y_plane(), WIDTH, 0..70, 70..170);
        assert!(
            left_of_box.iter().any(|&luma| luma < 40),
            "{}: bars left of the box",
            report.renderer_description
        );
    }
}

#[test]
fn zero_saturation_removes_the_colour() {
    let directory = tempfile::tempdir().unwrap();
    let red = red_video(directory.path());
    let black_and_white = EffectKind::ColorAdjustment(ColorAdjustment {
        saturation_multiplier: 0.0,
        ..ColorAdjustment::NEUTRAL
    });
    for (effects, expect_grey) in [(vec![], false), (vec![black_and_white], true)] {
        let project = project_with_effects(&red, 1, effects);
        let (output, report) = export(
            &project,
            directory.path(),
            RendererPreference::GpuIfAvailable,
        );
        let VideoFrame::Yuv420(picture) = frame_at(&output, 500) else {
            panic!("decoded videos are YUV");
        };
        let red_difference = picture.v_plane()[(60 * (WIDTH / 2) + 80) as usize];
        if expect_grey {
            assert!(
                red_difference.abs_diff(128) <= 3,
                "{}: V = {red_difference}",
                report.renderer_description
            );
        } else {
            assert!(red_difference > 220, "plain red: V = {red_difference}");
        }
    }
}

#[test]
fn a_tracked_blur_without_regions_is_reported() {
    let directory = tempfile::tempdir().unwrap();
    let project = project_with_effects(
        &barcode_video(directory.path()),
        1,
        vec![EffectKind::TrackedBlur(TrackedBlur::with_default_style())],
    );
    let (_, report) = export(&project, directory.path(), RendererPreference::CpuOnly);
    assert_eq!(report.warnings.len(), 1);
    assert!(
        report.warnings[0].contains("has no tracked region"),
        "{:?}",
        report.warnings
    );
}

/// A 2 × 2 × 2 LUT turning every colour into its opposite (red into cyan).
const INVERTING_CUBE: &str = "TITLE \"Invert\"\nLUT_3D_SIZE 2\n\
    1 1 1\n0 1 1\n1 0 1\n0 0 1\n1 1 0\n0 1 0\n1 0 0\n0 0 0\n";
/// A 2 × 2 × 2 LUT giving every colour pure green.
const ALL_GREEN_CUBE: &str = "TITLE \"All green, a longer title\"\nLUT_3D_SIZE 2\n\
    0 1 0\n0 1 0\n0 1 0\n0 1 0\n0 1 0\n0 1 0\n0 1 0\n0 1 0\n";

const BOTH_RENDERERS: [RendererPreference; 2] = [
    RendererPreference::CpuOnly,
    RendererPreference::GpuIfAvailable,
];

fn write_cube(directory: &Path, name: &str, text: &str) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

fn color_lut(file_path: &Path, strength_fraction: f32) -> EffectKind {
    EffectKind::ColorLut(ColorLut {
        file_path: file_path.to_path_buf(),
        strength_fraction,
    })
}

/// The chroma values (U, V) in the middle of the picture's top-left quarter.
fn middle_chroma(picture: &kadr_media::YuvPicture) -> (u8, u8) {
    let index = ((HEIGHT / 4) * (WIDTH / 2) + WIDTH / 4) as usize;
    (picture.u_plane()[index], picture.v_plane()[index])
}

/// The chroma values (U, V) of an exported video at 0.5 s.
fn exported_middle_chroma(output: &Path) -> (u8, u8) {
    let VideoFrame::Yuv420(picture) = frame_at(output, 500) else {
        panic!("decoded videos are YUV");
    };
    middle_chroma(&picture)
}

/// The brightness in the middle of an exported video at 0.5 s.
fn exported_middle_luma(output: &Path) -> u8 {
    let VideoFrame::Yuv420(picture) = frame_at(output, 500) else {
        panic!("decoded videos are YUV");
    };
    picture.y_plane()[((HEIGHT / 2) * WIDTH + WIDTH / 2) as usize]
}

#[test]
fn a_colour_lut_grades_a_video_the_size_of_the_canvas() {
    // The video is the canvas's size, so without colour changes it would be
    // passed through untouched: the LUT must still apply.
    let directory = tempfile::tempdir().unwrap();
    let red = red_video(directory.path());
    let invert = write_cube(directory.path(), "invert.cube", INVERTING_CUBE);
    let project = project_with_effects(&red, 1, vec![color_lut(&invert, 1.0)]);
    for renderer_preference in BOTH_RENDERERS {
        let (output, report) = export(&project, directory.path(), renderer_preference);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        // Red (V high) became cyan (V low, U above grey).
        let (u, v) = exported_middle_chroma(&output);
        assert!(
            v < 40 && u > 140,
            "{}: U = {u}, V = {v}",
            report.renderer_description
        );
    }
}

#[test]
fn half_strength_mixes_the_look_with_the_picture() {
    let directory = tempfile::tempdir().unwrap();
    let red = red_video(directory.path());
    let invert = write_cube(directory.path(), "invert.cube", INVERTING_CUBE);
    let project = project_with_effects(&red, 1, vec![color_lut(&invert, 0.5)]);
    for renderer_preference in BOTH_RENDERERS {
        let (output, report) = export(&project, directory.path(), renderer_preference);
        // Half red, half cyan: mid grey.
        let (u, v) = exported_middle_chroma(&output);
        assert!(
            u.abs_diff(128) <= 6 && v.abs_diff(128) <= 6,
            "{}: U = {u}, V = {v}",
            report.renderer_description
        );
    }
}

#[test]
fn the_lut_applies_after_the_colour_adjustments_whatever_the_list_order() {
    // The LUT comes first in the list, then an exposure that makes everything
    // black. Exposure then LUT gives white; the other way round would be black.
    let directory = tempfile::tempdir().unwrap();
    let red = red_video(directory.path());
    let invert = write_cube(directory.path(), "invert.cube", INVERTING_CUBE);
    let darkest = EffectKind::ColorAdjustment(ColorAdjustment {
        exposure_in_stops: -10.0,
        ..ColorAdjustment::NEUTRAL
    });
    let project = project_with_effects(&red, 1, vec![color_lut(&invert, 1.0), darkest]);
    for renderer_preference in BOTH_RENDERERS {
        let (output, report) = export(&project, directory.path(), renderer_preference);
        let luma = exported_middle_luma(&output);
        assert!(luma > 225, "{}: Y = {luma}", report.renderer_description);
    }
}

#[test]
fn a_disabled_lut_or_one_at_zero_strength_changes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let red = red_video(directory.path());
    let invert = write_cube(directory.path(), "invert.cube", INVERTING_CUBE);

    let at_zero = project_with_effects(&red, 1, vec![color_lut(&invert, 0.0)]);
    let mut session =
        EditSession::new(project_with_effects(&red, 1, vec![color_lut(&invert, 1.0)])).unwrap();
    let clip = session.project().timeline().all_clips().next().unwrap().1;
    let (clip_id, effect_id) = (clip.id, clip.effects[0].id);
    session
        .execute(SetEffectEnabled::new(clip_id, effect_id, false))
        .unwrap();
    let disabled = session.into_project();

    for project in [at_zero, disabled] {
        let (output, report) = export(
            &project,
            directory.path(),
            RendererPreference::GpuIfAvailable,
        );
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        let (_, v) = exported_middle_chroma(&output);
        assert!(v > 220, "{}: V = {v}", report.renderer_description);
    }
}

#[test]
fn a_missing_lut_is_reported_and_the_clip_is_drawn_without_it() {
    let directory = tempfile::tempdir().unwrap();
    let red = red_video(directory.path());
    let missing = directory.path().join("moved away.cube");
    let project = project_with_effects(&red, 1, vec![color_lut(&missing, 1.0)]);
    let clip = project.timeline().all_clips().next().unwrap().1;
    let (clip_id, effect_id) = (clip.id, clip.effects[0].id);

    let (output, report) = export(&project, directory.path(), RendererPreference::CpuOnly);

    assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
    let warning = &report.warnings[0];
    assert!(
        warning.contains(&format!(
            "the colour LUT (effect {effect_id}) of clip {clip_id} was not applied"
        )),
        "{warning}"
    );
    assert!(warning.contains("moved away.cube"), "{warning}");
    let (_, v) = exported_middle_chroma(&output);
    assert!(v > 220, "still red: V = {v}");
}

#[test]
fn a_second_lut_on_a_clip_is_reported_and_ignored() {
    let directory = tempfile::tempdir().unwrap();
    let red = red_video(directory.path());
    let invert = write_cube(directory.path(), "invert.cube", INVERTING_CUBE);
    let green = write_cube(directory.path(), "green.cube", ALL_GREEN_CUBE);
    let project = project_with_effects(
        &red,
        1,
        vec![color_lut(&invert, 1.0), color_lut(&green, 1.0)],
    );

    let (output, report) = export(&project, directory.path(), RendererPreference::CpuOnly);

    assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
    assert!(
        report.warnings[0].contains("has 2 colour LUTs; only the first was applied"),
        "{:?}",
        report.warnings
    );
    let (u, v) = exported_middle_chroma(&output);
    assert!(v < 40 && u > 140, "inverted, not green: U = {u}, V = {v}");
}

#[test]
fn the_preview_follows_a_lut_file_written_again() {
    let directory = tempfile::tempdir().unwrap();
    let backend = FfmpegBackend::new().unwrap();
    let red = red_video(directory.path());
    let look = write_cube(directory.path(), "look.cube", INVERTING_CUBE);
    let project = project_with_effects(&red, 1, vec![color_lut(&look, 1.0)]);
    let canvas = small_settings().canvas_size;
    let mut renderer = PreviewRenderer::new(RendererPreference::GpuIfAvailable).unwrap();
    let mut preview_chroma = || {
        let picture = renderer
            .render_frame(
                &project,
                &backend,
                Timestamp::from_milliseconds(500),
                canvas,
            )
            .unwrap();
        middle_chroma(&picture)
    };

    let (u, v) = preview_chroma();
    assert!(v < 40 && u > 140, "cyan: U = {u}, V = {v}");

    // Written again by a grading tool (with another length, so the change shows
    // even where the file clock is coarse): the next frame shows the new look.
    std::fs::write(&look, ALL_GREEN_CUBE).unwrap();
    let (u, v) = preview_chroma();
    assert!(u < 80 && v < 80, "green: U = {u}, V = {v}");
}
