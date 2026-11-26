//! Where the pixels of a picture sit in a MediaCodec buffer.
//!
//! Without an output surface ("byte buffer mode"), a decoder hands each picture
//! over as one block of bytes, and an encoder expects one. How the Y, U and V
//! planes sit in that block depends on the codec: planar (I420) or semi-planar
//! (NV12, NV21), rows padded to 16, 64 or 128 bytes, planes padded to a multiple
//! of 16 or 32 rows, and a crop rectangle when the coded size is larger than the
//! picture (1080p is usually coded as 1088 lines).
//!
//! Android describes the layout in two ways, used in this order:
//!
//! 1. the `image-data` entry of the codec's format: a `MediaImage2` structure
//!    giving, for each plane, its offset and the distance between two pixels and
//!    between two rows. This is what `MediaCodec.getOutputImage` relies on in Java,
//!    and it covers every layout;
//! 2. otherwise, the `color-format`, `stride` and `slice-height` entries, for the
//!    planar and semi-planar layouts and their vendor variants.
//!
//! Nothing here calls Android: the layouts are plain numbers, so they are tested
//! on the development machine.

use std::mem::MaybeUninit;

use crate::video_frame::{YuvColorSpace, YuvPicture};

/// `MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Planar`: I420.
pub(crate) const COLOR_FORMAT_YUV420_PLANAR: i32 = 19;
/// `COLOR_FormatYUV420PackedPlanar`: I420 without padding between planes.
const COLOR_FORMAT_YUV420_PACKED_PLANAR: i32 = 20;
/// `COLOR_FormatYUV420SemiPlanar`: NV12.
pub(crate) const COLOR_FORMAT_YUV420_SEMI_PLANAR: i32 = 21;
/// `COLOR_FormatYUV420PackedSemiPlanar`: NV12 without padding between planes.
const COLOR_FORMAT_YUV420_PACKED_SEMI_PLANAR: i32 = 39;
/// `COLOR_TI_FormatYUV420PackedSemiPlanar` (Texas Instruments): NV12.
const COLOR_FORMAT_TI_YUV420_PACKED_SEMI_PLANAR: i32 = 0x7F00_0100;
/// `COLOR_QCOM_FormatYUV420SemiPlanar` (Qualcomm): NV12.
const COLOR_FORMAT_QCOM_YUV420_SEMI_PLANAR: i32 = 0x7FA3_0C00;
/// `QOMX_COLOR_FORMATYUV420PackedSemiPlanar32m` (Qualcomm "Venus"): NV12 with
/// rows aligned on 128 bytes and planes on 32 rows.
const COLOR_FORMAT_QCOM_YUV420_SEMI_PLANAR_32M: i32 = 0x7FA3_0C04;
/// `COLOR_FormatYUV420Flexible`: "any YUV 4:2:0 layout, described by `image-data`".
/// This is what Kadr asks encoders for, as every Android encoder must accept it.
pub(crate) const COLOR_FORMAT_YUV420_FLEXIBLE: i32 = 0x7F42_0888;

/// Qualcomm "Venus" row alignment, in bytes.
const QCOM_32M_ROW_ALIGNMENT: usize = 128;
/// Qualcomm "Venus" plane alignment, in rows.
const QCOM_32M_PLANE_ALIGNMENT: usize = 32;

/// Size of the `MediaImage2` structure (Android 6 and later): six 32-bit fields,
/// then four planes of five 32-bit fields.
const MEDIA_IMAGE2_SIZE_IN_BYTES: usize = 6 * 4 + 4 * 5 * 4;
/// `MediaImage2::MEDIA_IMAGE_TYPE_YUV`.
const MEDIA_IMAGE_TYPE_YUV: u32 = 1;
/// Bits per sample Kadr reads and writes.
const SUPPORTED_BIT_DEPTH: u32 = 8;

/// How one plane (Y, U or V) is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlaneLayout {
    /// Where the plane's first sample is, from the start of the buffer.
    pub(crate) offset_in_bytes: usize,
    /// From one sample to the next one on the same row: 1 when planar, 2 when
    /// U and V are interleaved.
    pub(crate) pixel_step_in_bytes: usize,
    /// From one row to the next.
    pub(crate) row_step_in_bytes: usize,
}

