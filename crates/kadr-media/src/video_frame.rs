//! Decoded pictures, and the colour conversions between them.
//!
//! Most videos store pictures as **YUV 4:2:0**: a full-resolution brightness plane
//! (Y) and two quarter-resolution colour planes (U and V). Kadr keeps pictures in
//! that form as long as possible ([`VideoFrame::Yuv420`]): converting to RGBA and
//! back costs more than decoding itself. Pictures with transparency or stored as
//! RGB (PNG images, test patterns) are [`VideoFrame::Rgba`].
//!
//! Every conversion between YUV and RGB goes through [`YuvColorSpace`], with
//! explicit BT.601 / BT.709 coefficients and limited / full range. The GPU
//! renderer uses the very same coefficients ([`YuvColorSpace::yuv_to_rgb`],
//! [`YuvColorSpace::rgb_to_yuv`]), so CPU and GPU produce the same colours.
//! Exports are always BT.709, limited range, the standard for HD video.

use std::borrow::Cow;

use image::RgbaImage;
use rayon::prelude::*;

/// Bytes in one RGBA pixel.
const BYTES_PER_RGBA_PIXEL: usize = 4;
/// Largest 8-bit value, as a float.
const FULL_SCALE: f32 = 255.0;

/// Which formula links YUV and RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum YuvMatrix {
    /// Standard-definition video (and JPEG).
    Bt601,
    /// High-definition video: the standard for 720p and above, and for Kadr's exports.
    Bt709,
}

impl YuvMatrix {
    /// The matrix to assume when a file does not say: BT.709 from 720 lines up,
    /// BT.601 below, as video players do.
    pub fn assumed_for_height(height_in_pixels: u32) -> Self {
        if height_in_pixels >= 720 {
            Self::Bt709
        } else {
            Self::Bt601
        }
    }

    /// Contribution of red and blue to brightness (Kr, Kb); green gets the rest.
    fn red_and_blue_weights(self) -> (f32, f32) {
        match self {
            Self::Bt601 => (0.299, 0.114),
            Self::Bt709 => (0.2126, 0.0722),
        }
    }
}

/// Which values the YUV planes use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum YuvRange {
    /// Y from 16 to 235, U and V from 16 to 240 ("TV" or "MPEG" range): almost all videos.
    Limited,
    /// Every value from 0 to 255 ("PC" or "JPEG" range).
    Full,
}

/// The colour space of a YUV picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct YuvColorSpace {
    /// The YUV ↔ RGB formula.
    pub matrix: YuvMatrix,
    /// The range of values.
    pub range: YuvRange,
}

/// An affine conversion between two colour triples, all values from 0 to 1:
/// `output[row] = coefficients[row][0..3] · input + coefficients[row][3]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorConversion {
    /// One row per output channel: three weights, then an offset.
    pub rows: [[f32; 4]; 3],
}

impl ColorConversion {
    /// Applies the conversion to a triple of values from 0 to 1, clamping the result.
    pub fn apply(&self, input: [f32; 3]) -> [f32; 3] {
        self.rows.map(|[first, second, third, offset]| {
            (first * input[0] + second * input[1] + third * input[2] + offset).clamp(0.0, 1.0)
        })
    }
}

impl YuvColorSpace {
    /// BT.709, limited range: the colour space of every Kadr export.
    pub const BT709_LIMITED: Self = Self {
        matrix: YuvMatrix::Bt709,
        range: YuvRange::Limited,
    };

    /// `(scale, offset)` mapping a stored brightness value (0–1) to Y' (0–1), and
    /// the same for a stored colour value (0–1) to Cb/Cr (−0.5–0.5).
    fn stored_value_mapping(self) -> ((f32, f32), (f32, f32)) {
        match self.range {
            YuvRange::Limited => (
                (FULL_SCALE / 219.0, -16.0 / 219.0),
                (FULL_SCALE / 224.0, -128.0 / 224.0),
            ),
            YuvRange::Full => ((1.0, 0.0), (1.0, -128.0 / FULL_SCALE)),
        }
    }

