//! Writing the rendered video and sound into an MP4 file (H.264 + AAC) with the
//! phone's encoders.
//!
//! - The H.264 encoder is the one Android prefers for `video/avc` (the hardware
//!   one on phones), or Android's own software encoder for
//!   [`VideoEncoderPreference::SoftwareOnly`].
//! - Pictures are given in the encoder's own buffer layout, asked for as
//!   "flexible YUV 4:2:0" (which every Android encoder accepts) and described
//!   back by the encoder (see `picture_layout`).
//! - Each encoder announces its output format (with its set-up data) only after
//!   its first input. The MP4 writer needs both tracks before it starts, so the
//!   first compressed samples wait in memory until then.

use std::path::{Path, PathBuf};
use std::time::Duration;

use kadr_core::time::FrameRate;
use ndk::media::media_codec::{
    DequeuedInputBufferResult, InputBuffer, MediaCodec, MediaCodecDirection,
};
use ndk::media::media_format::MediaFormat;

use super::codec_support::{
    BUFFER_FLAG_CODEC_CONFIG, BUFFER_FLAG_END_OF_STREAM, CODEC_POLL_TIMEOUT, COLOR_RANGE_LIMITED,
    COLOR_STANDARD_BT709, COLOR_TRANSFER_SDR_VIDEO, CodecOutput, KEY_AAC_PROFILE, KEY_BIT_RATE,
    KEY_CHANNEL_COUNT, KEY_COLOR_FORMAT, KEY_COLOR_RANGE, KEY_COLOR_STANDARD, KEY_COLOR_TRANSFER,
    KEY_FRAME_RATE, KEY_HEIGHT, KEY_KEYFRAME_INTERVAL, KEY_MAXIMUM_INPUT_SIZE, KEY_MIME,
    KEY_SAMPLE_RATE, KEY_WIDTH, StallWatch, codec_failure, next_output, picture_layout_of,
};
use super::media_time::{frame_start_in_microseconds, sample_start_in_microseconds};
use super::muxer::Mp4Muxer;
use super::pcm::encode_signed_16_bit;
use super::picture_layout::{COLOR_FORMAT_YUV420_FLEXIBLE, PictureBufferLayout};
use crate::audio_buffer::{AudioBuffer, STEREO_CHANNEL_COUNT};
use crate::backend::{ExportSink, ExportSinkSettings, VideoEncoderPreference};
use crate::error::MediaError;
use crate::video_frame::{VideoFrame, YuvColorSpace};

/// The H.264 codec type.
const H264_MIME_TYPE: &str = "video/avc";
/// The AAC codec type.
const AAC_MIME_TYPE: &str = "audio/mp4a-latm";
/// Android's own software H.264 encoders: Codec 2 (Android 10 and later), then
/// OpenMAX (Android 9).
const SOFTWARE_H264_ENCODERS: [&str; 2] = ["c2.android.avc.encoder", "OMX.google.h264.encoder"];
/// Name reported when Android does not say which encoder it picked.
const UNNAMED_ENCODER: &str = "android-default-h264";

/// Bits per pixel per frame, as for the FFmpeg hardware encoders: about
/// 6 Mbit/s for 1080p at 30 fps.
const BITS_PER_PIXEL: f64 = 0.1;
/// Lowest video bit rate.
const MINIMUM_VIDEO_BIT_RATE: f64 = 1_000_000.0;
/// AAC bit rate for stereo.
const AUDIO_BIT_RATE: i32 = 192_000;
/// `MediaCodecInfo.CodecProfileLevel.AACObjectLC`: AAC Low Complexity.
const AAC_LOW_COMPLEXITY_PROFILE: i32 = 2;
/// One keyframe every this many seconds, so the file can be cut and seeked easily.
const SECONDS_BETWEEN_KEYFRAMES: i32 = 2;
/// Sound is given to the AAC encoder in pieces of this many samples per channel
/// (one AAC frame).
const AUDIO_SAMPLES_PER_INPUT: usize = 1_024;
/// Bytes of 16-bit stereo for one input piece, plus room.
const AUDIO_MAXIMUM_INPUT_SIZE: i32 =
    (AUDIO_SAMPLES_PER_INPUT * STEREO_CHANNEL_COUNT * 2 * 2) as i32;

