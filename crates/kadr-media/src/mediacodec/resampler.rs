//! Changing the sample rate of stereo sound (for example 44.1 kHz → 48 kHz).
//!
//! FFmpeg's `aresample` does this on desktop; on Android, Kadr does it itself with
//! the classic **windowed-sinc interpolation** (J. O. Smith, "Digital Audio
//! Resampling", CCRMA, 2002):
//!
//! 1. Each output sample sits at a fractional position of the input
//!    (`output_index × input_rate / output_rate`).
//! 2. Its value is the sum of the input samples around that position, each
//!    weighted by the ideal low-pass filter `sinc`, cut to a few samples on each
//!    side by a Blackman window so that it stays short.
//! 3. When lowering the rate, the filter's cut-off moves down to the new Nyquist
//!    frequency, so that frequencies the new rate cannot hold are removed instead of
//!    folding back as noise (aliasing).
//!
//! The weights are precomputed in a table (`KERNEL_STEPS_PER_SAMPLE` steps per
//! input sample) and interpolated linearly.

use std::f64::consts::PI;

use crate::audio_buffer::STEREO_CHANNEL_COUNT;

/// Input samples used on each side of an output position, at the same or a
/// higher rate. More is cleaner and slower; 16 keeps every alias below −80 dB.
const KERNEL_HALF_WIDTH_IN_SAMPLES: usize = 16;
/// Resolution of the precomputed kernel table.
const KERNEL_STEPS_PER_SAMPLE: usize = 256;
/// Slightly below the Nyquist frequency, so the window's transition band stays
/// out of the audible top octave's aliases.
const CUT_OFF_FRACTION_OF_NYQUIST: f64 = 0.95;

/// A sample-rate converter between two fixed rates.
#[derive(Debug, Clone)]
pub(crate) struct StereoResampler {
    input_rate_in_hertz: u32,
    output_rate_in_hertz: u32,
    /// Cut-off frequency relative to the input Nyquist frequency.
    cut_off: f64,
    /// Input samples used on each side, widened when lowering the rate.
    half_width_in_input_samples: usize,
    /// The windowed sinc from distance 0 to `half_width_in_input_samples`.
    kernel_table: Vec<f32>,
}

impl StereoResampler {
    /// A converter from `input_rate_in_hertz` to `output_rate_in_hertz`.
    pub(crate) fn new(input_rate_in_hertz: u32, output_rate_in_hertz: u32) -> Self {
        let rate_ratio = f64::from(output_rate_in_hertz) / f64::from(input_rate_in_hertz.max(1));
        let cut_off = rate_ratio.min(1.0) * CUT_OFF_FRACTION_OF_NYQUIST;
        // A lower cut-off needs a proportionally longer filter for the same sharpness.
        let half_width_in_input_samples =
            (KERNEL_HALF_WIDTH_IN_SAMPLES as f64 / rate_ratio.min(1.0)).ceil() as usize;
        let table_length = half_width_in_input_samples * KERNEL_STEPS_PER_SAMPLE + 1;
        let kernel_table = (0..table_length)
            .map(|step| {
                let distance = step as f64 / KERNEL_STEPS_PER_SAMPLE as f64;
                windowed_sinc(distance, cut_off, half_width_in_input_samples as f64) as f32
            })
            .collect();
        Self {
            input_rate_in_hertz,
            output_rate_in_hertz,
            cut_off,
            half_width_in_input_samples,
            kernel_table,
        }
    }

    /// Converts interleaved stereo samples. The result lasts as long as the
    /// input (rounded to whole output samples).
    pub(crate) fn resample(&self, interleaved_input: &[f32]) -> Vec<f32> {
        if self.input_rate_in_hertz == self.output_rate_in_hertz || self.input_rate_in_hertz == 0 {
            return interleaved_input.to_vec();
        }
        let input_frame_count = interleaved_input.len() / STEREO_CHANNEL_COUNT;
        let output_frame_count = (input_frame_count as u64 * u64::from(self.output_rate_in_hertz))
            .div_ceil(u64::from(self.input_rate_in_hertz))
            as usize;
        let mut output = Vec::with_capacity(output_frame_count * STEREO_CHANNEL_COUNT);
        for output_index in 0..output_frame_count {
            // Exact position in the input, as a whole part and a fraction.
            let scaled_position = output_index as u64 * u64::from(self.input_rate_in_hertz);
            let whole_position = (scaled_position / u64::from(self.output_rate_in_hertz)) as usize;
            let fraction = (scaled_position % u64::from(self.output_rate_in_hertz)) as f64
                / f64::from(self.output_rate_in_hertz);
            output.extend(self.interpolate_at(interleaved_input, whole_position, fraction));
        }
        output
    }

