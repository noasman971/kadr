//! Finding the silent parts of a sound, by measuring its loudness.
//!
//! This is the analysis behind "cut the silences": it tells where nobody speaks
//! and nothing plays, so an editor (or Claude) can remove those parts.
//!
//! ## How it works
//!
//! 1. The sound (mono samples from −1 to 1) is cut into short **analysis
//!    windows** of [`ANALYSIS_WINDOW_IN_SECONDS`] (10 ms).
//! 2. The loudness of each window is its **RMS level** (root mean square of the
//!    samples), expressed in **dBFS**: 0 dBFS is the loudest possible level,
//!    −20 dBFS is ten times quieter in amplitude, and pure silence is −∞.
//! 3. A window is *quiet* when its level is below the threshold
//!    ([`SilenceDetectionSettings::threshold_in_decibels`], −40 dBFS by default:
//!    quieter than normal speech, louder than the background noise of most
//!    recordings).
//! 4. Consecutive quiet windows form a quiet run. Runs at least
//!    [`SilenceDetectionSettings::minimum_silence_in_seconds`] long are reported
//!    as [`SilentRange`]s; shorter pauses (between words) are kept as sound.
//!
//! The RMS level over a window, rather than the value of single samples, makes
//! the detection robust: a waveform crosses zero hundreds of times per second
//! even in loud speech, but its RMS level does not.
//!
//! The detector is incremental ([`SilenceDetector`]): long files are analysed
//! piece by piece without holding all their samples in memory. Positions are
//! sample indices, so this module needs no time type and is tested alone on
//! synthetic signals.

/// Length of one analysis window, in seconds. Short enough to locate the edges
/// of a silence within 10 ms, long enough to contain a full period of any
/// audible frequency above 100 Hz.
pub const ANALYSIS_WINDOW_IN_SECONDS: f64 = 0.010;

/// Default threshold: quieter than this is silence. Normal speech is around
/// −20 dBFS, room noise in a home recording around −50 to −60 dBFS.
pub const DEFAULT_SILENCE_THRESHOLD_IN_DECIBELS: f32 = -40.0;

/// Default shortest silence worth reporting, in seconds. Pauses between words
/// last about 0.1 to 0.3 s; half a second is a pause a viewer notices.
pub const DEFAULT_MINIMUM_SILENCE_IN_SECONDS: f64 = 0.5;

/// How the silences are recognised.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SilenceDetectionSettings {
    /// Windows whose RMS level is below this many dBFS are quiet (for example −40).
    pub threshold_in_decibels: f32,
    /// Quiet runs shorter than this many seconds are not reported.
    pub minimum_silence_in_seconds: f64,
}

impl Default for SilenceDetectionSettings {
    fn default() -> Self {
        Self {
            threshold_in_decibels: DEFAULT_SILENCE_THRESHOLD_IN_DECIBELS,
            minimum_silence_in_seconds: DEFAULT_MINIMUM_SILENCE_IN_SECONDS,
        }
    }
}

/// A silent part of the sound, in samples from the start of the analysed sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SilentRange {
    /// First silent sample.
    pub start_sample: usize,
    /// First sample after the silence (excluded).
    pub end_sample: usize,
}

impl SilentRange {
    /// Number of silent samples.
    pub fn sample_count(&self) -> usize {
        self.end_sample - self.start_sample
    }
}

