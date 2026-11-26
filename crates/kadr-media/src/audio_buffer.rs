//! Decoded sound: stereo, 32-bit floating-point samples.
//!
//! Every sound Kadr handles internally is converted to this one format, whatever
//! the source file uses (mono, 5.1, 16-bit, 44.1 kHz…). It keeps mixing and
//! encoding simple.

use kadr_core::time::{TICKS_PER_SECOND, TimeSpan, Timestamp};

/// Kadr always works in stereo.
pub const STEREO_CHANNEL_COUNT: usize = 2;

/// Stereo sound at a given sample rate. Samples are interleaved
/// (left, right, left, right…) and nominally between −1 and 1.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioBuffer {
    sample_rate_in_hertz: u32,
    interleaved_samples: Vec<f32>,
}

impl AudioBuffer {
    /// Silence lasting `samples_per_channel` samples.
    pub fn silence(sample_rate_in_hertz: u32, samples_per_channel: usize) -> Self {
        Self {
            sample_rate_in_hertz,
            interleaved_samples: vec![0.0; samples_per_channel * STEREO_CHANNEL_COUNT],
        }
    }

    /// Wraps interleaved stereo samples. A trailing half sample (odd length) is dropped.
    pub fn from_interleaved_stereo(
        sample_rate_in_hertz: u32,
        mut interleaved_samples: Vec<f32>,
    ) -> Self {
        let whole_length =
            interleaved_samples.len() - interleaved_samples.len() % STEREO_CHANNEL_COUNT;
        interleaved_samples.truncate(whole_length);
        Self {
            sample_rate_in_hertz,
            interleaved_samples,
        }
    }

    /// Samples per second.
    pub fn sample_rate_in_hertz(&self) -> u32 {
        self.sample_rate_in_hertz
    }

    /// Number of samples in each channel.
    pub fn samples_per_channel(&self) -> usize {
        self.interleaved_samples.len() / STEREO_CHANNEL_COUNT
    }

    /// How long the sound lasts.
    pub fn duration(&self) -> TimeSpan {
        span_of_sample_count(self.samples_per_channel(), self.sample_rate_in_hertz)
    }

    /// The samples, interleaved left/right.
    pub fn interleaved_samples(&self) -> &[f32] {
        &self.interleaved_samples
    }

    /// The samples of the range `first_sample..first_sample + sample_count` (per
    /// channel), interleaved. Clamped to the buffer.
    pub fn interleaved_range(&self, first_sample: usize, sample_count: usize) -> &[f32] {
        let start = (first_sample * STEREO_CHANNEL_COUNT).min(self.interleaved_samples.len());
        let end = ((first_sample + sample_count) * STEREO_CHANNEL_COUNT)
            .min(self.interleaved_samples.len());
        &self.interleaved_samples[start..end]
    }

    /// Adds silence at the end, or cuts the end, to get exactly `samples_per_channel`.
    pub fn resize(&mut self, samples_per_channel: usize) {
        self.interleaved_samples
            .resize(samples_per_channel * STEREO_CHANNEL_COUNT, 0.0);
    }

    /// Adds `other` into this buffer, starting at sample `first_sample`, with a
    /// gain computed by `gain_at(sample index in other)`. Samples past the end
    /// of this buffer are ignored. Returns `false`, and mixes nothing, if the
    /// sample rates differ.
    pub fn mix_in(
        &mut self,
        other: &AudioBuffer,
        first_sample: usize,
        gain_at: impl Fn(usize) -> f32,
    ) -> bool {
        if other.sample_rate_in_hertz != self.sample_rate_in_hertz {
            return false;
        }
        let available_samples = self.samples_per_channel().saturating_sub(first_sample);
        let mixed_sample_count = other.samples_per_channel().min(available_samples);
        for sample_index in 0..mixed_sample_count {
            let gain = gain_at(sample_index);
            for channel in 0..STEREO_CHANNEL_COUNT {
                let destination = (first_sample + sample_index) * STEREO_CHANNEL_COUNT + channel;
                let source = sample_index * STEREO_CHANNEL_COUNT + channel;
                self.interleaved_samples[destination] += other.interleaved_samples[source] * gain;
            }
        }
        true
    }

    /// Keeps every sample between −1 and 1, to avoid wrap-around noise when encoding.
    pub fn clamp_to_full_scale(&mut self) {
        for sample in &mut self.interleaved_samples {
            *sample = sample.clamp(-1.0, 1.0);
        }
    }

    /// The left and right channels averaged, for analysis.
    pub fn to_mono(&self) -> Vec<f32> {
        let (stereo_pairs, _) = self.interleaved_samples.as_chunks::<STEREO_CHANNEL_COUNT>();
        stereo_pairs
            .iter()
            .map(|[left, right]| (left + right) / 2.0)
            .collect()
    }
}

/// The index of the sample playing at `time` (rounded down).
pub fn sample_index_at(time: Timestamp, sample_rate_in_hertz: u32) -> usize {
    let index =
        u128::from(time.ticks()) * u128::from(sample_rate_in_hertz) / u128::from(TICKS_PER_SECOND);
    usize::try_from(index).unwrap_or(usize::MAX)
}

/// How long `sample_count` samples last.
pub fn span_of_sample_count(sample_count: usize, sample_rate_in_hertz: u32) -> TimeSpan {
    if sample_rate_in_hertz == 0 {
        return TimeSpan::ZERO;
    }
    let ticks =
        sample_count as u128 * u128::from(TICKS_PER_SECOND) / u128::from(sample_rate_in_hertz);
    TimeSpan::from_ticks(u64::try_from(ticks).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_and_sample_positions_are_exact() {
        let buffer = AudioBuffer::silence(48_000, 96_000);
        assert_eq!(buffer.duration(), TimeSpan::from_whole_seconds(2));
        assert_eq!(
            sample_index_at(Timestamp::from_milliseconds(1_500), 48_000),
            72_000
        );
        assert_eq!(
            sample_index_at(Timestamp::from_milliseconds(1_500), 44_100),
            66_150
        );
    }

    #[test]
    fn mixing_adds_with_gain_at_an_offset() {
        let mut mix = AudioBuffer::silence(8_000, 4);
        let tone =
            AudioBuffer::from_interleaved_stereo(8_000, vec![1.0, -1.0, 1.0, -1.0, 1.0, -1.0]);

        assert!(mix.mix_in(&tone, 2, |_| 0.5));

        assert_eq!(
            mix.interleaved_samples(),
            &[0.0, 0.0, 0.0, 0.0, 0.5, -0.5, 0.5, -0.5]
        );
    }

    #[test]
    fn mixing_different_sample_rates_is_refused() {
        let mut mix = AudioBuffer::silence(48_000, 10);
        assert!(!mix.mix_in(&AudioBuffer::silence(44_100, 10), 0, |_| 1.0));
    }

    #[test]
    fn resizing_pads_with_silence_or_cuts() {
        let mut buffer = AudioBuffer::from_interleaved_stereo(8_000, vec![0.1, 0.2, 0.3]);
        assert_eq!(buffer.samples_per_channel(), 1);
        buffer.resize(3);
        assert_eq!(
            buffer.interleaved_samples(),
            &[0.1, 0.2, 0.0, 0.0, 0.0, 0.0]
        );
        buffer.resize(0);
        assert_eq!(buffer.samples_per_channel(), 0);
    }
}