    /// The value of both channels at `whole_position + fraction` in the input.
    fn interpolate_at(
        &self,
        interleaved_input: &[f32],
        whole_position: usize,
        fraction: f64,
    ) -> [f32; 2] {
        let input_frame_count = interleaved_input.len() / STEREO_CHANNEL_COUNT;
        let first_tap = whole_position.saturating_sub(self.half_width_in_input_samples - 1);
        let last_tap = (whole_position + self.half_width_in_input_samples)
            .min(input_frame_count.saturating_sub(1));
        let mut sums = [0.0_f32; 2];
        for tap in first_tap..=last_tap {
            let distance = (tap as f64 - whole_position as f64 - fraction).abs();
            let weight = self.kernel_at(distance);
            sums[0] += interleaved_input[tap * STEREO_CHANNEL_COUNT] * weight;
            sums[1] += interleaved_input[tap * STEREO_CHANNEL_COUNT + 1] * weight;
        }
        sums
    }

    /// The kernel at `distance` input samples, interpolated in the table.
    fn kernel_at(&self, distance: f64) -> f32 {
        let table_position = distance * KERNEL_STEPS_PER_SAMPLE as f64;
        let index = table_position.floor() as usize;
        let Some(&before) = self.kernel_table.get(index) else {
            return 0.0;
        };
        let after = self.kernel_table.get(index + 1).copied().unwrap_or(0.0);
        let blend = (table_position - index as f64) as f32;
        before + (after - before) * blend
    }

    /// The filter's cut-off, relative to the input Nyquist frequency.
    #[cfg(test)]
    fn cut_off(&self) -> f64 {
        self.cut_off
    }
}

/// `cut_off · sinc(cut_off · distance)` times a Blackman window reaching zero at
/// `half_width`. Integrates to 1, so a constant signal keeps its level.
fn windowed_sinc(distance: f64, cut_off: f64, half_width: f64) -> f64 {
    if distance >= half_width {
        return 0.0;
    }
    let argument = PI * cut_off * distance;
    let sinc = if argument.abs() < 1e-12 {
        1.0
    } else {
        argument.sin() / argument
    };
    // Blackman window over [−half_width, half_width], centred on zero.
    let phase = PI * distance / half_width;
    let window = 0.42 + 0.5 * phase.cos() + 0.08 * (2.0 * phase).cos();
    cut_off * sinc * window
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_patterns::tone_strength;

    fn stereo_sine(
        frequency_in_hertz: f64,
        sample_rate_in_hertz: u32,
        sample_count: usize,
    ) -> Vec<f32> {
        (0..sample_count)
            .flat_map(|index| {
                let value = (0.5
                    * (2.0 * PI * frequency_in_hertz * index as f64
                        / f64::from(sample_rate_in_hertz))
                    .sin()) as f32;
                [value, value]
            })
            .collect()
    }

    fn left_channel(interleaved: &[f32]) -> Vec<f32> {
        interleaved.iter().step_by(2).copied().collect()
    }

    #[test]
    fn same_rate_is_a_copy() {
        let input = stereo_sine(440.0, 48_000, 100);
        assert_eq!(StereoResampler::new(48_000, 48_000).resample(&input), input);
    }

    #[test]
    fn length_follows_the_rate_ratio() {
        let input = stereo_sine(440.0, 44_100, 44_100);

        let output = StereoResampler::new(44_100, 48_000).resample(&input);

        assert_eq!(output.len(), 48_000 * 2);
    }

    #[test]
    fn a_tone_keeps_its_pitch_and_level() {
        let input = stereo_sine(1_000.0, 44_100, 44_100);

        let output = StereoResampler::new(44_100, 48_000).resample(&input);
        // Away from the edges, where the filter runs out of input.
        let middle = &left_channel(&output)[4_800..43_200];

        let strength_at_tone = tone_strength(middle, 48_000, 1_000.0);
        let strength_off_tone = tone_strength(middle, 48_000, 1_300.0);
        assert!((strength_at_tone - 0.25).abs() < 0.02, "{strength_at_tone}");
        assert!(strength_off_tone < 0.001, "{strength_off_tone}");
    }

    #[test]
    fn constant_sound_keeps_its_level() {
        let input = vec![0.3_f32; 2 * 1_000];

        let output = StereoResampler::new(48_000, 44_100).resample(&input);

        for sample in &output[200..1_600] {
            assert!((sample - 0.3).abs() < 0.003, "{sample}");
        }
    }

    #[test]
    fn lowering_the_rate_removes_what_the_new_rate_cannot_hold() {
        // 20 kHz cannot exist at 22.05 kHz (Nyquist 11.025 kHz): it must vanish,
        // not fold back to 2.05 kHz.
        let input = stereo_sine(20_000.0, 44_100, 44_100);
        let resampler = StereoResampler::new(44_100, 22_050);

        let output = resampler.resample(&input);
        let middle = &left_channel(&output)[2_000..20_000];

        assert!(resampler.cut_off() < 0.5);
        assert!(tone_strength(middle, 22_050, 2_050.0) < 0.000_1);
    }

    #[test]
    fn the_kernel_is_zero_beyond_its_width() {
        assert_eq!(windowed_sinc(16.0, 0.95, 16.0), 0.0);
        assert!((windowed_sinc(0.0, 0.95, 16.0) - 0.95).abs() < 1e-12);
    }
}