    /// From stored (Y, U, V), each 0–1, to (R, G, B), each 0–1.
    pub fn yuv_to_rgb(self) -> ColorConversion {
        let (red_weight, blue_weight) = self.matrix.red_and_blue_weights();
        let green_weight = 1.0 - red_weight - blue_weight;
        let ((brightness_scale, brightness_offset), (color_scale, color_offset)) =
            self.stored_value_mapping();
        // R = Y' + 2(1−Kr)·Cr ; B = Y' + 2(1−Kb)·Cb ; G = Y' − g_b·Cb − g_r·Cr
        let red_from_cr = 2.0 * (1.0 - red_weight);
        let blue_from_cb = 2.0 * (1.0 - blue_weight);
        let green_from_cb = 2.0 * blue_weight * (1.0 - blue_weight) / green_weight;
        let green_from_cr = 2.0 * red_weight * (1.0 - red_weight) / green_weight;
        ColorConversion {
            rows: [
                [
                    brightness_scale,
                    0.0,
                    red_from_cr * color_scale,
                    brightness_offset + red_from_cr * color_offset,
                ],
                [
                    brightness_scale,
                    -green_from_cb * color_scale,
                    -green_from_cr * color_scale,
                    brightness_offset - (green_from_cb + green_from_cr) * color_offset,
                ],
                [
                    brightness_scale,
                    blue_from_cb * color_scale,
                    0.0,
                    brightness_offset + blue_from_cb * color_offset,
                ],
            ],
        }
    }

    /// From (R, G, B), each 0–1, to stored (Y, U, V), each 0–1.
    pub fn rgb_to_yuv(self) -> ColorConversion {
        let (red_weight, blue_weight) = self.matrix.red_and_blue_weights();
        let green_weight = 1.0 - red_weight - blue_weight;
        let (brightness_scale, brightness_offset, color_scale) = match self.range {
            YuvRange::Limited => (219.0 / FULL_SCALE, 16.0 / FULL_SCALE, 224.0 / FULL_SCALE),
            YuvRange::Full => (1.0, 0.0, 1.0),
        };
        let color_offset = 128.0 / FULL_SCALE;
        // Cb = (B − Y') / 2(1−Kb) ; Cr = (R − Y') / 2(1−Kr)
        let cb_divisor = 2.0 * (1.0 - blue_weight);
        let cr_divisor = 2.0 * (1.0 - red_weight);
        ColorConversion {
            rows: [
                [
                    red_weight * brightness_scale,
                    green_weight * brightness_scale,
                    blue_weight * brightness_scale,
                    brightness_offset,
                ],
                [
                    -red_weight / cb_divisor * color_scale,
                    -green_weight / cb_divisor * color_scale,
                    0.5 * color_scale,
                    color_offset,
                ],
                [
                    0.5 * color_scale,
                    -green_weight / cr_divisor * color_scale,
                    -blue_weight / cr_divisor * color_scale,
                    color_offset,
                ],
            ],
        }
    }
}

/// Converts a value from 0 to 1 to the nearest 8-bit value.
fn to_byte(value: f32) -> u8 {
    (value * FULL_SCALE).round().clamp(0.0, FULL_SCALE) as u8
}

/// A picture stored as YUV 4:2:0: Y at full size, U and V at half width and half
/// height (rounded up). Rows are tightly packed.
#[derive(Debug, Clone, PartialEq)]
pub struct YuvPicture {
    width: u32,
    height: u32,
    y_plane: Vec<u8>,
    u_plane: Vec<u8>,
    v_plane: Vec<u8>,
    color_space: YuvColorSpace,
}

/// Why a YUV picture could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "a {width}×{height} YUV 4:2:0 picture needs planes of {expected_luma} and 2 × {expected_chroma} bytes"
)]
pub struct YuvPlaneSizeError {
    /// Picture width.
    pub width: u32,
    /// Picture height.
    pub height: u32,
    /// Expected size of the Y plane.
    pub expected_luma: usize,
    /// Expected size of the U and V planes.
    pub expected_chroma: usize,
}

