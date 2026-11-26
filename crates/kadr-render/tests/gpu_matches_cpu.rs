//! The GPU renderer must draw the same pictures as the CPU reference renderer.
//!
//! Without a usable GPU these tests are skipped, unless the environment variable
//! `KADR_REQUIRE_GPU` is `1` (as in continuous integration, which uses a software
//! adapter), in which case a missing GPU is a failure.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use image::{Rgba, RgbaImage};
use kadr_core::clip::CropInsets;
use kadr_core::color::Color;
use kadr_core::effect::ColorAdjustment;
use kadr_core::media_asset::PictureSize;
use kadr_media::{VideoFrame, YuvColorSpace, YuvMatrix, YuvPicture, YuvRange};
use kadr_render::{
    CanvasDescription, CpuRenderer, FrameRenderer, GpuRenderer, LayerPlacement, VisualLayer,
};

/// The GPU renderer, or `None` (test skipped) when there is no GPU and it is not required.
fn gpu_renderer() -> Option<GpuRenderer> {
    match GpuRenderer::new() {
        Ok(renderer) => Some(renderer),
        Err(error) if std::env::var("KADR_REQUIRE_GPU").is_ok_and(|value| value == "1") => {
            panic!("a GPU is required but none works: {error}")
        }
        Err(error) => {
            eprintln!("skipped: {error}");
            None
        }
    }
}

fn canvas(width_in_pixels: u32, height_in_pixels: u32) -> CanvasDescription {
    CanvasDescription {
        size: PictureSize {
            width_in_pixels,
            height_in_pixels,
        },
        background: Color::from_rgb(20, 40, 60),
    }
}

/// A video-like picture: smooth colour gradients plus a sharp white square.
fn gradient_video_picture(width: u32, height: u32, color_space: YuvColorSpace) -> VideoFrame {
    let image = RgbaImage::from_fn(width, height, |x, y| {
        let inside_square =
            (width / 3..width / 2).contains(&x) && (height / 3..height / 2).contains(&y);
        if inside_square {
            Rgba([255, 255, 255, 255])
        } else {
            Rgba([(x * 255 / width) as u8, (y * 255 / height) as u8, 128, 255])
        }
    });
    VideoFrame::Yuv420(YuvPicture::from_rgba(&image, color_space))
}

/// A semi-transparent logo-like picture: an orange disc on a transparent background.
fn transparent_logo(size: u32) -> VideoFrame {
    let center = size as f32 / 2.0;
    VideoFrame::Rgba(RgbaImage::from_fn(size, size, |x, y| {
        let distance =
            ((x as f32 + 0.5 - center).powi(2) + (y as f32 + 0.5 - center).powi(2)).sqrt();
        if distance < center * 0.8 {
            Rgba([255, 140, 0, 220])
        } else {
            Rgba([0, 0, 0, 0])
        }
    }))
}

/// Differences between two YUV pictures, plane by plane.
struct PlaneDifference {
    mean: f64,
    share_above_ten: f64,
}

fn compare_planes(first: &[u8], second: &[u8]) -> PlaneDifference {
    assert_eq!(first.len(), second.len());
    let differences: Vec<u8> = first
        .iter()
        .zip(second)
        .map(|(a, b)| a.abs_diff(*b))
        .collect();
    let total: u64 = differences
        .iter()
        .map(|&difference| u64::from(difference))
        .sum();
    let above_ten = differences
        .iter()
        .filter(|&&difference| difference > 10)
        .count();
    PlaneDifference {
        mean: total as f64 / differences.len() as f64,
        share_above_ten: above_ten as f64 / differences.len() as f64,
    }
}

