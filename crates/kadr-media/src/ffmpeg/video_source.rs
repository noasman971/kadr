//! Frame-accurate reading of the pictures of a video file.
//!
//! Video files only allow jumping to *keyframes*, which can be seconds apart.
//! To show the exact frame at a given time, the source jumps to the keyframe
//! before it and decodes forward until it reaches the frame that is on screen
//! at that time. Requests in increasing order (playback, export) just keep
//! decoding forward without jumping.
//!
//! The source always keeps one decoded frame ahead (`upcoming_frame`): that is how
//! it knows the current frame is still the one on screen.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ffmpeg_next as ffmpeg;
use ffmpeg_next::Rational;
use kadr_core::time::{TimeSpan, Timestamp};

use super::time_conversion::{microseconds_from_timestamp, timestamp_from_stream_units};
use super::{decoding_failure, find_picture_stream, is_still_image_format, open_input_file};
use crate::backend::VideoFrameSource;
use crate::error::MediaError;
use crate::video_frame::VideoFrame;

/// Beyond this distance ahead, jumping to a keyframe is faster than decoding forward.
const MAXIMUM_FORWARD_DECODING_DISTANCE: TimeSpan = TimeSpan::from_milliseconds(2_000);

/// A frame decoded from the file, converted to RGBA only when it is shown.
struct DecodedFrame {
    source_time: Timestamp,
    raw_picture: ffmpeg::frame::Video,
    converted_picture: Option<Arc<VideoFrame>>,
}

/// Reads the pictures of one file with FFmpeg.
pub(crate) struct FfmpegVideoFrameSource {
    file_path: PathBuf,
    input: ffmpeg::format::context::Input,
    stream_index: usize,
    stream_time_base: Rational,
    stream_start_in_stream_units: i64,
    decoder: ffmpeg::decoder::Video,
    rgba_converter: Option<RgbaConverter>,
    is_still_image: bool,
    end_of_file_reached: bool,
    /// The frame on screen at the last requested time.
    current_frame: Option<DecodedFrame>,
    /// The frame after `current_frame`, already decoded.
    upcoming_frame: Option<DecodedFrame>,
}

/// Converts decoded pictures of one pixel format and size to RGBA.
struct RgbaConverter {
    source_format: ffmpeg::format::Pixel,
    width: u32,
    height: u32,
    scaler: ffmpeg::software::scaling::Context,
}

impl FfmpegVideoFrameSource {
    /// Opens the pictures of `file_path`.
    pub(crate) fn open(file_path: &Path) -> Result<Self, MediaError> {
        let input = open_input_file(file_path)?;
        let is_still_image = is_still_image_format(&input);
        let stream = find_picture_stream(&input).ok_or_else(|| MediaError::NoVideoStream {
            file_path: file_path.to_path_buf(),
        })?;
        let stream_index = stream.index();
        let stream_time_base = stream.time_base();
        let stream_start_in_stream_units = stream.start_time().max(0);

        let mut codec_context = ffmpeg::codec::context::Context::from_parameters(
            stream.parameters(),
        )
        .map_err(|error| decoding_failure(file_path, "reading the video parameters", error))?;
        // Let FFmpeg decode several frames in parallel (0 = one thread per core).
        codec_context.set_threading(ffmpeg::codec::threading::Config::kind(
            ffmpeg::codec::threading::Type::Frame,
        ));
        let mut decoder = codec_context
            .decoder()
            .video()
            .map_err(|error| decoding_failure(file_path, "opening the video decoder", error))?;
        decoder.set_packet_time_base(stream_time_base);

        Ok(Self {
            file_path: file_path.to_path_buf(),
            input,
            stream_index,
            stream_time_base,
            stream_start_in_stream_units,
            decoder,
            rgba_converter: None,
            is_still_image,
            end_of_file_reached: false,
            current_frame: None,
            upcoming_frame: None,
        })
    }