impl YuvPicture {
    /// Width of the U and V planes for a picture `width` wide.
    pub fn chroma_width(width: u32) -> u32 {
        width.div_ceil(2)
    }

    /// Height of the U and V planes for a picture `height` high.
    pub fn chroma_height(height: u32) -> u32 {
        height.div_ceil(2)
    }

    /// Wraps three tightly packed planes.
    ///
    /// # Errors
    ///
    /// Fails when a plane does not have the size the dimensions require.
    pub fn from_planes(
        width: u32,
        height: u32,
        y_plane: Vec<u8>,
        u_plane: Vec<u8>,
        v_plane: Vec<u8>,
        color_space: YuvColorSpace,
    ) -> Result<Self, YuvPlaneSizeError> {
        let expected_luma = width as usize * height as usize;
        let expected_chroma =
            Self::chroma_width(width) as usize * Self::chroma_height(height) as usize;
        if y_plane.len() != expected_luma
            || u_plane.len() != expected_chroma
            || v_plane.len() != expected_chroma
        {
            return Err(YuvPlaneSizeError {
                width,
                height,
                expected_luma,
                expected_chroma,
            });
        }
        Ok(Self {
            width,
            height,
            y_plane,
            u_plane,
            v_plane,
            color_space,
        })
    }

    /// Picture width.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Picture height.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The brightness plane, `width × height` bytes.
    pub fn y_plane(&self) -> &[u8] {
        &self.y_plane
    }

    /// The blue-difference plane, `chroma_width × chroma_height` bytes.
    pub fn u_plane(&self) -> &[u8] {
        &self.u_plane
    }

    /// The red-difference plane, `chroma_width × chroma_height` bytes.
    pub fn v_plane(&self) -> &[u8] {
        &self.v_plane
    }

    /// The colour space of the values.
    pub fn color_space(&self) -> YuvColorSpace {
        self.color_space
    }

    /// Converts to RGBA (opaque), each pixel using the colour of its 2×2 block.
    /// Rows are converted in parallel.
    pub fn to_rgba(&self) -> RgbaImage {
        let conversion = self.color_space.yuv_to_rgb();
        let (width, chroma_width) = (self.width as usize, Self::chroma_width(self.width) as usize);
        let mut pixels = vec![0; width * self.height as usize * BYTES_PER_RGBA_PIXEL];
        pixels
            .par_chunks_exact_mut(width * BYTES_PER_RGBA_PIXEL)
            .enumerate()
            .for_each(|(row, row_pixels)| {
                let luma_row = &self.y_plane[row * width..(row + 1) * width];
                let chroma_row_start = (row / 2) * chroma_width;
                for (column, pixel) in row_pixels
                    .as_chunks_mut::<BYTES_PER_RGBA_PIXEL>()
                    .0
                    .iter_mut()
                    .enumerate()
                {
                    let chroma_index = chroma_row_start + column / 2;
                    let [red, green, blue] = conversion.apply([
                        f32::from(luma_row[column]) / FULL_SCALE,
                        f32::from(self.u_plane[chroma_index]) / FULL_SCALE,
                        f32::from(self.v_plane[chroma_index]) / FULL_SCALE,
                    ]);
                    *pixel = [to_byte(red), to_byte(green), to_byte(blue), 255];
                }
            });
        RgbaImage::from_raw(self.width, self.height, pixels)
            .unwrap_or_else(|| RgbaImage::new(self.width, self.height))
    }

