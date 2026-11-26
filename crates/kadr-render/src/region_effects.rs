//! Hiding regions of a picture: blur, pixelate or fill, inside a rectangle or an
//! ellipse with soft edges.
//!
//! This works in the **source picture's own space**, before the picture is placed
//! on the canvas: the tracked regions are measured in source pixels, and the
//! result is the same whichever renderer draws it afterwards. YUV pictures are
//! processed plane by plane (blurring and averaging are linear, so blurring Y, U
//! and V separately equals blurring the colours); RGBA pictures channel by
//! channel, alpha untouched.
//!
//! The gaussian blur is approximated by three box blurs in a row, which costs the
//! same whatever the radius.

use kadr_core::color::Color;
use kadr_core::effect::{BlurShape, BlurStyle, PixelRegion};
use kadr_media::{VideoFrame, YuvColorSpace, YuvPicture};

/// One area to hide.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HiddenRegion {
    /// The area, in source picture pixels (margin already included).
    pub area: PixelRegion,
    /// Blur, pixelate or fill.
    pub style: BlurStyle,
    /// Rectangle or ellipse.
    pub shape: BlurShape,
    /// Width of the soft transition at the edge, in source pixels (0 = hard edge).
    pub edge_softness_in_pixels: f32,
}

/// Number of box blurs approximating one gaussian blur.
const BOX_BLUR_PASSES: usize = 3;

/// A copy of `picture` with every region hidden.
pub fn hide_regions(picture: &VideoFrame, regions: &[HiddenRegion]) -> VideoFrame {
    let mut hidden = picture.clone();
    for region in regions {
        match &mut hidden {
            VideoFrame::Yuv420(yuv_picture) => hide_region_in_yuv(yuv_picture, region),
            VideoFrame::Rgba(image) => {
                let fill = region_fill_color(region);
                let (width, height) = image.dimensions();
                for (channel, fill_value) in
                    [fill.red, fill.green, fill.blue].into_iter().enumerate()
                {
                    let mut plane = PlaneView {
                        values: image.as_mut(),
                        width,
                        height,
                        step: 4,
                        offset: channel,
                    };
                    hide_region_in_plane(&mut plane, region, 1.0, fill_value);
                }
            }
        }
    }
    hidden
}

/// The colour of a solid fill (black for the other styles, unused).
fn region_fill_color(region: &HiddenRegion) -> Color {
    match region.style {
        BlurStyle::SolidFill { color } => color,
        BlurStyle::GaussianBlur { .. } | BlurStyle::Pixelate { .. } => Color::BLACK,
    }
}

fn hide_region_in_yuv(picture: &mut YuvPicture, region: &HiddenRegion) {
    let color_space = picture.color_space();
    let fill_values = fill_color_as_yuv(region_fill_color(region), color_space);
    let (width, height) = (picture.width(), picture.height());
    let (chroma_width, chroma_height) = (
        YuvPicture::chroma_width(width),
        YuvPicture::chroma_height(height),
    );
    let (y_plane, u_plane, v_plane) = picture.planes_mut();
    let planes: [(&mut [u8], u32, u32, f32, u8); 3] = [
        (y_plane, width, height, 1.0, fill_values[0]),
        (u_plane, chroma_width, chroma_height, 0.5, fill_values[1]),
        (v_plane, chroma_width, chroma_height, 0.5, fill_values[2]),
    ];
    for (values, plane_width, plane_height, scale, fill_value) in planes {
        let mut plane = PlaneView {
            values,
            width: plane_width,
            height: plane_height,
            step: 1,
            offset: 0,
        };
        hide_region_in_plane(&mut plane, region, scale, fill_value);
    }
}

/// The stored Y, U, V values of a colour in `color_space`.
fn fill_color_as_yuv(color: Color, color_space: YuvColorSpace) -> [u8; 3] {
    let rgb = [color.red, color.green, color.blue].map(|value| f32::from(value) / 255.0);
    color_space
        .rgb_to_yuv()
        .apply(rgb)
        .map(|value| (value * 255.0).round().clamp(0.0, 255.0) as u8)
}

