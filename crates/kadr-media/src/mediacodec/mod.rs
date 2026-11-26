//! The Android backend: the phone's own codecs through the NDK media API
//! (`MediaCodec`, `MediaExtractor`, `MediaMuxer`). Feature `mediacodec`.
//!
//! It implements the same [`MediaBackend`](crate::MediaBackend) traits as the
//! FFmpeg backend, with the same guarantees: exact frames at any time, sound of
//! the exact requested length at any speed, MP4 files in H.264 + AAC, BT.709.
//!
//! | Job | How |
//! |---|---|
//! | Probing | `MediaExtractor` track formats; the frame rate is measured from the first frames (`frame_rate`); pictures by the `image` crate (`still_image`) |
//! | Pictures | hardware decoder in byte-buffer mode, frames copied out in their codec layout (`picture_layout`), seeking as on desktop (`media_time`) |
//! | Sound | decoder → stereo floats (`pcm`) → pitch-keeping speed change (`time_stretch`, WSOLA) → sample-rate conversion (`resampler`, windowed sinc) |
//! | Export | H.264 and AAC encoders, `MediaMuxer` |
//!
//! The modules that do arithmetic only (`frame_rate`, `media_time`, `pcm`,
//! `picture_layout`, `resampler`, `still_image`, `time_stretch`) are compiled on
//! the development machine too, so their tests run without a phone. The others
//! call Android and only exist when building for it. Only `extractor` and
//! `muxer` contain `unsafe` code (decision record 0042).
//!
//! See `docs/android.md`.

// On the development machine, the arithmetic modules are compiled for their tests
// only, and the Android code that calls the rest of them is absent.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

#[cfg(any(target_os = "android", test))]
mod frame_rate;
#[cfg(any(target_os = "android", test))]
mod media_time;
#[cfg(any(target_os = "android", test))]
mod pcm;
#[cfg(any(target_os = "android", test))]
mod picture_layout;
#[cfg(any(target_os = "android", test))]
mod resampler;
#[cfg(any(target_os = "android", test))]
mod still_image;
#[cfg(any(target_os = "android", test))]
mod time_stretch;

#[cfg(target_os = "android")]
mod audio_segment;
#[cfg(target_os = "android")]
mod codec_support;
#[cfg(target_os = "android")]
mod extractor;
#[cfg(target_os = "android")]
mod mp4_sink;
#[cfg(target_os = "android")]
mod muxer;
#[cfg(target_os = "android")]
mod probe;
#[cfg(target_os = "android")]
mod video_source;

#[cfg(target_os = "android")]
pub use android_backend::MediaCodecBackend;

/// The file name, used as the name shown in the media library.
#[cfg(any(target_os = "android", test))]
fn display_name_of(file_path: &std::path::Path) -> String {
    file_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| file_path.to_string_lossy().into_owned())
}

#[cfg(target_os = "android")]
mod android_backend {
    use std::path::Path;

    use kadr_core::media_asset::MediaAssetDescription;

    use super::{audio_segment, mp4_sink, probe, still_image, video_source};
    use crate::audio_buffer::AudioBuffer;
    use crate::backend::{
        AudioSegmentRequest, ExportSink, ExportSinkSettings, MediaBackend, VideoFrameSource,
    };
    use crate::error::MediaError;

    /// Reads and writes media files with the phone's own codecs.
    ///
    /// Hardware decoders and encoders are used when the phone has them, which is
    /// the case for H.264 and HEVC on every recent phone.
    #[derive(Debug, Default)]
    pub struct MediaCodecBackend {
        _private: (),
    }

    impl MediaCodecBackend {
        /// The backend. Nothing to initialise: Android loads its codecs on demand.
        pub fn new() -> Self {
            Self::default()
        }
    }

    impl MediaBackend for MediaCodecBackend {
        fn probe(&self, file_path: &Path) -> Result<MediaAssetDescription, MediaError> {
            probe::probe_media_file(file_path)
        }

        fn open_video_source(
            &self,
            file_path: &Path,
        ) -> Result<Box<dyn VideoFrameSource>, MediaError> {
            if still_image::picture_format_of(file_path).is_some() {
                return Ok(Box::new(still_image::StillPictureSource::open(file_path)?));
            }
            Ok(Box::new(video_source::MediaCodecVideoFrameSource::open(
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
            Ok(Box::new(mp4_sink::MediaCodecExportSink::create(settings)?))
        }
    }
}
