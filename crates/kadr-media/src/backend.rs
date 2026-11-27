//! The interface between Kadr and a media library.
//!
//! The engine never calls FFmpeg directly: it talks to a [`MediaBackend`]. On
//! desktop the backend is [`FfmpegBackend`](crate::FfmpegBackend); on Android it
//! will use the system's `MediaCodec` (phase 9). Tests can also provide their own.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kadr_core::clip::PlaybackSpeed;
use kadr_core::media_asset::{MediaAssetDescription, PictureSize};
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

    /// Opens the pictures of a proxy made by [`Self::make_video_proxy`], decoding
    /// so that each jump answers quickly (the preview jumps a lot while
    /// scrubbing). By default, the same as [`Self::open_video_source`].
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be opened or has no pictures.
    fn open_video_proxy(&self, file_path: &Path) -> Result<Box<dyn VideoFrameSource>, MediaError> {
        self.open_video_source(file_path)
    }

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

    /// Describes how a file's pictures are stored (codec, bit depth, colour
    /// subsampling, bit rate), to judge how hard they are to decode.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be opened or has no pictures, or when the
    /// backend cannot tell (the default).
    fn describe_video_stream(&self, file_path: &Path) -> Result<VideoStreamDetails, MediaError> {
        Err(MediaError::InvalidRequest {
            reason: format!(
                "this media backend cannot describe the pictures of \"{}\"",
                file_path.display()
            ),
        })
    }

    /// Writes a *proxy* of a video: its pictures, smaller and easy to decode, with
    /// the same frame times, and no sound (decision record 0055). `on_progress`
    /// receives the source time of each frame written, and stops the work by
    /// returning [`ControlFlow::Break`]; the unfinished file is then left for the
    /// caller to delete.
    ///
    /// # Errors
    ///
    /// Fails when the source cannot be decoded or the proxy cannot be written, or
    /// when the backend cannot make proxies (the default).
    fn make_video_proxy(
        &self,
        settings: &VideoProxySettings,
        on_progress: &mut dyn FnMut(Timestamp) -> ControlFlow<()>,
    ) -> Result<VideoProxyOutcome, MediaError> {
        let _ = on_progress;
        Err(MediaError::InvalidRequest {
            reason: format!(
                "this media backend cannot make a proxy of \"{}\"",
                settings.source_path.display()
            ),
        })
    }
}

/// How a video's pictures are stored, as far as decoding cost goes.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoStreamDetails {
    /// The codec's short name, as FFmpeg writes it: `"h264"`, `"hevc"`, `"av1"`,
    /// `"vp9"`, `"prores"`…
    pub codec_name: String,
    /// Size of the pictures.
    pub picture_size: PictureSize,
    /// Frames per second, when the file says.
    pub frame_rate: Option<FrameRate>,
    /// Bits per colour sample: 8 for most videos, 10 or 12 for HDR or log footage.
    pub bits_per_sample: u8,
    /// How much colour detail the pictures keep.
    pub chroma_subsampling: ChromaSubsampling,
    /// The stream's average bit rate (the whole file's when the stream does not
    /// say), when known.
    pub bit_rate_in_bits_per_second: Option<u64>,
}

impl VideoStreamDetails {
    /// Whether pictures arrive as 8-bit YUV 4:2:0, which Kadr uses as it is; any
    /// other layout is converted on every frame.
    pub fn is_eight_bit_yuv420(&self) -> bool {
        self.bits_per_sample <= 8 && self.chroma_subsampling == ChromaSubsampling::Yuv420
    }
}

/// How the colour planes of a picture compare with its brightness plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromaSubsampling {
    /// Colour at half width and half height (almost every video).
    Yuv420,
    /// Colour at half width (professional cameras).
    Yuv422,
    /// Colour at full size (screen recordings, some intermediate codecs).
    Yuv444,
    /// Anything else: grey levels, palettes, RGB with alpha…
    Other,
}

/// What [`MediaBackend::make_video_proxy`] writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoProxySettings {
    /// The video to copy.
    pub source_path: PathBuf,
    /// The MP4 file to write (replaced if it exists).
    pub output_path: PathBuf,
    /// Size of the proxy's pictures (even sides).
    pub picture_size: PictureSize,
    /// One keyframe at least every this many frames: the most a jump must decode.
    pub frames_per_keyframe: u32,
}

/// How [`MediaBackend::make_video_proxy`] ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoProxyOutcome {
    /// The proxy is complete.
    Finished,
    /// `on_progress` asked to stop: the file is incomplete.
    Cancelled,
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
