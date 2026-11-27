//! How long one frame takes to render, on the CPU and on the GPU, in 1080p and 4K.
//!
//! ```powershell
//! cargo bench -p kadr-render
//! ```
//!
//! Scenes:
//! - `scaled_video`: one 720p video scaled up to fill the canvas (the most common
//!   case once sources and project sizes differ);
//! - `video_with_logo`: the same, plus a rotated semi-transparent logo on top.
//!
//! The GPU numbers include uploading the pictures and reading the YUV result
//! back, i.e. everything the export pays for.

// Benchmark helpers panic with the error on failure.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use image::{Rgba, RgbaImage};
use kadr_core::color::Color;
use kadr_core::media_asset::PictureSize;
use kadr_media::{VideoFrame, YuvColorSpace, YuvPicture};
use kadr_render::{
    CanvasDescription, CpuRenderer, FrameRenderer, GpuRenderer, LayerPlacement, VisualLayer,
};

fn video_picture(width: u32, height: u32) -> VideoFrame {
    let image = RgbaImage::from_fn(width, height, |x, y| {
        Rgba([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8, 255])
    });
    VideoFrame::Yuv420(YuvPicture::from_rgba(&image, YuvColorSpace::BT709_LIMITED))
}

fn logo_picture() -> VideoFrame {
    VideoFrame::Rgba(RgbaImage::from_fn(256, 256, |x, y| {
        let is_inside = (x as i32 - 128).pow(2) + (y as i32 - 128).pow(2) < 100 * 100;
        if is_inside {
            Rgba([255, 140, 0, 220])
        } else {
            Rgba([0, 0, 0, 0])
        }
    }))
}

fn canvas(width_in_pixels: u32, height_in_pixels: u32) -> CanvasDescription {
    CanvasDescription {
        size: PictureSize {
            width_in_pixels,
            height_in_pixels,
        },
        background: Color::BLACK,
    }
}

fn render_frames(criterion: &mut Criterion) {
    let video = video_picture(1280, 720);
    let logo = logo_picture();
    let scaled_video = [VisualLayer {
        picture: &video,
        placement: LayerPlacement::FITTED,
        color_adjustments: &[],
        color_lut: None,
    }];
    let video_with_logo = [
        scaled_video[0],
        VisualLayer {
            picture: &logo,
            placement: LayerPlacement {
                scale_factor: 0.25,
                offset_x_in_pixels: 300.0,
                offset_y_in_pixels: -200.0,
                rotation_in_degrees: 15.0,
                opacity: 0.8,
                ..LayerPlacement::FITTED
            },
            color_adjustments: &[],
            color_lut: None,
        },
    ];
    let mut renderers: Vec<(&str, Box<dyn FrameRenderer>)> = vec![("cpu", Box::new(CpuRenderer))];
    match GpuRenderer::new() {
        Ok(gpu_renderer) => renderers.push(("gpu", Box::new(gpu_renderer))),
        Err(error) => eprintln!("GPU benchmarks skipped: {error}"),
    }

    let mut group = criterion.benchmark_group("render_frame");
    group
        .sample_size(20)
        .measurement_time(Duration::from_secs(4));
    for (resolution_name, canvas) in [("1080p", canvas(1920, 1080)), ("4k", canvas(3840, 2160))] {
        for (scene_name, layers) in [
            ("scaled_video", &scaled_video[..]),
            ("video_with_logo", &video_with_logo[..]),
        ] {
            for (renderer_name, renderer) in &mut renderers {
                let benchmark_id =
                    BenchmarkId::new(format!("{scene_name}/{renderer_name}"), resolution_name);
                group.bench_function(benchmark_id, |bencher| {
                    bencher.iter(|| black_box(renderer.render_frame(&canvas, layers).unwrap()));
                });
            }
        }
    }
    group.finish();
}

criterion_group!(benches, render_frames);
criterion_main!(benches);
