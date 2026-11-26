//! The FFmpeg backend, used on Windows, macOS and Linux.
//!
//! Built on the `ffmpeg-next` crate, which links to the FFmpeg libraries
//! (version 7 or newer). See README.md for how to install them.

mod audio_segment;
mod mp4_sink;
mod picture_conversion;
mod probe;
mod time_conversion;
mod video_source;

use std::path::Path;
use std::sync::OnceLock;

use ffmpeg_next as ffmpeg;
use kadr_core::media_asset::MediaAssetDescription;

use crate::audio_buffer::AudioBuffer;
use crate::backend::{
    AudioSegmentRequest, ExportSink, ExportSinkSettings, MediaBackend, VideoFrameSource,
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

/// Whether FFmpeg opened the file as a still picture (PNG, JPEG, WebP…).
fn is_still_image_format(input: &ffmpeg::format::context::Input) -> bool {
    let input_format = input.format();
    let format_name = input_format.name();
    format_name == "image2" || format_name.ends_with("_pipe")
}