/// Finds silences in sound given piece by piece.
///
/// ```
/// use kadr_vision::silence_detection::{SilenceDetectionSettings, SilenceDetector};
///
/// let sample_rate_in_hertz = 16_000;
/// let one_second_of_silence = vec![0.0_f32; 16_000];
/// let one_second_of_tone: Vec<f32> = (0..16_000)
///     .map(|index| 0.5 * (index as f32 * 0.2).sin())
///     .collect();
///
/// let mut detector =
///     SilenceDetector::new(sample_rate_in_hertz, SilenceDetectionSettings::default());
/// detector.push_samples(&one_second_of_tone);
/// detector.push_samples(&one_second_of_silence);
/// detector.push_samples(&one_second_of_tone);
/// let silences = detector.finish();
///
/// assert_eq!(silences.len(), 1);
/// assert_eq!(silences[0].start_sample, 16_000);
/// assert_eq!(silences[0].end_sample, 32_000);
/// ```
#[derive(Debug, Clone)]
pub struct SilenceDetector {
    window_length_in_samples: usize,
    minimum_silence_in_samples: usize,
    /// Mean square below which a window is quiet (the threshold, converted once).
    quiet_mean_square_limit: f64,
    /// Samples of the window being filled, and the sum of their squares.
    samples_in_current_window: usize,
    sum_of_squares_in_current_window: f64,
    /// Samples received so far.
    total_sample_count: usize,
    /// Where the current quiet run started, if the last window was quiet.
    quiet_run_start_sample: Option<usize>,
    found_ranges: Vec<SilentRange>,
}

impl SilenceDetector {
    /// A detector for sound at `sample_rate_in_hertz`.
    pub fn new(sample_rate_in_hertz: u32, settings: SilenceDetectionSettings) -> Self {
        let sample_rate = f64::from(sample_rate_in_hertz.max(1));
        let window_length_in_samples = (sample_rate * ANALYSIS_WINDOW_IN_SECONDS).round().max(1.0);
        let minimum_silence_in_samples =
            (sample_rate * settings.minimum_silence_in_seconds.max(0.0)).round();
        Self {
            window_length_in_samples: window_length_in_samples as usize,
            minimum_silence_in_samples: minimum_silence_in_samples as usize,
            quiet_mean_square_limit: mean_square_for_decibels(settings.threshold_in_decibels),
            samples_in_current_window: 0,
            sum_of_squares_in_current_window: 0.0,
            total_sample_count: 0,
            quiet_run_start_sample: None,
            found_ranges: Vec::new(),
        }
    }

    /// Adds the next mono samples (from −1 to 1).
    pub fn push_samples(&mut self, mono_samples: &[f32]) {
        for &sample in mono_samples {
            self.sum_of_squares_in_current_window += f64::from(sample) * f64::from(sample);
            self.samples_in_current_window += 1;
            self.total_sample_count += 1;
            if self.samples_in_current_window == self.window_length_in_samples {
                self.close_current_window();
            }
        }
    }

    /// Ends the analysis and returns the silences found, in order. A last
    /// partial window counts like a full one.
    pub fn finish(mut self) -> Vec<SilentRange> {
        if self.samples_in_current_window > 0 {
            self.close_current_window();
        }
        self.end_quiet_run(self.total_sample_count);
        self.found_ranges
    }

    /// Classifies the window just filled as quiet or not.
    fn close_current_window(&mut self) {
        let window_start_sample = self.total_sample_count - self.samples_in_current_window;
        let mean_square =
            self.sum_of_squares_in_current_window / self.samples_in_current_window as f64;
        let is_quiet = mean_square < self.quiet_mean_square_limit;
        match (is_quiet, self.quiet_run_start_sample) {
            (true, None) => self.quiet_run_start_sample = Some(window_start_sample),
            (false, Some(_)) => self.end_quiet_run(window_start_sample),
            _ => {}
        }
        self.samples_in_current_window = 0;
        self.sum_of_squares_in_current_window = 0.0;
    }

    /// Ends the current quiet run at `end_sample`, keeping it if it is long enough.
    fn end_quiet_run(&mut self, end_sample: usize) {
        let Some(start_sample) = self.quiet_run_start_sample.take() else {
            return;
        };
        let range = SilentRange {
            start_sample,
            end_sample,
        };
        if range.sample_count() > 0 && range.sample_count() >= self.minimum_silence_in_samples {
            self.found_ranges.push(range);
        }
    }
}

/// Finds the silences of a whole sound at once. See [`SilenceDetector`].
pub fn detect_silent_ranges(
    mono_samples: &[f32],
    sample_rate_in_hertz: u32,
    settings: SilenceDetectionSettings,
) -> Vec<SilentRange> {
    let mut detector = SilenceDetector::new(sample_rate_in_hertz, settings);
    detector.push_samples(mono_samples);
    detector.finish()
}