/// One channel of a picture: values at `offset + (row * width + column) * step`.
struct PlaneView<'values> {
    values: &'values mut [u8],
    width: u32,
    height: u32,
    step: usize,
    offset: usize,
}

impl PlaneView<'_> {
    fn index_of(&self, column: u32, row: u32) -> usize {
        self.offset + (row as usize * self.width as usize + column as usize) * self.step
    }

    fn get(&self, column: u32, row: u32) -> u8 {
        self.values[self.index_of(column, row)]
    }

    fn set(&mut self, column: u32, row: u32, value: u8) {
        let index = self.index_of(column, row);
        self.values[index] = value;
    }
}

/// An area in a plane's own pixels, as whole-pixel bounds clamped to the plane.
#[derive(Debug, Clone, Copy)]
struct PlaneBox {
    first_column: u32,
    end_column: u32,
    first_row: u32,
    end_row: u32,
}

impl PlaneBox {
    fn around(left: f32, top: f32, right: f32, bottom: f32, plane: &PlaneView<'_>) -> Self {
        let clamp_column = |value: f32| value.clamp(0.0, plane.width as f32) as u32;
        let clamp_row = |value: f32| value.clamp(0.0, plane.height as f32) as u32;
        Self {
            first_column: clamp_column(left.floor()),
            end_column: clamp_column(right.ceil()),
            first_row: clamp_row(top.floor()),
            end_row: clamp_row(bottom.ceil()),
        }
    }

    fn width(&self) -> usize {
        self.end_column.saturating_sub(self.first_column) as usize
    }

    fn height(&self) -> usize {
        self.end_row.saturating_sub(self.first_row) as usize
    }
}

/// How much of the effect shows at a pixel centre: 1 inside, 0 outside, a ramp
/// of `softness` pixels across the edge.
fn mask_coverage(
    center_x: f32,
    center_y: f32,
    area: &PixelRegion,
    shape: BlurShape,
    softness: f32,
) -> f32 {
    let (left, top, width, height) = (
        area.left_in_pixels,
        area.top_in_pixels,
        area.width_in_pixels,
        area.height_in_pixels,
    );
    let distance_inside = match shape {
        BlurShape::Rectangle => (center_x - left)
            .min(left + width - center_x)
            .min(center_y - top)
            .min(top + height - center_y),
        BlurShape::Ellipse => {
            let (radius_x, radius_y) = (width / 2.0, height / 2.0);
            let normalized_x = (center_x - (left + radius_x)) / radius_x;
            let normalized_y = (center_y - (top + radius_y)) / radius_y;
            let normalized_distance =
                (normalized_x * normalized_x + normalized_y * normalized_y).sqrt();
            (1.0 - normalized_distance) * radius_x.min(radius_y)
        }
    };
    if softness <= 0.0 {
        return if distance_inside >= 0.0 { 1.0 } else { 0.0 };
    }
    (distance_inside / softness + 0.5).clamp(0.0, 1.0)
}

