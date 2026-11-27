//! The CPU renderer: the reference implementation, and the fallback when no GPU
//! is available.
//!
//! Each layer is drawn by **inverse mapping**: for every canvas pixel the layer
//! may cover, the compositor computes which point of the source picture lands
//! there (undoing position, rotation, scale, crop and the source correction),
//! reads that point with bilinear interpolation, and blends it over what is
//! already on the canvas. Rows are processed in parallel. The result is
//! converted to BT.709 YUV 4:2:0.

use std::borrow::Cow;

use image::{Rgba, RgbaImage};
use kadr_media::{VideoFrame, YuvColorSpace, YuvPicture};
use rayon::prelude::*;

use crate::color_adjustment::adjust_color_in_sequence;
use crate::color_lookup_table::LayerColorLut;
use crate::layers::{CanvasDescription, LayerGeometry, VisualLayer};
use crate::renderer::{FrameRenderer, RenderError, pass_through_frame};

/// Bytes in one RGBA pixel.
const BYTES_PER_PIXEL: usize = 4;
/// Largest 8-bit channel value.
const FULL_CHANNEL: f32 = 255.0;

/// Renders frames on the CPU.
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuRenderer;

impl FrameRenderer for CpuRenderer {
    fn description(&self) -> String {
        "CPU".to_string()
    }

