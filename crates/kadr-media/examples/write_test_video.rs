//! Writes Kadr's barcode test video with FFmpeg: 4 seconds, 320×240, 30 fps,
//! each frame showing its number as a barcode, a 1 kHz beep at every whole
//! second. libx264 puts B-frames in it and FFmpeg's AAC encoder an encoder delay,
//! so it checks that another backend reads real desktop-made files exactly.
//!
//! `scripts/run-android-tests.ps1` copies it to the phone for the MediaCodec
//! round-trip tests:
//!
//! ```text
//! cargo run -p kadr-media --example write_test_video -- barcode.mp4
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use kadr_core::time::{FrameRate, TimeSpan};
use kadr_media::FfmpegBackend;
use kadr_media::test_patterns::{BeepPattern, TestVideoDescription, write_test_video};

fn main() -> ExitCode {
    let Some(output_path) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: write_test_video <output.mp4>");
        return ExitCode::FAILURE;
    };
    let duration = TimeSpan::from_whole_seconds(4);
    let description = TestVideoDescription {
        output_path,
        width_in_pixels: 320,
        height_in_pixels: 240,
        frame_rate: FrameRate::FPS_30,
        duration,
        audio_sample_rate_in_hertz: 48_000,
        sound: BeepPattern::every_second(1_000.0, 0.5, duration, TimeSpan::from_milliseconds(100)),
    };
    let result = FfmpegBackend::new().and_then(|backend| write_test_video(&backend, &description));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