/// Which half of the file a compressed sample belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrackRole {
    Video,
    Audio,
}

/// A compressed sample produced before the MP4 writer could start.
struct WaitingSample {
    role: TrackRole,
    data: Vec<u8>,
    presentation_time_in_microseconds: i64,
    flags: u32,
}

/// The MP4 writer, and the samples waiting for it to start.
struct MuxerState {
    muxer: Mp4Muxer,
    video_track_index: Option<usize>,
    audio_track_index: Option<usize>,
    waiting_samples: Vec<WaitingSample>,
}

impl MuxerState {
    /// Adds the track an encoder just described; starts writing once both are known.
    fn add_track(&mut self, role: TrackRole, format: &MediaFormat) -> Result<(), String> {
        let track_index = self.muxer.add_track(format)?;
        match role {
            TrackRole::Video => self.video_track_index = Some(track_index),
            TrackRole::Audio => self.audio_track_index = Some(track_index),
        }
        if self.video_track_index.is_some() && self.audio_track_index.is_some() {
            self.start_and_write_waiting_samples()?;
        }
        Ok(())
    }

    /// Starts the MP4 writer with the tracks known so far and writes what waited.
    fn start_and_write_waiting_samples(&mut self) -> Result<(), String> {
        if self.video_track_index.is_none() && self.audio_track_index.is_none() {
            return Err("neither encoder produced anything".to_string());
        }
        self.muxer.start()?;
        for sample in std::mem::take(&mut self.waiting_samples) {
            self.write(
                sample.role,
                &sample.data,
                sample.presentation_time_in_microseconds,
                sample.flags,
            )?;
        }
        Ok(())
    }

    /// Writes a compressed sample now, or keeps it until the writer starts.
    fn write(
        &mut self,
        role: TrackRole,
        data: &[u8],
        presentation_time_in_microseconds: i64,
        flags: u32,
    ) -> Result<(), String> {
        let track_index = match role {
            TrackRole::Video => self.video_track_index,
            TrackRole::Audio => self.audio_track_index,
        };
        match track_index {
            Some(track_index) if self.muxer.is_started() => {
                self.muxer
                    .write_sample(track_index, data, presentation_time_in_microseconds, flags)
            }
            _ => {
                self.waiting_samples.push(WaitingSample {
                    role,
                    data: data.to_vec(),
                    presentation_time_in_microseconds,
                    flags,
                });
                Ok(())
            }
        }
    }
}

/// What the encoders' output goes to: the MP4 writer, and whether each encoder
/// reached the end of its stream.
struct EncodedOutput {
    muxer_state: MuxerState,
    video_is_finished: bool,
    audio_is_finished: bool,
}

impl EncodedOutput {
    /// Writes what both encoders have finished, waiting up to `timeout` for each.
    fn drain_both(
        &mut self,
        video_encoder: &MediaCodec,
        audio_encoder: &MediaCodec,
        timeout: Duration,
    ) -> Result<(), String> {
        self.video_is_finished |= drain_encoder(
            video_encoder,
            TrackRole::Video,
            &mut self.muxer_state,
            timeout,
        )?;
        self.audio_is_finished |= drain_encoder(
            audio_encoder,
            TrackRole::Audio,
            &mut self.muxer_state,
            timeout,
        )?;
        Ok(())
    }
}

/// An MP4 file being written with `MediaCodec`.
pub(crate) struct MediaCodecExportSink {
    output_path: PathBuf,
    video_encoder: MediaCodec,
    video_encoder_name: String,
    video_layout: PictureBufferLayout,
    frame_rate: FrameRate,
    written_frame_count: u64,
    audio_encoder: MediaCodec,
    audio_sample_rate_in_hertz: u32,
    pending_interleaved_samples: Vec<f32>,
    written_sample_count: u64,
    output: EncodedOutput,
}

