//! What the decoders and encoders of the Android backend share: the names of
//! format entries, reading picture layouts and colour spaces from a format, and
//! the synchronous "give a sample, take a result" loop of `MediaCodec`.
//!
//! Codecs run in synchronous mode: Kadr asks for a free input buffer, fills it,
//! then asks for a finished output buffer, with short timeouts. A codec that
//! makes no progress for [`STALLED_CODEC_TIMEOUT`] is reported as stuck instead
//! of blocking the export forever.

use std::time::{Duration, Instant};

use ndk::media::media_codec::{
    DequeuedInputBufferResult, DequeuedOutputBufferInfoResult, MediaCodec, MediaCodecDirection,
    OutputBuffer,
};
use ndk::media::media_format::MediaFormat;

use super::extractor::MediaExtractor;
use super::picture_layout::{
    CropRectangle, PictureBufferLayout, PictureFormatEntries, PictureLayoutError,
};
use crate::video_frame::{YuvColorSpace, YuvMatrix, YuvRange};

/// Format entry: the codec type, for example `video/avc`.
pub(crate) const KEY_MIME: &str = "mime";
/// Format entry: picture width.
pub(crate) const KEY_WIDTH: &str = "width";
/// Format entry: picture height.
pub(crate) const KEY_HEIGHT: &str = "height";
/// Format entry: how pixels are stored (see `picture_layout`).
pub(crate) const KEY_COLOR_FORMAT: &str = "color-format";
/// Format entry: bytes from one row to the next.
const KEY_STRIDE: &str = "stride";
/// Format entry: rows from the Y plane to the colour planes.
const KEY_SLICE_HEIGHT: &str = "slice-height";
/// Format entry: the visible part of the coded picture (inclusive bounds).
const KEY_CROP: &str = "crop";
/// Format entry: the exact layout, as a `MediaImage2` structure.
const KEY_IMAGE_DATA: &str = "image-data";
/// Format entry: `MediaFormat.COLOR_STANDARD_*`.
pub(crate) const KEY_COLOR_STANDARD: &str = "color-standard";
/// Format entry: `MediaFormat.COLOR_RANGE_*`.
pub(crate) const KEY_COLOR_RANGE: &str = "color-range";
/// Format entry: `MediaFormat.COLOR_TRANSFER_*`.
pub(crate) const KEY_COLOR_TRANSFER: &str = "color-transfer";
/// Format entry: duration of the track, in microseconds.
pub(crate) const KEY_DURATION: &str = "durationUs";
/// Format entry: frames per second (a whole number in files, any number for encoders).
pub(crate) const KEY_FRAME_RATE: &str = "frame-rate";
/// Format entry: samples per second.
pub(crate) const KEY_SAMPLE_RATE: &str = "sample-rate";
/// Format entry: number of sound channels.
pub(crate) const KEY_CHANNEL_COUNT: &str = "channel-count";
/// Format entry: how decoded sound samples are stored (`AudioFormat.ENCODING_*`).
pub(crate) const KEY_PCM_ENCODING: &str = "pcm-encoding";
/// Format entry: target bit rate of an encoder.
pub(crate) const KEY_BIT_RATE: &str = "bitrate";
/// Format entry: seconds between two keyframes.
pub(crate) const KEY_KEYFRAME_INTERVAL: &str = "i-frame-interval";
/// Format entry: AAC profile (`MediaCodecInfo.CodecProfileLevel.AACObject*`).
pub(crate) const KEY_AAC_PROFILE: &str = "aac-profile";
/// Format entry: largest input buffer an encoder must accept.
pub(crate) const KEY_MAXIMUM_INPUT_SIZE: &str = "max-input-size";

/// `MediaFormat.COLOR_STANDARD_BT709`.
pub(crate) const COLOR_STANDARD_BT709: i32 = 1;
/// `MediaFormat.COLOR_STANDARD_BT601_PAL`.
const COLOR_STANDARD_BT601_PAL: i32 = 2;
/// `MediaFormat.COLOR_STANDARD_BT601_NTSC`.
const COLOR_STANDARD_BT601_NTSC: i32 = 4;
/// `MediaFormat.COLOR_RANGE_FULL`.
const COLOR_RANGE_FULL: i32 = 1;
/// `MediaFormat.COLOR_RANGE_LIMITED`.
pub(crate) const COLOR_RANGE_LIMITED: i32 = 2;
/// `MediaFormat.COLOR_TRANSFER_SDR_VIDEO`: the BT.709 transfer curve.
pub(crate) const COLOR_TRANSFER_SDR_VIDEO: i32 = 3;

