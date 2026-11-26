//! Decoding a range of a file's sound, at any speed, to stereo, on Android.
//!
//! The FFmpeg backend does this with one filter graph. Here the same steps are
//! done one after the other:
//!
//! ```text
//! extractor + decoder (range and a margin around it) → PCM to stereo floats
//!   → time stretch keeping pitch (speed) → resampling (project rate)
//!   → margin removed, padded or cut to the exact length
//! ```
//!
//! The margin gives the resampler and the time stretch real sound on both sides
//! of the range, so its first and last milliseconds are not faded.

use std::path::Path;

use kadr_core::time::{TimeSpan, Timestamp};
use ndk::media::media_codec::MediaCodec;
use ndk::media::media_format::MediaFormat;

use super::codec_support::{
    BUFFER_FLAG_END_OF_STREAM, CODEC_POLL_TIMEOUT, CodecOutput, FeedOutcome, KEY_CHANNEL_COUNT,
    KEY_PCM_ENCODING, KEY_SAMPLE_RATE, StallWatch, feed_decoder, next_output, start_decoder,
};
use super::extractor::MediaExtractor;
use super::media_time::{microseconds_from_timestamp, sample_start_in_microseconds};
use super::pcm::{PCM_ENCODING_SIGNED_16_BIT, PcmEncoding, decode_to_stereo};
use super::probe::{TrackKind, find_track};
use super::resampler::StereoResampler;
use super::time_stretch::stretch_stereo;
use crate::audio_buffer::{AudioBuffer, STEREO_CHANNEL_COUNT};
use crate::backend::AudioSegmentRequest;
use crate::error::MediaError;

/// Sound decoded on each side of the requested range.
const DECODING_MARGIN: TimeSpan = TimeSpan::from_milliseconds(100);
/// A gap between two decoded buffers larger than this is filled with silence.
const LARGEST_IGNORED_GAP_IN_MICROSECONDS: i64 = 1_000;

/// Decodes the requested range of a file's sound.
pub(crate) fn decode_audio_segment(
    request: &AudioSegmentRequest,
) -> Result<AudioBuffer, MediaError> {
    validate_request(request)?;
    let file_path = request.file_path.as_path();
    let decode_from = request
        .source_start
        .checked_sub(DECODING_MARGIN)
        .unwrap_or(Timestamp::ZERO);
    let decode_until = request.source_end + DECODING_MARGIN;
    let decoded = decode_source_range(file_path, decode_from, decode_until)?;
    Ok(fit_to_request(decoded, request))
}

fn validate_request(request: &AudioSegmentRequest) -> Result<(), MediaError> {
    if request.source_start >= request.source_end {
        return Err(MediaError::InvalidRequest {
            reason: format!(
                "the sound range starts at {} and ends at {}",
                request.source_start, request.source_end
            ),
        });
    }
    if request.output_sample_rate_in_hertz == 0 {
        return Err(MediaError::InvalidRequest {
            reason: "the output sample rate is zero".to_string(),
        });
    }
    Ok(())
}

/// Decoded stereo sound and where it starts in the file.
struct DecodedSound {
    interleaved_samples: Vec<f32>,
    sample_rate_in_hertz: u32,
    /// Source time of the first sample, in microseconds (may be before or after
    /// the decoded range's start: sound comes in whole compressed frames).
    first_sample_time_in_microseconds: i64,
}

/// How the decoder delivers its sound.
#[derive(Debug, Clone, Copy)]
struct PcmFormat {
    sample_rate_in_hertz: u32,
    channel_count: usize,
    encoding: PcmEncoding,
}

/// The state of one decoding pass.
struct SoundDecoder<'path> {
    file_path: &'path Path,
    extractor: MediaExtractor,
    decoder: MediaCodec,
    stream_start_in_microseconds: i64,
    pcm_format: Option<PcmFormat>,
    input_ended: bool,
    sound: DecodedSound,
}

