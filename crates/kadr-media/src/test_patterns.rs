//! Synthetic pictures and sounds with known content, for automated tests and demos.
//!
//! - Each picture shows its own frame number as a **barcode** of black and white
//!   vertical bars, which survives video compression. Reading the barcode back
//!   from an exported video tells exactly which source frame ended up where.
//! - Sounds are made of **beeps** at known times. Finding the beeps in an
//!   exported file (with the Goertzel algorithm, which measures the strength of
//!   one frequency) tells whether sound and picture stay in sync.

use std::path::PathBuf;

use image::{Rgba, RgbaImage};
use kadr_core::time::{FrameRate, TimeSpan, Timestamp};

use crate::audio_buffer::{AudioBuffer, sample_index_at, span_of_sample_count};
use crate::backend::{EncodingSpeed, ExportSinkSettings, MediaBackend, VideoEncoderPreference};
use crate::error::MediaError;
use crate::video_frame::VideoFrame;

/// Number of bars of the barcode: frame numbers up to 1023.
pub const BARCODE_BIT_COUNT: u32 = 10;

/// Grey level above which a bar is read as white.
const BARCODE_WHITE_THRESHOLD: f32 = 128.0;

/// A picture showing `frame_index` as black (0) and white (1) vertical bars,
/// least significant bit on the left.
pub fn frame_index_barcode(
    width_in_pixels: u32,
    height_in_pixels: u32,
    frame_index: u32,
) -> VideoFrame {
    let bar_width = (width_in_pixels / BARCODE_BIT_COUNT).max(1);
    VideoFrame::Rgba(RgbaImage::from_fn(
        width_in_pixels,
        height_in_pixels,
        |x, _| {
            let bit = (x / bar_width).min(BARCODE_BIT_COUNT - 1);
            let level = if (frame_index >> bit) & 1 == 1 {
                255
            } else {
                0
            };
            Rgba([level, level, level, 255])
        },
    ))
}

/// Reads back the frame number drawn by [`frame_index_barcode`], by averaging
/// the middle of each bar.
pub fn read_frame_index_barcode(frame: &VideoFrame) -> u32 {
    let frame = frame.to_rgba();
    let bar_width = (frame.width() / BARCODE_BIT_COUNT).max(1);
    let (first_row, last_row) = (frame.height() / 4, frame.height() * 3 / 4);
    (0..BARCODE_BIT_COUNT)
        .filter(|&bit| {
            let middle_column = (bit * bar_width + bar_width / 2).min(frame.width() - 1);
            let mut grey_total = 0.0;
            for row in first_row..last_row {
                let pixel = frame.get_pixel(middle_column, row);
                grey_total +=
                    (f32::from(pixel[0]) + f32::from(pixel[1]) + f32::from(pixel[2])) / 3.0;
            }
            grey_total / (last_row - first_row).max(1) as f32 > BARCODE_WHITE_THRESHOLD
        })
        .fold(0, |frame_index, bit| frame_index | (1 << bit))
}

/// A sine tone of `frequency_in_hertz`, present only during each beep.
#[derive(Debug, Clone, PartialEq)]
pub struct BeepPattern {
    /// Pitch of the beeps.
    pub frequency_in_hertz: f32,
    /// Peak level, from 0 to 1.
    pub amplitude: f32,
    /// When each beep starts.
    pub beep_starts: Vec<Timestamp>,
    /// How long each beep lasts.
    pub beep_length: TimeSpan,
}

impl BeepPattern {
    /// A beep of `beep_length` at every whole second of `duration`.
    pub fn every_second(
        frequency_in_hertz: f32,
        amplitude: f32,
        duration: TimeSpan,
        beep_length: TimeSpan,
    ) -> Self {
        let whole_seconds = duration.ticks() / TimeSpan::from_whole_seconds(1).ticks();
        Self {
            frequency_in_hertz,
            amplitude,
            beep_starts: (0..=whole_seconds)
                .map(Timestamp::from_whole_seconds)
                .collect(),
            beep_length,
        }
    }