impl MediaCodecExportSink {
    /// Creates the file and starts the encoders.
    pub(crate) fn create(settings: &ExportSinkSettings) -> Result<Self, MediaError> {
        validate_settings(settings)?;
        let output_path = settings.output_path.clone();
        let muxer = Mp4Muxer::create(&output_path)
            .map_err(|reason| encoding_failure(&output_path, reason))?;
        let (video_encoder, video_encoder_name, video_layout) = open_video_encoder(settings)?;
        let audio_encoder = open_audio_encoder(settings)
            .map_err(|reason| encoding_failure(&output_path, reason))?;
        Ok(Self {
            output_path,
            video_encoder,
            video_encoder_name,
            video_layout,
            frame_rate: settings.frame_rate,
            written_frame_count: 0,
            audio_encoder,
            audio_sample_rate_in_hertz: settings.audio_sample_rate_in_hertz,
            pending_interleaved_samples: Vec::new(),
            written_sample_count: 0,
            output: EncodedOutput {
                muxer_state: MuxerState {
                    muxer,
                    video_track_index: None,
                    audio_track_index: None,
                    waiting_samples: Vec::new(),
                },
                video_is_finished: false,
                audio_is_finished: false,
            },
        })
    }

    /// Gives the next `samples_per_channel` samples of the pending sound to the
    /// AAC encoder, marked as the end of the sound when `is_last`.
    fn encode_audio_piece(
        &mut self,
        samples_per_channel: usize,
        is_last: bool,
    ) -> Result<(), MediaError> {
        let piece: Vec<f32> = self
            .pending_interleaved_samples
            .drain(..samples_per_channel * STEREO_CHANNEL_COUNT)
            .collect();
        let bytes = encode_signed_16_bit(&piece);
        let presentation_time = sample_start_in_microseconds(
            self.written_sample_count,
            self.audio_sample_rate_in_hertz,
        );
        let flags = if is_last {
            BUFFER_FLAG_END_OF_STREAM
        } else {
            0
        };
        let (video_encoder, audio_encoder, output) =
            (&self.video_encoder, &self.audio_encoder, &mut self.output);
        let mut input = wait_for_input_buffer(audio_encoder, || {
            output.drain_both(video_encoder, audio_encoder, CODEC_POLL_TIMEOUT)
        })
        .map_err(|reason| encoding_failure(&self.output_path, reason))?;
        let slots = input.buffer_mut();
        if bytes.len() > slots.len() {
            return Err(encoding_failure(
                &self.output_path,
                format!(
                    "the AAC encoder's input buffer holds {} bytes, {} are needed",
                    slots.len(),
                    bytes.len()
                ),
            ));
        }
        for (slot, byte) in slots.iter_mut().zip(&bytes) {
            slot.write(*byte);
        }
        audio_encoder
            .queue_input_buffer(
                input,
                0,
                bytes.len(),
                unsigned_time(presentation_time),
                flags,
            )
            .map_err(|error| {
                encoding_failure(
                    &self.output_path,
                    codec_failure("giving sound to the encoder", error),
                )
            })?;
        self.written_sample_count += samples_per_channel as u64;
        Ok(())
    }

    /// Tells the video encoder that no more pictures will come.
    fn end_video_input(&mut self) -> Result<(), MediaError> {
        let presentation_time =
            frame_start_in_microseconds(self.written_frame_count, self.frame_rate);
        let (video_encoder, audio_encoder, output) =
            (&self.video_encoder, &self.audio_encoder, &mut self.output);
        let input = wait_for_input_buffer(video_encoder, || {
            output.drain_both(video_encoder, audio_encoder, CODEC_POLL_TIMEOUT)
        })
        .map_err(|reason| encoding_failure(&self.output_path, reason))?;
        video_encoder
            .queue_input_buffer(
                input,
                0,
                0,
                unsigned_time(presentation_time),
                BUFFER_FLAG_END_OF_STREAM,
            )
            .map_err(|error| {
                encoding_failure(&self.output_path, codec_failure("ending the video", error))
            })
    }