/// How a YUV 4:2:0 picture of the coded size is stored in a codec buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PictureBufferLayout {
    /// Width of the coded picture (before cropping), in pixels.
    pub(crate) coded_width: usize,
    /// Height of the coded picture (before cropping), in pixels.
    pub(crate) coded_height: usize,
    /// The Y, U and V planes, in this order.
    pub(crate) planes: [PlaneLayout; 3],
}

/// The part of the coded picture that is the real picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CropRectangle {
    /// First column.
    pub(crate) left: usize,
    /// First row.
    pub(crate) top: usize,
    /// Number of columns.
    pub(crate) width: usize,
    /// Number of rows.
    pub(crate) height: usize,
}

impl CropRectangle {
    /// The whole picture.
    pub(crate) fn whole_picture(width: usize, height: usize) -> Self {
        Self {
            left: 0,
            top: 0,
            width,
            height,
        }
    }

    /// From the inclusive bounds Android uses (`crop` rectangle, or `crop-left`,
    /// `crop-top`, `crop-right`, `crop-bottom`). `None` when they make no sense.
    pub(crate) fn from_inclusive_bounds(
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    ) -> Option<Self> {
        let left = usize::try_from(left).ok()?;
        let top = usize::try_from(top).ok()?;
        let right = usize::try_from(right).ok()?;
        let bottom = usize::try_from(bottom).ok()?;
        (right >= left && bottom >= top).then_some(Self {
            left,
            top,
            width: right - left + 1,
            height: bottom - top + 1,
        })
    }
}

/// Why a codec buffer could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum PictureLayoutError {
    /// The codec uses a pixel layout Kadr does not know.
    #[error(
        "the codec stores pictures in a layout Kadr cannot read (colour format {color_format:#x})"
    )]
    UnsupportedColorFormat {
        /// The `color-format` the codec announced.
        color_format: i32,
    },
    /// The `image-data` description is not 8-bit YUV 4:2:0.
    #[error("the codec describes its pictures as something other than 8-bit YUV 4:2:0")]
    UnsupportedImageDescription,
    /// The buffer is smaller than the layout says.
    #[error("the codec buffer holds {available} bytes but the picture needs {needed}")]
    BufferTooSmall {
        /// Bytes the layout reaches.
        needed: usize,
        /// Bytes in the buffer.
        available: usize,
    },
    /// The crop rectangle goes past the coded picture.
    #[error("the picture's crop rectangle goes past the decoded picture")]
    CropOutsidePicture,
    /// The picture to encode does not have the size the encoder was set up for.
    #[error(
        "the picture is {actual_width}×{actual_height} but the encoder expects {expected_width}×{expected_height}"
    )]
    WrongPictureSize {
        /// Size the encoder was configured with.
        expected_width: usize,
        /// Size the encoder was configured with.
        expected_height: usize,
        /// Size of the picture given.
        actual_width: usize,
        /// Size of the picture given.
        actual_height: usize,
    },
}

/// What a codec format says about its pictures, read from its entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PictureFormatEntries {
    /// `width`.
    pub(crate) width: usize,
    /// `height`.
    pub(crate) height: usize,
    /// `color-format`.
    pub(crate) color_format: i32,
    /// `stride`, when present and positive.
    pub(crate) stride: Option<usize>,
    /// `slice-height`, when present and positive.
    pub(crate) slice_height: Option<usize>,
}

impl PictureBufferLayout {
    /// The layout described by a codec format: its `image-data` structure when
    /// present and usable, otherwise its colour format and padding.
    ///
    /// # Errors
    ///
    /// Fails when neither describes an 8-bit YUV 4:2:0 layout Kadr can use.
    pub(crate) fn from_format(
        entries: PictureFormatEntries,
        image_data: Option<&[u8]>,
    ) -> Result<Self, PictureLayoutError> {
        if let Some(layout) = image_data.and_then(Self::from_media_image2) {
            return Ok(layout);
        }
        Self::from_color_format(entries)
    }

