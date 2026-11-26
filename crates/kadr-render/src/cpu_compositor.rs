//! The CPU compositor: draws pictures onto the canvas, one layer after another.
//!
//! Each layer is drawn by **inverse mapping**: for every canvas pixel the layer
//! may cover, the compositor computes which point of the source picture lands
//! there (undoing position, rotation, scale and crop), reads that point with
//! bilinear interpolation, and blends it over what is already on the canvas.
//! Rows are processed in parallel.
//!
//! This version runs on the CPU so the export works everywhere from phase 2.
//! Phase 3 moves the same operations to the GPU (`wgpu`) for speed; large
//! downscales (4K into 1080p) will then also get proper filtering.

use image::{Rgba, RgbaImage};
use kadr_core::clip::{AnimatableProperty, Clip, CropInsets};
use kadr_core::color::Color;
use kadr_core::media_asset::PictureSize;
use kadr_core::time::TimeSpan;
use rayon::prelude::*;

/// Bytes in one RGBA pixel.
const BYTES_PER_PIXEL: usize = 4;
/// Largest 8-bit channel value.
const FULL_CHANNEL: f32 = 255.0;

/// Where and how one picture is drawn on the canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerPlacement {
    /// Horizontal offset of the picture's centre from the canvas centre; right is positive.
    pub offset_x_in_pixels: f32,
    /// Vertical offset of the picture's centre from the canvas centre; down is positive.
    pub offset_y_in_pixels: f32,
    /// Size multiplier. At 1 the (cropped) picture fits entirely inside the canvas.
    pub scale_factor: f32,
    /// Clockwise rotation around the picture's centre.
    pub rotation_in_degrees: f32,
    /// Edges removed from the source picture before fitting.
    pub crop: CropInsets,
    /// 0 invisible to 1 opaque.
    pub opacity: f32,
}

impl LayerPlacement {
    /// Centred, fitted, opaque, not rotated nor cropped.
    pub const FITTED: Self = Self {
        offset_x_in_pixels: 0.0,
        offset_y_in_pixels: 0.0,
        scale_factor: 1.0,
        rotation_in_degrees: 0.0,
        crop: CropInsets {
            left_fraction: 0.0,
            top_fraction: 0.0,
            right_fraction: 0.0,
            bottom_fraction: 0.0,
        },
        opacity: 1.0,
    };

    /// The placement of a clip at `time_offset` from its start, with its
    /// keyframe animations evaluated at that moment.
    pub fn of_clip_at(clip: &Clip, time_offset: TimeSpan) -> Self {
        let value_of = |property| clip.animated_property(property).value_at(time_offset);
        Self {
            offset_x_in_pixels: value_of(AnimatableProperty::PositionX),
            offset_y_in_pixels: value_of(AnimatableProperty::PositionY),
            scale_factor: value_of(AnimatableProperty::Scale),
            rotation_in_degrees: value_of(AnimatableProperty::Rotation),
            crop: clip.transform.crop,
            opacity: value_of(AnimatableProperty::Opacity),
        }
    }
}

/// A picture to draw, and how.
#[derive(Debug, Clone, Copy)]
pub struct VisualLayer<'picture> {
    /// The source picture.
    pub picture: &'picture RgbaImage,
    /// Where and how to draw it.
    pub placement: LayerPlacement,
}