    /// Writes what both encoders have finished, waiting up to `timeout` for each.
    fn drain_both_encoders(&mut self, timeout: Duration) -> Result<(), MediaError> {
        self.output
            .drain_both(&self.video_encoder, &self.audio_encoder, timeout)
            .map_err(|reason| encoding_failure(&self.output_path, reason))
    }
}

impl ExportSink for MediaCodecExportSink {
    fn video_encoder_name(&self) -> &str {
        &self.video_encoder_name
    }

    fn write_video_frame(&mut self, frame: &VideoFrame) -> Result<(), MediaError> {
        let (expected_width, expected_height) = (
            self.video_layout.coded_width,
            self.video_layout.coded_height,
        );
        if frame.width() as usize != expected_width || frame.height() as usize != expected_height {
            return Err(MediaError::InvalidRequest {
                reason: format!(
                    "the frame is {}×{} but the video is {expected_width}×{expected_height}",
                    frame.width(),
                    frame.height(),
                ),
            });
        }
        let picture = frame.to_yuv420(YuvColorSpace::BT709_LIMITED);
        let presentation_time =
            frame_start_in_microseconds(self.written_frame_count, self.frame_rate);
        let (video_encoder, audio_encoder, output) =
            (&self.video_encoder, &self.audio_encoder, &mut self.output);
        let mut input = wait_for_input_buffer(video_encoder, || {
            output.drain_both(video_encoder, audio_encoder, CODEC_POLL_TIMEOUT)
        })
        .map_err(|reason| encoding_failure(&self.output_path, reason))?;
        let size = self
            .video_layout
            .write_picture(&picture, input.buffer_mut())
            .map_err(|error| encoding_failure(&self.output_path, error.to_string()))?;
        video_encoder
            .queue_input_buffer(input, 0, size, unsigned_time(presentation_time), 0)
            .map_err(|error| {
                encoding_failure(
                    &self.output_path,
                    codec_failure("giving a picture to the encoder", error),
                )
            })?;
        self.written_frame_count += 1;
        self.drain_both_encoders(Duration::ZERO)
    }

    fn write_audio(&mut self, audio: &AudioBuffer) -> Result<(), MediaError> {
        if audio.sample_rate_in_hertz() != self.audio_sample_rate_in_hertz {
            return Err(MediaError::InvalidRequest {
                reason: format!(
                    "the sound is at {} Hz but the file is at {} Hz",
                    audio.sample_rate_in_hertz(),
                    self.audio_sample_rate_in_hertz
                ),
            });
        }
        self.pending_interleaved_samples
            .extend_from_slice(audio.interleaved_samples());
        while self.pending_interleaved_samples.len()
            >= AUDIO_SAMPLES_PER_INPUT * STEREO_CHANNEL_COUNT
        {
            self.encode_audio_piece(AUDIO_SAMPLES_PER_INPUT, false)?;
        }
        self.drain_both_encoders(Duration::ZERO)
    }

    fn finish(mut self: Box<Self>) -> Result<(), MediaError> {
        let remaining_samples = self.pending_interleaved_samples.len() / STEREO_CHANNEL_COUNT;
        self.encode_audio_piece(remaining_samples, true)?;
        self.end_video_input()?;
        let mut stall_watch = StallWatch::new();
        while !(self.output.video_is_finished && self.output.audio_is_finished) {
            let finished_before = (self.output.video_is_finished, self.output.audio_is_finished);
            self.drain_both_encoders(CODEC_POLL_TIMEOUT)?;
            if (self.output.video_is_finished, self.output.audio_is_finished) != finished_before {
                stall_watch.record_progress();
            }
            stall_watch
                .check("finishing the file")
                .map_err(|reason| encoding_failure(&self.output_path, reason))?;
        }
        if !self.output.muxer_state.muxer.is_started() {
            // An encoder never described its output (for example no sound at all).
            self.output
                .muxer_state
                .start_and_write_waiting_samples()
                .map_err(|reason| encoding_failure(&self.output_path, reason))?;
        }
        let Self {
            output_path,
            video_encoder,
            audio_encoder,
            output,
            ..
        } = *self;
        // The encoders close first: the MP4 writer must be the last.
        drop(video_encoder);
        drop(audio_encoder);
        output
            .muxer_state
            .muxer
            .stop()
            .map_err(|reason| encoding_failure(&output_path, reason))
    }
}

