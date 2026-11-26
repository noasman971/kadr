//! Preparing pictures for Claude: scaled down to a reasonable size and encoded
//! as JPEG.
//!
//! Claude reads pictures of at most about a thousand pixels per side; larger
//! ones only cost more of the conversation. Every picture is therefore scaled so
//! its longest side is at most [`MAXIMUM_PICTURE_SIDE_IN_PIXELS`], and the tool
//! states the original size and the scale factor so Claude can convert what it
//! measures on the picture back to source pixels.

use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::{DynamicImage, RgbaImage};

use crate::output::ToolError;

/// The longest side of a picture sent to Claude, in pixels.
pub const MAXIMUM_PICTURE_SIDE_IN_PIXELS: u32 = 1024;

/// JPEG quality: high enough for small text and faces to stay readable.
const JPEG_QUALITY: u8 = 85;

/// A picture ready to send, with what Claude needs to read coordinates on it.
#[derive(Debug, Clone, PartialEq)]
pub struct PictureForClaude {
    /// The JPEG file.
    pub jpeg_data: Vec<u8>,
    /// Size of the picture before scaling.
    pub original_width_in_pixels: u32,
    /// Size of the picture before scaling.
    pub original_height_in_pixels: u32,
    /// Size of the JPEG picture.
    pub sent_width_in_pixels: u32,
    /// Size of the JPEG picture.
    pub sent_height_in_pixels: u32,
}

impl PictureForClaude {
    /// Scales `picture` down (never up) so its longest side is at most
    /// [`MAXIMUM_PICTURE_SIDE_IN_PIXELS`], and encodes it.
    ///
    /// # Errors
    ///
    /// Fails if JPEG encoding fails.
    pub fn prepare(picture: &RgbaImage) -> Result<Self, ToolError> {
        let (original_width, original_height) = picture.dimensions();
        let (sent_width, sent_height) = scaled_size(original_width, original_height);
        let scaled = if (sent_width, sent_height) == (original_width, original_height) {
            picture.clone()
        } else {
            imageops::resize(picture, sent_width, sent_height, FilterType::CatmullRom)
        };
        Ok(Self {
            jpeg_data: encode_jpeg(scaled)?,
            original_width_in_pixels: original_width,
            original_height_in_pixels: original_height,
            sent_width_in_pixels: sent_width,
            sent_height_in_pixels: sent_height,
        })
    }

    /// How many original pixels one pixel of the sent picture covers (1 when
    /// not scaled).
    pub fn original_pixels_per_sent_pixel(&self) -> f64 {
        f64::from(self.original_width_in_pixels) / f64::from(self.sent_width_in_pixels.max(1))
    }

    /// A sentence giving both sizes and how to convert coordinates.
    pub fn describe_size(&self) -> String {
        let original = format!(
            "{}×{} pixels",
            self.original_width_in_pixels, self.original_height_in_pixels
        );
        if self.sent_width_in_pixels == self.original_width_in_pixels {
            return format!("The picture is shown at its original size, {original}.");
        }
        format!(
            "Original size: {original}. The picture is scaled down to {}×{} pixels to save \
             space: multiply positions and sizes measured on it by {:.4} to get original pixels.",
            self.sent_width_in_pixels,
            self.sent_height_in_pixels,
            self.original_pixels_per_sent_pixel()
        )
    }
}

/// The size with the longest side at most the maximum, proportions kept.
fn scaled_size(width: u32, height: u32) -> (u32, u32) {
    let longest_side = width.max(height);
    if longest_side <= MAXIMUM_PICTURE_SIDE_IN_PIXELS {
        return (width, height);
    }
    let factor = f64::from(MAXIMUM_PICTURE_SIDE_IN_PIXELS) / f64::from(longest_side);
    let scale = |side: u32| ((f64::from(side) * factor).round() as u32).max(1);
    (scale(width), scale(height))
}

/// Encodes an RGBA picture as JPEG (which has no transparency: alpha is dropped).
fn encode_jpeg(picture: RgbaImage) -> Result<Vec<u8>, ToolError> {
    let rgb_picture = DynamicImage::ImageRgba8(picture).into_rgb8();
    let mut jpeg_data = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg_data, JPEG_QUALITY)
        .encode_image(&rgb_picture)
        .map_err(|error| ToolError::new(format!("the picture could not be encoded: {error}")))?;
    Ok(jpeg_data)
}

#[cfg(test)]
mod tests {
    use image::Rgba;

    use super::*;

    #[test]
    fn large_pictures_are_scaled_to_1024_pixels_keeping_proportions() {
        let picture = RgbaImage::from_pixel(1920, 1080, Rgba([10, 200, 30, 255]));
        let prepared = PictureForClaude::prepare(&picture).unwrap();
        assert_eq!(
            (
                prepared.sent_width_in_pixels,
                prepared.sent_height_in_pixels
            ),
            (1024, 576)
        );
        assert_eq!(
            (
                prepared.original_width_in_pixels,
                prepared.original_height_in_pixels
            ),
            (1920, 1080)
        );
        assert!((prepared.original_pixels_per_sent_pixel() - 1.875).abs() < 1e-9);
        assert!(prepared.describe_size().contains("1.8750"));
        // A JPEG file starts with the bytes FF D8.
        assert_eq!(&prepared.jpeg_data[..2], &[0xFF, 0xD8]);

        let decoded = image::load_from_memory(&prepared.jpeg_data).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1024, 576));
    }

    #[test]
    fn small_pictures_are_not_enlarged() {
        let picture = RgbaImage::from_pixel(640, 360, Rgba([0, 0, 0, 255]));
        let prepared = PictureForClaude::prepare(&picture).unwrap();
        assert_eq!(
            (
                prepared.sent_width_in_pixels,
                prepared.sent_height_in_pixels
            ),
            (640, 360)
        );
        assert!(prepared.describe_size().contains("original size"));
    }

    #[test]
    fn vertical_pictures_are_limited_by_their_height() {
        assert_eq!(scaled_size(1080, 1920), (576, 1024));
    }
}