    /// Reads a `MediaImage2` structure (native byte order: little endian on every
    /// Android processor). `None` when it is not 8-bit YUV 4:2:0 with positive steps.
    pub(crate) fn from_media_image2(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < MEDIA_IMAGE2_SIZE_IN_BYTES {
            return None;
        }
        let field = |index: usize| -> u32 {
            let start = index * 4;
            u32::from_le_bytes([
                bytes[start],
                bytes[start + 1],
                bytes[start + 2],
                bytes[start + 3],
            ])
        };
        let (image_type, plane_count, width, height, bit_depth, allocated_bit_depth) =
            (field(0), field(1), field(2), field(3), field(4), field(5));
        let is_eight_bit_yuv = image_type == MEDIA_IMAGE_TYPE_YUV
            && plane_count == 3
            && bit_depth == SUPPORTED_BIT_DEPTH
            && allocated_bit_depth == SUPPORTED_BIT_DEPTH;
        if !is_eight_bit_yuv {
            return None;
        }
        const FIELDS_BEFORE_PLANES: usize = 6;
        const FIELDS_PER_PLANE: usize = 5;
        let read_plane = |plane_index: usize, expected_subsampling: u32| -> Option<PlaneLayout> {
            let first_field = FIELDS_BEFORE_PLANES + plane_index * FIELDS_PER_PLANE;
            // The steps are signed in C; a negative one (a flipped picture) is refused.
            let pixel_step = i32::from_le_bytes(field(first_field + 1).to_le_bytes());
            let row_step = i32::from_le_bytes(field(first_field + 2).to_le_bytes());
            let horizontal_subsampling = field(first_field + 3);
            let vertical_subsampling = field(first_field + 4);
            if horizontal_subsampling != expected_subsampling
                || vertical_subsampling != expected_subsampling
            {
                return None;
            }
            Some(PlaneLayout {
                offset_in_bytes: usize::try_from(field(first_field)).ok()?,
                pixel_step_in_bytes: usize::try_from(pixel_step).ok().filter(|step| *step > 0)?,
                row_step_in_bytes: usize::try_from(row_step).ok().filter(|step| *step > 0)?,
            })
        };
        Some(Self {
            coded_width: usize::try_from(width).ok()?,
            coded_height: usize::try_from(height).ok()?,
            planes: [read_plane(0, 1)?, read_plane(1, 2)?, read_plane(2, 2)?],
        })
    }

    /// The layout of the standard planar and semi-planar colour formats.
    fn from_color_format(entries: PictureFormatEntries) -> Result<Self, PictureLayoutError> {
        let PictureFormatEntries {
            width,
            height,
            color_format,
            stride,
            slice_height,
        } = entries;
        let is_qcom_32m = color_format == COLOR_FORMAT_QCOM_YUV420_SEMI_PLANAR_32M;
        let default_stride = if is_qcom_32m {
            width.next_multiple_of(QCOM_32M_ROW_ALIGNMENT)
        } else {
            width
        };
        let default_slice_height = if is_qcom_32m {
            height.next_multiple_of(QCOM_32M_PLANE_ALIGNMENT)
        } else {
            height
        };
        // Some decoders announce a stride or slice height smaller than the picture
        // (or zero): the picture size is the floor.
        let stride = stride.unwrap_or(default_stride).max(width);
        let slice_height = slice_height.unwrap_or(default_slice_height).max(height);
        let luma_size = stride * slice_height;
        let luma = PlaneLayout {
            offset_in_bytes: 0,
            pixel_step_in_bytes: 1,
            row_step_in_bytes: stride,
        };
        let planes = match color_format {
            COLOR_FORMAT_YUV420_PLANAR | COLOR_FORMAT_YUV420_PACKED_PLANAR => {
                let chroma_stride = stride.div_ceil(2);
                let chroma_size = chroma_stride * slice_height.div_ceil(2);
                [
                    luma,
                    PlaneLayout {
                        offset_in_bytes: luma_size,
                        pixel_step_in_bytes: 1,
                        row_step_in_bytes: chroma_stride,
                    },
                    PlaneLayout {
                        offset_in_bytes: luma_size + chroma_size,
                        pixel_step_in_bytes: 1,
                        row_step_in_bytes: chroma_stride,
                    },
                ]
            }
            COLOR_FORMAT_YUV420_SEMI_PLANAR
            | COLOR_FORMAT_YUV420_PACKED_SEMI_PLANAR
            | COLOR_FORMAT_TI_YUV420_PACKED_SEMI_PLANAR
            | COLOR_FORMAT_QCOM_YUV420_SEMI_PLANAR
            | COLOR_FORMAT_QCOM_YUV420_SEMI_PLANAR_32M => [
                luma,
                PlaneLayout {
                    offset_in_bytes: luma_size,
                    pixel_step_in_bytes: 2,
                    row_step_in_bytes: stride,
                },
                PlaneLayout {
                    offset_in_bytes: luma_size + 1,
                    pixel_step_in_bytes: 2,
                    row_step_in_bytes: stride,
                },
            ],
            _ => return Err(PictureLayoutError::UnsupportedColorFormat { color_format }),
        };
        Ok(Self {
            coded_width: width,
            coded_height: height,
            planes,
        })
    }

