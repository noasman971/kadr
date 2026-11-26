//! MediaCodec round trips, on an Android phone or emulator: write a synthetic
//! video with the phone's encoders, then probe it, read every frame back and
//! decode its sound, checking exact frame numbers and beep times. The same checks
//! as `ffmpeg_round_trip.rs` on desktop.
//!
//! Run with `scripts/run-android-tests.ps1` (see docs/android.md). When the
//! script also copies a video made on the desktop with FFmpeg (B-frames, AAC with
//! encoder delay), its path is in `KADR_DESKTOP_MADE_VIDEO` and it is checked too.
//!
//! On the development machine this file compiles to an empty test program.

#![cfg(all(feature = "mediacodec", target_os = "android"))]
// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use kadr_core::clip::PlaybackSpeed;
use kadr_core::media_asset::MediaKind;
use kadr_core::time::{FrameRate, TimeSpan, Timestamp};
use kadr_media::test_patterns::{
    BeepPattern, TestVideoDescription, find_beep_starts, frame_index_barcode,
    read_frame_index_barcode, write_test_video,
};
use kadr_media::{
    AudioSegmentRequest, EncodingSpeed, ExportSinkSettings, MediaBackend, MediaCodecBackend,
    MediaError, VideoEncoderPreference,
};

const SAMPLE_RATE: u32 = 48_000;
const BEEP_FREQUENCY: f32 = 1_000.0;
/// Allowed error on beep positions at normal speed, as on desktop.
const BEEP_TOLERANCE: TimeSpan = TimeSpan::from_milliseconds(10);
/// Allowed error when the speed changes (the time stretch moves sounds by a few
/// milliseconds, without drifting), as on desktop.
const STRETCHED_BEEP_TOLERANCE: TimeSpan = TimeSpan::from_milliseconds(20);
/// Environment variable naming a video made on the desktop by
/// `cargo run -p kadr-media --example write_test_video`.
const DESKTOP_MADE_VIDEO_VARIABLE: &str = "KADR_DESKTOP_MADE_VIDEO";

/// A barcode video with a beep at every whole second.
fn write_barcode_video(directory: &Path, frame_rate: FrameRate, seconds: u64) -> PathBuf {
    let output_path = directory.join("barcode.mp4");
    let duration = TimeSpan::from_whole_seconds(seconds);
    write_test_video(
        &MediaCodecBackend::new(),
        &TestVideoDescription {
            output_path: output_path.clone(),
            width_in_pixels: 320,
            height_in_pixels: 240,
            frame_rate,
            duration,
            audio_sample_rate_in_hertz: SAMPLE_RATE,
            sound: BeepPattern::every_second(
                BEEP_FREQUENCY,
                0.5,
                duration,
                TimeSpan::from_milliseconds(100),
            ),
        },
    )
    .unwrap();
    output_path
}

fn assert_close(found: Timestamp, expected: Timestamp, tolerance: TimeSpan, context: &str) {
    let difference = found.ticks().abs_diff(expected.ticks());
    assert!(
        difference <= tolerance.ticks(),
        "{context}: found {found}, expected {expected}"
    );
}

/// Every frame in order, then jumps backwards, far ahead and to mid-frame times.
fn check_every_frame(video_path: &Path, frame_rate: FrameRate, frame_count: u64) {
    let mut source = MediaCodecBackend::new()
        .open_video_source(video_path)
        .unwrap();
    for frame_index in 0..frame_count {
        let frame = source
            .frame_at(frame_rate.timestamp_of_frame(frame_index))
            .unwrap();
        assert_eq!(
            u64::from(read_frame_index_barcode(&frame)),
            frame_index,
            "frame {frame_index} of {}",
            video_path.display()
        );
    }
    let last_frame = frame_count - 1;
    for frame_index in [
        frame_count * 5 / 8,
        frame_count / 8,
        last_frame,
        frame_count / 4,
        0,
    ] {
        // A millisecond into the frame: still that frame.
        let time = frame_rate.timestamp_of_frame(frame_index) + TimeSpan::from_milliseconds(1);
        let frame = source.frame_at(time).unwrap();
        assert_eq!(
            u64::from(read_frame_index_barcode(&frame)),
            frame_index,
            "jump to {time}"
        );
    }
}

/// Beeps at every whole second of a 4-second file land where they should.
fn check_sound_range(video_path: &Path) {
    let sound = MediaCodecBackend::new()
        .decode_audio_segment(&AudioSegmentRequest {
            file_path: video_path.to_path_buf(),
            source_start: Timestamp::from_milliseconds(950),
            source_end: Timestamp::from_milliseconds(3_050),
            speed: PlaybackSpeed::NORMAL,
            output_sample_rate_in_hertz: SAMPLE_RATE,
            output_samples_per_channel: 100_800, // 2.1 s
        })
        .unwrap();

    assert_eq!(sound.samples_per_channel(), 100_800);
    let beep_starts = find_beep_starts(&sound, BEEP_FREQUENCY);
    assert_eq!(beep_starts.len(), 3, "{beep_starts:?}");
    for (found, expected_in_milliseconds) in beep_starts.iter().zip([50, 1_050, 2_050]) {
        assert_close(
            *found,
            Timestamp::from_milliseconds(expected_in_milliseconds),
            BEEP_TOLERANCE,
            &format!("beep in {}", video_path.display()),
        );
    }
}

