//! The GPU renderer must draw the same pictures as the CPU reference renderer.
//!
//! Without a usable GPU these tests are skipped, unless the environment variable
//! `KADR_REQUIRE_GPU` is `1` (as in continuous integration, which uses a software
//! adapter), in which case a missing GPU is a failure.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use image::{Rgba, RgbaImage};
use kadr_core::clip::{CropInsets, TextStyle};
use kadr_core::color::Color;
use kadr_core::effect::ColorAdjustment;
use kadr_core::media_asset::PictureSize;
use kadr_core::text_animation::TextAnimationStyle;
use kadr_core::transition::{EdgePhase, SlideDirection, TransitionStyle, ZoomDirection};
use kadr_media::{VideoFrame, YuvColorSpace, YuvMatrix, YuvPicture, YuvRange};
use kadr_render::{
    CanvasDescription, ColorLookupTable, CpuRenderer, FrameRenderer, GpuRenderer, LayerColorLut,
    LayerPlacement, LayerSizing, LookupTableShape, SourceCorrection, TextRasterizer,
    TransitionRole, VisualLayer, text_animation_frame, transition_motion,
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
        color_lut: None,
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
        color_lut: None,
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
            color_lut: None,
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
            color_lut: None,
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
            color_lut: None,
        },
        VisualLayer {
            picture: &logo,
            placement: LayerPlacement {
                scale_factor: 0.4,
                ..LayerPlacement::FITTED
            },
            color_adjustments: &black_and_white,
            color_lut: None,
        },
    ];
    assert_gpu_matches_cpu("colour adjustments", &canvas(320, 180), &layers);
}

/// A moderate 3D look of `size`: warmer highlights, cooler shadows, a little
/// more contrast. Moderate on purpose: a steep LUT would magnify the known
/// chroma differences between the renderers (see `assert_gpu_matches_cpu`).
fn warm_look(size: usize) -> ColorLookupTable {
    let last = (size - 1) as f32;
    let entries = (0..size * size * size)
        .map(|index| {
            let red = (index % size) as f32 / last;
            let green = (index / size % size) as f32 / last;
            let blue = (index / (size * size)) as f32 / last;
            let brightness = 0.2126 * red + 0.7152 * green + 0.0722 * blue;
            let contrast = |channel: f32| (channel - 0.5) * 1.15 + 0.5;
            [
                contrast(red) + 0.06 * brightness,
                contrast(green),
                contrast(blue) + 0.06 * (1.0 - brightness) - 0.03,
            ]
        })
        .collect();
    ColorLookupTable::from_entries(LookupTableShape::ThreeDimensional, size, entries).unwrap()
}

/// A 1D look: a soft S-curve on every channel.
fn s_curve(size: usize) -> ColorLookupTable {
    let entries = (0..size)
        .map(|index| {
            let value = index as f32 / (size - 1) as f32;
            let curved = value * value * (3.0 - 2.0 * value);
            [0.5 * value + 0.5 * curved; 3]
        })
        .collect();
    ColorLookupTable::from_entries(LookupTableShape::OneDimensional, size, entries).unwrap()
}

#[test]
fn colour_lut_layers_match() {
    let video = gradient_video_picture(320, 180, YuvColorSpace::BT709_LIMITED);
    let logo = transparent_logo(64);
    let look = warm_look(17);
    let curve = s_curve(300);
    let adjustment = [ColorAdjustment {
        exposure_in_stops: 0.2,
        ..ColorAdjustment::NEUTRAL
    }];
    let layers = [
        VisualLayer {
            picture: &video,
            placement: LayerPlacement::FITTED,
            color_adjustments: &adjustment,
            color_lut: Some(LayerColorLut {
                table: &look,
                strength_fraction: 0.7,
            }),
        },
        VisualLayer {
            picture: &logo,
            placement: LayerPlacement {
                scale_factor: 0.4,
                ..LayerPlacement::FITTED
            },
            color_adjustments: &[],
            color_lut: Some(LayerColorLut {
                table: &curve,
                strength_fraction: 1.0,
            }),
        },
    ];
    assert_gpu_matches_cpu("colour LUTs", &canvas(640, 360), &layers);
}