/// Renders the same scene on both renderers and checks they agree.
///
/// Small differences are expected: the GPU interpolates the colour planes of
/// YUV pictures smoothly where the CPU reuses each 2×2 block's colour, and the
/// edges of rotated layers may round differently.
fn assert_gpu_matches_cpu(
    scene_name: &str,
    canvas: &CanvasDescription,
    layers: &[VisualLayer<'_>],
) {
    let Some(mut gpu_renderer) = gpu_renderer() else {
        return;
    };
    let cpu_frame = CpuRenderer.render_frame(canvas, layers).unwrap();
    let gpu_frame = gpu_renderer.render_frame(canvas, layers).unwrap();
    let (VideoFrame::Yuv420(cpu_picture), VideoFrame::Yuv420(gpu_picture)) =
        (&cpu_frame, &gpu_frame)
    else {
        panic!("both renderers must return YUV");
    };
    assert_eq!(gpu_picture.color_space(), YuvColorSpace::BT709_LIMITED);
    for (plane_name, cpu_plane, gpu_plane) in [
        ("Y", cpu_picture.y_plane(), gpu_picture.y_plane()),
        ("U", cpu_picture.u_plane(), gpu_picture.u_plane()),
        ("V", cpu_picture.v_plane(), gpu_picture.v_plane()),
    ] {
        let difference = compare_planes(cpu_plane, gpu_plane);
        assert!(
            difference.mean < 1.5 && difference.share_above_ten < 0.02,
            "{scene_name}, plane {plane_name} ({}): mean difference {:.2}, {:.2} % of values off by more than 10",
            gpu_renderer.description(),
            difference.mean,
            difference.share_above_ten * 100.0
        );
    }
}

#[test]
fn a_scaled_video_matches() {
    let video = gradient_video_picture(320, 180, YuvColorSpace::BT709_LIMITED);
    let layers = [VisualLayer {
        picture: &video,
        placement: LayerPlacement::FITTED,
        color_adjustments: &[],
    }];
    assert_gpu_matches_cpu("scaled video", &canvas(640, 360), &layers);
}

#[test]
fn a_standard_definition_full_range_video_matches() {
    let full_range_bt601 = YuvColorSpace {
        matrix: YuvMatrix::Bt601,
        range: YuvRange::Full,
    };
    let video = gradient_video_picture(240, 180, full_range_bt601);
    let layers = [VisualLayer {
        picture: &video,
        placement: LayerPlacement::FITTED,
        color_adjustments: &[],
    }];
    assert_gpu_matches_cpu("BT.601 full range video", &canvas(320, 240), &layers);
}

#[test]
fn a_rotated_cropped_semi_transparent_stack_matches() {
    let video = gradient_video_picture(320, 240, YuvColorSpace::BT709_LIMITED);
    let logo = transparent_logo(64);
    let layers = [
        VisualLayer {
            picture: &video,
            placement: LayerPlacement {
                crop: CropInsets {
                    left_fraction: 0.1,
                    top_fraction: 0.05,
                    right_fraction: 0.2,
                    bottom_fraction: 0.0,
                },
                rotation_in_degrees: 12.0,
                scale_factor: 0.9,
                ..LayerPlacement::FITTED
            },
            color_adjustments: &[],
        },
        VisualLayer {
            picture: &logo,
            placement: LayerPlacement {
                scale_factor: 0.3,
                offset_x_in_pixels: 90.0,
                offset_y_in_pixels: -60.0,
                rotation_in_degrees: -30.0,
                opacity: 0.7,
                ..LayerPlacement::FITTED
            },
            color_adjustments: &[],
        },
    ];
    assert_gpu_matches_cpu("rotated stack", &canvas(400, 300), &layers);
}

#[test]
fn an_empty_canvas_is_the_background_colour() {
    let Some(mut gpu_renderer) = gpu_renderer() else {
        return;
    };
    let frame = gpu_renderer.render_frame(&canvas(64, 32), &[]).unwrap();
    let expected = YuvPicture::from_rgba(
        &RgbaImage::from_pixel(64, 32, Rgba([20, 40, 60, 255])),
        YuvColorSpace::BT709_LIMITED,
    );
    let VideoFrame::Yuv420(picture) = frame else {
        panic!("expected YUV");
    };
    for (rendered, reference) in [
        (picture.y_plane(), expected.y_plane()),
        (picture.u_plane(), expected.u_plane()),
        (picture.v_plane(), expected.v_plane()),
    ] {
        assert!(
            rendered
                .iter()
                .zip(reference)
                .all(|(a, b)| a.abs_diff(*b) <= 1)
        );
    }
}

#[test]
fn colour_adjusted_layers_match() {
    let video = gradient_video_picture(320, 180, YuvColorSpace::BT709_LIMITED);
    let logo = transparent_logo(64);
    let warm_and_punchy = [ColorAdjustment {
        exposure_in_stops: 0.4,
        contrast_multiplier: 1.3,
        saturation_multiplier: 1.5,
        temperature_shift: 0.4,
    }];
    let black_and_white = [
        ColorAdjustment {
            saturation_multiplier: 0.0,
            ..ColorAdjustment::NEUTRAL
        },
        ColorAdjustment {
            exposure_in_stops: -0.5,
            ..ColorAdjustment::NEUTRAL
        },
    ];
    let layers = [
        VisualLayer {
            picture: &video,
            placement: LayerPlacement::FITTED,
            color_adjustments: &warm_and_punchy,
        },
        VisualLayer {
            picture: &logo,
            placement: LayerPlacement {
                scale_factor: 0.4,
                ..LayerPlacement::FITTED
            },
            color_adjustments: &black_and_white,
        },
    ];
    assert_gpu_matches_cpu("colour adjustments", &canvas(320, 180), &layers);
}
