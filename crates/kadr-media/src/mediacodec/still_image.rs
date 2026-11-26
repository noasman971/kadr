//! Still pictures (PNG, JPEG, WebP) for the Android backend.
//!
//! Android's `MediaExtractor` only reads video and audio files. Pictures are
//! decoded with the pure-Rust decoders of the `image` crate instead, which need
//! no system library and behave the same everywhere. The FFmpeg backend reads
//! the same formats on desktop.

use std::path::Path;
use std::sync::Arc;

use image::{ImageFormat, ImageReader};
use kadr_core::media_asset::{MediaAssetDescription, MediaKind, PictureSize};
use kadr_core::time::Timestamp;

use crate::backend::VideoFrameSource;
use crate::error::MediaError;
use crate::video_frame::VideoFrame;

/// Picture formats the Android backend reads.
const SUPPORTED_PICTURE_FORMATS: [ImageFormat; 3] =
    [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP];

/// The picture format of the file, recognised from its first bytes; `None` when
/// it is not a supported picture (a video, a sound, or a missing file).
pub(crate) fn picture_format_of(file_path: &Path) -> Option<ImageFormat> {
    let format = ImageReader::open(file_path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .format()?;
    SUPPORTED_PICTURE_FORMATS
        .contains(&format)
        .then_some(format)
}

/// Describes a picture file without decoding all of it.
///
/// # Errors
///
/// Fails when the file cannot be read as a picture.
pub(crate) fn probe_picture(file_path: &Path) -> Result<MediaAssetDescription, MediaError> {
    let (width_in_pixels, height_in_pixels) = open_reader(file_path)?
        .into_dimensions()
        .map_err(|error| picture_failure(file_path, "reading the picture size", error))?;
    Ok(MediaAssetDescription {
        file_path: file_path.to_path_buf(),
        display_name: super::display_name_of(file_path),
        kind: MediaKind::Image,
        duration: None,
        picture_size: Some(PictureSize {
            width_in_pixels,
            height_in_pixels,
        }),
        frame_rate: None,
        audio_format: None,
    })
}

/// A picture file, decoded once and shown at every time.
pub(crate) struct StillPictureSource {
    picture: Arc<VideoFrame>,
}

impl StillPictureSource {
    /// Decodes the picture at `file_path`.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be decoded.
    pub(crate) fn open(file_path: &Path) -> Result<Self, MediaError> {
        let picture = open_reader(file_path)?
            .decode()
            .map_err(|error| picture_failure(file_path, "decoding the picture", error))?;
        Ok(Self {
            picture: Arc::new(VideoFrame::Rgba(picture.to_rgba8())),
        })
    }
}

impl VideoFrameSource for StillPictureSource {
    fn frame_at(&mut self, _source_time: Timestamp) -> Result<Arc<VideoFrame>, MediaError> {
        Ok(Arc::clone(&self.picture))
    }
}

/// Opens a picture file with its format recognised from its content.
fn open_reader(
    file_path: &Path,
) -> Result<ImageReader<std::io::BufReader<std::fs::File>>, MediaError> {
    ImageReader::open(file_path)
        .and_then(ImageReader::with_guessed_format)
        .map_err(|error| MediaError::CannotOpenFile {
            file_path: file_path.to_path_buf(),
            reason: error.to_string(),
        })
}

/// Builds the "could not decode" error for a picture.
fn picture_failure(file_path: &Path, step: &str, error: impl std::fmt::Display) -> MediaError {
    MediaError::DecodingFailed {
        file_path: file_path.to_path_buf(),
        reason: format!("{step}: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use image::{Rgb, RgbImage};

    use super::*;

    fn write_picture(directory: &Path, name: &str, format: ImageFormat) -> std::path::PathBuf {
        let path = directory.join(name);
        // RGB rather than RGBA: JPEG has no transparency.
        let picture = RgbImage::from_fn(6, 4, |x, y| Rgb([x as u8 * 40, y as u8 * 60, 200]));
        picture.save_with_format(&path, format).unwrap();
        path
    }

    #[test]
    fn pictures_are_recognised_by_their_content_not_their_name() {
        let directory = tempfile::tempdir().unwrap();
        let png_named_jpg = write_picture(directory.path(), "photo.jpg", ImageFormat::Png);
        let not_a_picture = directory.path().join("clip.mp4");
        std::fs::write(&not_a_picture, b"\x00\x00\x00\x18ftypmp42").unwrap();

        assert_eq!(picture_format_of(&png_named_jpg), Some(ImageFormat::Png));
        assert_eq!(picture_format_of(&not_a_picture), None);
        assert_eq!(
            picture_format_of(&directory.path().join("missing.png")),
            None
        );
    }

    #[test]
    fn probing_gives_the_size_and_no_duration() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_picture(directory.path(), "logo.png", ImageFormat::Png);

        let description = probe_picture(&path).unwrap();

        assert_eq!(description.kind, MediaKind::Image);
        assert_eq!(description.duration, None);
        assert_eq!(
            description.picture_size,
            Some(PictureSize {
                width_in_pixels: 6,
                height_in_pixels: 4
            })
        );
        assert_eq!(description.display_name, "logo.png");
    }

    #[test]
    fn the_same_picture_is_shown_at_every_time() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_picture(directory.path(), "photo.jpg", ImageFormat::Jpeg);
        let mut source = StillPictureSource::open(&path).unwrap();

        let first = source.frame_at(Timestamp::ZERO).unwrap();
        let later = source.frame_at(Timestamp::from_whole_seconds(30)).unwrap();

        assert!(Arc::ptr_eq(&first, &later));
        assert_eq!((first.width(), first.height()), (6, 4));
    }
}