/// `AMEDIACODEC_BUFFER_FLAG_CODEC_CONFIG`: set-up data, not a sample.
pub(crate) const BUFFER_FLAG_CODEC_CONFIG: u32 = 2;
/// `AMEDIACODEC_BUFFER_FLAG_END_OF_STREAM`.
pub(crate) const BUFFER_FLAG_END_OF_STREAM: u32 = 4;

/// How long one wait for a codec buffer lasts.
pub(crate) const CODEC_POLL_TIMEOUT: Duration = Duration::from_millis(10);
/// A codec that produces nothing for this long is considered stuck.
pub(crate) const STALLED_CODEC_TIMEOUT: Duration = Duration::from_secs(10);

/// Describes a failed `MediaCodec` call.
pub(crate) fn codec_failure(step: &str, error: impl std::fmt::Display) -> String {
    format!("{step}: {error}")
}

/// A positive whole-number entry as a size, when present.
fn positive_size_entry(format: &MediaFormat, key: &str) -> Option<usize> {
    format
        .i32(key)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
}

/// The codec type of a track, for example `video/avc` or `audio/mp4a-latm`.
pub(crate) fn mime_type_of(format: &mut MediaFormat) -> Option<String> {
    format.str(KEY_MIME).map(str::to_owned)
}

/// Creates, configures and starts a decoder for a track.
///
/// # Errors
///
/// Fails when Android has no decoder for the track's codec, or it cannot start.
pub(crate) fn start_decoder(track_format: &mut MediaFormat) -> Result<MediaCodec, String> {
    let mime_type =
        mime_type_of(track_format).ok_or("the track does not say which codec it uses")?;
    let decoder = MediaCodec::from_decoder_type(&mime_type)
        .ok_or_else(|| format!("this phone has no decoder for {mime_type}"))?;
    decoder
        .configure(track_format, None, MediaCodecDirection::Decoder)
        .map_err(|error| codec_failure("setting up the decoder", error))?;
    decoder
        .start()
        .map_err(|error| codec_failure("starting the decoder", error))?;
    Ok(decoder)
}

/// The layout of the pictures a codec format describes.
///
/// # Errors
///
/// Fails when the layout is not one Kadr can read or write.
pub(crate) fn picture_layout_of(
    format: &MediaFormat,
) -> Result<PictureBufferLayout, PictureLayoutError> {
    let entries = PictureFormatEntries {
        width: positive_size_entry(format, KEY_WIDTH).unwrap_or(0),
        height: positive_size_entry(format, KEY_HEIGHT).unwrap_or(0),
        color_format: format.i32(KEY_COLOR_FORMAT).unwrap_or(0),
        stride: positive_size_entry(format, KEY_STRIDE),
        slice_height: positive_size_entry(format, KEY_SLICE_HEIGHT),
    };
    PictureBufferLayout::from_format(entries, format.buffer(KEY_IMAGE_DATA))
}

/// The visible part of the pictures: the `crop` entry, or the whole coded picture.
pub(crate) fn crop_of(format: &MediaFormat, layout: &PictureBufferLayout) -> CropRectangle {
    format
        .rect(KEY_CROP)
        .and_then(|(left, top, right, bottom)| {
            CropRectangle::from_inclusive_bounds(left, top, right, bottom)
        })
        .unwrap_or_else(|| CropRectangle::whole_picture(layout.coded_width, layout.coded_height))
}

/// The colour space a format announces, or the one video players assume.
pub(crate) fn color_space_of(format: &MediaFormat, picture_height: usize) -> YuvColorSpace {
    let matrix = match format.i32(KEY_COLOR_STANDARD) {
        Some(COLOR_STANDARD_BT709) => YuvMatrix::Bt709,
        Some(COLOR_STANDARD_BT601_PAL | COLOR_STANDARD_BT601_NTSC) => YuvMatrix::Bt601,
        _ => YuvMatrix::assumed_for_height(u32::try_from(picture_height).unwrap_or(u32::MAX)),
    };
    let range = match format.i32(KEY_COLOR_RANGE) {
        Some(COLOR_RANGE_FULL) => YuvRange::Full,
        _ => YuvRange::Limited,
    };
    YuvColorSpace { matrix, range }
}