/// Hides `region` in one plane. `scale` converts source pixels to plane pixels
/// (0.5 for the U and V planes of YUV 4:2:0).
fn hide_region_in_plane(
    plane: &mut PlaneView<'_>,
    region: &HiddenRegion,
    scale: f32,
    fill_value: u8,
) {
    let area = PixelRegion {
        left_in_pixels: region.area.left_in_pixels * scale,
        top_in_pixels: region.area.top_in_pixels * scale,
        width_in_pixels: region.area.width_in_pixels * scale,
        height_in_pixels: region.area.height_in_pixels * scale,
    };
    let softness = region.edge_softness_in_pixels * scale;
    let affected = PlaneBox::around(
        area.left_in_pixels - softness,
        area.top_in_pixels - softness,
        area.left_in_pixels + area.width_in_pixels + softness,
        area.top_in_pixels + area.height_in_pixels + softness,
        plane,
    );
    if affected.width() == 0 || affected.height() == 0 {
        return;
    }
    let hidden_values = match region.style {
        BlurStyle::GaussianBlur { radius_in_pixels } => {
            blurred_values(plane, &affected, radius_in_pixels * scale)
        }
        BlurStyle::Pixelate {
            block_size_in_pixels,
        } => {
            let block_size = ((block_size_in_pixels as f32 * scale).round() as u32).max(1);
            pixelated_values(plane, &affected, &area, block_size)
        }
        BlurStyle::SolidFill { .. } => vec![fill_value; affected.width() * affected.height()],
    };
    for row in affected.first_row..affected.end_row {
        for column in affected.first_column..affected.end_column {
            let coverage = mask_coverage(
                column as f32 + 0.5,
                row as f32 + 0.5,
                &area,
                region.shape,
                softness,
            );
            if coverage <= 0.0 {
                continue;
            }
            let box_index = (row - affected.first_row) as usize * affected.width()
                + (column - affected.first_column) as usize;
            let original = f32::from(plane.get(column, row));
            let hidden = f32::from(hidden_values[box_index]);
            plane.set(
                column,
                row,
                (original + (hidden - original) * coverage).round() as u8,
            );
        }
    }
}

/// The values of `affected` after a gaussian-like blur of `radius` plane pixels.
/// Pixels around the box (up to the radius) are read so the edges blur naturally.
fn blurred_values(plane: &PlaneView<'_>, affected: &PlaneBox, radius: f32) -> Vec<u8> {
    // Three box blurs of radius ≈ sigma give a gaussian of standard deviation sigma.
    let box_radius = ((radius / 2.0).round() as usize).max(1);
    let reach = (box_radius * BOX_BLUR_PASSES) as f32;
    let source = PlaneBox::around(
        affected.first_column as f32 - reach,
        affected.first_row as f32 - reach,
        affected.end_column as f32 + reach,
        affected.end_row as f32 + reach,
        plane,
    );
    let (source_width, source_height) = (source.width(), source.height());
    let mut values: Vec<f32> = (source.first_row..source.end_row)
        .flat_map(|row| (source.first_column..source.end_column).map(move |column| (column, row)))
        .map(|(column, row)| f32::from(plane.get(column, row)))
        .collect();
    for _ in 0..BOX_BLUR_PASSES {
        values = box_blur_rows(&values, source_width, source_height, box_radius);
        values = transpose(
            &box_blur_rows(
                &transpose(&values, source_width, source_height),
                source_height,
                source_width,
                box_radius,
            ),
            source_height,
            source_width,
        );
    }
    let mut result = Vec::with_capacity(affected.width() * affected.height());
    for row in affected.first_row..affected.end_row {
        for column in affected.first_column..affected.end_column {
            let index = (row - source.first_row) as usize * source_width
                + (column - source.first_column) as usize;
            result.push(values[index].round().clamp(0.0, 255.0) as u8);
        }
    }
    result
}

/// Each row averaged over a sliding window of `2 × radius + 1` values (edges
/// repeat the first and last value).
fn box_blur_rows(values: &[f32], width: usize, height: usize, radius: usize) -> Vec<f32> {
    let mut blurred = vec![0.0; values.len()];
    let window_length = (2 * radius + 1) as f32;
    for row in 0..height {
        let row_values = &values[row * width..(row + 1) * width];
        let value_at = |position: isize| row_values[position.clamp(0, width as isize - 1) as usize];
        let mut window_sum: f32 = (-(radius as isize)..=radius as isize).map(value_at).sum();
        for column in 0..width {
            blurred[row * width + column] = window_sum / window_length;
            let leaving = column as isize - radius as isize;
            let entering = column as isize + radius as isize + 1;
            window_sum += value_at(entering) - value_at(leaving);
        }
    }
    blurred
}

