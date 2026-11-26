//! Playing stereo sound faster or slower **without changing its pitch**.
//!
//! FFmpeg's `atempo` does this on desktop; on Android, Kadr uses the classic
//! **WSOLA** algorithm (Waveform Similarity Overlap-Add; W. Verhelst and M. Roelands,
//! "An overlap-add technique based on waveform similarity (WSOLA) for high quality
//! time-scale modification of speech", ICASSP 1993):
//!
//! 1. The output is built from short overlapping pieces (`FRAME_DURATION`, half
//!    overlapping), each faded in and out with a Hann window so they add up to a
//!    constant level.
//! 2. Output piece *k* is taken around input position `k × hop × speed`: playing
//!    at 2× takes pieces twice as far apart, so the sound lasts half as long, but
//!    each piece keeps its own waveform, hence its pitch.
//! 3. Taken exactly there, consecutive pieces would rarely line up and the sound
//!    would warble. So each piece may move by up to `SEARCH_RADIUS_DURATION`: the
//!    chosen position is the one whose start looks most like the natural
//!    continuation of the previous piece (highest normalised cross-correlation).
//!
//! The search moves pieces by a few milliseconds but never accumulates: the
//! nominal positions advance exactly with the speed, so the sound does not drift.

use crate::audio_buffer::STEREO_CHANNEL_COUNT;

/// Length of each piece. 40 ms is longer than the period of any voice or
/// instrument note, so a piece always holds whole periods.
const FRAME_DURATION_IN_SECONDS: f64 = 0.040;
/// How far a piece may move from its nominal position to line up.
const SEARCH_RADIUS_IN_SECONDS: f64 = 0.010;
/// The coarse search compares every this many samples, then refines around the best.
const COARSE_SEARCH_STEP: usize = 4;
/// Below this distance from 1, the speed is treated as exactly 1 (no change).
const SPEED_TOLERANCE: f64 = 1e-6;

/// Changes the duration of interleaved stereo sound by `1 / speed`, keeping its
/// pitch. `speed` above 1 makes it shorter and faster.
pub(crate) fn stretch_stereo(
    interleaved_input: &[f32],
    sample_rate_in_hertz: u32,
    speed: f64,
) -> Vec<f32> {
    if (speed - 1.0).abs() < SPEED_TOLERANCE || speed <= 0.0 || sample_rate_in_hertz == 0 {
        return interleaved_input.to_vec();
    }
    let input_frame_count = interleaved_input.len() / STEREO_CHANNEL_COUNT;
    let frame_length = ((FRAME_DURATION_IN_SECONDS * f64::from(sample_rate_in_hertz)) as usize)
        .next_multiple_of(2)
        .max(4);
    let hop = frame_length / 2;
    let search_radius = (SEARCH_RADIUS_IN_SECONDS * f64::from(sample_rate_in_hertz)) as usize;
    let output_frame_count = (input_frame_count as f64 / speed).round() as usize;
    let window = hann_window(frame_length);
    let mono: Vec<f32> = interleaved_input
        .as_chunks::<STEREO_CHANNEL_COUNT>()
        .0
        .iter()
        .map(|[left, right]| (left + right) / 2.0)
        .collect();

    let mut output = vec![0.0_f32; (output_frame_count + frame_length) * STEREO_CHANNEL_COUNT];
    // Where the previous piece was taken from, to find the best continuation.
    let mut previous_piece_start: Option<usize> = None;
    let mut piece_index = 0_usize;
    loop {
        let output_start = piece_index * hop;
        if output_start >= output_frame_count {
            break;
        }
        let nominal_input_start = (output_start as f64 * speed).round() as usize;
        let input_start = match previous_piece_start {
            // The first piece has nothing to line up with.
            None => nominal_input_start,
            Some(previous_start) => find_best_aligned_start(
                &mono,
                previous_start + hop,
                nominal_input_start,
                search_radius,
                hop,
            ),
        };
        add_windowed_piece(
            interleaved_input,
            input_start,
            &window,
            &mut output,
            output_start,
        );
        previous_piece_start = Some(input_start);
        piece_index += 1;
    }
    output.truncate(output_frame_count * STEREO_CHANNEL_COUNT);
    output
}

