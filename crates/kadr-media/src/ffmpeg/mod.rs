//! The FFmpeg backend, used on Windows, macOS and Linux.
//!
//! Built on the `ffmpeg-next` crate, which links to the FFmpeg libraries
//! (version 7 or newer). See README.md for how to install them.

mod audio_segment;
mod mp4_sink;
mod picture_conversion;
mod probe;
mod time_conversion;
mod video_proxy;
mod video_source;

use std::ops::ControlFlow;
use std::path::Path;
use std::sync::OnceLock;

use ffmpeg_next as ffmpeg;
use kadr_core::media_asset::MediaAssetDescription;
use kadr_core::time::Timestamp;

use crate::audio_buffer::AudioBuffer;
use crate::backend::{
    AudioSegmentRequest, ExportSink, ExportSinkSettings, MediaBackend, VideoFrameSource,
    VideoProxyOutcome, VideoProxySettings, VideoStreamDetails,
};
use crate::error::MediaError;

/// Reads and writes media files with FFmpeg.
#[derive(Debug)]
pub struct FfmpegBackend {
    _initialized: (),
}

/// The result of initialising FFmpeg, shared by every backend instance.
static FFMPEG_INITIALIZATION: OnceLock<Result<(), String>> = OnceLock::new();

impl FfmpegBackend {
    /// Initialises FFmpeg (once per program) and returns the backend.
    ///
    /// # Errors
    ///
    /// Fails if the FFmpeg libraries cannot be initialised.
    pub fn new() -> Result<Self, MediaError> {
        let initialization = FFMPEG_INITIALIZATION.get_or_init(|| {
            ffmpeg::init().map_err(|error| error.to_string())?;
            // FFmpeg prints warnings and errors on stderr by default. Kadr reports
            // every failure itself through `MediaError`, and some FFmpeg messages are
            // expected (an encoder probing for hardware that is absent), so only
            // fatal messages are let through.
            ffmpeg::log::set_level(ffmpeg::log::Level::Fatal);
            Ok(())
        });
        match initialization {
            Ok(()) => Ok(Self { _initialized: () }),
            Err(reason) => Err(MediaError::InvalidRequest {
                reason: format!("FFmpeg could not be initialised: {reason}"),
            }),
        }
    }
}

impl MediaBackend for FfmpegBackend {
    fn probe(&self, file_path: &Path) -> Result<MediaAssetDescription, MediaError> {
        probe::probe_media_file(file_path)
    }

    fn open_video_source(&self, file_path: &Path) -> Result<Box<dyn VideoFrameSource>, MediaError> {
        Ok(Box::new(video_source::FfmpegVideoFrameSource::open(
            file_path,
            DecoderThreading::Frames,
        )?))
    }

    fn open_video_proxy(&self, file_path: &Path) -> Result<Box<dyn VideoFrameSource>, MediaError> {
        Ok(Box::new(video_source::FfmpegVideoFrameSource::open(
            file_path,
            DecoderThreading::Slices,
        )?))
    }

    fn decode_audio_segment(
        &self,
        request: &AudioSegmentRequest,
    ) -> Result<AudioBuffer, MediaError> {
        audio_segment::decode_audio_segment(request)
    }

    fn create_export_sink(
        &self,
        settings: &ExportSinkSettings,
    ) -> Result<Box<dyn ExportSink>, MediaError> {
        Ok(Box::new(mp4_sink::Mp4ExportSink::create(settings)?))
    }

    fn describe_video_stream(&self, file_path: &Path) -> Result<VideoStreamDetails, MediaError> {
        video_proxy::describe_video_stream(file_path)
    }

    fn make_video_proxy(
        &self,
        settings: &VideoProxySettings,
        on_progress: &mut dyn FnMut(Timestamp) -> ControlFlow<()>,
    ) -> Result<VideoProxyOutcome, MediaError> {
        video_proxy::make_video_proxy(settings, on_progress)
    }
}

/// Opens a file for reading, with a clear error.
fn open_input_file(file_path: &Path) -> Result<ffmpeg::format::context::Input, MediaError> {
    ffmpeg::format::input(file_path).map_err(|error| MediaError::CannotOpenFile {
        file_path: file_path.to_path_buf(),
        reason: error.to_string(),
    })
}

/// Builds the "could not decode" error for a file.
fn decoding_failure(file_path: &Path, step: &str, error: impl std::fmt::Display) -> MediaError {
    MediaError::DecodingFailed {
        file_path: file_path.to_path_buf(),
        reason: format!("{step}: {error}"),
    }
}

/// The best video stream of a file, ignoring cover art embedded in music files.
fn find_picture_stream(
    input: &ffmpeg::format::context::Input,
) -> Option<ffmpeg::format::stream::Stream<'_>> {
    input
        .streams()
        .best(ffmpeg::media::Type::Video)
        .filter(|stream| {
            !stream
                .disposition()
                .contains(ffmpeg::format::stream::Disposition::ATTACHED_PIC)
        })
}

/// The decoder of a file's pictures, with what is needed to read its frame times.
struct PictureDecoder {
    stream_index: usize,
    stream_time_base: ffmpeg::Rational,
    /// Frame times are measured from here (negative starts count as zero).
    stream_start_in_stream_units: i64,
    decoder: ffmpeg::decoder::Video,
}

/// How a decoder spreads its work over the processor's cores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecoderThreading {
    /// Several frames at once: the most frames per second, but the first frame
    /// after a jump only comes out once a frame per core has been read. For
    /// files read in order (export, playback).
    Frames,
    /// Parts (slices) of one frame at once: each frame comes out as soon as it
    /// is decoded. For proxies, written in slices and read with many jumps.
    Slices,
}

/// Opens the decoder of the best picture stream of `input`.
fn open_picture_decoder(
    input: &ffmpeg::format::context::Input,
    file_path: &Path,
    threading: DecoderThreading,
) -> Result<PictureDecoder, MediaError> {
    let stream = find_picture_stream(input).ok_or_else(|| MediaError::NoVideoStream {
        file_path: file_path.to_path_buf(),
    })?;
    let stream_time_base = stream.time_base();
    let mut codec_context =
        ffmpeg::codec::context::Context::from_parameters(stream.parameters())
            .map_err(|error| decoding_failure(file_path, "reading the video parameters", error))?;
    // One thread per core (a count of 0), on frames or on slices.
    codec_context.set_threading(ffmpeg::codec::threading::Config::kind(match threading {
        DecoderThreading::Frames => ffmpeg::codec::threading::Type::Frame,
        DecoderThreading::Slices => ffmpeg::codec::threading::Type::Slice,
    }));
    let mut decoder = codec_context
        .decoder()
        .video()
        .map_err(|error| decoding_failure(file_path, "opening the video decoder", error))?;
    decoder.set_packet_time_base(stream_time_base);
    Ok(PictureDecoder {
        stream_index: stream.index(),
        stream_time_base,
        stream_start_in_stream_units: stream.start_time().max(0),
        decoder,
    })
}

/// Whether FFmpeg opened the file as a still picture (PNG, JPEG, WebP…).
fn is_still_image_format(input: &ffmpeg::format::context::Input) -> bool {
    let input_format = input.format();
    let format_name = input_format.name();
    format_name == "image2" || format_name.ends_with("_pipe")
}
