//! How fast the camera's motion is measured, for video stabilisation.
//!
//! ```powershell
//! cargo bench -p kadr-vision --bench camera_motion
//! ```
//!
//! - `push_frame_640x360`: measuring the camera's motion into a 640 × 360 frame
//!   (a 1080p video shrunk three times): its pyramid, Shi and Tomasi points, the
//!   optical flow both ways, the similarity fit and the cut check, per frame.

// Benchmark helpers panic with the error on failure.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::hint::black_box;

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use kadr_vision::camera_motion::CameraMotionEstimator;
use kadr_vision::synthetic_scenes::ShakenCameraFrame;
use kadr_vision::{GreyImage, Point, SimilarityTransform};

const WIDTH: usize = 640;
const HEIGHT: usize = 360;
const FRAME_COUNT: usize = 30;

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

criterion_group!(benches, benchmark_camera_motion);
criterion_main!(benches);