    /// Converts an RGBA picture (alpha ignored) to YUV 4:2:0. Each U and V value
    /// is the average over its 2×2 block. Rows are converted in parallel.
    pub fn from_rgba(image: &RgbaImage, color_space: YuvColorSpace) -> Self {
        let conversion = color_space.rgb_to_yuv();
        let (width, height) = (image.width(), image.height());
        let row_length = width as usize * BYTES_PER_RGBA_PIXEL;
        let pixels = image.as_raw();
        let rgb_at = |column: usize, row: usize| {
            let start = row * row_length + column * BYTES_PER_RGBA_PIXEL;
            [pixels[start], pixels[start + 1], pixels[start + 2]]
                .map(|value| f32::from(value) / FULL_SCALE)
        };

        let mut y_plane = vec![0; width as usize * height as usize];
        y_plane
            .par_chunks_exact_mut(width as usize)
            .enumerate()
            .for_each(|(row, luma_row)| {
                for (column, luma) in luma_row.iter_mut().enumerate() {
                    *luma = to_byte(conversion.apply(rgb_at(column, row))[0]);
                }
            });

        let (chroma_width, chroma_height) = (
            Self::chroma_width(width) as usize,
            Self::chroma_height(height) as usize,
        );
        let mut chroma_pairs = vec![(0_u8, 0_u8); chroma_width * chroma_height];
        chroma_pairs
            .par_chunks_exact_mut(chroma_width)
            .enumerate()
            .for_each(|(chroma_row, chroma_row_values)| {
                for (chroma_column, pair) in chroma_row_values.iter_mut().enumerate() {
                    let block_average = average_rgb_of_block(
                        &rgb_at,
                        chroma_column,
                        chroma_row,
                        width as usize,
                        height as usize,
                    );
                    let [_, u, v] = conversion.apply(block_average);
                    *pair = (to_byte(u), to_byte(v));
                }
            });
        let (u_plane, v_plane) = chroma_pairs.into_iter().unzip();

        Self {
            width,
            height,
            y_plane,
            u_plane,
            v_plane,
            color_space,
        }
    }
}

/// The average RGB of the (up to) 2×2 pixels covered by one chroma sample.
fn average_rgb_of_block(
    rgb_at: &(impl Fn(usize, usize) -> [f32; 3] + Sync),
    chroma_column: usize,
    chroma_row: usize,
    width: usize,
    height: usize,
) -> [f32; 3] {
    let mut total = [0.0; 3];
    let mut pixel_count = 0.0;
    for row in (chroma_row * 2)..(chroma_row * 2 + 2).min(height) {
        for column in (chroma_column * 2)..(chroma_column * 2 + 2).min(width) {
            let rgb = rgb_at(column, row);
            for channel in 0..3 {
                total[channel] += rgb[channel];
            }
            pixel_count += 1.0;
        }
    }
    total.map(|channel_total| channel_total / pixel_count)
}

/// One decoded picture.
#[derive(Debug, Clone, PartialEq)]
pub enum VideoFrame {
    /// 8-bit RGBA, top row first, straight (not premultiplied) alpha.
    Rgba(RgbaImage),
    /// YUV 4:2:0, as most videos store pictures. Always opaque.
    Yuv420(YuvPicture),
}

impl VideoFrame {
    /// Picture width.
    pub fn width(&self) -> u32 {
        match self {
            Self::Rgba(image) => image.width(),
            Self::Yuv420(picture) => picture.width(),
        }
    }

    /// Picture height.
    pub fn height(&self) -> u32 {
        match self {
            Self::Rgba(image) => image.height(),
            Self::Yuv420(picture) => picture.height(),
        }
    }

    /// The picture as RGBA, converting only if needed.
    pub fn to_rgba(&self) -> Cow<'_, RgbaImage> {
        match self {
            Self::Rgba(image) => Cow::Borrowed(image),
            Self::Yuv420(picture) => Cow::Owned(picture.to_rgba()),
        }
    }

    /// The picture as YUV 4:2:0 in `color_space`, converting only if needed.
    pub fn to_yuv420(&self, color_space: YuvColorSpace) -> Cow<'_, YuvPicture> {
        match self {
            Self::Yuv420(picture) if picture.color_space() == color_space => Cow::Borrowed(picture),
            Self::Yuv420(picture) => {
                Cow::Owned(YuvPicture::from_rgba(&picture.to_rgba(), color_space))
            }
            Self::Rgba(image) => Cow::Owned(YuvPicture::from_rgba(image, color_space)),
        }
    }
}