/// A small RGBA picture whose colours change smoothly in every direction.
fn small_rgba_gradient() -> VideoFrame {
    VideoFrame::Rgba(RgbaImage::from_fn(64, 32, |x, y| {
        Rgba([(x * 4) as u8, (y * 8) as u8, (255 - x * 4) as u8, 255])
    }))
}

/// A 3D LUT swapping red and blue: an axis mix-up would show at once.
fn red_and_blue_swapped() -> ColorLookupTable {
    let size = 9;
    let last = (size - 1) as f32;
    let entries = (0..size * size * size)
        .map(|index| {
            let red = (index % size) as f32 / last;
            let green = (index / size % size) as f32 / last;
            let blue = (index / (size * size)) as f32 / last;
            [blue, green * 0.9, red]
        })
        .collect();
    ColorLookupTable::from_entries(LookupTableShape::ThreeDimensional, size, entries).unwrap()
}

/// The largest difference between the renderers' Y, U and V values for the
/// same scene.
fn largest_difference_between_renderers(
    gpu_renderer: &mut GpuRenderer,
    canvas: &CanvasDescription,
    layers: &[VisualLayer<'_>],
) -> u8 {
    let cpu_frame = CpuRenderer.render_frame(canvas, layers).unwrap();
    let gpu_frame = gpu_renderer.render_frame(canvas, layers).unwrap();
    let (VideoFrame::Yuv420(cpu_picture), VideoFrame::Yuv420(gpu_picture)) =
        (&cpu_frame, &gpu_frame)
    else {
        panic!("both renderers must return YUV");
    };
    cpu_picture
        .y_plane()
        .iter()
        .zip(gpu_picture.y_plane())
        .chain(cpu_picture.u_plane().iter().zip(gpu_picture.u_plane()))
        .chain(cpu_picture.v_plane().iter().zip(gpu_picture.v_plane()))
        .map(|(cpu, gpu)| cpu.abs_diff(*gpu))
        .max()
        .unwrap()
}

/// An RGBA picture of the canvas size is read texel for texel by both
/// renderers, so their colours only differ by rounding: this checks the
/// shader's interpolation and the order of the table's axes closely.
#[test]
fn a_colour_lut_gives_the_same_colours_on_both_renderers() {
    let Some(mut gpu_renderer) = gpu_renderer() else {
        return;
    };
    let gradient = small_rgba_gradient();
    for (name, table) in [("3D", &red_and_blue_swapped()), ("1D", &s_curve(40))] {
        let layers = [VisualLayer {
            picture: &gradient,
            placement: LayerPlacement::FITTED,
            color_adjustments: &[],
            color_lut: Some(LayerColorLut {
                table,
                strength_fraction: 0.8,
            }),
        }];
        let largest_difference =
            largest_difference_between_renderers(&mut gpu_renderer, &canvas(64, 32), &layers);
        assert!(
            largest_difference <= 2,
            "{name} LUT: values differ by up to {largest_difference}"
        );
    }
}

/// The GPU keeps uploaded tables between frames: a different table on the same
/// layer must not reuse the previous one.
#[test]
fn a_new_colour_lut_on_the_next_frame_replaces_the_previous_one() {
    let Some(mut gpu_renderer) = gpu_renderer() else {
        return;
    };
    let video = gradient_video_picture(160, 90, YuvColorSpace::BT709_LIMITED);
    let (first_look, second_look) = (warm_look(5), s_curve(16));
    let canvas = canvas(320, 180);
    for look in [&first_look, &second_look, &first_look] {
        let layers = [VisualLayer {
            picture: &video,
            placement: LayerPlacement::FITTED,
            color_adjustments: &[],
            color_lut: Some(LayerColorLut {
                table: look,
                strength_fraction: 1.0,
            }),
        }];
        let cpu_frame = CpuRenderer.render_frame(&canvas, &layers).unwrap();
        let gpu_frame = gpu_renderer.render_frame(&canvas, &layers).unwrap();
        let (VideoFrame::Yuv420(cpu_picture), VideoFrame::Yuv420(gpu_picture)) =
            (&cpu_frame, &gpu_frame)
        else {
            panic!("both renderers must return YUV");
        };
        let difference = compare_planes(cpu_picture.y_plane(), gpu_picture.y_plane());
        assert!(
            difference.mean < 1.5,
            "mean difference {:.2}",
            difference.mean
        );
    }
}

/// A second, different video picture: a checkerboard of a light warm colour and
/// a dark cool one.
///
/// Its edges are sharp in brightness, which both renderers compare at full
/// resolution, so a misplaced picture shows. They are milder in colour on
/// purpose: at a sharp edge between two very different colours (red and green
/// differ by about 130 in V), the renderers' different sampling of the
/// half-size colour planes (see [`assert_gpu_matches_cpu`]) puts 13 % of the V
/// values more than 10 apart, with the picture simply drawn scaled, no
/// transition at all.
fn checkerboard_video_picture(width: u32, height: u32) -> VideoFrame {
    let image = RgbaImage::from_fn(width, height, |x, y| {
        if (x / 20 + y / 20) % 2 == 0 {
            Rgba([230, 210, 180, 255])
        } else {
            Rgba([40, 55, 85, 255])
        }
    });
    VideoFrame::Yuv420(YuvPicture::from_rgba(&image, YuvColorSpace::BT709_LIMITED))
}

/// The two layers of a transition from `outgoing` to `incoming`, `progress` of
/// the way through `incoming`'s entry.
fn transition_layers<'picture>(
    outgoing: &'picture VideoFrame,
    incoming: &'picture VideoFrame,
    style: TransitionStyle,
    progress: f32,
    canvas_size: PictureSize,
) -> [VisualLayer<'picture>; 2] {
    let phase = EdgePhase::Entering { progress };
    let placement_for =
        |role| LayerPlacement::FITTED.moved_by(transition_motion(style, phase, role, canvas_size));
    [
        VisualLayer {
            picture: outgoing,
            placement: placement_for(TransitionRole::TouchingClip),
            color_adjustments: &[],
            color_lut: None,
        },
        VisualLayer {
            picture: incoming,
            placement: placement_for(TransitionRole::TransitioningClip),
            color_adjustments: &[],
            color_lut: None,
        },
    ]
}