/// Writes the finished output of one encoder. Returns whether it reached the
/// end of its stream.
fn drain_encoder(
    encoder: &MediaCodec,
    role: TrackRole,
    muxer_state: &mut MuxerState,
    timeout: Duration,
) -> Result<bool, String> {
    loop {
        let buffer = match next_output(encoder, timeout)? {
            CodecOutput::Buffer(buffer) => buffer,
            CodecOutput::FormatChanged => {
                muxer_state.add_track(role, &encoder.output_format())?;
                continue;
            }
            CodecOutput::NothingYet => return Ok(false),
        };
        let information = *buffer.info();
        let is_set_up_data = information.flags() & BUFFER_FLAG_CODEC_CONFIG != 0;
        if !is_set_up_data && information.size() > 0 {
            let offset = usize::try_from(information.offset()).unwrap_or(0);
            let size = usize::try_from(information.size()).unwrap_or(0);
            let data = buffer
                .buffer()
                .get(offset..offset + size)
                .unwrap_or_default();
            muxer_state.write(
                role,
                data,
                information.presentation_time_us(),
                information.flags() & !BUFFER_FLAG_END_OF_STREAM,
            )?;
        }
        encoder
            .release_output_buffer(buffer, false)
            .map_err(|error| codec_failure("handing a buffer back to the encoder", error))?;
        if information.flags() & BUFFER_FLAG_END_OF_STREAM != 0 {
            return Ok(true);
        }
    }
}

/// Waits for a free input buffer of `encoder`, calling `while_waiting` (which
/// writes the encoders' output, so they never block each other) in between.
fn wait_for_input_buffer<'codec>(
    encoder: &'codec MediaCodec,
    mut while_waiting: impl FnMut() -> Result<(), String>,
) -> Result<InputBuffer<'codec>, String> {
    let mut stall_watch = StallWatch::new();
    loop {
        match encoder
            .dequeue_input_buffer(Duration::ZERO)
            .map_err(|error| codec_failure("waiting for an encoder", error))?
        {
            DequeuedInputBufferResult::Buffer(buffer) => return Ok(buffer),
            DequeuedInputBufferResult::TryAgainLater => {
                while_waiting()?;
                stall_watch.check("waiting for an encoder")?;
            }
        }
    }
}

/// A presentation time as MediaCodec takes it (never negative).
fn unsigned_time(presentation_time_in_microseconds: i64) -> u64 {
    u64::try_from(presentation_time_in_microseconds).unwrap_or(0)
}

/// Builds the "could not write" error for the output file.
fn encoding_failure(output_path: &Path, reason: impl Into<String>) -> MediaError {
    MediaError::EncodingFailed {
        file_path: output_path.to_path_buf(),
        reason: reason.into(),
    }
}

fn validate_settings(settings: &ExportSinkSettings) -> Result<(), MediaError> {
    let (width, height) = (settings.width_in_pixels, settings.height_in_pixels);
    if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
        return Err(MediaError::InvalidRequest {
            reason: format!("the video size must be even and non-zero, but is {width}×{height}"),
        });
    }
    if settings.audio_sample_rate_in_hertz == 0 {
        return Err(MediaError::InvalidRequest {
            reason: "the audio sample rate is zero".to_string(),
        });
    }
    Ok(())
}

/// The H.264 encoders to try, in order: `None` means "the one Android prefers".
fn video_encoder_candidates(preference: VideoEncoderPreference) -> Vec<Option<&'static str>> {
    let software = SOFTWARE_H264_ENCODERS.iter().copied().map(Some);
    match preference {
        VideoEncoderPreference::HardwareFirst => std::iter::once(None).chain(software).collect(),
        VideoEncoderPreference::SoftwareOnly => software.collect(),
    }
}