/// A periodic Hann window: shifted by half its length, two copies add up to 1.
fn hann_window(length: usize) -> Vec<f32> {
    (0..length)
        .map(|index| {
            let phase = std::f64::consts::TAU * index as f64 / length as f64;
            (0.5 - 0.5 * phase.cos()) as f32
        })
        .collect()
}

/// Adds the input piece starting at `input_start`, faded by `window`, to the
/// output at `output_start`. Input past the end counts as silence.
fn add_windowed_piece(
    interleaved_input: &[f32],
    input_start: usize,
    window: &[f32],
    output: &mut [f32],
    output_start: usize,
) {
    let input_frame_count = interleaved_input.len() / STEREO_CHANNEL_COUNT;
    for (offset, weight) in window.iter().enumerate() {
        let input_frame = input_start + offset;
        if input_frame >= input_frame_count {
            break;
        }
        for channel in 0..STEREO_CHANNEL_COUNT {
            let destination = (output_start + offset) * STEREO_CHANNEL_COUNT + channel;
            if let Some(slot) = output.get_mut(destination) {
                *slot += interleaved_input[input_frame * STEREO_CHANNEL_COUNT + channel] * weight;
            }
        }
    }
}

/// The start, within `search_radius` of `nominal_start`, whose first
/// `comparison_length` samples best match those at `natural_continuation`.
fn find_best_aligned_start(
    mono: &[f32],
    natural_continuation: usize,
    nominal_start: usize,
    search_radius: usize,
    comparison_length: usize,
) -> usize {
    let first_candidate = nominal_start.saturating_sub(search_radius);
    let last_candidate = nominal_start + search_radius;
    let score = |candidate: usize, step: usize| {
        normalised_cross_correlation(
            mono,
            natural_continuation,
            candidate,
            comparison_length,
            step,
        )
    };
    // Coarse search on a sparse grid, comparing every few samples…
    let mut best_start = nominal_start;
    let mut best_score = f32::NEG_INFINITY;
    for candidate in (first_candidate..=last_candidate).step_by(COARSE_SEARCH_STEP) {
        let candidate_score = score(candidate, COARSE_SEARCH_STEP);
        if candidate_score > best_score {
            (best_start, best_score) = (candidate, candidate_score);
        }
    }
    // …then every position and every sample around the best one.
    let refine_from = best_start
        .saturating_sub(COARSE_SEARCH_STEP - 1)
        .max(first_candidate);
    let refine_to = (best_start + COARSE_SEARCH_STEP - 1).min(last_candidate);
    best_score = f32::NEG_INFINITY;
    for candidate in refine_from..=refine_to {
        let candidate_score = score(candidate, 1);
        if candidate_score > best_score {
            (best_start, best_score) = (candidate, candidate_score);
        }
    }
    best_start
}

/// How much the samples at `candidate` look like those at `reference`, from −1
/// to 1, comparing every `step`-th sample. Past the end of the sound counts as
/// silence; silence matches nothing better than anything else (score 0).
fn normalised_cross_correlation(
    mono: &[f32],
    reference: usize,
    candidate: usize,
    length: usize,
    step: usize,
) -> f32 {
    let sample = |position: usize| mono.get(position).copied().unwrap_or(0.0);
    let (mut product_sum, mut reference_energy, mut candidate_energy) = (0.0_f32, 0.0_f32, 0.0_f32);
    for offset in (0..length).step_by(step) {
        let (reference_value, candidate_value) =
            (sample(reference + offset), sample(candidate + offset));
        product_sum += reference_value * candidate_value;
        reference_energy += reference_value * reference_value;
        candidate_energy += candidate_value * candidate_value;
    }
    let energy = (reference_energy * candidate_energy).sqrt();
    if energy <= f32::EPSILON {
        return 0.0;
    }
    product_sum / energy
}