    /// One past the last byte the planes of a `width × height` picture reach.
    fn bytes_reached(&self, width: usize, height: usize) -> usize {
        let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
        self.planes
            .iter()
            .enumerate()
            .map(|(plane_index, plane)| {
                let (columns, rows) = if plane_index == 0 {
                    (width, height)
                } else {
                    (chroma_width, chroma_height)
                };
                if columns == 0 || rows == 0 {
                    return plane.offset_in_bytes;
                }
                plane.offset_in_bytes
                    + (rows - 1) * plane.row_step_in_bytes
                    + (columns - 1) * plane.pixel_step_in_bytes
                    + 1
            })
            .max()
            .unwrap_or(0)
    }

    /// How many bytes an encoder input buffer holding a whole picture contains.
    pub(crate) fn picture_size_in_bytes(&self) -> usize {
        self.bytes_reached(self.coded_width, self.coded_height)
    }

    /// Copies the `crop` part of the picture in `buffer` into a tightly packed
    /// [`YuvPicture`].
    ///
    /// # Errors
    ///
    /// Fails when the crop goes past the coded picture or the buffer is too small.
    pub(crate) fn read_picture(
        &self,
        buffer: &[u8],
        crop: CropRectangle,
        color_space: YuvColorSpace,
    ) -> Result<YuvPicture, PictureLayoutError> {
        let crop_fits = crop.left + crop.width <= self.coded_width
            && crop.top + crop.height <= self.coded_height
            && crop.width > 0
            && crop.height > 0;
        if !crop_fits {
            return Err(PictureLayoutError::CropOutsidePicture);
        }
        let needed = self.bytes_reached(crop.left + crop.width, crop.top + crop.height);
        if needed > buffer.len() {
            return Err(PictureLayoutError::BufferTooSmall {
                needed,
                available: buffer.len(),
            });
        }
        let (width, height) = (crop.width, crop.height);
        let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
        let luma_window = PlaneWindow {
            first_column: crop.left,
            first_row: crop.top,
            columns: width,
            rows: height,
        };
        let chroma_window = PlaneWindow {
            first_column: crop.left / 2,
            first_row: crop.top / 2,
            columns: chroma_width,
            rows: chroma_height,
        };
        let y_plane = gather_plane(buffer, self.planes[0], luma_window);
        let u_plane = gather_plane(buffer, self.planes[1], chroma_window);
        let v_plane = gather_plane(buffer, self.planes[2], chroma_window);
        YuvPicture::from_planes(
            width as u32,
            height as u32,
            y_plane,
            u_plane,
            v_plane,
            color_space,
        )
        // The planes were sized from the same width and height: this cannot fail.
        .map_err(|_| PictureLayoutError::CropOutsidePicture)
    }