#[test]
fn probing_describes_the_written_video() {
    let directory = tempfile::tempdir().unwrap();
    let video_path = write_barcode_video(directory.path(), FrameRate::FPS_30, 4);

    let description = MediaCodecBackend::new().probe(&video_path).unwrap();

    assert_eq!(description.kind, MediaKind::Video);
    assert_eq!(description.display_name, "barcode.mp4");
    assert_eq!(description.frame_rate, Some(FrameRate::FPS_30));
    let picture_size = description.picture_size.unwrap();
    assert_eq!(
        (picture_size.width_in_pixels, picture_size.height_in_pixels),
        (320, 240)
    );
    let audio_format = description.audio_format.unwrap();
    assert_eq!(
        (
            audio_format.sample_rate_in_hertz,
            audio_format.channel_count
        ),
        (SAMPLE_RATE, 2)
    );
    let duration = description.duration.unwrap();
    assert!(
        duration
            .ticks()
            .abs_diff(TimeSpan::from_whole_seconds(4).ticks())
            <= TimeSpan::from_milliseconds(50).ticks(),
        "duration {duration}"
    );
}

#[test]
fn ntsc_frame_rates_are_measured_exactly() {
    let directory = tempfile::tempdir().unwrap();
    let video_path = write_barcode_video(directory.path(), FrameRate::FPS_29_97, 3);

    let description = MediaCodecBackend::new().probe(&video_path).unwrap();

    assert_eq!(description.frame_rate, Some(FrameRate::FPS_29_97));
}

#[test]
fn every_frame_is_read_back_exactly_in_order_and_out_of_order() {
    let directory = tempfile::tempdir().unwrap();
    let video_path = write_barcode_video(directory.path(), FrameRate::FPS_30, 4);

    check_every_frame(&video_path, FrameRate::FPS_30, 120);
}

#[test]
fn a_sound_range_is_decoded_at_its_exact_length_with_beeps_in_place() {
    let directory = tempfile::tempdir().unwrap();
    let video_path = write_barcode_video(directory.path(), FrameRate::FPS_30, 4);

    check_sound_range(&video_path);
}

#[test]
fn a_faster_speed_brings_the_beeps_closer_together() {
    let directory = tempfile::tempdir().unwrap();
    let video_path = write_barcode_video(directory.path(), FrameRate::FPS_30, 4);

    let sound = MediaCodecBackend::new()
        .decode_audio_segment(&AudioSegmentRequest {
            file_path: video_path,
            source_start: Timestamp::ZERO,
            source_end: Timestamp::from_whole_seconds(4),
            speed: PlaybackSpeed::from_fraction(2, 1).unwrap(),
            output_sample_rate_in_hertz: SAMPLE_RATE,
            output_samples_per_channel: 96_000, // 2 s
        })
        .unwrap();

    let beep_starts = find_beep_starts(&sound, BEEP_FREQUENCY);
    assert_eq!(beep_starts.len(), 4, "{beep_starts:?}");
    for (found, expected_in_milliseconds) in beep_starts.iter().zip([0, 500, 1_000, 1_500]) {
        assert_close(
            *found,
            Timestamp::from_milliseconds(expected_in_milliseconds),
            STRETCHED_BEEP_TOLERANCE,
            "beep at 2×",
        );
    }
}

#[test]
fn the_hardware_encoder_produces_a_playable_file() {
    let directory = tempfile::tempdir().unwrap();
    let backend = MediaCodecBackend::new();
    let output_path = directory.path().join("hardware.mp4");
    let mut sink = backend
        .create_export_sink(&ExportSinkSettings {
            output_path: output_path.clone(),
            width_in_pixels: 1280,
            height_in_pixels: 720,
            frame_rate: FrameRate::FPS_25,
            audio_sample_rate_in_hertz: SAMPLE_RATE,
            video_encoder_preference: VideoEncoderPreference::HardwareFirst,
            encoding_speed: EncodingSpeed::Balanced,
        })
        .unwrap();
    let encoder_name = sink.video_encoder_name().to_string();
    for frame_index in 0..50 {
        sink.write_video_frame(&frame_index_barcode(1280, 720, frame_index))
            .unwrap();
    }
    // No sound at all: the file must still be written.
    sink.finish().unwrap();

    let description = backend.probe(&output_path).unwrap();
    assert_eq!(description.kind, MediaKind::Video, "encoder {encoder_name}");
    check_every_frame(&output_path, FrameRate::FPS_25, 50);
    println!("automatic encoder choice on this device: {encoder_name}");
}

#[test]
fn a_file_that_is_not_media_is_reported_clearly() {
    let directory = tempfile::tempdir().unwrap();
    let text_path = directory.path().join("notes.txt");
    std::fs::write(&text_path, "not a video").unwrap();

    let error = MediaCodecBackend::new().probe(&text_path).unwrap_err();

    assert!(
        matches!(error, MediaError::CannotOpenFile { .. }),
        "{error}"
    );
    assert!(error.to_string().contains("notes.txt"));
}

#[test]
fn a_video_made_on_the_desktop_is_read_exactly() {
    let Some(video_path) = std::env::var_os(DESKTOP_MADE_VIDEO_VARIABLE).map(PathBuf::from) else {
        println!("{DESKTOP_MADE_VIDEO_VARIABLE} is not set: skipped");
        return;
    };

    let description = MediaCodecBackend::new().probe(&video_path).unwrap();

    assert_eq!(description.frame_rate, Some(FrameRate::FPS_30));
    check_every_frame(&video_path, FrameRate::FPS_30, 120);
    check_sound_range(&video_path);
}