/// Draws `layers` in order (the first one at the bottom) over a canvas filled
/// with `background`. The result is fully opaque.
pub fn composite_layers(
    canvas_size: PictureSize,
    background: Color,
    layers: &[VisualLayer<'_>],
) -> RgbaImage {
    let opaque_background = Rgba([background.red, background.green, background.blue, 255]);
    let mut canvas = RgbaImage::from_pixel(
        canvas_size.width_in_pixels,
        canvas_size.height_in_pixels,
        opaque_background,
    );
    for layer in layers {
        draw_layer(&mut canvas, layer);
    }
    canvas
}

/// The mapping between canvas pixels and source pixels for one layer.
#[derive(Debug, Clone, Copy)]
struct LayerGeometry {
    /// Visible part of the source picture, in source pixels.
    crop_left: f32,
    crop_top: f32,
    crop_right: f32,
    crop_bottom: f32,
    /// The same visible part as whole pixel indexes (both ends included), used
    /// to keep interpolation inside it.
    first_visible_column: i64,
    last_visible_column: i64,
    first_visible_row: i64,
    last_visible_row: i64,
    /// Centre of the visible part, in source pixels.
    source_center_x: f32,
    source_center_y: f32,
    /// Where that centre lands on the canvas.
    canvas_center_x: f32,
    canvas_center_y: f32,
    /// Source pixels per canvas pixel.
    source_pixels_per_canvas_pixel: f32,
    /// Displayed size of the visible part, in canvas pixels.
    displayed_width: f32,
    displayed_height: f32,
    rotation_cosine: f32,
    rotation_sine: f32,
}

impl LayerGeometry {
    /// `None` when the layer cannot show anything (empty picture, zero scale, crop
    /// removing everything).
    fn new(canvas: &RgbaImage, picture: &RgbaImage, placement: &LayerPlacement) -> Option<Self> {
        let (picture_width, picture_height) = (picture.width() as f32, picture.height() as f32);
        let crop = placement.crop;
        let crop_left = crop.left_fraction * picture_width;
        let crop_right = picture_width - crop.right_fraction * picture_width;
        let crop_top = crop.top_fraction * picture_height;
        let crop_bottom = picture_height - crop.bottom_fraction * picture_height;
        let (visible_width, visible_height) = (crop_right - crop_left, crop_bottom - crop_top);
        if visible_width < 1.0 || visible_height < 1.0 || placement.scale_factor <= 0.0 {
            return None;
        }
        let (canvas_width, canvas_height) = (canvas.width() as f32, canvas.height() as f32);
        let fit_scale = (canvas_width / visible_width).min(canvas_height / visible_height);
        let total_scale = fit_scale * placement.scale_factor;
        let rotation_in_radians = placement.rotation_in_degrees.to_radians();
        let first_visible_column = crop_left.floor() as i64;
        let first_visible_row = crop_top.floor() as i64;
        Some(Self {
            crop_left,
            crop_top,
            crop_right,
            crop_bottom,
            first_visible_column,
            last_visible_column: (crop_right.ceil() as i64 - 1).max(first_visible_column),
            first_visible_row,
            last_visible_row: (crop_bottom.ceil() as i64 - 1).max(first_visible_row),
            source_center_x: crop_left + visible_width / 2.0,
            source_center_y: crop_top + visible_height / 2.0,
            canvas_center_x: canvas_width / 2.0 + placement.offset_x_in_pixels,
            canvas_center_y: canvas_height / 2.0 + placement.offset_y_in_pixels,
            source_pixels_per_canvas_pixel: 1.0 / total_scale,
            displayed_width: visible_width * total_scale,
            displayed_height: visible_height * total_scale,
            rotation_cosine: rotation_in_radians.cos(),
            rotation_sine: rotation_in_radians.sin(),
        })
    }

    /// Whether canvas pixel (x, y) shows source pixel (x, y) exactly: same
    /// size, no crop, no move, no rotation, no scaling.
    fn maps_pixels_one_to_one(
        &self,
        canvas: &RgbaImage,
        picture: &RgbaImage,
        placement: &LayerPlacement,
    ) -> bool {
        canvas.dimensions() == picture.dimensions()
            && placement.crop == CropInsets::default()
            && placement.offset_x_in_pixels == 0.0
            && placement.offset_y_in_pixels == 0.0
            && placement.rotation_in_degrees == 0.0
            && self.source_pixels_per_canvas_pixel == 1.0
    }

    /// The canvas rows and columns the layer may cover (rotated bounding box).
    fn covered_canvas_area(&self, canvas: &RgbaImage) -> (u32, u32, u32, u32) {
        let (half_width, half_height) = (self.displayed_width / 2.0, self.displayed_height / 2.0);
        let half_extent_x =
            half_width * self.rotation_cosine.abs() + half_height * self.rotation_sine.abs();
        let half_extent_y =
            half_width * self.rotation_sine.abs() + half_height * self.rotation_cosine.abs();
        let clamp_column = |value: f32| value.clamp(0.0, canvas.width() as f32) as u32;
        let clamp_row = |value: f32| value.clamp(0.0, canvas.height() as f32) as u32;
        (
            clamp_column((self.canvas_center_x - half_extent_x).floor()),
            clamp_column((self.canvas_center_x + half_extent_x).ceil()),
            clamp_row((self.canvas_center_y - half_extent_y).floor()),
            clamp_row((self.canvas_center_y + half_extent_y).ceil()),
        )
    }

    /// The source point shown at the centre of canvas pixel (x, y), or `None` if
    /// it falls outside the visible part of the picture.
    fn source_point_for(&self, canvas_x: u32, canvas_y: u32) -> Option<(f32, f32)> {
        let from_center_x = canvas_x as f32 + 0.5 - self.canvas_center_x;
        let from_center_y = canvas_y as f32 + 0.5 - self.canvas_center_y;
        // Undo the clockwise rotation (y points down), then the scaling.
        let unrotated_x = from_center_x * self.rotation_cosine + from_center_y * self.rotation_sine;
        let unrotated_y =
            -from_center_x * self.rotation_sine + from_center_y * self.rotation_cosine;
        let source_x = self.source_center_x + unrotated_x * self.source_pixels_per_canvas_pixel;
        let source_y = self.source_center_y + unrotated_y * self.source_pixels_per_canvas_pixel;
        let is_inside = source_x >= self.crop_left
            && source_x < self.crop_right
            && source_y >= self.crop_top
            && source_y < self.crop_bottom;
        is_inside.then_some((source_x, source_y))
    }

    /// Reads the source picture at a point with bilinear interpolation, using
    /// only pixels inside the visible part.
    fn sample(&self, picture: &RgbaImage, source_x: f32, source_y: f32) -> [f32; 4] {
        // Pixel centres are at +0.5.
        let (position_x, position_y) = (source_x - 0.5, source_y - 0.5);
        let (left_column, top_row) = (position_x.floor(), position_y.floor());
        let (weight_right, weight_bottom) = (position_x - left_column, position_y - top_row);
        let column = |value: f32| {
            (value as i64).clamp(self.first_visible_column, self.last_visible_column) as usize
        };
        let row = |value: f32| {
            (value as i64).clamp(self.first_visible_row, self.last_visible_row) as usize
        };
        let (left, right) = (column(left_column), column(left_column + 1.0));
        let (top, bottom) = (row(top_row), row(top_row + 1.0));

        // Read each of the four neighbouring pixels once.
        let pixels = picture.as_raw();
        let row_length = picture.width() as usize * BYTES_PER_PIXEL;
        let pixel_at = |x: usize, y: usize| {
            let start = y * row_length + x * BYTES_PER_PIXEL;
            &pixels[start..start + BYTES_PER_PIXEL]
        };
        let (top_left, top_right) = (pixel_at(left, top), pixel_at(right, top));
        let (bottom_left, bottom_right) = (pixel_at(left, bottom), pixel_at(right, bottom));

        let mut sampled = [0.0; 4];
        for (channel, value) in sampled.iter_mut().enumerate() {
            let top_mix = f32::from(top_left[channel]) * (1.0 - weight_right)
                + f32::from(top_right[channel]) * weight_right;
            let bottom_mix = f32::from(bottom_left[channel]) * (1.0 - weight_right)
                + f32::from(bottom_right[channel]) * weight_right;
            *value = top_mix * (1.0 - weight_bottom) + bottom_mix * weight_bottom;
        }
        sampled
    }
}

/// Blends a straight-alpha colour over an opaque canvas pixel.
fn blend_over(destination: &mut [u8], source: [f32; 4], opacity: f32) {
    let coverage = (source[3] / FULL_CHANNEL) * opacity;
    if coverage <= 0.0 {
        return;
    }
    for channel in 0..3 {
        let blended =
            source[channel] * coverage + f32::from(destination[channel]) * (1.0 - coverage);
        destination[channel] = blended.round().clamp(0.0, FULL_CHANNEL) as u8;
    }
    destination[3] = 255;
}

fn draw_layer(canvas: &mut RgbaImage, layer: &VisualLayer<'_>) {
    let placement = &layer.placement;
    let opacity = placement.opacity.clamp(0.0, 1.0);
    if opacity <= 0.0 {
        return;
    }
    let Some(geometry) = LayerGeometry::new(canvas, layer.picture, placement) else {
        return;
    };
    if geometry.maps_pixels_one_to_one(canvas, layer.picture, placement) {
        draw_one_to_one(canvas, layer.picture, opacity);
        return;
    }
    let (first_column, end_column, first_row, end_row) = geometry.covered_canvas_area(canvas);
    let row_length = canvas.width() as usize * BYTES_PER_PIXEL;
    canvas
        .as_mut()
        .par_chunks_exact_mut(row_length)
        .enumerate()
        .skip(first_row as usize)
        .take(end_row.saturating_sub(first_row) as usize)
        .for_each(|(row_index, row_bytes)| {
            for column in first_column..end_column {
                if let Some((source_x, source_y)) =
                    geometry.source_point_for(column, row_index as u32)
                {
                    let sampled = geometry.sample(layer.picture, source_x, source_y);
                    let pixel_start = column as usize * BYTES_PER_PIXEL;
                    blend_over(
                        &mut row_bytes[pixel_start..pixel_start + BYTES_PER_PIXEL],
                        sampled,
                        opacity,
                    );
                }
            }
        });
}

/// Fast path: the picture covers the canvas pixel for pixel. Opaque rows are
/// copied in one go; others are blended pixel by pixel.
fn draw_one_to_one(canvas: &mut RgbaImage, picture: &RgbaImage, opacity: f32) {
    let row_length = canvas.width() as usize * BYTES_PER_PIXEL;
    canvas
        .as_mut()
        .par_chunks_exact_mut(row_length)
        .zip(picture.as_raw().par_chunks_exact(row_length))
        .for_each(|(destination_row, source_row)| {
            let (source_pixels, _) = source_row.as_chunks::<BYTES_PER_PIXEL>();
            let is_opaque_row = opacity >= 1.0 && source_pixels.iter().all(|pixel| pixel[3] == 255);
            if is_opaque_row {
                destination_row.copy_from_slice(source_row);
                return;
            }
            let (destination_pixels, _) = destination_row.as_chunks_mut::<BYTES_PER_PIXEL>();
            for (destination, source) in destination_pixels.iter_mut().zip(source_pixels) {
                blend_over(destination, source.map(f32::from), opacity);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLACK: Color = Color::BLACK;

    fn size(width_in_pixels: u32, height_in_pixels: u32) -> PictureSize {
        PictureSize {
            width_in_pixels,
            height_in_pixels,
        }
    }

    fn solid(width: u32, height: u32, red: u8, green: u8, blue: u8) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba([red, green, blue, 255]))
    }

    /// A picture whose left half is red and right half is blue.
    fn red_left_blue_right(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, _| {
            if x < width / 2 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 255, 255])
            }
        })
    }

    fn layer(picture: &RgbaImage, placement: LayerPlacement) -> VisualLayer<'_> {
        VisualLayer { picture, placement }
    }

    #[test]
    fn a_picture_of_the_canvas_size_is_copied_exactly() {
        let picture = red_left_blue_right(64, 32);
        let result = composite_layers(
            size(64, 32),
            BLACK,
            &[layer(&picture, LayerPlacement::FITTED)],
        );
        assert_eq!(result, picture);
    }

    #[test]
    fn a_wide_picture_is_fitted_with_bars_above_and_below() {
        let picture = solid(100, 50, 255, 255, 255);
        let result = composite_layers(
            size(200, 200),
            BLACK,
            &[layer(&picture, LayerPlacement::FITTED)],
        );
        assert_eq!(result.get_pixel(100, 20)[0], 0, "bar above");
        assert_eq!(result.get_pixel(100, 100)[0], 255, "picture");
        assert_eq!(result.get_pixel(100, 180)[0], 0, "bar below");
        assert_eq!(result.get_pixel(0, 100)[0], 255, "fills the width");
    }

    #[test]
    fn opacity_blends_with_what_is_below() {
        let white = solid(10, 10, 255, 255, 255);
        let half_visible = LayerPlacement {
            opacity: 0.5,
            ..LayerPlacement::FITTED
        };
        let result = composite_layers(size(10, 10), BLACK, &[layer(&white, half_visible)]);
        assert_eq!(result.get_pixel(5, 5)[0], 128);
    }

    #[test]
    fn later_layers_are_drawn_on_top() {
        let red = solid(10, 10, 255, 0, 0);
        let small_green = solid(10, 10, 0, 255, 0);
        let half_size = LayerPlacement {
            scale_factor: 0.5,
            ..LayerPlacement::FITTED
        };
        let result = composite_layers(
            size(10, 10),
            BLACK,
            &[
                layer(&red, LayerPlacement::FITTED),
                layer(&small_green, half_size),
            ],
        );
        assert_eq!(
            result.get_pixel(5, 5).0,
            [0, 255, 0, 255],
            "centre is green"
        );
        assert_eq!(result.get_pixel(0, 0).0, [255, 0, 0, 255], "corner is red");
    }

    #[test]
    fn an_offset_moves_the_picture() {
        let white = solid(10, 10, 255, 255, 255);
        let moved = LayerPlacement {
            scale_factor: 0.5,
            offset_x_in_pixels: 5.0,
            ..LayerPlacement::FITTED
        };
        let result = composite_layers(size(20, 20), BLACK, &[layer(&white, moved)]);
        assert_eq!(
            result.get_pixel(4, 10)[0],
            0,
            "former left edge is now empty"
        );
        assert_eq!(result.get_pixel(14, 10)[0], 255, "picture moved right");
    }

    #[test]
    fn half_a_turn_swaps_left_and_right() {
        let picture = red_left_blue_right(20, 20);
        let upside_down = LayerPlacement {
            rotation_in_degrees: 180.0,
            ..LayerPlacement::FITTED
        };
        let result = composite_layers(size(20, 20), BLACK, &[layer(&picture, upside_down)]);
        assert_eq!(result.get_pixel(2, 10).0, [0, 0, 255, 255]);
        assert_eq!(result.get_pixel(17, 10).0, [255, 0, 0, 255]);
    }

    #[test]
    fn cropping_keeps_only_the_visible_part_and_fits_it() {
        let picture = red_left_blue_right(20, 10);
        let right_half_only = LayerPlacement {
            crop: CropInsets {
                left_fraction: 0.5,
                ..CropInsets::default()
            },
            ..LayerPlacement::FITTED
        };
        let result = composite_layers(size(10, 10), BLACK, &[layer(&picture, right_half_only)]);
        assert!(result.pixels().all(|pixel| pixel.0 == [0, 0, 255, 255]));
    }

    #[test]
    fn invisible_layers_change_nothing() {
        let white = solid(10, 10, 255, 255, 255);
        let hidden = LayerPlacement {
            opacity: 0.0,
            ..LayerPlacement::FITTED
        };
        let result = composite_layers(
            size(10, 10),
            Color::from_rgb(1, 2, 3),
            &[layer(&white, hidden)],
        );
        assert!(result.pixels().all(|pixel| pixel.0 == [1, 2, 3, 255]));
    }
}