/// What feeding a decoder did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FeedOutcome {
    /// One compressed sample was handed over.
    SampleGiven,
    /// The file is finished; the decoder was told so.
    EndOfStreamGiven,
    /// The decoder had no free input buffer.
    DecoderBusy,
}

/// Hands the extractor's current sample to the decoder and moves to the next.
///
/// # Errors
///
/// Fails when the sample cannot be read or the decoder refuses it.
pub(crate) fn feed_decoder(
    decoder: &MediaCodec,
    extractor: &mut MediaExtractor,
) -> Result<FeedOutcome, String> {
    let mut input_buffer = match decoder
        .dequeue_input_buffer(Duration::ZERO)
        .map_err(|error| codec_failure("waiting for the decoder", error))?
    {
        DequeuedInputBufferResult::Buffer(input_buffer) => input_buffer,
        DequeuedInputBufferResult::TryAgainLater => return Ok(FeedOutcome::DecoderBusy),
    };
    let sample = extractor.read_sample(input_buffer.buffer_mut())?;
    match sample {
        Some(sample) => {
            let presentation_time =
                u64::try_from(sample.presentation_time_in_microseconds).unwrap_or(0);
            decoder
                .queue_input_buffer(input_buffer, 0, sample.size_in_bytes, presentation_time, 0)
                .map_err(|error| codec_failure("giving a sample to the decoder", error))?;
            extractor.advance();
            Ok(FeedOutcome::SampleGiven)
        }
        None => {
            decoder
                .queue_input_buffer(input_buffer, 0, 0, 0, BUFFER_FLAG_END_OF_STREAM)
                .map_err(|error| codec_failure("ending the decoder's input", error))?;
            Ok(FeedOutcome::EndOfStreamGiven)
        }
    }
}

/// What a decoder or encoder produced.
pub(crate) enum CodecOutput<'codec> {
    /// A finished buffer: a picture, sound, or compressed data.
    Buffer(OutputBuffer<'codec>),
    /// The format of the next buffers changed: read it with `output_format()`.
    FormatChanged,
    /// Nothing ready within the timeout.
    NothingYet,
}

/// Waits up to `timeout` for the next output of `codec`.
///
/// # Errors
///
/// Fails when the codec reports an error.
pub(crate) fn next_output(
    codec: &MediaCodec,
    timeout: Duration,
) -> Result<CodecOutput<'_>, String> {
    match codec
        .dequeue_output_buffer(timeout)
        .map_err(|error| codec_failure("waiting for the codec's output", error))?
    {
        DequeuedOutputBufferInfoResult::Buffer(buffer) => Ok(CodecOutput::Buffer(buffer)),
        DequeuedOutputBufferInfoResult::OutputFormatChanged => Ok(CodecOutput::FormatChanged),
        DequeuedOutputBufferInfoResult::TryAgainLater
        | DequeuedOutputBufferInfoResult::OutputBuffersChanged => Ok(CodecOutput::NothingYet),
    }
}

/// Watches a codec loop and reports a codec that stopped making progress.
pub(crate) struct StallWatch {
    last_progress: Instant,
}

impl StallWatch {
    /// Starts watching now.
    pub(crate) fn new() -> Self {
        Self {
            last_progress: Instant::now(),
        }
    }

    /// Records that the codec did something.
    pub(crate) fn record_progress(&mut self) {
        self.last_progress = Instant::now();
    }

    /// Fails when nothing happened for [`STALLED_CODEC_TIMEOUT`].
    ///
    /// # Errors
    ///
    /// Returns a message naming `what` the codec was doing.
    pub(crate) fn check(&self, what: &str) -> Result<(), String> {
        if self.last_progress.elapsed() > STALLED_CODEC_TIMEOUT {
            Err(format!(
                "the codec stopped responding while {what} (nothing for {} s)",
                STALLED_CODEC_TIMEOUT.as_secs()
            ))
        } else {
            Ok(())
        }
    }
}