    /// Writes `picture` (which must have the coded size) into an encoder input
    /// buffer, and returns how many bytes to hand to the encoder. Padding bytes
    /// are left as they are.
    ///
    /// # Errors
    ///
    /// Fails when the picture has the wrong size or the buffer is too small.
    pub(crate) fn write_picture<Byte: WritableByte>(
        &self,
        picture: &YuvPicture,
        buffer: &mut [Byte],
    ) -> Result<usize, PictureLayoutError> {
        let (width, height) = (picture.width() as usize, picture.height() as usize);
        if width != self.coded_width || height != self.coded_height {
            return Err(PictureLayoutError::WrongPictureSize {
                expected_width: self.coded_width,
                expected_height: self.coded_height,
                actual_width: width,
                actual_height: height,
            });
        }
        let needed = self.picture_size_in_bytes();
        if needed > buffer.len() {
            return Err(PictureLayoutError::BufferTooSmall {
                needed,
                available: buffer.len(),
            });
        }
        let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
        scatter_plane(picture.y_plane(), self.planes[0], width, height, buffer);
        scatter_plane(
            picture.u_plane(),
            self.planes[1],
            chroma_width,
            chroma_height,
            buffer,
        );
        scatter_plane(
            picture.v_plane(),
            self.planes[2],
            chroma_width,
            chroma_height,
            buffer,
        );
        Ok(needed)
    }
}

/// A byte of a buffer Kadr writes into: plain bytes, or the not-yet-initialised
/// bytes of a MediaCodec input buffer.
pub(crate) trait WritableByte {
    /// Stores `value` in this byte.
    fn store(&mut self, value: u8);
}

impl WritableByte for u8 {
    fn store(&mut self, value: u8) {
        *self = value;
    }
}

impl WritableByte for MaybeUninit<u8> {
    fn store(&mut self, value: u8) {
        self.write(value);
    }
}

/// The samples of a plane to read.
#[derive(Debug, Clone, Copy)]
struct PlaneWindow {
    first_column: usize,
    first_row: usize,
    columns: usize,
    rows: usize,
}

/// Copies a window of a plane into a tightly packed vector. The caller checked
/// that the window lies inside the buffer.
fn gather_plane(buffer: &[u8], plane: PlaneLayout, window: PlaneWindow) -> Vec<u8> {
    let mut samples = Vec::with_capacity(window.columns * window.rows);
    for row in window.first_row..window.first_row + window.rows {
        let row_start = plane.offset_in_bytes
            + row * plane.row_step_in_bytes
            + window.first_column * plane.pixel_step_in_bytes;
        if plane.pixel_step_in_bytes == 1 {
            samples.extend_from_slice(&buffer[row_start..row_start + window.columns]);
        } else {
            samples.extend(
                (0..window.columns)
                    .map(|column| buffer[row_start + column * plane.pixel_step_in_bytes]),
            );
        }
    }
    samples
}