    fn render_frame(
        &mut self,
        canvas: &CanvasDescription,
        layers: &[VisualLayer<'_>],
    ) -> Result<VideoFrame, RenderError> {
        if let Some(frame) = pass_through_frame(canvas, layers) {
            return Ok(frame);
        }
        let rgba_canvas = composite_layers_to_rgba(canvas, layers);
        Ok(VideoFrame::Yuv420(YuvPicture::from_rgba(
            &rgba_canvas,
            YuvColorSpace::BT709_LIMITED,
        )))
    }
}

/// Draws `layers` in order (the first one at the bottom) over the canvas
/// background and returns the RGBA result (fully opaque). YUV layers are
/// converted to RGBA first.
pub fn composite_layers_to_rgba(
    canvas: &CanvasDescription,
    layers: &[VisualLayer<'_>],
) -> RgbaImage {
    let background = canvas.background;
    let opaque_background = Rgba([background.red, background.green, background.blue, 255]);
    let mut canvas_pixels = RgbaImage::from_pixel(
        canvas.size.width_in_pixels,
        canvas.size.height_in_pixels,
        opaque_background,
    );
    for layer in layers {
        let rgba_picture: Cow<'_, RgbaImage> = layer.picture.to_rgba();
        draw_layer(&mut canvas_pixels, &rgba_picture, layer, canvas);
    }
    canvas_pixels
}

/// The visible part of the source picture as whole pixel indexes (both ends
/// included), used to keep interpolation inside it.
#[derive(Debug, Clone, Copy)]
struct VisiblePixels {
    first_column: i64,
    last_column: i64,
    first_row: i64,
    last_row: i64,
}

impl VisiblePixels {
    /// The crop's pixels, or the whole picture's when a source correction
    /// moves the picture inside the window: the window may then show pixels
    /// beyond the crop, and points beyond the picture repeat its edge pixels,
    /// like the GPU's clamp-to-edge sampler.
    fn of(geometry: &LayerGeometry) -> Self {
        if geometry.has_source_correction {
            return Self {
                first_column: 0,
                last_column: (geometry.picture_width as i64 - 1).max(0),
                first_row: 0,
                last_row: (geometry.picture_height as i64 - 1).max(0),
            };
        }
        let first_column = geometry.crop_left.floor() as i64;
        let first_row = geometry.crop_top.floor() as i64;
        Self {
            first_column,
            last_column: (geometry.crop_right.ceil() as i64 - 1).max(first_column),
            first_row,
            last_row: (geometry.crop_bottom.ceil() as i64 - 1).max(first_row),
        }
    }
}

/// Reads the source picture at a point with bilinear interpolation, using only
/// pixels inside the visible part.
fn sample_bilinear(
    picture: &RgbaImage,
    visible: VisiblePixels,
    source_x: f32,
    source_y: f32,
) -> [f32; 4] {
    // Pixel centres are at +0.5.
    let (position_x, position_y) = (source_x - 0.5, source_y - 0.5);
    let (left_column, top_row) = (position_x.floor(), position_y.floor());
    let (weight_right, weight_bottom) = (position_x - left_column, position_y - top_row);
    let column =
        |value: f32| (value as i64).clamp(visible.first_column, visible.last_column) as usize;
    let row = |value: f32| (value as i64).clamp(visible.first_row, visible.last_row) as usize;
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

/// Applies the layer's colour adjustments, then its colour LUT, to a sampled
/// colour (0–255 channels). Alpha is left as it is.
fn graded(sampled: [f32; 4], layer: &VisualLayer<'_>) -> [f32; 4] {
    if !layer.changes_colors() {
        return sampled;
    }
    let rgb = [sampled[0], sampled[1], sampled[2]].map(|channel| channel / FULL_CHANNEL);
    let adjusted = if layer.color_adjustments.is_empty() {
        rgb
    } else {
        adjust_color_in_sequence(rgb, layer.color_adjustments)
    };
    let looked_up = match layer.color_lut.filter(LayerColorLut::is_visible) {
        Some(color_lut) => color_lut.apply(adjusted),
        None => adjusted,
    };
    let [red, green, blue] = looked_up.map(|channel| channel * FULL_CHANNEL);
    [red, green, blue, sampled[3]]
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

fn draw_layer(
    canvas_pixels: &mut RgbaImage,
    picture: &RgbaImage,
    layer: &VisualLayer<'_>,
    canvas: &CanvasDescription,
) {
    let placement = &layer.placement;
    let opacity = placement.clamped_opacity();
    if opacity <= 0.0 {
        return;
    }
    let Some(geometry) =
        LayerGeometry::new(canvas.size, picture.width(), picture.height(), placement)
    else {
        return;
    };
    if geometry.maps_pixels_one_to_one(canvas.size, picture.width(), picture.height(), placement) {
        draw_one_to_one(canvas_pixels, picture, opacity, layer);
        return;
    }
    let visible = VisiblePixels::of(&geometry);
    let (first_column, end_column, first_row, end_row) = geometry.covered_canvas_area(canvas.size);
    let row_length = canvas_pixels.width() as usize * BYTES_PER_PIXEL;
    canvas_pixels
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
                    let sampled =
                        graded(sample_bilinear(picture, visible, source_x, source_y), layer);
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
fn draw_one_to_one(
    canvas_pixels: &mut RgbaImage,
    picture: &RgbaImage,
    opacity: f32,
    layer: &VisualLayer<'_>,
) {
    let row_length = canvas_pixels.width() as usize * BYTES_PER_PIXEL;
    canvas_pixels
        .as_mut()
        .par_chunks_exact_mut(row_length)
        .zip(picture.as_raw().par_chunks_exact(row_length))
        .for_each(|(destination_row, source_row)| {
            let (source_pixels, _) = source_row.as_chunks::<BYTES_PER_PIXEL>();
            let is_opaque_row = opacity >= 1.0
                && !layer.changes_colors()
                && source_pixels.iter().all(|pixel| pixel[3] == 255);
            if is_opaque_row {
                destination_row.copy_from_slice(source_row);
                return;
            }
            let (destination_pixels, _) = destination_row.as_chunks_mut::<BYTES_PER_PIXEL>();
            for (destination, source) in destination_pixels.iter_mut().zip(source_pixels) {
                blend_over(destination, graded(source.map(f32::from), layer), opacity);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::{LayerPlacement, SourceCorrection};
    use kadr_core::clip::CropInsets;
    use kadr_core::color::Color;
    use kadr_core::media_asset::PictureSize;

    fn canvas(width_in_pixels: u32, height_in_pixels: u32, background: Color) -> CanvasDescription {
        CanvasDescription {
            size: PictureSize {
                width_in_pixels,
                height_in_pixels,
            },
            background,
        }
    }

    fn solid(width: u32, height: u32, red: u8, green: u8, blue: u8) -> VideoFrame {
        VideoFrame::Rgba(RgbaImage::from_pixel(
            width,
            height,
            Rgba([red, green, blue, 255]),
        ))
    }

    /// A picture whose left half is red and right half is blue.
    fn red_left_blue_right(width: u32, height: u32) -> VideoFrame {
        VideoFrame::Rgba(RgbaImage::from_fn(width, height, |x, _| {
            if x < width / 2 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 255, 255])
            }
        }))
    }

    fn layer(picture: &VideoFrame, placement: LayerPlacement) -> VisualLayer<'_> {
        VisualLayer {
            picture,
            placement,
            color_adjustments: &[],
            color_lut: None,
        }
    }

    #[test]
    fn a_picture_of_the_canvas_size_is_copied_exactly() {
        let picture = red_left_blue_right(64, 32);
        let result = composite_layers_to_rgba(
            &canvas(64, 32, Color::BLACK),
            &[layer(&picture, LayerPlacement::FITTED)],
        );
        assert_eq!(VideoFrame::Rgba(result), picture);
    }

    #[test]
    fn a_wide_picture_is_fitted_with_bars_above_and_below() {
        let picture = solid(100, 50, 255, 255, 255);
        let result = composite_layers_to_rgba(
            &canvas(200, 200, Color::BLACK),
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
        let result = composite_layers_to_rgba(
            &canvas(10, 10, Color::BLACK),
            &[layer(&white, half_visible)],
        );
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
        let result = composite_layers_to_rgba(
            &canvas(10, 10, Color::BLACK),
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
        let result =
            composite_layers_to_rgba(&canvas(20, 20, Color::BLACK), &[layer(&white, moved)]);
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
        let result = composite_layers_to_rgba(
            &canvas(20, 20, Color::BLACK),
            &[layer(&picture, upside_down)],
        );
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
        let result = composite_layers_to_rgba(
            &canvas(10, 10, Color::BLACK),
            &[layer(&picture, right_half_only)],
        );
        assert!(result.pixels().all(|pixel| pixel.0 == [0, 0, 255, 255]));
    }

    #[test]
    fn invisible_layers_change_nothing() {
        let white = solid(10, 10, 255, 255, 255);
        let hidden = LayerPlacement {
            opacity: 0.0,
            ..LayerPlacement::FITTED
        };
        let result = composite_layers_to_rgba(
            &canvas(10, 10, Color::from_rgb(1, 2, 3)),
            &[layer(&white, hidden)],
        );
        assert!(result.pixels().all(|pixel| pixel.0 == [1, 2, 3, 255]));
    }

    /// A 2 × 2 × 2 cube turning every colour into its negative.
    fn inverting_table() -> crate::color_lookup_table::ColorLookupTable {
        let entries = (0..8)
            .map(|index| {
                let corner = [index & 1, (index >> 1) & 1, (index >> 2) & 1];
                corner.map(|bit| 1.0 - bit as f32)
            })
            .collect();
        crate::color_lookup_table::ColorLookupTable::from_entries(
            crate::color_lookup_table::LookupTableShape::ThreeDimensional,
            2,
            entries,
        )
        .unwrap()
    }

    fn inverted<'layer>(
        picture: &'layer VideoFrame,
        table: &'layer crate::color_lookup_table::ColorLookupTable,
    ) -> VisualLayer<'layer> {
        VisualLayer {
            color_lut: Some(LayerColorLut {
                table,
                strength_fraction: 1.0,
            }),
            ..layer(picture, LayerPlacement::FITTED)
        }
    }

    #[test]
    fn a_colour_lut_is_applied_even_where_pictures_are_usually_copied() {
        let table = inverting_table();
        // An opaque picture of the canvas size, usually copied row by row.
        let green = solid(4, 2, 10, 200, 30);
        let result =
            composite_layers_to_rgba(&canvas(4, 2, Color::BLACK), &[inverted(&green, &table)]);
        assert_eq!(*result.get_pixel(1, 1), Rgba([245, 55, 225, 255]));
        // A BT.709 video of the canvas size, usually passed through untouched.
        let video = VideoFrame::Yuv420(YuvPicture::from_rgba(
            &RgbaImage::from_pixel(16, 8, Rgba([10, 200, 30, 255])),
            YuvColorSpace::BT709_LIMITED,
        ));
        let rendered = CpuRenderer
            .render_frame(&canvas(16, 8, Color::BLACK), &[inverted(&video, &table)])
            .unwrap();
        assert_ne!(rendered, video);
        // At strength 0 the shortcut applies again.
        let untouched = VisualLayer {
            color_lut: Some(LayerColorLut {
                table: &table,
                strength_fraction: 0.0,
            }),
            ..layer(&video, LayerPlacement::FITTED)
        };
        let rendered = CpuRenderer
            .render_frame(&canvas(16, 8, Color::BLACK), &[untouched])
            .unwrap();
        assert_eq!(rendered, video);
    }

    /// Fitted, with the source picture moved `shift_x_in_pixels` to the right
    /// inside its window.
    fn shifted_right(shift_x_in_pixels: f32) -> LayerPlacement {
        LayerPlacement {
            source_correction: SourceCorrection {
                shift_x_in_pixels,
                ..SourceCorrection::IDENTITY
            },
            ..LayerPlacement::FITTED
        }
    }

    #[test]
    fn a_stabilized_canvas_sized_video_is_not_passed_through() {
        let VideoFrame::Rgba(halves) = red_left_blue_right(16, 8) else {
            panic!("expected RGBA");
        };
        let video =
            VideoFrame::Yuv420(YuvPicture::from_rgba(&halves, YuvColorSpace::BT709_LIMITED));
        let rendered = CpuRenderer
            .render_frame(
                &canvas(16, 8, Color::BLACK),
                &[layer(&video, shifted_right(2.0))],
            )
            .unwrap();
        assert_ne!(rendered, video);
        // Nor is a picture of the canvas size copied row by row: the boundary
        // moves 2 pixels to the right, and the left edge repeats its pixels.
        let picture = red_left_blue_right(20, 10);
        let result = composite_layers_to_rgba(
            &canvas(20, 10, Color::BLACK),
            &[layer(&picture, shifted_right(2.0))],
        );
        assert_eq!(result.get_pixel(11, 5).0, [255, 0, 0, 255]);
        assert_eq!(result.get_pixel(12, 5).0, [0, 0, 255, 255]);
        assert_eq!(result.get_pixel(0, 5).0, [255, 0, 0, 255], "edge repeated");
    }

    #[test]
    fn a_corrected_layer_shows_pixels_beyond_its_crop() {
        // The window keeps the blue right half; the correction moves the
        // picture 5 pixels to the right, so the window's left half shows red
        // pixels that the crop alone would have removed.
        let picture = red_left_blue_right(20, 10);
        let placement = LayerPlacement {
            crop: CropInsets {
                left_fraction: 0.5,
                ..CropInsets::default()
            },
            ..shifted_right(5.0)
        };
        let result =
            composite_layers_to_rgba(&canvas(10, 10, Color::BLACK), &[layer(&picture, placement)]);
        assert_eq!(result.get_pixel(0, 5).0, [255, 0, 0, 255]);
        assert_eq!(result.get_pixel(4, 5).0, [255, 0, 0, 255]);
        assert_eq!(result.get_pixel(5, 5).0, [0, 0, 255, 255]);
        assert_eq!(result.get_pixel(9, 5).0, [0, 0, 255, 255]);
    }

    #[test]
    fn a_bt709_video_at_canvas_size_passes_through_untouched() {
        let video_picture = YuvPicture::from_rgba(
            &RgbaImage::from_pixel(16, 8, Rgba([10, 200, 30, 255])),
            YuvColorSpace::BT709_LIMITED,
        );
        let frame = VideoFrame::Yuv420(video_picture);
        let rendered = CpuRenderer
            .render_frame(
                &canvas(16, 8, Color::BLACK),
                &[layer(&frame, LayerPlacement::FITTED)],
            )
            .unwrap();
        assert_eq!(rendered, frame);
    }

    #[test]
    fn the_output_is_bt709_yuv() {
        let rendered = CpuRenderer
            .render_frame(&canvas(4, 4, Color::WHITE), &[])
            .unwrap();
        let VideoFrame::Yuv420(picture) = rendered else {
            panic!("expected YUV");
        };
        assert_eq!(picture.color_space(), YuvColorSpace::BT709_LIMITED);
        assert!(picture.y_plane().iter().all(|&luma| luma == 235));
    }
}