/// The mean square of samples whose RMS level is `decibels` dBFS:
/// level = 20·log10(rms), so mean square = rms² = 10^(level / 10).
fn mean_square_for_decibels(decibels: f32) -> f64 {
    10_f64.powf(f64::from(decibels) / 10.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: u32 = 16_000;

    /// A sine tone at `amplitude` (RMS level = amplitude / √2).
    fn tone(duration_in_seconds: f64, amplitude: f32) -> Vec<f32> {
        let sample_count = (f64::from(SAMPLE_RATE) * duration_in_seconds) as usize;
        (0..sample_count)
            .map(|index| {
                let phase = index as f32 / SAMPLE_RATE as f32 * 440.0 * std::f32::consts::TAU;
                amplitude * phase.sin()
            })
            .collect()
    }

    fn silence(duration_in_seconds: f64) -> Vec<f32> {
        vec![0.0; (f64::from(SAMPLE_RATE) * duration_in_seconds) as usize]
    }

    fn seconds(sample: usize) -> f64 {
        sample as f64 / f64::from(SAMPLE_RATE)
    }

    #[test]
    fn finds_a_silence_between_two_sounds_to_the_window() {
        let sound = [tone(1.0, 0.5), silence(1.5), tone(1.0, 0.5)].concat();
        let ranges = detect_silent_ranges(&sound, SAMPLE_RATE, SilenceDetectionSettings::default());
        assert_eq!(ranges.len(), 1);
        assert!((seconds(ranges[0].start_sample) - 1.0).abs() <= ANALYSIS_WINDOW_IN_SECONDS);
        assert!((seconds(ranges[0].end_sample) - 2.5).abs() <= ANALYSIS_WINDOW_IN_SECONDS);
    }

    #[test]
    fn pauses_shorter_than_the_minimum_are_kept_as_sound() {
        let sound = [tone(1.0, 0.5), silence(0.2), tone(1.0, 0.5)].concat();
        let ranges = detect_silent_ranges(&sound, SAMPLE_RATE, SilenceDetectionSettings::default());
        assert!(ranges.is_empty(), "{ranges:?}");
    }

    #[test]
    fn silences_at_the_start_and_the_end_are_found() {
        let sound = [silence(1.0), tone(1.0, 0.5), silence(0.75)].concat();
        let ranges = detect_silent_ranges(&sound, SAMPLE_RATE, SilenceDetectionSettings::default());
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].start_sample, 0);
        assert_eq!(ranges[1].end_sample, sound.len());
    }

    #[test]
    fn quiet_noise_under_the_threshold_counts_as_silence() {
        // RMS of 0.002 is about −57 dBFS: background noise.
        let sound = [
            tone(1.0, 0.5),
            tone(1.0, 0.002 * std::f32::consts::SQRT_2),
            tone(1.0, 0.5),
        ]
        .concat();
        let ranges = detect_silent_ranges(&sound, SAMPLE_RATE, SilenceDetectionSettings::default());
        assert_eq!(ranges.len(), 1);

        // With a stricter threshold, the same noise is sound.
        let strict = SilenceDetectionSettings {
            threshold_in_decibels: -70.0,
            ..SilenceDetectionSettings::default()
        };
        assert!(detect_silent_ranges(&sound, SAMPLE_RATE, strict).is_empty());
    }

    #[test]
    fn feeding_the_sound_in_pieces_gives_the_same_result() {
        let sound = [tone(0.7, 0.5), silence(1.3), tone(0.9, 0.3), silence(0.6)].concat();
        let settings = SilenceDetectionSettings::default();
        let at_once = detect_silent_ranges(&sound, SAMPLE_RATE, settings);

        let mut detector = SilenceDetector::new(SAMPLE_RATE, settings);
        for piece in sound.chunks(1_237) {
            detector.push_samples(piece);
        }
        assert_eq!(detector.finish(), at_once);
        assert_eq!(at_once.len(), 2);
    }

    #[test]
    fn an_empty_sound_has_no_silence() {
        assert!(
            detect_silent_ranges(&[], SAMPLE_RATE, SilenceDetectionSettings::default()).is_empty()
        );
    }
}
