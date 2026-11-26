//! How fast the region tracker runs.
//!
//! ```powershell
//! cargo bench -p kadr-vision
//! ```
//!
//! - `shrink_1080p_luma`: turning a 1920 × 1080 Y plane into the 640 × 360
//!   grey image the tracker works on;
//! - `follow_visible_region`: tracking a textured object moving over a textured
//!   background, 640 × 360, per frame (optical flow, correlation, learning);
//! - `search_hidden_region`: a frame where the region is hidden, which runs the
//!   wider recovery search.

// Benchmark helpers panic with the error on failure.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use std::hint::black_box;

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use kadr_vision::synthetic_scenes::{SceneFrame, SceneObject};
use kadr_vision::{GreyImage, Region, RegionTracker, shrink_factor_for_tracking};

const WIDTH: usize = 640;
const HEIGHT: usize = 360;
const FRAME_COUNT: usize = 30;

fn scene_frame(index: usize, is_hidden: bool) -> GreyImage {
    let region = Region::from_top_left(
        100.0 + 8.0 * index as f32,
        120.0 + 2.0 * index as f32,
        60.0,
        72.0,
    );
    SceneFrame {
        width: WIDTH,
        height: HEIGHT,
        background_seed: 3,
        background_offset_in_pixels: 0.0,
        object: (!is_hidden).then_some(SceneObject {
            region,
            texture_seed: 4,
        }),
        obstacle: is_hidden.then_some(region.scaled_by(1.5)),
    }
    .render()
}

fn benchmark_shrinking(criterion: &mut Criterion) {
    let luma: Vec<u8> = (0..1920 * 1080)
        .map(|index| (index * 7 % 251) as u8)
        .collect();
    let shrink_factor = shrink_factor_for_tracking(1920);
    criterion.bench_function("shrink_1080p_luma", |bencher| {
        bencher.iter(|| {
            GreyImage::from_luma_shrunk(1920, 1080, 1920, black_box(&luma), shrink_factor).unwrap()
        });
    });
}

fn benchmark_following(criterion: &mut Criterion) {
    let frames: Vec<GreyImage> = (0..=FRAME_COUNT)
        .map(|index| scene_frame(index, false))
        .collect();
    let selection = Region::from_top_left(100.0, 120.0, 60.0, 72.0);
    let mut group = criterion.benchmark_group("tracker");
    group.throughput(Throughput::Elements(FRAME_COUNT as u64));
    group.bench_function("follow_visible_region", |bencher| {
        bencher.iter_batched(
            || {
                (
                    RegionTracker::start(frames[0].clone(), selection).unwrap(),
                    frames[1..].to_vec(),
                )
            },
            |(mut tracker, frames)| {
                for frame in frames {
                    black_box(tracker.track(frame).unwrap());
                }
            },
            BatchSize::LargeInput,
        );
    });
    let hidden: Vec<GreyImage> = (0..FRAME_COUNT)
        .map(|index| scene_frame(1 + index, true))
        .collect();
    group.bench_function("search_hidden_region", |bencher| {
        bencher.iter_batched(
            || {
                (
                    RegionTracker::start(frames[0].clone(), selection).unwrap(),
                    hidden.clone(),
                )
            },
            |(mut tracker, frames)| {
                for frame in frames {
                    black_box(tracker.track(frame).unwrap());
                }
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();
}

criterion_group!(benches, benchmark_shrinking, benchmark_following);
criterion_main!(benches);
