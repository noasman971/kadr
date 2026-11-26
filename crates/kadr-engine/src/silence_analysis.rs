//! Finding the silent parts of a media file's sound.
//!
//! The sound is decoded piece by piece (mono, at [`ANALYSIS_SAMPLE_RATE_IN_HERTZ`])
//! and fed to `kadr_vision`'s [`SilenceDetector`], so even a long file never sits
//! whole in memory. The silences come back as times in the file.

use std::path::Path;

use kadr_core::clip::PlaybackSpeed;
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_media::audio_buffer::{sample_index_at, span_of_sample_count};
use kadr_media::{AudioSegmentRequest, MediaBackend, MediaError};
use kadr_vision::silence_detection::SilenceDetector;
pub use kadr_vision::silence_detection::{
    DEFAULT_MINIMUM_SILENCE_IN_SECONDS, DEFAULT_SILENCE_THRESHOLD_IN_DECIBELS,
    SilenceDetectionSettings,
};

/// Loudness is measured on sound resampled to 16 kHz: plenty for levels (speech
/// energy lies below 8 kHz) and three times less work than 48 kHz.
pub const ANALYSIS_SAMPLE_RATE_IN_HERTZ: u32 = 16_000;

/// Length of each decoded piece of sound.
const DECODED_PIECE_LENGTH: TimeSpan = TimeSpan::from_whole_seconds(30);

/// A silent part of a file, in times measured from the start of the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SilentPart {
    /// Where the silence starts.
    pub start: Timestamp,
    /// Where it ends (excluded).
    pub end: Timestamp,
}

/// Finds the silences in `source_start`..`source_end` of the file's sound.
///
/// # Errors
///
/// Fails when the file cannot be opened or has no sound.
pub fn find_silent_parts(
    backend: &dyn MediaBackend,
    file_path: &Path,
    source_start: Timestamp,
    source_end: Timestamp,
    settings: SilenceDetectionSettings,
) -> Result<Vec<SilentPart>, MediaError> {
    let mut detector = SilenceDetector::new(ANALYSIS_SAMPLE_RATE_IN_HERTZ, settings);
    let mut piece_start = source_start;
    while piece_start < source_end {
        let piece_end = piece_start
            .checked_add(DECODED_PIECE_LENGTH)
            .unwrap_or(source_end)
            .min(source_end);
        let first_sample = sample_index_at(piece_start, ANALYSIS_SAMPLE_RATE_IN_HERTZ);
        let end_sample = sample_index_at(piece_end, ANALYSIS_SAMPLE_RATE_IN_HERTZ);
        let piece = backend.decode_audio_segment(&AudioSegmentRequest {
            file_path: file_path.to_path_buf(),
            source_start: piece_start,
            source_end: piece_end,
            speed: PlaybackSpeed::NORMAL,
            output_sample_rate_in_hertz: ANALYSIS_SAMPLE_RATE_IN_HERTZ,
            output_samples_per_channel: end_sample.saturating_sub(first_sample),
        })?;
        detector.push_samples(&piece.to_mono());
        piece_start = piece_end;
    }
    let silent_parts = detector
        .finish()
        .into_iter()
        .map(|range| SilentPart {
            start: source_start
                + span_of_sample_count(range.start_sample, ANALYSIS_SAMPLE_RATE_IN_HERTZ),
            end: (source_start
                + span_of_sample_count(range.end_sample, ANALYSIS_SAMPLE_RATE_IN_HERTZ))
            .min(source_end),
        })
        .collect();
    Ok(silent_parts)
}

#[cfg(test)]
mod tests {
    use kadr_core::time::FrameRate;
    use kadr_media::FfmpegBackend;
    use kadr_media::test_patterns::{BeepPattern, TestVideoDescription, write_test_video};

    use super::*;

    /// Encoding and decoding sound shifts edges by a few milliseconds.
    const EDGE_TOLERANCE: TimeSpan = TimeSpan::from_milliseconds(30);

    fn assert_close(found: Timestamp, expected_in_milliseconds: u64) {
        let expected = Timestamp::from_milliseconds(expected_in_milliseconds);
        assert!(
            found.ticks().abs_diff(expected.ticks()) <= EDGE_TOLERANCE.ticks(),
            "{found} instead of {expected}"
        );
    }

    #[test]
    fn the_silences_between_beeps_are_found_in_file_time() {
        let folder = tempfile::tempdir().unwrap();
        let file_path = folder.path().join("beeps.mp4");
        let backend = FfmpegBackend::new().unwrap();
        let duration = TimeSpan::from_whole_seconds(3);
        write_test_video(
            &backend,
            &TestVideoDescription {
                output_path: file_path.clone(),
                width_in_pixels: 160,
                height_in_pixels: 120,
                frame_rate: FrameRate::FPS_25,
                duration,
                audio_sample_rate_in_hertz: 48_000,
                sound: BeepPattern::every_second(
                    880.0,
                    0.5,
                    duration,
                    TimeSpan::from_milliseconds(200),
                ),
            },
        )
        .unwrap();

        // From 0.4 s: the beeps at 1 s and 2 s, and the silences around them.
        let silences = find_silent_parts(
            &backend,
            &file_path,
            Timestamp::from_milliseconds(400),
            Timestamp::ZERO + duration,
            SilenceDetectionSettings::default(),
        )
        .unwrap();
        assert_eq!(silences.len(), 3, "{silences:?}");
        assert_close(silences[0].start, 400);
        assert_close(silences[0].end, 1_000);
        assert_close(silences[1].start, 1_200);
        assert_close(silences[1].end, 2_000);
        assert_close(silences[2].start, 2_200);
        assert_eq!(silences[2].end, Timestamp::ZERO + duration);
    }
}
