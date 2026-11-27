//! Tracks a region through a whole video file and prints every frame as CSV, to
//! measure the tracker on real videos (accuracy against a known trajectory, speed).
//!
//! ```powershell
//! cargo run --release -p kadr-engine --example track_video -- video.mp4 2.5 1360,576,200,240 > track.csv
//! ```
//!
//! Columns: `time_in_seconds,left,top,width,height,confidence,peak_to_sidelobe_ratio`.
//! The time the analysis took goes to the error output.

use std::error::Error;
use std::path::PathBuf;
use std::time::Instant;

use kadr_core::effect::{PixelRegion, RegionSampleOrigin, TrackingConfidence};
use kadr_core::time::Timestamp;
use kadr_engine::{RegionTrackingRequest, TrackingCancellation, track_region};
use kadr_media::{FfmpegBackend, MediaBackend};

fn main() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [video, at_seconds, rectangle] = &arguments[..] else {
        return Err("usage: track_video <video> <seconds> <left,top,width,height>".into());
    };
    let numbers: Vec<f32> = rectangle
        .split(',')
        .map(str::parse)
        .collect::<Result<_, _>>()?;
    let [left, top, width, height] = numbers[..] else {
        return Err("the rectangle is left,top,width,height".into());
    };
    let backend = FfmpegBackend::new()?;
    let video_path = PathBuf::from(video);
    let description = backend.probe(&video_path)?;
    let (Some(picture_size), Some(frame_rate), Some(duration)) = (
        description.picture_size,
        description.frame_rate,
        description.duration,
    ) else {
        return Err(format!("{video} is not a video with a known length").into());
    };
    let request = RegionTrackingRequest {
        video_path,
        frame_rate,
        picture_size,
        source_start: Timestamp::ZERO,
        source_end: Timestamp::ZERO + duration,
        selection_time: Timestamp::from_seconds(at_seconds.parse()?)?,
        selection: PixelRegion {
            left_in_pixels: left,
            top_in_pixels: top,
            width_in_pixels: width,
            height_in_pixels: height,
        },
        selection_origin: RegionSampleOrigin::UserSelection,
    };
    let started_at = Instant::now();
    let report = track_region(
        &backend,
        &request,
        &mut |_| {},
        &TrackingCancellation::new(),
    )?;
    let elapsed = started_at.elapsed().as_secs_f64();
    eprintln!(
        "{} frames in {elapsed:.1} s ({:.0} frames/s), {} samples kept",
        report.frames.len(),
        report.frames.len() as f64 / elapsed.max(0.001),
        report.region_samples.len()
    );
    println!("time_in_seconds,left,top,width,height,confidence,peak_to_sidelobe_ratio");
    for frame in &report.frames {
        let confidence = match frame.confidence {
            TrackingConfidence::Confident => "confident",
            TrackingConfidence::Uncertain => "uncertain",
            TrackingConfidence::Lost => "lost",
        };
        println!(
            "{:.4},{:.2},{:.2},{:.2},{:.2},{confidence},{:.2}",
            frame.source_time.as_seconds(),
            frame.region.left_in_pixels,
            frame.region.top_in_pixels,
            frame.region.width_in_pixels,
            frame.region.height_in_pixels,
            frame.peak_to_sidelobe_ratio.unwrap_or(f32::INFINITY)
        );
    }
    Ok(())
}