/// Decodes the sound of `file_path` from `decode_from` to at least `decode_until`.
fn decode_source_range(
    file_path: &Path,
    decode_from: Timestamp,
    decode_until: Timestamp,
) -> Result<DecodedSound, MediaError> {
    let mut extractor = MediaExtractor::open(file_path)?;
    let (track_index, mut track_format) =
        find_track(&extractor, TrackKind::Audio).ok_or_else(|| MediaError::NoAudioStream {
            file_path: file_path.to_path_buf(),
        })?;
    extractor
        .select_track(track_index)
        .map_err(|reason| decoding_failure(file_path, reason))?;
    let stream_start_in_microseconds = extractor.sample_time_in_microseconds().unwrap_or(0).max(0);
    if decode_from > Timestamp::ZERO {
        extractor
            .seek_to_keyframe_before(
                stream_start_in_microseconds + microseconds_from_timestamp(decode_from),
            )
            .map_err(|reason| decoding_failure(file_path, reason))?;
    }
    let decoder =
        start_decoder(&mut track_format).map_err(|reason| decoding_failure(file_path, reason))?;
    let mut sound_decoder = SoundDecoder {
        file_path,
        extractor,
        decoder,
        stream_start_in_microseconds,
        pcm_format: None,
        input_ended: false,
        sound: DecodedSound {
            interleaved_samples: Vec::new(),
            sample_rate_in_hertz: 0,
            first_sample_time_in_microseconds: microseconds_from_timestamp(decode_from),
        },
    };
    sound_decoder.decode_until(microseconds_from_timestamp(decode_until))?;
    Ok(sound_decoder.sound)
}

impl SoundDecoder<'_> {
    /// Feeds the decoder and collects its sound until `end_in_microseconds` (source
    /// time) is passed or the file ends.
    fn decode_until(&mut self, end_in_microseconds: i64) -> Result<(), MediaError> {
        let mut stall_watch = StallWatch::new();
        let mut is_first_buffer = true;
        loop {
            if !self.input_ended {
                match feed_decoder(&self.decoder, &mut self.extractor)
                    .map_err(|reason| decoding_failure(self.file_path, reason))?
                {
                    FeedOutcome::SampleGiven => stall_watch.record_progress(),
                    FeedOutcome::EndOfStreamGiven => self.input_ended = true,
                    FeedOutcome::DecoderBusy => {}
                }
            }
            let output = next_output(&self.decoder, CODEC_POLL_TIMEOUT)
                .map_err(|reason| decoding_failure(self.file_path, reason))?;
            let buffer = match output {
                CodecOutput::Buffer(buffer) => buffer,
                CodecOutput::FormatChanged => {
                    self.pcm_format = Some(self.read_pcm_format()?);
                    continue;
                }
                CodecOutput::NothingYet => {
                    stall_watch
                        .check("decoding the sound")
                        .map_err(|reason| decoding_failure(self.file_path, reason))?;
                    continue;
                }
            };
            stall_watch.record_progress();
            let information = *buffer.info();
            let buffer_time =
                information.presentation_time_us() - self.stream_start_in_microseconds;
            let is_end_of_stream = information.flags() & BUFFER_FLAG_END_OF_STREAM != 0;
            let decoded_stereo = if information.size() > 0 {
                let pcm_format = match self.pcm_format {
                    Some(pcm_format) => pcm_format,
                    None => self.read_pcm_format()?,
                };
                self.pcm_format = Some(pcm_format);
                let offset = usize::try_from(information.offset()).unwrap_or(0);
                let size = usize::try_from(information.size()).unwrap_or(0);
                let pcm_bytes = buffer
                    .buffer()
                    .get(offset..offset + size)
                    .unwrap_or_default();
                Some((
                    pcm_format.sample_rate_in_hertz,
                    decode_to_stereo(pcm_bytes, pcm_format.encoding, pcm_format.channel_count),
                ))
            } else {
                None
            };
            self.decoder
                .release_output_buffer(buffer, false)
                .map_err(|error| {
                    decoding_failure(self.file_path, format!("handing sound back: {error}"))
                })?;
            if let Some((sample_rate_in_hertz, stereo)) = decoded_stereo {
                if is_first_buffer {
                    self.sound.first_sample_time_in_microseconds = buffer_time;
                    self.sound.sample_rate_in_hertz = sample_rate_in_hertz;
                    is_first_buffer = false;
                }
                self.append_at(buffer_time, &stereo);
            }
            if is_end_of_stream || buffer_time > end_in_microseconds {
                return Ok(());
            }
        }
    }

    /// Adds decoded sound starting at `buffer_time`, filling any gap with silence.
    fn append_at(&mut self, buffer_time_in_microseconds: i64, stereo: &[f32]) {
        let sample_rate = self.sound.sample_rate_in_hertz;
        let decoded_frames = self.sound.interleaved_samples.len() / STEREO_CHANNEL_COUNT;
        let expected_time = self.sound.first_sample_time_in_microseconds
            + sample_start_in_microseconds(decoded_frames as u64, sample_rate);
        let gap_in_microseconds = buffer_time_in_microseconds - expected_time;
        if gap_in_microseconds > LARGEST_IGNORED_GAP_IN_MICROSECONDS {
            let missing_frames =
                (gap_in_microseconds as u128 * u128::from(sample_rate) / 1_000_000) as usize;
            self.sound.interleaved_samples.resize(
                (decoded_frames + missing_frames) * STEREO_CHANNEL_COUNT,
                0.0,
            );
        }
        self.sound.interleaved_samples.extend_from_slice(stereo);
    }

    /// Reads the decoder's current sample rate, channel count and sample encoding.
    fn read_pcm_format(&self) -> Result<PcmFormat, MediaError> {
        let output_format: MediaFormat = self.decoder.output_format();
        let sample_rate_in_hertz = output_format
            .i32(KEY_SAMPLE_RATE)
            .and_then(|rate| u32::try_from(rate).ok())
            .filter(|rate| *rate > 0)
            .ok_or_else(|| decoding_failure(self.file_path, "the decoder gives no sample rate"))?;
        let channel_count = output_format
            .i32(KEY_CHANNEL_COUNT)
            .and_then(|count| usize::try_from(count).ok())
            .filter(|count| *count > 0)
            .unwrap_or(STEREO_CHANNEL_COUNT);
        let encoding_value = output_format
            .i32(KEY_PCM_ENCODING)
            .unwrap_or(PCM_ENCODING_SIGNED_16_BIT);
        let encoding = PcmEncoding::from_android_value(encoding_value).ok_or_else(|| {
            decoding_failure(
                self.file_path,
                format!("the decoder gives sound in an unknown encoding ({encoding_value})"),
            )
        })?;
        Ok(PcmFormat {
            sample_rate_in_hertz,
            channel_count,
            encoding,
        })
    }
}