#[test]
fn transitions_between_two_videos_match() {
    let scene_canvas = canvas(320, 180);
    let outgoing = gradient_video_picture(320, 180, YuvColorSpace::BT709_LIMITED);
    let incoming = checkerboard_video_picture(256, 144);
    let scenes = [
        ("dissolve", TransitionStyle::Fade, 0.4),
        (
            "push to the left",
            TransitionStyle::Slide {
                direction: SlideDirection::Left,
            },
            0.35,
        ),
        (
            "push upwards",
            TransitionStyle::Slide {
                direction: SlideDirection::Up,
            },
            0.6,
        ),
        (
            "zoom in",
            TransitionStyle::Zoom {
                direction: ZoomDirection::In,
            },
            0.5,
        ),
        (
            "zoom out",
            TransitionStyle::Zoom {
                direction: ZoomDirection::Out,
            },
            0.3,
        ),
    ];
    for (scene_name, style, progress) in scenes {
        let layers = transition_layers(&outgoing, &incoming, style, progress, scene_canvas.size);
        assert_gpu_matches_cpu(scene_name, &scene_canvas, &layers);
    }
}

#[test]
fn an_animated_text_matches() {
    let scene_canvas = canvas(320, 180);
    let background = gradient_video_picture(320, 180, YuvColorSpace::BT709_LIMITED);
    let style = TextStyle {
        font_size_in_pixels: 40.0,
        ..TextStyle::default()
    };
    let mut rasterizer = TextRasterizer::new();
    let mut layers_at = |animation, progress| {
        let (motion, reveal) = text_animation_frame(
            animation,
            EdgePhase::Entering { progress },
            style.font_size_in_pixels,
        );
        let text_picture =
            VideoFrame::Rgba(rasterizer.rasterize_revealed("Hello Kadr", &style, reveal));
        let placement = LayerPlacement {
            sizing: LayerSizing::NativePixels,
            ..LayerPlacement::FITTED
        }
        .moved_by(motion);
        (text_picture, placement)
    };
    for (scene_name, animation, progress) in [
        ("typed-on text", TextAnimationStyle::TypeOn, 0.55),
        ("popping text", TextAnimationStyle::Pop, 0.7),
        (
            "sliding text",
            TextAnimationStyle::Slide {
                direction: SlideDirection::Up,
            },
            0.5,
        ),
    ] {
        let (text_picture, placement) = layers_at(animation, progress);
        let layers = [
            VisualLayer {
                picture: &background,
                placement: LayerPlacement::FITTED,
                color_adjustments: &[],
                color_lut: None,
            },
            VisualLayer {
                picture: &text_picture,
                placement,
                color_adjustments: &[],
                color_lut: None,
            },
        ];
        assert_gpu_matches_cpu(scene_name, &scene_canvas, &layers);
    }
}

