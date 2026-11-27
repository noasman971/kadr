//! Measures the camera path of a whole video and prints, for every frame, the
//! measured path and the correction that steadies it, as CSV. Used to measure
//! the stabilisation on a video with a known shake
//! (`scripts/generate-stabilization-benchmark.ps1`,
//! `scripts/compare-stabilization-with-truth.py`).
//!
//! ```text
//! cargo run --release -p kadr-engine --example stabilize_video -- <video> [smoothing_in_seconds] > path.csv
//! ```
//!
//! The speed (frames per second, decoding included) goes to the error output.

use std::error::Error;
use std::path::PathBuf;
use std::time::Instant;

use kadr_core::effect::Stabilization;
use kadr_core::time::Timestamp;
use kadr_engine::{
    StabilizationRequest, TrackingCancellation, analyse_camera_path, stabilization_correction_at,
    stabilization_summary,
};
use kadr_media::{FfmpegBackend, MediaBackend};

fn main() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let (video, smoothing_in_seconds) = match &arguments[..] {
        [video] => (video, Stabilization::DEFAULT_SMOOTHING_IN_SECONDS),
        [video, smoothing] => (video, smoothing.parse()?),
        _ => return Err("usage: stabilize_video <video> [smoothing_in_seconds]".into()),
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
    let request = StabilizationRequest {
        video_path,
        frame_rate,
        picture_size,
        source_start: Timestamp::ZERO,
        source_end: Timestamp::ZERO + duration,
    };

    let started_at = Instant::now();
    let report = analyse_camera_path(
        &backend,
        &request,
        &mut |_| {},
        &TrackingCancellation::new(),
    )?;
    let elapsed = started_at.elapsed().as_secs_f64();
    let stabilization = report.stabilization_with(smoothing_in_seconds);
    let summary = stabilization_summary(&stabilization);
    eprintln!(
        "{} frames in {elapsed:.1} s ({:.0} frames/s), {} not measured, zoom {:.3}, {} partly corrected",
        report.camera_path.len(),
        report.camera_path.len() as f64 / elapsed.max(0.001),
        report.unmeasured_frame_count,
        summary.zoom_factor,
        summary.partly_corrected_frame_count
    );

    println!(
        "frame,time_in_seconds,motion_was_measured,path_x,path_y,path_rotation,path_scale,\
         correction_x,correction_y,correction_rotation,correction_scale,fraction_applied,zoom"
    );
    for (frame_index, sample) in stabilization.camera_path.iter().enumerate() {
        let frame = stabilization_correction_at(&stabilization, sample.source_time, picture_size);
        let correction = frame.source_correction;
        println!(
            "{frame_index},{:.5},{},{:.2},{:.2},{:.3},{:.5},{:.3},{:.3},{:.4},{:.5},{:.3},{:.3}",
            sample.source_time.as_seconds(),
            u8::from(sample.motion_was_measured),
            sample.shift_x_in_pixels,
            sample.shift_y_in_pixels,
            sample.rotation_in_degrees,
            sample.scale_factor,
            correction.shift_x_in_pixels,
            correction.shift_y_in_pixels,
            correction.rotation_in_degrees,
            correction.scale_factor,
            frame.fraction_applied,
            stabilization.zoom_factor
        );
    }
    Ok(())
}