/// Cuts the decoded sound to the request: speed, sample rate and exact length.
fn fit_to_request(decoded: DecodedSound, request: &AudioSegmentRequest) -> AudioBuffer {
    let output_rate = request.output_sample_rate_in_hertz;
    if decoded.sample_rate_in_hertz == 0 {
        // Nothing was decoded (the range is past the end of the sound).
        return AudioBuffer::silence(output_rate, request.output_samples_per_channel);
    }
    let source_rate = decoded.sample_rate_in_hertz;
    let speed = request.speed.as_multiplier();
    // Samples between the start of the decoded sound and the start of the range;
    // negative when the sound starts after the range does.
    let lead_in_microseconds = microseconds_from_timestamp(request.source_start)
        - decoded.first_sample_time_in_microseconds;
    let lead_in_source_samples = lead_in_microseconds * i64::from(source_rate) / 1_000_000;
    let mut samples = decoded.interleaved_samples;
    if lead_in_source_samples < 0 {
        let silence_frames = lead_in_source_samples.unsigned_abs() as usize;
        samples.splice(
            0..0,
            std::iter::repeat_n(0.0, silence_frames * STEREO_CHANNEL_COUNT),
        );
    }
    let lead_in_source_samples = lead_in_source_samples.max(0) as f64;

    let stretched = stretch_stereo(&samples, source_rate, speed);
    let resampled = StereoResampler::new(source_rate, output_rate).resample(&stretched);
    let lead_in_output_samples = (lead_in_source_samples / speed * f64::from(output_rate)
        / f64::from(source_rate))
    .round() as usize;

    let start = (lead_in_output_samples * STEREO_CHANNEL_COUNT).min(resampled.len());
    let end =
        (start + request.output_samples_per_channel * STEREO_CHANNEL_COUNT).min(resampled.len());
    let mut result =
        AudioBuffer::from_interleaved_stereo(output_rate, resampled[start..end].to_vec());
    result.resize(request.output_samples_per_channel);
    result
}

/// Builds the "could not decode" error for a sound.
fn decoding_failure(file_path: &Path, reason: impl Into<String>) -> MediaError {
    MediaError::DecodingFailed {
        file_path: file_path.to_path_buf(),
        reason: reason.into(),
    }
}
