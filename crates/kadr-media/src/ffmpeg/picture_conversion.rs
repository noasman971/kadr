//! Turning FFmpeg's decoded pictures into Kadr [`VideoFrame`]s.
//!
//! - 8-bit YUV 4:2:0 pictures (almost every video) are **copied as they are**:
//!   no colour conversion at all.
//! - Other YUV layouts (4:2:2, 4:4:4, 10-bit…) are converted to 8-bit 4:2:0 by
//!   swscale, which keeps the colour matrix.
//! - RGB pictures and pictures with transparency (PNG, WebP…) become RGBA.

use std::path::Path;

use ffmpeg_next as ffmpeg;
use ffmpeg_next::format::Pixel;
use image::RgbaImage;

use super::decoding_failure;
use crate::error::MediaError;
use crate::video_frame::{VideoFrame, YuvColorSpace, YuvMatrix, YuvPicture, YuvRange};

/// What a decoded picture becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConversionTarget {
    /// Already 8-bit YUV 4:2:0: copy the planes.
    CopyYuv420,
    /// Another YUV layout: convert to 8-bit YUV 4:2:0.
    ScaleToYuv420,
    /// RGB or transparent: convert to RGBA.
    ScaleToRgba,
}

/// Chooses the target for a pixel format.
fn conversion_target_for(format: Pixel) -> ConversionTarget {
    if matches!(format, Pixel::YUV420P | Pixel::YUVJ420P) {
        return ConversionTarget::CopyYuv420;
    }
    let name = pixel_format_name(format);
    let is_rgb_or_transparent = ["rgb", "bgr", "gbr", "pal8", "yuva", "ya8", "ya16"]
        .iter()
        .any(|marker| name.contains(marker));
    if is_rgb_or_transparent {
        ConversionTarget::ScaleToRgba
    } else {
        ConversionTarget::ScaleToYuv420
    }
}

/// Whether the pixel format itself says "full range" (the old `yuvj` formats).
pub(crate) fn is_full_range_format(format: Pixel) -> bool {
    pixel_format_name(format).starts_with("yuvj")
}

/// FFmpeg's name of a pixel format, for example `"yuv422p10le"` (empty if unknown).
pub(crate) fn pixel_format_name(format: Pixel) -> &'static str {
    format
        .descriptor()
        .map_or("", |descriptor| descriptor.name())
}

/// The colour space of a decoded YUV picture, from its metadata, its format and,
/// when the file says nothing, its height.
pub(crate) fn color_space_of(picture: &ffmpeg::frame::Video) -> YuvColorSpace {
    let matrix = match picture.color_space() {
        ffmpeg::color::Space::BT709 => YuvMatrix::Bt709,
        ffmpeg::color::Space::BT470BG
        | ffmpeg::color::Space::SMPTE170M
        | ffmpeg::color::Space::FCC => YuvMatrix::Bt601,
        _ => YuvMatrix::assumed_for_height(picture.height()),
    };
    let range = match picture.color_range() {
        ffmpeg::color::Range::JPEG => YuvRange::Full,
        ffmpeg::color::Range::MPEG => YuvRange::Limited,
        ffmpeg::color::Range::Unspecified if is_full_range_format(picture.format()) => {
            YuvRange::Full
        }
        ffmpeg::color::Range::Unspecified => YuvRange::Limited,
    };
    YuvColorSpace { matrix, range }
}

/// Converts decoded pictures, reusing one swscale context while the pictures
/// keep the same format and size.
#[derive(Default)]
pub(crate) struct PictureConverter {
    cached_scaler: Option<CachedScaler>,
}

struct CachedScaler {
    source_format: Pixel,
    width: u32,
    height: u32,
    scaler: ffmpeg::software::scaling::Context,
}

impl PictureConverter {
    /// Converts one decoded picture.
    pub(crate) fn convert(
        &mut self,
        picture: &ffmpeg::frame::Video,
        file_path: &Path,
    ) -> Result<VideoFrame, MediaError> {
        match conversion_target_for(picture.format()) {
            ConversionTarget::CopyYuv420 => Ok(VideoFrame::Yuv420(copy_yuv420_planes(
                picture,
                color_space_of(picture),
                file_path,
            )?)),
            ConversionTarget::ScaleToYuv420 => {
                let converted = self.scale(picture, Pixel::YUV420P, file_path)?;
                // swscale turns the old full-range "yuvj" formats into limited range;
                // otherwise it keeps the values as they are.
                let mut color_space = color_space_of(picture);
                if is_full_range_format(picture.format()) {
                    color_space.range = YuvRange::Limited;
                }
                Ok(VideoFrame::Yuv420(copy_yuv420_planes(
                    &converted,
                    color_space,
                    file_path,
                )?))
            }
            ConversionTarget::ScaleToRgba => {
                let converted = self.scale(picture, Pixel::RGBA, file_path)?;
                Ok(VideoFrame::Rgba(copy_rgba_plane(&converted, file_path)?))
            }
        }
    }