/// Copies a tightly packed plane into a buffer with the given layout. The caller
/// checked that the buffer is large enough.
fn scatter_plane<Byte: WritableByte>(
    samples: &[u8],
    plane: PlaneLayout,
    columns: usize,
    rows: usize,
    buffer: &mut [Byte],
) {
    for (row, row_samples) in samples.chunks_exact(columns.max(1)).take(rows).enumerate() {
        let row_start = plane.offset_in_bytes + row * plane.row_step_in_bytes;
        for (column, sample) in row_samples.iter().enumerate() {
            buffer[row_start + column * plane.pixel_step_in_bytes].store(*sample);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLOR_SPACE: YuvColorSpace = YuvColorSpace::BT709_LIMITED;

    /// A picture whose every sample says where it is: Y = 10·row + column,
    /// U = 100 + …, V = 200 + … (wrapping), so misplaced bytes show up.
    fn numbered_picture(width: u32, height: u32) -> YuvPicture {
        let numbered = |base: usize, columns: usize, rows: usize| -> Vec<u8> {
            (0..rows)
                .flat_map(|row| {
                    (0..columns).map(move |column| ((base + 10 * row + column) % 256) as u8)
                })
                .collect()
        };
        let (chroma_width, chroma_height) = (
            YuvPicture::chroma_width(width) as usize,
            YuvPicture::chroma_height(height) as usize,
        );
        YuvPicture::from_planes(
            width,
            height,
            numbered(0, width as usize, height as usize),
            numbered(100, chroma_width, chroma_height),
            numbered(200, chroma_width, chroma_height),
            COLOR_SPACE,
        )
        .unwrap()
    }

    fn entries(width: usize, height: usize, color_format: i32) -> PictureFormatEntries {
        PictureFormatEntries {
            width,
            height,
            color_format,
            stride: None,
            slice_height: None,
        }
    }

    fn round_trip(layout: &PictureBufferLayout, picture: &YuvPicture) -> YuvPicture {
        let mut buffer = vec![0xEE_u8; layout.picture_size_in_bytes()];
        let written = layout.write_picture(picture, &mut buffer).unwrap();
        assert_eq!(written, buffer.len());
        let whole = CropRectangle::whole_picture(layout.coded_width, layout.coded_height);
        layout.read_picture(&buffer, whole, COLOR_SPACE).unwrap()
    }

    #[test]
    fn planar_layout_puts_u_then_v_after_the_brightness() {
        let layout =
            PictureBufferLayout::from_color_format(entries(4, 2, COLOR_FORMAT_YUV420_PLANAR))
                .unwrap();
        let picture = numbered_picture(4, 2);
        let mut buffer = vec![0_u8; layout.picture_size_in_bytes()];

        layout.write_picture(&picture, &mut buffer).unwrap();

        assert_eq!(buffer, [0, 1, 2, 3, 10, 11, 12, 13, 100, 101, 200, 201]);
    }

    #[test]
    fn semi_planar_layout_interleaves_u_and_v() {
        let layout =
            PictureBufferLayout::from_color_format(entries(4, 2, COLOR_FORMAT_YUV420_SEMI_PLANAR))
                .unwrap();
        let picture = numbered_picture(4, 2);
        let mut buffer = vec![0_u8; layout.picture_size_in_bytes()];

        layout.write_picture(&picture, &mut buffer).unwrap();

        assert_eq!(buffer, [0, 1, 2, 3, 10, 11, 12, 13, 100, 200, 101, 201]);
    }

    #[test]
    fn padded_rows_and_planes_round_trip() {
        let layout = PictureBufferLayout::from_color_format(PictureFormatEntries {
            stride: Some(64),
            slice_height: Some(16),
            ..entries(30, 10, COLOR_FORMAT_YUV420_SEMI_PLANAR)
        })
        .unwrap();
        let picture = numbered_picture(30, 10);

        assert_eq!(round_trip(&layout, &picture), picture);
    }

    #[test]
    fn qualcomm_venus_layout_is_aligned_without_being_told() {
        let layout = PictureBufferLayout::from_color_format(entries(
            1920,
            1080,
            COLOR_FORMAT_QCOM_YUV420_SEMI_PLANAR_32M,
        ))
        .unwrap();

        assert_eq!(layout.planes[0].row_step_in_bytes, 1920);
        assert_eq!(layout.planes[1].offset_in_bytes, 1920 * 1088);
    }

    #[test]
    fn odd_sizes_round_trip() {
        for color_format in [COLOR_FORMAT_YUV420_PLANAR, COLOR_FORMAT_YUV420_SEMI_PLANAR] {
            let layout =
                PictureBufferLayout::from_color_format(entries(7, 5, color_format)).unwrap();
            let picture = numbered_picture(7, 5);
            assert_eq!(round_trip(&layout, &picture), picture);
        }
    }

    #[test]
    fn crop_keeps_only_the_visible_picture() {
        let layout =
            PictureBufferLayout::from_color_format(entries(8, 8, COLOR_FORMAT_YUV420_PLANAR))
                .unwrap();
        let coded = numbered_picture(8, 8);
        let mut buffer = vec![0_u8; layout.picture_size_in_bytes()];
        layout.write_picture(&coded, &mut buffer).unwrap();
        let crop = CropRectangle::from_inclusive_bounds(2, 2, 5, 3).unwrap();

        let cropped = layout.read_picture(&buffer, crop, COLOR_SPACE).unwrap();

        assert_eq!((cropped.width(), cropped.height()), (4, 2));
        assert_eq!(cropped.y_plane(), &[22, 23, 24, 25, 32, 33, 34, 35]);
        assert_eq!(cropped.u_plane(), &[111, 112]);
        assert_eq!(cropped.v_plane(), &[211, 212]);
    }

    #[test]
    fn crop_past_the_picture_is_refused() {
        let layout =
            PictureBufferLayout::from_color_format(entries(8, 8, COLOR_FORMAT_YUV420_PLANAR))
                .unwrap();
        let buffer = vec![0_u8; layout.picture_size_in_bytes()];
        let crop = CropRectangle::from_inclusive_bounds(4, 0, 8, 7).unwrap();

        assert_eq!(
            layout.read_picture(&buffer, crop, COLOR_SPACE),
            Err(PictureLayoutError::CropOutsidePicture)
        );
    }

    #[test]
    fn short_buffers_are_refused_instead_of_read_past() {
        let layout =
            PictureBufferLayout::from_color_format(entries(8, 8, COLOR_FORMAT_YUV420_PLANAR))
                .unwrap();
        let buffer = vec![0_u8; 50];

        let error = layout
            .read_picture(&buffer, CropRectangle::whole_picture(8, 8), COLOR_SPACE)
            .unwrap_err();

        assert_eq!(
            error,
            PictureLayoutError::BufferTooSmall {
                needed: 96,
                available: 50
            }
        );
    }

    #[test]
    fn unknown_colour_formats_are_named_in_the_error() {
        let error = PictureBufferLayout::from_color_format(entries(8, 8, 0x7FA3_0C03)).unwrap_err();

        assert!(error.to_string().contains("0x7fa30c03"));
    }

    /// Encodes a `MediaImage2` structure as a codec would.
    fn media_image2_bytes(width: u32, height: u32, planes: [(u32, i32, i32, u32); 3]) -> Vec<u8> {
        let mut fields: Vec<u32> = vec![MEDIA_IMAGE_TYPE_YUV, 3, width, height, 8, 8];
        for (offset, pixel_step, row_step, subsampling) in planes {
            fields.extend([
                offset,
                pixel_step as u32,
                row_step as u32,
                subsampling,
                subsampling,
            ]);
        }
        fields.extend([0; 5]);
        fields
            .iter()
            .flat_map(|field| field.to_le_bytes())
            .collect()
    }

    #[test]
    fn image_data_describes_nv21_which_has_v_first() {
        let (width, height) = (6_u32, 4_u32);
        let luma_size = width * height;
        let image_data = media_image2_bytes(
            width,
            height,
            [(0, 1, 6, 1), (luma_size + 1, 2, 6, 2), (luma_size, 2, 6, 2)],
        );
        let layout = PictureBufferLayout::from_format(
            entries(6, 4, COLOR_FORMAT_YUV420_FLEXIBLE),
            Some(&image_data),
        )
        .unwrap();
        let picture = numbered_picture(width, height);
        let mut buffer = vec![0_u8; layout.picture_size_in_bytes()];

        layout.write_picture(&picture, &mut buffer).unwrap();

        assert_eq!(&buffer[24..30], &[200, 100, 201, 101, 202, 102]);
        assert_eq!(round_trip(&layout, &picture), picture);
    }

    #[test]
    fn unusable_image_data_falls_back_to_the_colour_format() {
        let mut image_data = media_image2_bytes(4, 2, [(0, 1, 4, 1), (8, 1, 2, 2), (10, 1, 2, 2)]);
        image_data[16] = 10; // ten-bit samples

        let layout = PictureBufferLayout::from_format(
            entries(4, 2, COLOR_FORMAT_YUV420_SEMI_PLANAR),
            Some(&image_data),
        )
        .unwrap();

        assert_eq!(layout.planes[1].pixel_step_in_bytes, 2);
    }

    #[test]
    fn flexible_format_without_description_is_refused() {
        assert!(matches!(
            PictureBufferLayout::from_format(entries(4, 2, COLOR_FORMAT_YUV420_FLEXIBLE), None),
            Err(PictureLayoutError::UnsupportedColorFormat { .. })
        ));
    }

    #[test]
    fn uninitialised_encoder_buffers_can_be_written() {
        let layout =
            PictureBufferLayout::from_color_format(entries(4, 2, COLOR_FORMAT_YUV420_PLANAR))
                .unwrap();
        let mut buffer = vec![MaybeUninit::<u8>::uninit(); 64];

        let written = layout
            .write_picture(&numbered_picture(4, 2), &mut buffer)
            .unwrap();

        assert_eq!(written, 12);
    }

    #[test]
    fn inclusive_bounds_that_make_no_sense_give_no_crop() {
        assert_eq!(CropRectangle::from_inclusive_bounds(5, 0, 4, 10), None);
        assert_eq!(CropRectangle::from_inclusive_bounds(-1, 0, 4, 10), None);
    }
}