    /// One tone lasting the whole `duration`.
    pub fn continuous(frequency_in_hertz: f32, amplitude: f32, duration: TimeSpan) -> Self {
        Self {
            frequency_in_hertz,
            amplitude,
            beep_starts: vec![Timestamp::ZERO],
            beep_length: duration,
        }
    }

    /// Renders the pattern as stereo sound lasting `duration`.
    pub fn render(&self, sample_rate_in_hertz: u32, duration: TimeSpan) -> AudioBuffer {
        let total_samples = sample_index_at(Timestamp::ZERO + duration, sample_rate_in_hertz);
        let mut interleaved_samples = vec![0.0; total_samples * 2];
        let angular_step =
            std::f32::consts::TAU * self.frequency_in_hertz / sample_rate_in_hertz as f32;
        for beep_start in &self.beep_starts {
            let first_sample = sample_index_at(*beep_start, sample_rate_in_hertz);
            let last_sample = sample_index_at(*beep_start + self.beep_length, sample_rate_in_hertz)
                .min(total_samples);
            for sample_index in first_sample..last_sample {
                let value =
                    self.amplitude * (angular_step * (sample_index - first_sample) as f32).sin();
                interleaved_samples[sample_index * 2] = value;
                interleaved_samples[sample_index * 2 + 1] = value;
            }
        }
        AudioBuffer::from_interleaved_stereo(sample_rate_in_hertz, interleaved_samples)
    }
}

/// How strongly `frequency_in_hertz` is present in `samples`, with the Goertzel
/// algorithm. A pure sine of amplitude `a` at that frequency gives about `a²`.
pub fn tone_strength(samples: &[f32], sample_rate_in_hertz: u32, frequency_in_hertz: f32) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let coefficient =
        2.0 * (std::f32::consts::TAU * frequency_in_hertz / sample_rate_in_hertz as f32).cos();
    let (mut previous, mut before_previous) = (0.0_f32, 0.0_f32);
    for &sample in samples {
        let current = sample + coefficient * previous - before_previous;
        before_previous = previous;
        previous = current;
    }
    let power = previous * previous + before_previous * before_previous
        - coefficient * previous * before_previous;
    let half_length = samples.len() as f32 / 2.0;
    power / (half_length * half_length)
}

/// Analysis window length for [`find_beep_starts`].
const BEEP_ANALYSIS_WINDOW: TimeSpan = TimeSpan::from_milliseconds(10);
/// Distance between two analysis windows.
const BEEP_ANALYSIS_STEP: TimeSpan = TimeSpan::from_milliseconds(1);

/// Finds when beeps of `frequency_in_hertz` start in `audio`.
///
/// The strength of the frequency is measured on short sliding windows. A beep
/// starts when the strength rises above a quarter of the loudest value found
/// (that is, when half of the window is inside the beep), so the start is the
/// middle of that window. Precision: about 2 ms.
pub fn find_beep_starts(audio: &AudioBuffer, frequency_in_hertz: f32) -> Vec<Timestamp> {
    let sample_rate = audio.sample_rate_in_hertz();
    let mono_samples = audio.to_mono();
    let window_length = sample_index_at(Timestamp::ZERO + BEEP_ANALYSIS_WINDOW, sample_rate).max(1);
    let step_length = sample_index_at(Timestamp::ZERO + BEEP_ANALYSIS_STEP, sample_rate).max(1);

    let strengths: Vec<(usize, f32)> = (0..mono_samples.len().saturating_sub(window_length))
        .step_by(step_length)
        .map(|first_sample| {
            let window = &mono_samples[first_sample..first_sample + window_length];
            (
                first_sample,
                tone_strength(window, sample_rate, frequency_in_hertz),
            )
        })
        .collect();
    let loudest = strengths
        .iter()
        .map(|(_, strength)| *strength)
        .fold(0.0, f32::max);
    let threshold = loudest / 4.0;
    if loudest <= f32::EPSILON {
        return Vec::new();
    }

    let mut beep_starts = Vec::new();
    let mut is_inside_beep = false;
    for (first_sample, strength) in strengths {
        if !is_inside_beep && strength > threshold {
            // A beep already playing in the very first window started at the beginning.
            let estimated_start_sample = if first_sample == 0 {
                0
            } else {
                first_sample + window_length / 2
            };
            beep_starts
                .push(Timestamp::ZERO + span_of_sample_count(estimated_start_sample, sample_rate));
        }
        is_inside_beep = strength > threshold;
    }
    beep_starts
}