#[cfg(test)]
mod tests {
    use kadr_core::time::{TimeSpan, Timestamp};

    use super::*;
    use crate::audio_buffer::AudioBuffer;
    use crate::test_patterns::{BeepPattern, find_beep_starts, tone_strength};

    const SAMPLE_RATE: u32 = 48_000;

    fn left_channel(interleaved: &[f32]) -> Vec<f32> {
        interleaved.iter().step_by(2).copied().collect()
    }

    fn continuous_tone(frequency_in_hertz: f32, seconds: u64) -> Vec<f32> {
        BeepPattern::continuous(
            frequency_in_hertz,
            0.5,
            TimeSpan::from_whole_seconds(seconds),
        )
        .render(SAMPLE_RATE, TimeSpan::from_whole_seconds(seconds))
        .interleaved_samples()
        .to_vec()
    }

    #[test]
    fn speed_one_changes_nothing() {
        let input = continuous_tone(440.0, 1);
        assert_eq!(stretch_stereo(&input, SAMPLE_RATE, 1.0), input);
    }

    #[test]
    fn duration_is_divided_by_the_speed() {
        let input = continuous_tone(440.0, 3);

        for (speed, expected_frames) in [(1.5, 96_000), (2.0, 72_000), (0.5, 288_000)] {
            let output = stretch_stereo(&input, SAMPLE_RATE, speed);
            assert_eq!(output.len(), expected_frames * 2, "speed {speed}");
        }
    }

    #[test]
    fn pitch_is_kept() {
        let input = continuous_tone(1_000.0, 2);

        for speed in [0.5, 1.5, 2.0] {
            let output = left_channel(&stretch_stereo(&input, SAMPLE_RATE, speed));
            let middle = &output[output.len() / 4..output.len() * 3 / 4];
            let at_original_pitch = tone_strength(middle, SAMPLE_RATE, 1_000.0);
            let at_shifted_pitch = tone_strength(middle, SAMPLE_RATE, (1_000.0 * speed) as f32);
            assert!(
                at_original_pitch > 0.2,
                "speed {speed}: {at_original_pitch}"
            );
            assert!(at_shifted_pitch < 0.01, "speed {speed}: {at_shifted_pitch}");
        }
    }

    #[test]
    fn beeps_move_with_the_speed_without_drifting() {
        let duration = TimeSpan::from_whole_seconds(8);
        let beeps =
            BeepPattern::every_second(1_000.0, 0.5, duration, TimeSpan::from_milliseconds(100));
        let input = beeps.render(SAMPLE_RATE, duration);

        let output = AudioBuffer::from_interleaved_stereo(
            SAMPLE_RATE,
            stretch_stereo(input.interleaved_samples(), SAMPLE_RATE, 2.0),
        );
        let found = find_beep_starts(&output, 1_000.0);

        // At 2×, the beeps at 0…7 s of the source come every half second.
        assert_eq!(found.len(), 8, "{found:?}");
        for (index, beep_start) in found.iter().enumerate() {
            let expected = Timestamp::from_milliseconds(500 * index as u64);
            let error_in_seconds = (beep_start.as_seconds() - expected.as_seconds()).abs();
            assert!(error_in_seconds < 0.015, "beep {index} at {beep_start}");
        }
    }

    #[test]
    fn silence_stays_silent() {
        let output = stretch_stereo(&vec![0.0; 2 * 10_000], SAMPLE_RATE, 1.5);
        assert!(output.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn hann_windows_shifted_by_half_add_up_to_one() {
        let window = hann_window(8);
        for index in 0..4 {
            assert!((window[index] + window[index + 4] - 1.0).abs() < 1e-6);
        }
    }
}