impl From<RgbaImage> for VideoFrame {
    fn from(image: RgbaImage) -> Self {
        Self::Rgba(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn yuv_of(rgb: [u8; 3], color_space: YuvColorSpace) -> [u8; 3] {
        let picture = YuvPicture::from_rgba(
            &RgbaImage::from_pixel(2, 2, Rgba([rgb[0], rgb[1], rgb[2], 255])),
            color_space,
        );
        [
            picture.y_plane()[0],
            picture.u_plane()[0],
            picture.v_plane()[0],
        ]
    }

    #[test]
    fn bt709_limited_reference_values() {
        let space = YuvColorSpace::BT709_LIMITED;
        assert_eq!(yuv_of([255, 255, 255], space), [235, 128, 128]);
        assert_eq!(yuv_of([0, 0, 0], space), [16, 128, 128]);
        // Pure red: Y = 16 + 219 × 0.2126 ≈ 62.6, V at its maximum (240).
        assert_eq!(yuv_of([255, 0, 0], space), [63, 102, 240]);
    }

    #[test]
    fn full_range_uses_every_value() {
        let space = YuvColorSpace {
            matrix: YuvMatrix::Bt601,
            range: YuvRange::Full,
        };
        assert_eq!(yuv_of([255, 255, 255], space), [255, 128, 128]);
        assert_eq!(yuv_of([0, 0, 0], space), [0, 128, 128]);
    }

    #[test]
    fn colours_survive_a_round_trip_through_every_colour_space() {
        let colours = [
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [200, 120, 40],
            [17, 200, 230],
            [128, 128, 128],
        ];
        for matrix in [YuvMatrix::Bt601, YuvMatrix::Bt709] {
            for range in [YuvRange::Limited, YuvRange::Full] {
                let space = YuvColorSpace { matrix, range };
                for colour in colours {
                    let image =
                        RgbaImage::from_pixel(4, 4, Rgba([colour[0], colour[1], colour[2], 255]));
                    let back = YuvPicture::from_rgba(&image, space).to_rgba();
                    let pixel = back.get_pixel(1, 1);
                    for channel in 0..3 {
                        assert!(
                            pixel[channel].abs_diff(colour[channel]) <= 2,
                            "{colour:?} became {pixel:?} in {space:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn odd_sizes_get_rounded_up_colour_planes() {
        let picture = YuvPicture::from_rgba(&RgbaImage::new(5, 3), YuvColorSpace::BT709_LIMITED);
        assert_eq!(picture.u_plane().len(), 3 * 2);
        assert_eq!(picture.to_rgba().dimensions(), (5, 3));
        assert!(
            YuvPicture::from_planes(
                4,
                4,
                vec![0; 16],
                vec![0; 4],
                vec![0; 3],
                YuvColorSpace::BT709_LIMITED
            )
            .is_err()
        );
    }

    #[test]
    fn converting_to_the_same_colour_space_borrows() {
        let picture = YuvPicture::from_rgba(&RgbaImage::new(2, 2), YuvColorSpace::BT709_LIMITED);
        let frame = VideoFrame::Yuv420(picture);
        assert!(matches!(
            frame.to_yuv420(YuvColorSpace::BT709_LIMITED),
            Cow::Borrowed(_)
        ));
        assert!(matches!(frame.to_rgba(), Cow::Owned(_)));
    }

    #[test]
    fn unknown_matrices_follow_the_picture_height() {
        assert_eq!(YuvMatrix::assumed_for_height(1080), YuvMatrix::Bt709);
        assert_eq!(YuvMatrix::assumed_for_height(480), YuvMatrix::Bt601);
    }
}