    /// Whether the frame already on screen is still the right one for `source_time`.
    fn current_frame_covers(&self, source_time: Timestamp) -> bool {
        let Some(current_frame) = &self.current_frame else {
            return false;
        };
        if self.is_still_image {
            return true;
        }
        let starts_in_time = current_frame.source_time <= source_time;
        let next_frame_not_started = match &self.upcoming_frame {
            Some(upcoming_frame) => source_time < upcoming_frame.source_time,
            None => self.end_of_file_reached,
        };
        starts_in_time && next_frame_not_started
    }

    /// Whether reaching `source_time` requires jumping instead of decoding forward.
    fn needs_seek_to(&self, source_time: Timestamp) -> bool {
        if self.is_still_image {
            return false;
        }
        let Some(current_frame) = &self.current_frame else {
            return source_time > Timestamp::ZERO;
        };
        let is_backwards = source_time < current_frame.source_time;
        let is_far_ahead = source_time
            .span_since(current_frame.source_time)
            .is_some_and(|distance| distance > MAXIMUM_FORWARD_DECODING_DISTANCE);
        is_backwards || is_far_ahead
    }

    /// Jumps to the keyframe at or before `source_time`.
    fn seek_to_keyframe_before(&mut self, source_time: Timestamp) -> Result<(), MediaError> {
        // Container timestamps include the stream's start offset.
        let stream_start =
            timestamp_from_stream_units(self.stream_start_in_stream_units, self.stream_time_base);
        let target_in_file = stream_start + TimeSpan::from_ticks(source_time.ticks());
        let target_in_microseconds = microseconds_from_timestamp(target_in_file);
        self.input
            .seek(target_in_microseconds, ..target_in_microseconds)
            .map_err(|error| decoding_failure(&self.file_path, "jumping to a keyframe", error))?;
        self.decoder.flush();
        self.end_of_file_reached = false;
        self.current_frame = None;
        self.upcoming_frame = None;
        Ok(())
    }

    /// Decodes the next frame of the file, or returns `None` at the end.
    fn decode_next_frame(&mut self) -> Result<Option<DecodedFrame>, MediaError> {
        loop {
            let mut raw_picture = ffmpeg::frame::Video::empty();
            if self.decoder.receive_frame(&mut raw_picture).is_ok() {
                let presentation_time = raw_picture
                    .timestamp()
                    .unwrap_or(self.stream_start_in_stream_units);
                let source_time = timestamp_from_stream_units(
                    presentation_time - self.stream_start_in_stream_units,
                    self.stream_time_base,
                );
                return Ok(Some(DecodedFrame {
                    source_time,
                    raw_picture,
                    converted_picture: None,
                }));
            }
            if self.end_of_file_reached {
                return Ok(None);
            }
            self.feed_decoder_with_next_packet()?;
        }
    }

    /// Reads the next packet of the picture stream and gives it to the decoder.
    fn feed_decoder_with_next_packet(&mut self) -> Result<(), MediaError> {
        for (stream, packet) in self.input.packets() {
            if stream.index() != self.stream_index {
                continue;
            }
            return self.decoder.send_packet(&packet).map_err(|error| {
                decoding_failure(&self.file_path, "decoding a video packet", error)
            });
        }
        self.end_of_file_reached = true;
        self.decoder.send_eof().map_err(|error| {
            decoding_failure(&self.file_path, "finishing the video decoding", error)
        })
    }

    /// Decodes forward until `current_frame` is the frame on screen at `source_time`.
    fn decode_forward_to(&mut self, source_time: Timestamp) -> Result<(), MediaError> {
        loop {
            if let Some(upcoming_frame) = self.upcoming_frame.take() {
                if upcoming_frame.source_time > source_time && self.current_frame.is_some() {
                    self.upcoming_frame = Some(upcoming_frame);
                    return Ok(());
                }
                self.current_frame = Some(upcoming_frame);
            }
            match self.decode_next_frame()? {
                Some(decoded_frame) => self.upcoming_frame = Some(decoded_frame),
                None => return Ok(()),
            }
        }
    }

