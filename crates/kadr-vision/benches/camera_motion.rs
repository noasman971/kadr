//! How fast the camera's motion is measured, for video stabilisation.
//!
//! ```powershell
//! cargo bench -p kadr-vision --bench camera_motion
//! ```
//!
//! - `push_frame_640x360`: measuring the camera's motion into a 640 × 360 frame
//!   (a 1080p video shrunk three times): its pyramid, Shi and Tomasi points, the
//!   optical flow both ways, the similarity fit and the cut check, per frame;
//! - `fitted_zoom_10_minutes`: the zoom that hides every frame's edges, for 10
//!   minutes at 30 frames per second (18 000 samples) smoothed over 0.5 s, which
//!   is redone whenever the smoothing changes;
//! - `correction_of_one_frame`: one frame's correction, as drawing a frame needs.

// Benchmark helpers panic with the error on failure.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::hint::black_box;

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use kadr_vision::camera_motion::CameraMotionEstimator;
use kadr_vision::camera_path::{
    CameraPathPoint, CameraPose, fitted_zoom_factor, stabilizing_correction_at,
};
use kadr_vision::synthetic_scenes::ShakenCameraFrame;
use kadr_vision::{GreyImage, Point, SimilarityTransform};

const WIDTH: usize = 640;
const HEIGHT: usize = 360;
const FRAME_COUNT: usize = 30;

/// Ten minutes at 30 frames per second.
const PATH_SAMPLE_COUNT: usize = 18_000;
const FRAMES_PER_SECOND: f64 = 30.0;
const SMOOTHING_IN_SECONDS: f64 = 0.5;

/// A shaky camera: small shifts, rolls and changes of size every frame.
fn shaken_frame(index: usize) -> GreyImage {
    let time = index as f32;
    ShakenCameraFrame {
        width: WIDTH,
        height: HEIGHT,
        background_seed: 3,
        camera: SimilarityTransform {
            center: Point {
                x: WIDTH as f32 / 2.0,
                y: HEIGHT as f32 / 2.0,
            },
            shift_x_in_pixels: 2.0 * time + 3.0 * (time * 0.9).sin(),
            shift_y_in_pixels: 2.5 * (time * 1.3).sin(),
            rotation_in_degrees: 0.4 * (time * 0.7).sin(),
            scale_factor: 1.0 + 0.005 * (time * 0.5).sin(),
        },
        moving_object: None,
        noise_amplitude: 3.0,
        noise_seed: index as u32,
    }
    .render()
}

fn benchmark_camera_motion(criterion: &mut Criterion) {
    let frames: Vec<GreyImage> = (0..=FRAME_COUNT).map(shaken_frame).collect();
    let mut group = criterion.benchmark_group("camera_motion");
    group.throughput(Throughput::Elements(FRAME_COUNT as u64));
    group.bench_function("push_frame_640x360", |bencher| {
        bencher.iter_batched(
            || {
                let mut estimator = CameraMotionEstimator::new();
                estimator.push_frame(frames[0].clone()).unwrap();
                (estimator, frames[1..].to_vec())
            },
            |(mut estimator, frames)| {
                for frame in frames {
                    black_box(estimator.push_frame(frame).unwrap());
                }
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();
}

/// A camera held by hand, as its path stores it. Its shake needs a zoom under the
/// cap, so that the fitted zoom reads the whole path.
fn shaky_path() -> Vec<CameraPathPoint> {
    (0..PATH_SAMPLE_COUNT)
        .map(|index| {
            let time = index as f64 / FRAMES_PER_SECOND;
            CameraPathPoint {
                time_in_seconds: time,
                pose: CameraPose {
                    shift_x_in_pixels: 6.0 * (time * 19.0).sin() + 3.0 * (time * 41.0).sin(),
                    shift_y_in_pixels: 4.0 * (time * 31.0).sin(),
                    rotation_in_degrees: 0.4 * (time * 13.0).sin(),
                    scale_factor: (0.003 * (time * 23.0).sin()).exp(),
                },
                motion_was_measured: index > 0,
            }
        })
        .collect()
}

fn benchmark_camera_path(criterion: &mut Criterion) {
    let path = shaky_path();
    let mut group = criterion.benchmark_group("camera_path");
    group.bench_function("fitted_zoom_10_minutes", |bencher| {
        bencher.iter(|| {
            fitted_zoom_factor(
                black_box(path.as_slice()),
                SMOOTHING_IN_SECONDS,
                1920.0,
                1080.0,
            )
        });
    });
    group.bench_function("correction_of_one_frame", |bencher| {
        bencher.iter(|| {
            stabilizing_correction_at(
                black_box(path.as_slice()),
                black_box(PATH_SAMPLE_COUNT / 2),
                SMOOTHING_IN_SECONDS,
            )
        });
    });
    group.finish();
}

criterion_group!(benches, benchmark_camera_motion, benchmark_camera_path);
criterion_main!(benches);
