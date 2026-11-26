//! Shrinking pictures for the interface: preview frames and thumbnails.
//!
//! Each output pixel is the **average of the source pixels it covers** (a box
//! filter). Unlike sampling one source pixel per output pixel, it keeps fine
//! detail (text, foliage) from turning into shimmering noise when a 1080p or 4K
//! frame is shown in a small panel, and it costs one addition per source value.
//! Rows are computed in parallel.
//!
//! Pictures are only ever shrunk here, never enlarged: the interface scales the
//! result up on screen if it must.

use image::RgbaImage;
use kadr_core::media_asset::PictureSize;
use kadr_media::{VideoFrame, YuvPicture};
use rayon::prelude::*;

/// Bytes per pixel of an RGBA picture.
const RGBA_CHANNEL_COUNT: usize = 4;

/// The largest size with the proportions of a `picture_width × picture_height`
/// picture that fits inside `maximum_size`.
///
/// - A picture that already fits keeps its size.
/// - A shrunk size has **even** sides (YUV 4:2:0 pictures and video encoders
///   prefer them), at least 2 × 2.
pub fn size_fitting_inside(
    picture_width: u32,
    picture_height: u32,
    maximum_size: PictureSize,
) -> PictureSize {
    let fits = picture_width <= maximum_size.width_in_pixels
        && picture_height <= maximum_size.height_in_pixels;
    if fits || picture_width == 0 || picture_height == 0 {
        return PictureSize {
            width_in_pixels: picture_width,
            height_in_pixels: picture_height,
        };
    }
    let scale = f64::min(
        f64::from(maximum_size.width_in_pixels) / f64::from(picture_width),
        f64::from(maximum_size.height_in_pixels) / f64::from(picture_height),
    );
    let even_side = |side: u32| {
        let scaled = (f64::from(side) * scale).floor() as u32;
        (scaled - scaled % 2).max(2)
    };
    PictureSize {
        width_in_pixels: even_side(picture_width),
        height_in_pixels: even_side(picture_height),
    }
}

/// `picture` shrunk to fit inside `maximum_size` (see [`size_fitting_inside`]),
/// in the same colour space. A picture that already fits is copied unchanged.
pub fn shrink_yuv_picture_to_fit(picture: &YuvPicture, maximum_size: PictureSize) -> YuvPicture {
    let target = size_fitting_inside(picture.width(), picture.height(), maximum_size);
    if target.width_in_pixels == picture.width() && target.height_in_pixels == picture.height() {
        return picture.clone();
    }
    let shrink_chroma_plane = |plane: &[u8]| {
        shrink_plane(
            &PlaneDescription {
                samples: plane,
                width: YuvPicture::chroma_width(picture.width()),
                height: YuvPicture::chroma_height(picture.height()),
                channel_count: 1,
            },
            YuvPicture::chroma_width(target.width_in_pixels),
            YuvPicture::chroma_height(target.height_in_pixels),
        )
    };
    let y_plane = shrink_plane(
        &PlaneDescription {
            samples: picture.y_plane(),
            width: picture.width(),
            height: picture.height(),
            channel_count: 1,
        },
        target.width_in_pixels,
        target.height_in_pixels,
    );
    let u_plane = shrink_chroma_plane(picture.u_plane());
    let v_plane = shrink_chroma_plane(picture.v_plane());
    YuvPicture::from_planes(
        target.width_in_pixels,
        target.height_in_pixels,
        y_plane,
        u_plane,
        v_plane,
        picture.color_space(),
    )
    // The planes were computed for exactly these sizes; this cannot fail.
    .unwrap_or_else(|_| picture.clone())
}

/// `image` shrunk to fit inside `maximum_size`. Transparency is averaged like the
/// colours. An image that already fits is copied unchanged.
pub fn shrink_rgba_image_to_fit(image: &RgbaImage, maximum_size: PictureSize) -> RgbaImage {
    let target = size_fitting_inside(image.width(), image.height(), maximum_size);
    if target.width_in_pixels == image.width() && target.height_in_pixels == image.height() {
        return image.clone();
    }
    let pixels = shrink_plane(
        &PlaneDescription {
            samples: image.as_raw(),
            width: image.width(),
            height: image.height(),
            channel_count: RGBA_CHANNEL_COUNT,
        },
        target.width_in_pixels,
        target.height_in_pixels,
    );
    RgbaImage::from_raw(target.width_in_pixels, target.height_in_pixels, pixels)
        .unwrap_or_else(|| image.clone())
}

/// Any frame as an RGBA picture fitting inside `maximum_size`, for thumbnails.
/// YUV frames are shrunk before being converted, so only the small picture is
/// converted.
pub fn shrink_frame_to_rgba(frame: &VideoFrame, maximum_size: PictureSize) -> RgbaImage {
    match frame {
        VideoFrame::Rgba(image) => shrink_rgba_image_to_fit(image, maximum_size),
        VideoFrame::Yuv420(picture) => shrink_yuv_picture_to_fit(picture, maximum_size).to_rgba(),
    }
}

/// One picture plane: rows of `width` pixels, `channel_count` bytes each, tightly packed.
struct PlaneDescription<'plane> {
    samples: &'plane [u8],
    width: u32,
    height: u32,
    channel_count: usize,
}

