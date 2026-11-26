//! Real FFmpeg round trips: write a synthetic video, then probe it, read every
//! frame back and decode its sound, checking exact frame numbers and beep times.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use kadr_core::clip::PlaybackSpeed;
use kadr_core::media_asset::MediaKind;
use kadr_core::time::{FrameRate, TimeSpan, Timestamp};
use kadr_media::test_patterns::{
    BeepPattern, TestVideoDescription, find_beep_starts, read_frame_index_barcode, write_test_video,
};
use kadr_media::{
    AudioSegmentRequest, EncodingSpeed, ExportSinkSettings, FfmpegBackend, MediaBackend,
    MediaError, VideoEncoderPreference,
};

const SAMPLE_RATE: u32 = 48_000;
const BEEP_FREQUENCY: f32 = 1_000.0;
/// Allowed error on beep positions at normal speed (AAC and resampling add a few
/// samples of delay).
const BEEP_TOLERANCE: TimeSpan = TimeSpan::from_milliseconds(10);
/// Allowed error when the speed changes: the pitch-keeping time stretch (`atempo`)
/// moves sounds by up to about 15 ms, without drifting. Still under one frame.
const STRETCHED_BEEP_TOLERANCE: TimeSpan = TimeSpan::from_milliseconds(20);

/// A 4-second, 30 fps, 320×240 barcode video with a beep at every whole second.
fn write_four_second_video(directory: &Path) -> PathBuf {
    let output_path = directory.join("barcode.mp4");
    let duration = TimeSpan::from_whole_seconds(4);
    write_test_video(
        &FfmpegBackend::new().unwrap(),
        &TestVideoDescription {
            output_path: output_path.clone(),
            width_in_pixels: 320,
            height_in_pixels: 240,
            frame_rate: FrameRate::FPS_30,
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

#[test]
fn probing_describes_the_written_video() {
    let directory = tempfile::tempdir().unwrap();
    let video_path = write_four_second_video(directory.path());

    let description = FfmpegBackend::new().unwrap().probe(&video_path).unwrap();

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
fn every_frame_is_read_back_exactly_in_order_and_out_of_order() {
    let directory = tempfile::tempdir().unwrap();
    let video_path = write_four_second_video(directory.path());
    let mut source = FfmpegBackend::new()
        .unwrap()
        .open_video_source(&video_path)
        .unwrap();

    for frame_index in 0..120 {
        let frame = source
            .frame_at(FrameRate::FPS_30.timestamp_of_frame(frame_index))
            .unwrap();
        assert_eq!(u64::from(read_frame_index_barcode(&frame)), frame_index);
    }
    // Backwards and far jumps, and times in the middle of a frame.
    for (time_in_milliseconds, expected_frame) in
        [(2_500, 75), (500, 15), (3_990, 119), (1_010, 30), (0, 0)]
    {
        let frame = source
            .frame_at(Timestamp::from_milliseconds(time_in_milliseconds))
            .unwrap();
        assert_eq!(
            read_frame_index_barcode(&frame),
            expected_frame,
            "at {time_in_milliseconds} ms"
        );
    }
}

#[test]
fn a_sound_range_is_decoded_at_its_exact_length_with_beeps_in_place() {
    let directory = tempfile::tempdir().unwrap();
    let video_path = write_four_second_video(directory.path());

    let sound = FfmpegBackend::new()
        .unwrap()
        .decode_audio_segment(&AudioSegmentRequest {
            file_path: video_path,
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
            "beep",
        );
    }
}

#[test]
fn a_faster_speed_brings_the_beeps_closer_together() {
    let directory = tempfile::tempdir().unwrap();
    let video_path = write_four_second_video(directory.path());

    let sound = FfmpegBackend::new()
        .unwrap()
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
fn the_automatic_encoder_choice_always_produces_a_playable_file() {
    let directory = tempfile::tempdir().unwrap();
    let backend = FfmpegBackend::new().unwrap();
    let output_path = directory.path().join("automatic.mp4");
    let mut sink = backend
        .create_export_sink(&ExportSinkSettings {
            output_path: output_path.clone(),
            width_in_pixels: 320,
            height_in_pixels: 240,
            frame_rate: FrameRate::FPS_25,
            audio_sample_rate_in_hertz: SAMPLE_RATE,
            video_encoder_preference: VideoEncoderPreference::HardwareFirst,
            encoding_speed: EncodingSpeed::Fast,
        })
        .unwrap();
    let encoder_name = sink.video_encoder_name().to_string();
    for frame_index in 0..25 {
        sink.write_video_frame(&kadr_media::test_patterns::frame_index_barcode(
            320,
            240,
            frame_index,
        ))
        .unwrap();
    }
    sink.finish().unwrap();

    let description = backend.probe(&output_path).unwrap();
    assert_eq!(description.kind, MediaKind::Video, "encoder {encoder_name}");
    println!("automatic encoder choice on this machine: {encoder_name}");
}

#[test]
fn a_file_that_is_not_media_is_reported_clearly() {
    let directory = tempfile::tempdir().unwrap();
    let text_path = directory.path().join("notes.txt");
    std::fs::write(&text_path, "not a video").unwrap();

    let error = FfmpegBackend::new().unwrap().probe(&text_path).unwrap_err();

    assert!(
        matches!(error, MediaError::CannotOpenFile { .. }),
        "{error}"
    );
    assert!(error.to_string().contains("notes.txt"));
}
