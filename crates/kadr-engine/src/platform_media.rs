//! The media backend of the platform this build of Kadr targets.
//!
//! Interfaces do not pick a media library themselves: they ask the engine for the
//! backend it was built with. Desktop builds use FFmpeg (feature `ffmpeg`, on by
//! default); Android builds are made with `--no-default-features` and will use the
//! system's MediaCodec (phase 9). See
//! `docs/decisions/0040-media-backend-features.md`.

use kadr_media::{MediaBackend, MediaError};

/// Creates the media backend this build of Kadr was made with.
///
/// # Errors
///
/// Fails when the backend cannot start (for example when the FFmpeg libraries
/// cannot be initialised), or when the engine was built without any media
/// backend.
pub fn create_platform_media_backend() -> Result<Box<dyn MediaBackend>, MediaError> {
    create_compiled_in_backend()
}

/// The FFmpeg backend: Windows, macOS and Linux.
#[cfg(feature = "ffmpeg")]
fn create_compiled_in_backend() -> Result<Box<dyn MediaBackend>, MediaError> {
    Ok(Box::new(kadr_media::FfmpegBackend::new()?))
}

/// No backend was compiled in: every media operation would fail, so say so at once.
#[cfg(not(feature = "ffmpeg"))]
fn create_compiled_in_backend() -> Result<Box<dyn MediaBackend>, MediaError> {
    Err(MediaError::InvalidRequest {
        reason: "this build of Kadr contains no media backend: build kadr-engine with \
                 the `ffmpeg` feature (desktop, on by default)"
            .to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "ffmpeg")]
    #[test]
    fn desktop_builds_get_a_working_backend() {
        let backend = create_platform_media_backend().unwrap();
        let missing_file = std::path::Path::new("no such file.mp4");

        let error = backend.probe(missing_file).unwrap_err();

        assert!(matches!(error, MediaError::CannotOpenFile { .. }));
    }

    #[cfg(not(feature = "ffmpeg"))]
    #[test]
    fn builds_without_backend_explain_which_feature_is_missing() {
        let Err(error) = create_platform_media_backend() else {
            panic!("a build without media backend must not return one");
        };

        assert!(
            error
                .to_string()
                .contains("kadr-engine with the `ffmpeg` feature")
        );
    }
}