/// For each output position, the range of source positions it covers.
///
/// Output position `index` covers `[index × source / target, (index + 1) × source / target)`,
/// rounded to whole pixels, and always at least one source pixel.
fn covered_source_ranges(source_length: u32, target_length: u32) -> Vec<(usize, usize)> {
    let (source_length, target_length) = (source_length as usize, target_length as usize);
    (0..target_length)
        .map(|index| {
            let start = (index * source_length / target_length).min(source_length - 1);
            let end = ((index + 1) * source_length / target_length).clamp(start + 1, source_length);
            (start, end)
        })
        .collect()
}

/// Averages the source pixels covered by each pixel of a `target_width × target_height` plane.
fn shrink_plane(source: &PlaneDescription<'_>, target_width: u32, target_height: u32) -> Vec<u8> {
    let channel_count = source.channel_count;
    let source_row_length = source.width as usize * channel_count;
    let target_row_length = target_width as usize * channel_count;
    let mut target = vec![0; target_row_length * target_height as usize];
    if source.width == 0 || source.height == 0 || target_row_length == 0 {
        return target;
    }
    let column_ranges = covered_source_ranges(source.width, target_width);
    let row_ranges = covered_source_ranges(source.height, target_height);

    target
        .par_chunks_exact_mut(target_row_length)
        .zip(row_ranges.par_iter())
        .for_each(|(target_row, &(first_row, end_row))| {
            let mut totals = vec![0_u32; target_row_length];
            for source_row in first_row..end_row {
                let row_start = source_row * source_row_length;
                let row_samples = &source.samples[row_start..row_start + source_row_length];
                for (target_column, &(first_column, end_column)) in column_ranges.iter().enumerate()
                {
                    for source_column in first_column..end_column {
                        for channel in 0..channel_count {
                            totals[target_column * channel_count + channel] +=
                                u32::from(row_samples[source_column * channel_count + channel]);
                        }
                    }
                }
            }
            let row_count = (end_row - first_row) as u32;
            for (target_column, &(first_column, end_column)) in column_ranges.iter().enumerate() {
                let covered_pixel_count = row_count * (end_column - first_column) as u32;
                for channel in 0..channel_count {
                    let index = target_column * channel_count + channel;
                    // Rounded to the nearest value.
                    target_row[index] =
                        ((totals[index] + covered_pixel_count / 2) / covered_pixel_count) as u8;
                }
            }
        });
    target
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    use kadr_media::YuvColorSpace;

    fn size(width_in_pixels: u32, height_in_pixels: u32) -> PictureSize {
        PictureSize {
            width_in_pixels,
            height_in_pixels,
        }
    }

    #[test]
    fn fitting_keeps_proportions_and_even_sides() {
        assert_eq!(
            size_fitting_inside(1920, 1080, size(960, 960)),
            size(960, 540)
        );
        assert_eq!(
            size_fitting_inside(1080, 1920, size(960, 540)),
            size(302, 540)
        );
        assert_eq!(
            size_fitting_inside(3840, 2160, size(1001, 1001)),
            size(1000, 562)
        );
    }

    #[test]
    fn fitting_never_enlarges() {
        assert_eq!(
            size_fitting_inside(640, 360, size(1920, 1080)),
            size(640, 360)
        );
        assert_eq!(size_fitting_inside(1, 1, size(1, 1)), size(1, 1));
    }

    #[test]
    fn shrinking_averages_the_covered_pixels() {
        // Black and white columns, two pixels wide each: halving gives grey.
        let mut image = RgbaImage::new(8, 4);
        for (column, _, pixel) in image.enumerate_pixels_mut() {
            let value = if column % 2 == 0 { 0 } else { 255 };
            *pixel = Rgba([value, value, value, 255]);
        }
        let shrunk = shrink_rgba_image_to_fit(&image, size(4, 2));
        assert_eq!(shrunk.dimensions(), (4, 2));
        for pixel in shrunk.pixels() {
            assert_eq!(pixel.0, [128, 128, 128, 255]);
        }
    }

    #[test]
    fn shrinking_a_yuv_picture_keeps_flat_colours_and_colour_space() {
        let color_space = YuvColorSpace::BT709_LIMITED;
        let picture = YuvPicture::from_planes(
            64,
            36,
            vec![100; 64 * 36],
            vec![90; 32 * 18],
            vec![200; 32 * 18],
            color_space,
        )
        .unwrap();
        let shrunk = shrink_yuv_picture_to_fit(&picture, size(20, 20));
        assert_eq!((shrunk.width(), shrunk.height()), (20, 10));
        assert_eq!(shrunk.color_space(), color_space);
        assert!(shrunk.y_plane().iter().all(|&value| value == 100));
        assert_eq!(shrunk.u_plane().len(), 10 * 5);
        assert!(shrunk.u_plane().iter().all(|&value| value == 90));
        assert!(shrunk.v_plane().iter().all(|&value| value == 200));
    }

    #[test]
    fn uneven_ratios_cover_every_source_pixel_once() {
        let ranges = covered_source_ranges(1080, 302);
        assert_eq!(ranges.first(), Some(&(0, 3)));
        assert_eq!(ranges.last().map(|range| range.1), Some(1080));
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].1, pair[1].0);
        }
    }
}
