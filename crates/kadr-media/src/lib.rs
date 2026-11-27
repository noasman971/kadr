//! # kadr-media — reading and writing video and audio files
//!
//! This crate turns media files into pictures and sound Kadr can work with, and
//! turns rendered pictures and sound back into a video file:
//!
//! - [`MediaBackend::probe`]: describe a file (kind, duration, picture size,
//!   frame rate, sound format);
//! - [`MediaBackend::open_video_source`]: read the exact picture shown at any time;
//! - [`MediaBackend::decode_audio_segment`]: decode a range of sound, at any
//!   speed, as stereo;
//! - [`MediaBackend::create_export_sink`]: write an MP4 file (H.264 + AAC), with a
//!   hardware encoder when one is available;
//! - [`MediaBackend::describe_video_stream`] and [`MediaBackend::make_video_proxy`]:
//!   judge how heavy a video is to decode, and write a small, easy-to-decode copy
//!   of its pictures for the preview (decision record 0055).
//!
//! Pictures are [`VideoFrame`]s: YUV 4:2:0 planes as videos store them, or RGBA
//! for images with transparency. Colour conversions are explicit (BT.601 / BT.709,
//! limited / full range) and exports are always BT.709. Sound is [`AudioBuffer`]s
//! (stereo, 32-bit float).
//!
//! ## Rules for this crate
//!
//! - Everything goes through the [`MediaBackend`] traits. The desktop backend is
//!   [`FfmpegBackend`] (feature `ffmpeg`, on by default); the Android backend will
//!   use `MediaCodec` (phase 9).
//! - Hardware encoding is used when available, with an automatic fallback to the
//!   software encoder. Hardware *decoding* is not used yet: decoding on the CPU
//!   costs about 1 ms per 1080p frame (see decision record 0006).
//!
//! See `docs/architecture.md`.

pub mod audio_buffer;
pub mod backend;
pub mod error;
#[cfg(feature = "ffmpeg")]
mod ffmpeg;
pub mod test_patterns;
pub mod video_frame;

pub use audio_buffer::AudioBuffer;
pub use backend::{
    AudioSegmentRequest, ChromaSubsampling, EncodingSpeed, ExportSink, ExportSinkSettings,
    MediaBackend, VideoEncoderPreference, VideoFrameSource, VideoProxyOutcome, VideoProxySettings,
    VideoStreamDetails,
};
pub use error::MediaError;
#[cfg(feature = "ffmpeg")]
pub use ffmpeg::FfmpegBackend;
pub use video_frame::{VideoFrame, YuvColorSpace, YuvMatrix, YuvPicture, YuvRange};