/// Description of a synthetic test video.
#[derive(Debug, Clone, PartialEq)]
pub struct TestVideoDescription {
    /// The MP4 file to write.
    pub output_path: PathBuf,
    /// Picture width (even).
    pub width_in_pixels: u32,
    /// Picture height (even).
    pub height_in_pixels: u32,
    /// Frames per second.
    pub frame_rate: FrameRate,
    /// How long the video lasts.
    pub duration: TimeSpan,
    /// Sample rate of the sound.
    pub audio_sample_rate_in_hertz: u32,
    /// The sound.
    pub sound: BeepPattern,
}

/// Writes a video whose frames show their frame number as a barcode, with the
/// given sound. Uses the software encoder, so the file is identical everywhere.
///
/// # Errors
///
/// Fails when the file cannot be written.
pub fn write_test_video(
    backend: &dyn MediaBackend,
    description: &TestVideoDescription,
) -> Result<(), MediaError> {
    let mut sink = backend.create_export_sink(&ExportSinkSettings {
        output_path: description.output_path.clone(),
        width_in_pixels: description.width_in_pixels,
        height_in_pixels: description.height_in_pixels,
        frame_rate: description.frame_rate,
        audio_sample_rate_in_hertz: description.audio_sample_rate_in_hertz,
        video_encoder_preference: VideoEncoderPreference::SoftwareOnly,
        encoding_speed: EncodingSpeed::Fast,
    })?;
    let frame_count = description
        .frame_rate
        .frame_count_covering(description.duration);
    for frame_index in 0..frame_count {
        let frame_number = u32::try_from(frame_index).unwrap_or(u32::MAX);
        sink.write_video_frame(&frame_index_barcode(
            description.width_in_pixels,
            description.height_in_pixels,
            frame_number,
        ))?;
    }
    sink.write_audio(
        &description
            .sound
            .render(description.audio_sample_rate_in_hertz, description.duration),
    )?;
    sink.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn barcodes_read_back_their_frame_number() {
        for frame_index in [0, 1, 2, 341, 512, 1023] {
            let barcode = frame_index_barcode(320, 240, frame_index);
            assert_eq!(read_frame_index_barcode(&barcode), frame_index);
        }
    }

    #[test]
    fn tone_strength_finds_the_right_frequency() {
        let tone = BeepPattern::continuous(1_000.0, 0.5, TimeSpan::from_whole_seconds(1))
            .render(48_000, TimeSpan::from_whole_seconds(1));
        let mono_samples = tone.to_mono();
        assert!((tone_strength(&mono_samples[..4_800], 48_000, 1_000.0) - 0.25).abs() < 0.01);
        assert!(tone_strength(&mono_samples[..4_800], 48_000, 300.0) < 0.001);
    }

    #[test]
    fn beep_starts_are_found_within_two_milliseconds() {
        let duration = TimeSpan::from_whole_seconds(3);
        let beeps =
            BeepPattern::every_second(1_000.0, 0.5, duration, TimeSpan::from_milliseconds(100));
        let mut audio = beeps.render(48_000, duration);
        // A low background tone must not disturb the detection.
        let background = BeepPattern::continuous(300.0, 0.3, duration).render(48_000, duration);
        audio.mix_in(&background, 0, |_| 1.0);

        let found = find_beep_starts(&audio, 1_000.0);

        assert_eq!(found.len(), 3, "{found:?}");
        for (found_start, expected_start) in found.iter().zip(&beeps.beep_starts) {
            let difference = found_start.ticks().abs_diff(expected_start.ticks());
            assert!(
                difference <= TimeSpan::from_milliseconds(2).ticks(),
                "{found_start} vs {expected_start}"
            );
        }
    }
}
