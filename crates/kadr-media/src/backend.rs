//! The interface between Kadr and a media library.
//!
//! The engine never calls FFmpeg directly: it talks to a [`MediaBackend`]. On
//! desktop the backend is [`FfmpegBackend`](crate::FfmpegBackend); on Android it
//! will use the system's `MediaCodec` (phase 9). Tests can also provide their own.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kadr_core::clip::PlaybackSpeed;
use kadr_core::media_asset::MediaAssetDescription;
use kadr_core::time::{FrameRate, Timestamp};

use crate::audio_buffer::AudioBuffer;
use crate::error::MediaError;
use crate::video_frame::VideoFrame;

/// A media library able to read and write files.
pub trait MediaBackend: Send + Sync {
    /// Describes a file: kind, duration, picture size, frame rate, sound format.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be opened or contains nothing usable.
    fn probe(&self, file_path: &Path) -> Result<MediaAssetDescription, MediaError>;

    /// Opens the pictures of a video or image file.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be opened or has no pictures.
    fn open_video_source(&self, file_path: &Path) -> Result<Box<dyn VideoFrameSource>, MediaError>;

    /// Decodes a range of a file's sound, at a given speed, as stereo at the
    /// requested sample rate.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be opened or has no sound.
    fn decode_audio_segment(
        &self,
        request: &AudioSegmentRequest,
    ) -> Result<AudioBuffer, MediaError>;

    /// Creates an output file to write the rendered video and sound into.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be created or no encoder can be started.
    fn create_export_sink(
        &self,
        settings: &ExportSinkSettings,
    ) -> Result<Box<dyn ExportSink>, MediaError>;
}

/// Gives the pictures of one video (or image) file, by time.
///
/// Not `Send`: the FFmpeg picture converter must stay on the thread that created
/// it, so a source is opened and used on the same thread (for example the
/// export thread).
pub trait VideoFrameSource {
    /// The picture shown at `source_time`, measured from the start of the file:
    /// the last frame starting at or before that time. Asking for times in
    /// increasing order is fastest; any order works.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be decoded at that point.
    fn frame_at(&mut self, source_time: Timestamp) -> Result<Arc<VideoFrame>, MediaError>;
}

/// A request to decode part of a file's sound.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioSegmentRequest {
    /// The file.
    pub file_path: PathBuf,
    /// Where the range starts in the file.
    pub source_start: Timestamp,
    /// Where the range ends in the file (excluded).
    pub source_end: Timestamp,
    /// Playback speed: the sound is time-stretched, keeping its pitch.
    pub speed: PlaybackSpeed,
    /// Sample rate of the result.
    pub output_sample_rate_in_hertz: u32,
    /// Exact length of the result, in samples per channel: the decoded sound is
    /// padded with silence or cut to this length, so clips stay in sync.
    pub output_samples_per_channel: usize,
}

/// Which video encoder to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoEncoderPreference {
    /// The first hardware encoder that works (NVIDIA, Intel, AMD), otherwise the
    /// software encoder.
    #[default]
    HardwareFirst,
    /// Always the software encoder (libx264): slower, identical on every machine.
    SoftwareOnly,
}

/// Speed / quality trade-off of the export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EncodingSpeed {
    /// Good quality, reasonable speed. For final exports.
    #[default]
    Balanced,
    /// Faster, larger files. For previews and tests.
    Fast,
}

/// What the output file should contain.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportSinkSettings {
    /// The MP4 file to create (overwritten if it exists).
    pub output_path: PathBuf,
    /// Picture width, in pixels (even).
    pub width_in_pixels: u32,
    /// Picture height, in pixels (even).
    pub height_in_pixels: u32,
    /// Frames per second.
    pub frame_rate: FrameRate,
    /// Samples per second of the sound.
    pub audio_sample_rate_in_hertz: u32,
    /// Which video encoder to use.
    pub video_encoder_preference: VideoEncoderPreference,
    /// Speed / quality trade-off.
    pub encoding_speed: EncodingSpeed,
}

/// An output file being written: video frames in order, and sound in order.
pub trait ExportSink {
    /// The name of the video encoder in use, for example `"h264_qsv"` or `"libx264"`.
    fn video_encoder_name(&self) -> &str;

    /// Adds the next video frame. Its size must match the settings.
    ///
    /// # Errors
    ///
    /// Fails when the frame has the wrong size or encoding fails.
    fn write_video_frame(&mut self, frame: &VideoFrame) -> Result<(), MediaError>;

    /// Adds the next piece of sound. Its sample rate must match the settings.
    ///
    /// # Errors
    ///
    /// Fails when the sample rate is wrong or encoding fails.
    fn write_audio(&mut self, audio: &AudioBuffer) -> Result<(), MediaError>;

    /// Flushes the encoders and closes the file. Nothing is playable before this.
    ///
    /// # Errors
    ///
    /// Fails when the end of the file cannot be written.
    fn finish(self: Box<Self>) -> Result<(), MediaError>;
}
