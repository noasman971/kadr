//! Frame-accurate reading of a video's pictures with the phone's hardware decoder.
//!
//! Same strategy as the FFmpeg backend: jump to the keyframe before the requested
//! time only when going backwards or more than two seconds ahead, then decode
//! forward, always keeping one decoded frame ahead to know when the current one
//! stops being on screen.
//!
//! Decoded pictures sit in the decoder's own buffers, which must be handed back
//! quickly. Each one is copied as raw bytes (a plain memory copy, reusing two
//! buffers) and only rearranged into a [`YuvPicture`](crate::YuvPicture) when it
//! is actually shown: after a jump, the frames decoded on the way are never
//! converted.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kadr_core::time::Timestamp;
use ndk::media::media_codec::MediaCodec;

use super::codec_support::{
    BUFFER_FLAG_END_OF_STREAM, CODEC_POLL_TIMEOUT, CodecOutput, FeedOutcome, StallWatch,
    color_space_of, crop_of, feed_decoder, next_output, picture_layout_of, start_decoder,
};
use super::extractor::MediaExtractor;
use super::media_time::{
    FRAME_START_TOLERANCE, frame_is_on_screen_at, microseconds_from_timestamp,
    needs_jump_to_keyframe, timestamp_from_microseconds,
};
use super::picture_layout::{CropRectangle, PictureBufferLayout};
use super::probe::{TrackKind, find_track};
use crate::backend::VideoFrameSource;
use crate::error::MediaError;
use crate::video_frame::{VideoFrame, YuvColorSpace};

/// How the decoder currently lays out its pictures (it may change mid-stream).
#[derive(Debug, Clone, Copy)]
struct DecodedPictureFormat {
    layout: PictureBufferLayout,
    crop: CropRectangle,
    color_space: YuvColorSpace,
}

/// A decoded frame, copied out of the decoder's buffer.
struct DecodedFrame {
    source_time: Timestamp,
    raw_bytes: Vec<u8>,
    format: DecodedPictureFormat,
    converted_picture: Option<Arc<VideoFrame>>,
}

/// Reads the pictures of one file with `MediaCodec`.
pub(crate) struct MediaCodecVideoFrameSource {
    file_path: PathBuf,
    extractor: MediaExtractor,
    decoder: MediaCodec,
    /// Presentation time of the first frame, subtracted from every frame's time
    /// so that source time 0 is the first frame (like FFmpeg's stream start).
    stream_start_in_microseconds: i64,
    picture_format: Option<DecodedPictureFormat>,
    input_ended: bool,
    output_ended: bool,
    current_frame: Option<DecodedFrame>,
    upcoming_frame: Option<DecodedFrame>,
    /// A byte buffer to reuse for the next decoded frame.
    spare_bytes: Vec<u8>,
}

impl MediaCodecVideoFrameSource {
    /// Opens the pictures of `file_path` and starts a decoder.
    pub(crate) fn open(file_path: &Path) -> Result<Self, MediaError> {
        let mut extractor = MediaExtractor::open(file_path)?;
        let (track_index, mut track_format) =
            find_track(&extractor, TrackKind::Video).ok_or_else(|| MediaError::NoVideoStream {
                file_path: file_path.to_path_buf(),
            })?;
        extractor
            .select_track(track_index)
            .map_err(|reason| decoding_failure(file_path, reason))?;
        let stream_start_in_microseconds =
            extractor.sample_time_in_microseconds().unwrap_or(0).max(0);
        let decoder = start_decoder(&mut track_format)
            .map_err(|reason| decoding_failure(file_path, reason))?;
        Ok(Self {
            file_path: file_path.to_path_buf(),
            extractor,
            decoder,
            stream_start_in_microseconds,
            picture_format: None,
            input_ended: false,
            output_ended: false,
            current_frame: None,
            upcoming_frame: None,
            spare_bytes: Vec::new(),
        })
    }

    /// Whether the frame already on screen is still the right one.
    fn current_frame_covers(&self, source_time: Timestamp) -> bool {
        let Some(current_frame) = &self.current_frame else {
            return false;
        };
        frame_is_on_screen_at(
            current_frame.source_time,
            self.upcoming_frame.as_ref().map(|frame| frame.source_time),
            self.output_ended,
            source_time,
        )
    }

    /// Jumps to the keyframe at or before `source_time` and empties the decoder.
    fn jump_to_keyframe_before(&mut self, source_time: Timestamp) -> Result<(), MediaError> {
        let target = self.stream_start_in_microseconds + microseconds_from_timestamp(source_time);
        self.extractor
            .seek_to_keyframe_before(target)
            .map_err(|reason| decoding_failure(&self.file_path, reason))?;
        self.decoder.flush().map_err(|error| {
            decoding_failure(&self.file_path, format!("emptying the decoder: {error}"))
        })?;
        self.input_ended = false;
        self.output_ended = false;
        for frame in [self.current_frame.take(), self.upcoming_frame.take()]
            .into_iter()
            .flatten()
        {
            self.spare_bytes = frame.raw_bytes;
        }
        Ok(())
    }

    /// Decodes forward until `current_frame` is the frame on screen at `source_time`.
    fn decode_forward_to(&mut self, source_time: Timestamp) -> Result<(), MediaError> {
        loop {
            if let Some(upcoming_frame) = self.upcoming_frame.take() {
                let is_after_target =
                    upcoming_frame.source_time > source_time + FRAME_START_TOLERANCE;
                if is_after_target && self.current_frame.is_some() {
                    self.upcoming_frame = Some(upcoming_frame);
                    return Ok(());
                }
                if let Some(replaced_frame) = self.current_frame.replace(upcoming_frame) {
                    self.spare_bytes = replaced_frame.raw_bytes;
                }
            }
            match self.decode_next_frame()? {
                Some(decoded_frame) => self.upcoming_frame = Some(decoded_frame),
                None => return Ok(()),
            }
        }
    }