    /// Converts the current frame to RGBA (once) and returns it.
    fn current_frame_as_rgba(
        &mut self,
        source_time: Timestamp,
    ) -> Result<Arc<VideoFrame>, MediaError> {
        let file_path = self.file_path.clone();
        let current_frame = self.current_frame.as_mut().ok_or_else(|| {
            decoding_failure(
                &file_path,
                &format!("reading the frame at {source_time}"),
                "the file has no frame there",
            )
        })?;
        if let Some(converted_picture) = &current_frame.converted_picture {
            return Ok(Arc::clone(converted_picture));
        }
        let converter = rgba_converter_for(
            &mut self.rgba_converter,
            &current_frame.raw_picture,
            &file_path,
        )?;
        let converted_picture =
            Arc::new(converter.convert(&current_frame.raw_picture, &file_path)?);
        current_frame.converted_picture = Some(Arc::clone(&converted_picture));
        Ok(converted_picture)
    }
}

impl VideoFrameSource for FfmpegVideoFrameSource {
    fn frame_at(&mut self, source_time: Timestamp) -> Result<Arc<VideoFrame>, MediaError> {
        if !self.current_frame_covers(source_time) {
            if self.needs_seek_to(source_time) {
                self.seek_to_keyframe_before(source_time)?;
            }
            self.decode_forward_to(source_time)?;
        }
        self.current_frame_as_rgba(source_time)
    }
}

/// The converter matching `picture`, created or replaced if the format or size changed.
fn rgba_converter_for<'converter>(
    slot: &'converter mut Option<RgbaConverter>,
    picture: &ffmpeg::frame::Video,
    file_path: &Path,
) -> Result<&'converter mut RgbaConverter, MediaError> {
    let matches_picture = slot.as_ref().is_some_and(|converter| {
        converter.source_format == picture.format()
            && converter.width == picture.width()
            && converter.height == picture.height()
    });
    if !matches_picture {
        let scaler = ffmpeg::software::scaling::Context::get(
            picture.format(),
            picture.width(),
            picture.height(),
            ffmpeg::format::Pixel::RGBA,
            picture.width(),
            picture.height(),
            ffmpeg::software::scaling::Flags::BILINEAR,
        )
        .map_err(|error| decoding_failure(file_path, "preparing the colour conversion", error))?;
        *slot = Some(RgbaConverter {
            source_format: picture.format(),
            width: picture.width(),
            height: picture.height(),
            scaler,
        });
    }
    slot.as_mut().ok_or_else(|| {
        decoding_failure(file_path, "preparing the colour conversion", "no converter")
    })
}

impl RgbaConverter {
    /// Converts a decoded picture to a tightly packed RGBA image.
    fn convert(
        &mut self,
        picture: &ffmpeg::frame::Video,
        file_path: &Path,
    ) -> Result<VideoFrame, MediaError> {
        let mut rgba_picture = ffmpeg::frame::Video::empty();
        self.scaler
            .run(picture, &mut rgba_picture)
            .map_err(|error| decoding_failure(file_path, "converting colours", error))?;
        copy_rgba_plane_into_image(&rgba_picture, file_path)
    }
}

/// Copies an FFmpeg RGBA picture (whose rows may be padded) into an image.
fn copy_rgba_plane_into_image(
    rgba_picture: &ffmpeg::frame::Video,
    file_path: &Path,
) -> Result<VideoFrame, MediaError> {
    let (width, height) = (rgba_picture.width(), rgba_picture.height());
    let row_length_in_bytes = width as usize * 4;
    let stride_in_bytes = rgba_picture.stride(0);
    let plane = rgba_picture.data(0);
    let mut pixels = Vec::with_capacity(row_length_in_bytes * height as usize);
    for row in 0..height as usize {
        let row_start = row * stride_in_bytes;
        pixels.extend_from_slice(&plane[row_start..row_start + row_length_in_bytes]);
    }
    VideoFrame::from_raw(width, height, pixels)
        .ok_or_else(|| decoding_failure(file_path, "converting colours", "unexpected picture size"))
}