/// A stabilisation's correction: a shift, a slight turn and an enlargement
/// large enough to keep the picture's edges outside a canvas-sized window.
const SHAKE_CORRECTION: SourceCorrection = SourceCorrection {
    shift_x_in_pixels: 7.3,
    shift_y_in_pixels: -4.1,
    rotation_in_degrees: 0.8,
    scale_factor: 1.06,
};

/// A stabilised video of the canvas size: without the correction it would be
/// passed through, with it both renderers warp it the same way.
#[test]
fn stabilized_video() {
    let video = checkerboard_video_picture(1920, 1080);
    let layers = [VisualLayer {
        picture: &video,
        placement: LayerPlacement {
            source_correction: SHAKE_CORRECTION,
            ..LayerPlacement::FITTED
        },
        color_adjustments: &[],
        color_lut: None,
    }];
    assert_gpu_matches_cpu("stabilized video", &canvas(1920, 1080), &layers);
}

/// The correction moves the picture inside a cropped, scaled, turned and moved
/// window, far enough to show pixels beyond the crop and to repeat the
/// picture's edge pixels.
#[test]
fn stabilized_cropped_scaled_and_rotated_video() {
    let video = gradient_video_picture(320, 240, YuvColorSpace::BT709_LIMITED);
    let layers = [VisualLayer {
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
            offset_x_in_pixels: 15.0,
            offset_y_in_pixels: -10.0,
            source_correction: SourceCorrection {
                shift_x_in_pixels: -12.5,
                shift_y_in_pixels: 18.0,
                rotation_in_degrees: -2.5,
                scale_factor: 1.04,
            },
            ..LayerPlacement::FITTED
        },
        color_adjustments: &[],
        color_lut: None,
    }];
    assert_gpu_matches_cpu(
        "stabilized, cropped and turned video",
        &canvas(400, 300),
        &layers,
    );
}

/// A smooth gradient of the canvas size, steadied and graded with a LUT: both
/// renderers interpolate it, so their colours still only differ by rounding.
/// A renderer warping the picture differently from the other (another sign,
/// centre or direction) moves the gradient by several values.
#[test]
fn stabilized_gradient_with_a_lut_matches_strictly() {
    let Some(mut gpu_renderer) = gpu_renderer() else {
        return;
    };
    let gradient = small_rgba_gradient();
    let swapped = red_and_blue_swapped();
    let layers = [VisualLayer {
        picture: &gradient,
        placement: LayerPlacement {
            source_correction: SourceCorrection {
                shift_x_in_pixels: 3.3,
                shift_y_in_pixels: -2.1,
                rotation_in_degrees: 2.0,
                scale_factor: 1.15,
            },
            ..LayerPlacement::FITTED
        },
        color_adjustments: &[],
        color_lut: Some(LayerColorLut {
            table: &swapped,
            strength_fraction: 0.8,
        }),
    }];
    let largest_difference =
        largest_difference_between_renderers(&mut gpu_renderer, &canvas(64, 32), &layers);
    assert!(
        largest_difference <= 2,
        "values differ by up to {largest_difference}"
    );
}