/// Swaps rows and columns of a `width × height` grid.
fn transpose(values: &[f32], width: usize, height: usize) -> Vec<f32> {
    let mut transposed = vec![0.0; values.len()];
    for row in 0..height {
        for column in 0..width {
            transposed[column * height + row] = values[row * width + column];
        }
    }
    transposed
}

/// The values of `affected` with each `block_size` square (aligned on the area's
/// top-left corner) replaced by its average.
fn pixelated_values(
    plane: &PlaneView<'_>,
    affected: &PlaneBox,
    area: &PixelRegion,
    block_size: u32,
) -> Vec<u8> {
    let origin_column = area.left_in_pixels.floor() as i64;
    let origin_row = area.top_in_pixels.floor() as i64;
    let block_index = |position: u32, origin: i64| {
        (i64::from(position) - origin).div_euclid(i64::from(block_size))
    };
    let block_start = |index: i64, origin: i64, limit: u32| {
        (origin + index * i64::from(block_size)).clamp(0, i64::from(limit)) as u32
    };

    let mut result = Vec::with_capacity(affected.width() * affected.height());
    let mut cached_block: Option<((i64, i64), u8)> = None;
    for row in affected.first_row..affected.end_row {
        for column in affected.first_column..affected.end_column {
            let block = (
                block_index(column, origin_column),
                block_index(row, origin_row),
            );
            let average = match cached_block {
                Some((cached, average)) if cached == block => average,
                _ => {
                    let (first_column, first_row) = (
                        block_start(block.0, origin_column, plane.width),
                        block_start(block.1, origin_row, plane.height),
                    );
                    let (end_column, end_row) = (
                        block_start(block.0 + 1, origin_column, plane.width),
                        block_start(block.1 + 1, origin_row, plane.height),
                    );
                    let average =
                        block_average(plane, first_column..end_column, first_row..end_row);
                    cached_block = Some((block, average));
                    average
                }
            };
            result.push(average);
        }
    }
    result
}