    /// Converts `picture` to `target_format` at the same size.
    fn scale(
        &mut self,
        picture: &ffmpeg::frame::Video,
        target_format: Pixel,
        file_path: &Path,
    ) -> Result<ffmpeg::frame::Video, MediaError> {
        let matches_picture = self.cached_scaler.as_ref().is_some_and(|cached| {
            cached.source_format == picture.format()
                && cached.width == picture.width()
                && cached.height == picture.height()
        });
        if !matches_picture {
            let scaler = ffmpeg::software::scaling::Context::get(
                picture.format(),
                picture.width(),
                picture.height(),
                target_format,
                picture.width(),
                picture.height(),
                ffmpeg::software::scaling::Flags::BILINEAR,
            )
            .map_err(|error| {
                decoding_failure(file_path, "preparing the picture conversion", error)
            })?;
            self.cached_scaler = Some(CachedScaler {
                source_format: picture.format(),
                width: picture.width(),
                height: picture.height(),
                scaler,
            });
        }
        let cached = self.cached_scaler.as_mut().ok_or_else(|| {
            decoding_failure(
                file_path,
                "preparing the picture conversion",
                "no converter",
            )
        })?;
        let mut converted = ffmpeg::frame::Video::empty();
        cached
            .scaler
            .run(picture, &mut converted)
            .map_err(|error| decoding_failure(file_path, "converting the picture", error))?;
        Ok(converted)
    }
}

/// Copies `rows` rows of `row_length` bytes from a plane whose rows are
/// `stride` bytes apart.
fn copy_plane(plane: &[u8], stride: usize, row_length: usize, rows: usize) -> Vec<u8> {
    let mut packed = Vec::with_capacity(row_length * rows);
    for row in 0..rows {
        let start = row * stride;
        packed.extend_from_slice(&plane[start..start + row_length]);
    }
    packed
}

/// Copies the three planes of an 8-bit YUV 4:2:0 picture.
fn copy_yuv420_planes(
    picture: &ffmpeg::frame::Video,
    color_space: YuvColorSpace,
    file_path: &Path,
) -> Result<YuvPicture, MediaError> {
    let (width, height) = (picture.width(), picture.height());
    let (chroma_width, chroma_height) = (
        YuvPicture::chroma_width(width) as usize,
        YuvPicture::chroma_height(height) as usize,
    );
    let y_plane = copy_plane(
        picture.data(0),
        picture.stride(0),
        width as usize,
        height as usize,
    );
    let u_plane = copy_plane(
        picture.data(1),
        picture.stride(1),
        chroma_width,
        chroma_height,
    );
    let v_plane = copy_plane(
        picture.data(2),
        picture.stride(2),
        chroma_width,
        chroma_height,
    );
    YuvPicture::from_planes(width, height, y_plane, u_plane, v_plane, color_space)
        .map_err(|error| decoding_failure(file_path, "reading the picture planes", error))
}

/// Copies an FFmpeg RGBA picture into an image.
fn copy_rgba_plane(
    picture: &ffmpeg::frame::Video,
    file_path: &Path,
) -> Result<RgbaImage, MediaError> {
    let (width, height) = (picture.width(), picture.height());
    let pixels = copy_plane(
        picture.data(0),
        picture.stride(0),
        width as usize * 4,
        height as usize,
    );
    RgbaImage::from_raw(width, height, pixels).ok_or_else(|| {
        decoding_failure(file_path, "reading the picture", "unexpected picture size")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_go_to_the_cheapest_target() {
        assert_eq!(
            conversion_target_for(Pixel::YUV420P),
            ConversionTarget::CopyYuv420
        );
        assert_eq!(
            conversion_target_for(Pixel::YUVJ420P),
            ConversionTarget::CopyYuv420
        );
        assert_eq!(
            conversion_target_for(Pixel::YUV422P10LE),
            ConversionTarget::ScaleToYuv420
        );
        assert_eq!(
            conversion_target_for(Pixel::NV12),
            ConversionTarget::ScaleToYuv420
        );
        assert_eq!(
            conversion_target_for(Pixel::RGBA),
            ConversionTarget::ScaleToRgba
        );
        assert_eq!(
            conversion_target_for(Pixel::RGB24),
            ConversionTarget::ScaleToRgba
        );
        assert_eq!(
            conversion_target_for(Pixel::YUVA420P),
            ConversionTarget::ScaleToRgba
        );
    }

    #[test]
    fn planes_are_copied_without_row_padding() {
        let padded_plane = [1, 2, 0, 0, 3, 4, 0, 0];
        assert_eq!(copy_plane(&padded_plane, 4, 2, 2), vec![1, 2, 3, 4]);
    }
}