/// Opens the first H.264 encoder that starts and describes a usable input layout.
fn open_video_encoder(
    settings: &ExportSinkSettings,
) -> Result<(MediaCodec, String, PictureBufferLayout), MediaError> {
    let mut tried_encoder_names = Vec::new();
    for candidate in video_encoder_candidates(settings.video_encoder_preference) {
        let candidate_name = candidate.unwrap_or(UNNAMED_ENCODER);
        tried_encoder_names.push(candidate_name.to_string());
        let encoder = match candidate {
            Some(name) => MediaCodec::from_codec_name(name),
            None => MediaCodec::from_encoder_type(H264_MIME_TYPE),
        };
        let Some(encoder) = encoder else {
            continue;
        };
        let started = encoder
            .configure(&video_format(settings), None, MediaCodecDirection::Encoder)
            .and_then(|()| encoder.start());
        if started.is_err() {
            continue;
        }
        let Ok(layout) = picture_layout_of(&encoder.input_format()) else {
            continue;
        };
        let name = encoder
            .name()
            .unwrap_or_else(|_| candidate_name.to_string());
        return Ok((encoder, name, layout));
    }
    Err(MediaError::NoUsableVideoEncoder {
        tried_encoder_names,
    })
}

/// The H.264 encoder settings.
fn video_format(settings: &ExportSinkSettings) -> MediaFormat {
    let frames_per_second = settings.frame_rate.as_frames_per_second();
    let pixels_per_second = f64::from(settings.width_in_pixels)
        * f64::from(settings.height_in_pixels)
        * frames_per_second;
    let bit_rate = (pixels_per_second * BITS_PER_PIXEL).max(MINIMUM_VIDEO_BIT_RATE);
    let mut format = MediaFormat::new();
    format.set_str(KEY_MIME, H264_MIME_TYPE);
    format.set_i32(KEY_WIDTH, settings.width_in_pixels as i32);
    format.set_i32(KEY_HEIGHT, settings.height_in_pixels as i32);
    format.set_i32(KEY_COLOR_FORMAT, COLOR_FORMAT_YUV420_FLEXIBLE);
    format.set_i32(KEY_BIT_RATE, bit_rate.min(f64::from(i32::MAX)) as i32);
    format.set_f32(KEY_FRAME_RATE, frames_per_second as f32);
    format.set_i32(KEY_KEYFRAME_INTERVAL, SECONDS_BETWEEN_KEYFRAMES);
    // Kadr's exports are BT.709, limited range: say so in the stream.
    format.set_i32(KEY_COLOR_STANDARD, COLOR_STANDARD_BT709);
    format.set_i32(KEY_COLOR_RANGE, COLOR_RANGE_LIMITED);
    format.set_i32(KEY_COLOR_TRANSFER, COLOR_TRANSFER_SDR_VIDEO);
    format
}

/// Opens and starts the AAC encoder.
fn open_audio_encoder(settings: &ExportSinkSettings) -> Result<MediaCodec, String> {
    let mut format = MediaFormat::new();
    format.set_str(KEY_MIME, AAC_MIME_TYPE);
    format.set_i32(KEY_SAMPLE_RATE, settings.audio_sample_rate_in_hertz as i32);
    format.set_i32(KEY_CHANNEL_COUNT, STEREO_CHANNEL_COUNT as i32);
    format.set_i32(KEY_AAC_PROFILE, AAC_LOW_COMPLEXITY_PROFILE);
    format.set_i32(KEY_BIT_RATE, AUDIO_BIT_RATE);
    format.set_i32(KEY_MAXIMUM_INPUT_SIZE, AUDIO_MAXIMUM_INPUT_SIZE);
    let encoder =
        MediaCodec::from_encoder_type(AAC_MIME_TYPE).ok_or("this phone has no AAC encoder")?;
    encoder
        .configure(&format, None, MediaCodecDirection::Encoder)
        .map_err(|error| codec_failure("setting up the AAC encoder", error))?;
    encoder
        .start()
        .map_err(|error| codec_failure("starting the AAC encoder", error))?;
    Ok(encoder)
}