fn block_average(
    plane: &PlaneView<'_>,
    columns: std::ops::Range<u32>,
    rows: std::ops::Range<u32>,
) -> u8 {
    let mut total = 0_u64;
    let mut count = 0_u64;
    for row in rows {
        for column in columns.clone() {
            total += u64::from(plane.get(column, row));
            count += 1;
        }
    }
    // Rounded to the nearest value; an empty block (never happens) gives 0.
    (total + count / 2).checked_div(count).unwrap_or(0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    /// A black and white checkerboard of 4-pixel squares.
    fn checkerboard(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            if (x / 4 + y / 4) % 2 == 0 {
                Rgba([255, 255, 255, 255])
            } else {
                Rgba([0, 0, 0, 255])
            }
        })
    }

    fn region(style: BlurStyle, shape: BlurShape) -> HiddenRegion {
        HiddenRegion {
            area: PixelRegion {
                left_in_pixels: 16.0,
                top_in_pixels: 16.0,
                width_in_pixels: 32.0,
                height_in_pixels: 32.0,
            },
            style,
            shape,
            edge_softness_in_pixels: 0.0,
        }
    }

    fn red_values(
        frame: &VideoFrame,
        columns: std::ops::Range<u32>,
        rows: std::ops::Range<u32>,
    ) -> Vec<u8> {
        let image = frame.to_rgba();
        rows.flat_map(|row| columns.clone().map(move |column| (column, row)))
            .map(|(column, row)| image.get_pixel(column, row)[0])
            .collect()
    }

    fn spread(values: &[u8]) -> u8 {
        values.iter().max().unwrap() - values.iter().min().unwrap()
    }

    #[test]
    fn a_blur_smooths_the_inside_and_leaves_the_outside_alone() {
        let original = VideoFrame::Rgba(checkerboard(64, 64));
        let hidden = hide_regions(
            &original,
            &[region(
                BlurStyle::GaussianBlur {
                    radius_in_pixels: 16.0,
                },
                BlurShape::Rectangle,
            )],
        );
        assert!(
            spread(&red_values(&hidden, 24..40, 24..40)) < 40,
            "the inside is smooth"
        );
        assert_eq!(
            red_values(&hidden, 0..12, 0..12),
            red_values(&original, 0..12, 0..12)
        );
    }

    #[test]
    fn pixelation_makes_uniform_blocks() {
        let original = VideoFrame::Rgba(RgbaImage::from_fn(64, 64, |x, y| {
            Rgba([(x * 4) as u8, (y * 4) as u8, 0, 255])
        }));
        let hidden = hide_regions(
            &original,
            &[region(
                BlurStyle::Pixelate {
                    block_size_in_pixels: 8,
                },
                BlurShape::Rectangle,
            )],
        );
        // The block from (16, 16) to (24, 24) has one colour.
        assert_eq!(spread(&red_values(&hidden, 16..24, 16..24)), 0);
        assert_ne!(
            red_values(&hidden, 16..17, 16..17),
            red_values(&hidden, 24..25, 16..17),
            "neighbouring blocks differ"
        );
    }

    #[test]
    fn a_solid_fill_on_yuv_uses_the_exact_colour() {
        let original = VideoFrame::Yuv420(YuvPicture::from_rgba(
            &checkerboard(64, 64),
            YuvColorSpace::BT709_LIMITED,
        ));
        let red_fill = BlurStyle::SolidFill {
            color: Color::from_rgb(255, 0, 0),
        };
        let hidden = hide_regions(&original, &[region(red_fill, BlurShape::Rectangle)]);
        let VideoFrame::Yuv420(picture) = &hidden else {
            panic!("YUV stays YUV");
        };
        assert_eq!(picture.y_plane()[32 * 64 + 32], 63);
        assert_eq!(picture.v_plane()[16 * 32 + 16], 240);
    }

    #[test]
    fn an_ellipse_leaves_the_corners_of_its_box_alone() {
        let original = VideoFrame::Rgba(checkerboard(64, 64));
        let black_fill = BlurStyle::SolidFill {
            color: Color::BLACK,
        };
        let hidden = hide_regions(&original, &[region(black_fill, BlurShape::Ellipse)]);
        assert_eq!(
            red_values(&hidden, 16..18, 16..18),
            red_values(&original, 16..18, 16..18),
            "corner untouched"
        );
        assert_eq!(
            red_values(&hidden, 32..33, 32..33),
            vec![0],
            "centre filled"
        );
    }

    #[test]
    fn soft_edges_blend_gradually() {
        let original = VideoFrame::Rgba(RgbaImage::from_pixel(64, 64, Rgba([255, 255, 255, 255])));
        let mut soft_region = region(
            BlurStyle::SolidFill {
                color: Color::BLACK,
            },
            BlurShape::Rectangle,
        );
        soft_region.edge_softness_in_pixels = 8.0;
        let hidden = hide_regions(&original, &[soft_region]);
        let across_the_edge = red_values(&hidden, 10..24, 32..33);
        assert!(
            across_the_edge.windows(2).all(|pair| pair[1] <= pair[0]),
            "{across_the_edge:?}"
        );
        assert!(
            across_the_edge
                .iter()
                .any(|&value| value > 20 && value < 235),
            "{across_the_edge:?}"
        );
    }

    #[test]
    fn regions_outside_the_picture_change_nothing() {
        let original = VideoFrame::Rgba(checkerboard(32, 32));
        let mut far_away = region(
            BlurStyle::GaussianBlur {
                radius_in_pixels: 10.0,
            },
            BlurShape::Ellipse,
        );
        far_away.area.left_in_pixels = 500.0;
        assert_eq!(hide_regions(&original, &[far_away]), original);
    }
}