    /// Decodes the next frame, or returns `None` at the end of the video.
    fn decode_next_frame(&mut self) -> Result<Option<DecodedFrame>, MediaError> {
        let mut stall_watch = StallWatch::new();
        while !self.output_ended {
            if !self.input_ended {
                match feed_decoder(&self.decoder, &mut self.extractor)
                    .map_err(|reason| decoding_failure(&self.file_path, reason))?
                {
                    FeedOutcome::SampleGiven => stall_watch.record_progress(),
                    FeedOutcome::EndOfStreamGiven => self.input_ended = true,
                    FeedOutcome::DecoderBusy => {}
                }
            }
            if let Some(frame) = self.take_decoder_output()? {
                return Ok(Some(frame));
            }
            stall_watch
                .check("decoding the video")
                .map_err(|reason| decoding_failure(&self.file_path, reason))?;
        }
        Ok(None)
    }

    /// Takes one output of the decoder, if there is one: a frame, a format change
    /// or the end of the video.
    fn take_decoder_output(&mut self) -> Result<Option<DecodedFrame>, MediaError> {
        let output = next_output(&self.decoder, CODEC_POLL_TIMEOUT)
            .map_err(|reason| decoding_failure(&self.file_path, reason))?;
        let buffer = match output {
            CodecOutput::Buffer(buffer) => buffer,
            CodecOutput::FormatChanged => {
                self.picture_format = Some(self.read_picture_format()?);
                return Ok(None);
            }
            CodecOutput::NothingYet => return Ok(None),
        };
        let information = *buffer.info();
        let is_end_of_stream = information.flags() & BUFFER_FLAG_END_OF_STREAM != 0;
        let has_picture = information.size() > 0;
        let decoded_frame = if has_picture {
            let format = match self.picture_format {
                Some(format) => format,
                None => self.read_picture_format()?,
            };
            self.picture_format = Some(format);
            let offset = usize::try_from(information.offset()).unwrap_or(0);
            let size = usize::try_from(information.size()).unwrap_or(0);
            let mut raw_bytes = std::mem::take(&mut self.spare_bytes);
            raw_bytes.clear();
            raw_bytes.extend_from_slice(
                buffer
                    .buffer()
                    .get(offset..offset + size)
                    .unwrap_or_default(),
            );
            Some(DecodedFrame {
                source_time: timestamp_from_microseconds(
                    information.presentation_time_us() - self.stream_start_in_microseconds,
                ),
                raw_bytes,
                format,
                converted_picture: None,
            })
        } else {
            None
        };
        self.decoder
            .release_output_buffer(buffer, false)
            .map_err(|error| {
                decoding_failure(&self.file_path, format!("handing a picture back: {error}"))
            })?;
        if is_end_of_stream {
            self.output_ended = true;
        }
        Ok(decoded_frame)
    }

    /// Reads the decoder's current picture layout, crop and colour space.
    fn read_picture_format(&self) -> Result<DecodedPictureFormat, MediaError> {
        let output_format = self.decoder.output_format();
        let layout = picture_layout_of(&output_format)
            .map_err(|error| decoding_failure(&self.file_path, error.to_string()))?;
        let crop = crop_of(&output_format, &layout);
        Ok(DecodedPictureFormat {
            layout,
            crop,
            color_space: color_space_of(&output_format, crop.height),
        })
    }

    /// Converts the current frame (once) and returns it.
    fn current_frame_converted(
        &mut self,
        source_time: Timestamp,
    ) -> Result<Arc<VideoFrame>, MediaError> {
        let file_path = self.file_path.clone();
        let current_frame = self.current_frame.as_mut().ok_or_else(|| {
            decoding_failure(
                &file_path,
                format!("reading the frame at {source_time}: the file has no frame there"),
            )
        })?;
        if let Some(converted_picture) = &current_frame.converted_picture {
            return Ok(Arc::clone(converted_picture));
        }
        let DecodedPictureFormat {
            layout,
            crop,
            color_space,
        } = current_frame.format;
        let picture = layout
            .read_picture(&current_frame.raw_bytes, crop, color_space)
            .map_err(|error| decoding_failure(&file_path, error.to_string()))?;
        let converted_picture = Arc::new(VideoFrame::Yuv420(picture));
        current_frame.converted_picture = Some(Arc::clone(&converted_picture));
        Ok(converted_picture)
    }
}

impl VideoFrameSource for MediaCodecVideoFrameSource {
    fn frame_at(&mut self, source_time: Timestamp) -> Result<Arc<VideoFrame>, MediaError> {
        if !self.current_frame_covers(source_time) {
            let current_start = self.current_frame.as_ref().map(|frame| frame.source_time);
            if needs_jump_to_keyframe(current_start, source_time) {
                self.jump_to_keyframe_before(source_time)?;
            }
            self.decode_forward_to(source_time)?;
        }
        self.current_frame_converted(source_time)
    }
}

/// Builds the "could not decode" error for a video.
fn decoding_failure(file_path: &Path, reason: impl Into<String>) -> MediaError {
    MediaError::DecodingFailed {
        file_path: file_path.to_path_buf(),
        reason: reason.into(),
    }
}
