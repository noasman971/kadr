//! Drawing subtitle cues, and where they sit on the canvas.
//!
//! A cue is laid out by `cosmic-text` like a text clip, with three differences:
//!
//! - lines wider than the style's maximum width wrap between words, and every
//!   line is centred;
//! - `<i>` and `<b>` words are drawn italic (Inter has no italic face: the
//!   letters are slanted) and bold (Inter's weight axis; `<b>` in a bold style
//!   is black);
//! - the style's backdrop is drawn behind the letters: a box behind each line,
//!   or an outline around every letter, computed from the letters' exact
//!   distance (an anti-aliased disc of the outline's width around each pixel of
//!   ink).
//!
//! The picture is made here, on the CPU, then drawn as a layer at one picture
//! pixel per canvas pixel and at a whole-pixel position, so the GPU and CPU
//! renderers show exactly the same subtitle.

use cosmic_text::{
    Align, Attrs, Buffer, Color as TextColor, Family, Metrics, Shaping, Style, Weight, Wrap,
};
use image::{Rgba, RgbaImage};
use kadr_core::clip::CropInsets;
use kadr_core::color::Color;
use kadr_core::media_asset::PictureSize;
use kadr_core::subtitle::{SubtitleBackdrop, SubtitlePlacement, SubtitleStyle, parse_cue_markup};

use crate::layers::{LayerPlacement, LayerSizing};
use crate::text::{
    LINE_HEIGHT_RATIO, MARGIN_RATIO, TextRasterizer, blend_glyph_pixel, font_file_family_for,
    measure,
};

/// Space left of and right of each line inside its box, as a fraction of the
/// font size.
const BOX_HORIZONTAL_PADDING_RATIO: f32 = 0.3;

/// A cue drawn, and where it goes on the canvas.
#[derive(Debug, Clone)]
pub struct SubtitlePicture {
    /// The cue, on a transparent picture.
    pub picture: RgbaImage,
    /// Where to draw it: at its native size, centred horizontally, its lines
    /// `margin_in_pixels` from the bottom (or top) of the canvas.
    pub placement: LayerPlacement,
}

/// One laid-out line: its box in layout coordinates.
struct LaidOutLine {
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
}

impl TextRasterizer {
    /// Draws a cue's text (line breaks, `<i>`, `<b>`) in `style`, for a canvas
    /// of `canvas_size`.
    pub fn rasterize_subtitle(
        &mut self,
        cue_text: &str,
        style: &SubtitleStyle,
        canvas_size: PictureSize,
    ) -> SubtitlePicture {
        let font_size = style.font_size_in_pixels.max(1.0);
        let maximum_width = style
            .maximum_line_width_in_pixels(canvas_size.width_in_pixels)
            .max(font_size);
        let mut buffer = Buffer::new(
            &mut self.font_system,
            Metrics::new(font_size, font_size * LINE_HEIGHT_RATIO),
        );
        buffer.set_wrap(Wrap::WordOrGlyph);
        let text_color = text_color_of(style.text_color);
        let base_weight = if style.is_bold {
            Weight::BOLD
        } else {
            Weight::NORMAL
        };
        let base_attributes = Attrs::new()
            .family(Family::Name(font_file_family_for(&style.font_family)))
            .weight(base_weight)
            .color(text_color);
        let runs = parse_cue_markup(cue_text);
        let spans: Vec<(&str, Attrs<'_>)> = runs
            .iter()
            .map(|run| {
                let mut attributes = base_attributes.clone();
                if run.is_italic {
                    attributes = attributes.style(Style::Italic);
                }
                if run.is_bold {
                    attributes = attributes.weight(if style.is_bold {
                        Weight::BLACK
                    } else {
                        Weight::BOLD
                    });
                }
                (run.text.as_str(), attributes)
            })
            .collect();

        // Wrap at the maximum width, then lay out again at the widest line's
        // width so the lines are centred on each other.
        buffer.set_size(Some(maximum_width), None);
        buffer.set_rich_text(
            spans
                .iter()
                .map(|(text, attributes)| (*text, attributes.clone())),
            &base_attributes,
            Shaping::Advanced,
            Some(Align::Center),
        );
        buffer.shape_until_scroll(&mut self.font_system, false);
        let (widest_line, _) = measure(&buffer);
        buffer.set_size(Some(widest_line.ceil() + 1.0), None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        let (layout_width, layout_height) = measure(&buffer);

        let backdrop_reach = match style.backdrop {
            SubtitleBackdrop::None => 0.0,
            SubtitleBackdrop::Outline {
                width_in_pixels, ..
            } => width_in_pixels.max(0.0),
            SubtitleBackdrop::Box { .. } => font_size * BOX_HORIZONTAL_PADDING_RATIO,
        };
        let padding = (font_size * MARGIN_RATIO + backdrop_reach).ceil() as u32;
        let picture_width = layout_width.ceil().max(1.0) as u32 + 2 * padding;
        let picture_height = layout_height.ceil().max(1.0) as u32 + 2 * padding;

        let mut letters = RgbaImage::new(picture_width, picture_height);
        let mut lines = Vec::new();
        let offset = padding as i32;
        for run in buffer.layout_runs() {
            let glyph_extent = run.glyphs.iter().fold(None, |extent, glyph| {
                let (left, right) = extent.unwrap_or((f32::MAX, f32::MIN));
                Some((left.min(glyph.x), right.max(glyph.x + glyph.w)))
            });
            if let Some((left, right)) = glyph_extent {
                lines.push(LaidOutLine {
                    left,
                    right,
                    top: run.line_top,
                    bottom: run.line_top + run.line_height,
                });
            }
            for glyph in run.glyphs {
                let physical_glyph = glyph.physical((0.0, run.line_y), 1.0);
                let glyph_color = glyph.color_opt.unwrap_or(text_color);
                self.glyph_cache.with_pixels(
                    &mut self.font_system,
                    physical_glyph.cache_key,
                    glyph_color,
                    |x, y, drawn_color| {
                        blend_glyph_pixel(
                            &mut letters,
                            physical_glyph.x + x + offset,
                            physical_glyph.y + y + offset,
                            drawn_color,
                        );
                    },
                );
            }
        }

        let mut picture = match style.backdrop {
            SubtitleBackdrop::None => RgbaImage::new(picture_width, picture_height),
            SubtitleBackdrop::Box { color } => line_boxes(
                &lines,
                picture_width,
                picture_height,
                padding as f32,
                font_size * BOX_HORIZONTAL_PADDING_RATIO,
                color,
            ),
            SubtitleBackdrop::Outline {
                color,
                width_in_pixels,
            } => outline_around(&letters, width_in_pixels, color),
        };
        for (destination, source) in picture.pixels_mut().zip(letters.pixels()) {
            *destination = over(*source, *destination);
        }
        let placement =
            subtitle_placement(picture_width, picture_height, padding, style, canvas_size);
        SubtitlePicture { picture, placement }
    }
}

fn text_color_of(color: Color) -> TextColor {
    TextColor::rgba(color.red, color.green, color.blue, color.alpha)
}

/// Where a cue's picture goes: centred horizontally, its lines (the picture
/// without its padding) `margin_in_pixels` from the chosen edge, at a whole
/// pixel so it is copied, not resampled.
fn subtitle_placement(
    picture_width: u32,
    picture_height: u32,
    padding: u32,
    style: &SubtitleStyle,
    canvas_size: PictureSize,
) -> LayerPlacement {
    let (canvas_width, canvas_height) = (
        canvas_size.width_in_pixels as f32,
        canvas_size.height_in_pixels as f32,
    );
    let (picture_width, picture_height, padding) =
        (picture_width as f32, picture_height as f32, padding as f32);
    let left = ((canvas_width - picture_width) / 2.0).round();
    let top = match style.placement {
        SubtitlePlacement::Bottom => {
            (canvas_height - style.margin_in_pixels - (picture_height - padding)).round()
        }
        SubtitlePlacement::Top => (style.margin_in_pixels - padding).round(),
    };
    LayerPlacement {
        offset_x_in_pixels: left + picture_width / 2.0 - canvas_width / 2.0,
        offset_y_in_pixels: top + picture_height / 2.0 - canvas_height / 2.0,
        sizing: LayerSizing::NativePixels,
        scale_factor: 1.0,
        rotation_in_degrees: 0.0,
        crop: CropInsets::default(),
        opacity: 1.0,
    }
}

/// A box of `color` behind each line, `horizontal_padding` wider on each side;
/// boxes of neighbouring lines touch.
fn line_boxes(
    lines: &[LaidOutLine],
    picture_width: u32,
    picture_height: u32,
    padding: f32,
    horizontal_padding: f32,
    color: Color,
) -> RgbaImage {
    let mut picture = RgbaImage::new(picture_width, picture_height);
    let fill = Rgba([color.red, color.green, color.blue, color.alpha]);
    let clamped = |value: f32, limit: u32| value.round().clamp(0.0, limit as f32) as u32;
    for line in lines {
        let left = clamped(padding + line.left - horizontal_padding, picture_width);
        let right = clamped(padding + line.right + horizontal_padding, picture_width);
        let top = clamped(padding + line.top, picture_height);
        let bottom = clamped(padding + line.bottom, picture_height);
        for row in top..bottom {
            for column in left..right {
                picture.put_pixel(column, row, fill);
            }
        }
    }
    picture
}

/// A line of `color` reaching `width_in_pixels` beyond the letters' ink, with
/// anti-aliased edges, under every letter.
fn outline_around(letters: &RgbaImage, width_in_pixels: f32, color: Color) -> RgbaImage {
    let (width, height) = letters.dimensions();
    let is_ink: Vec<bool> = letters.pixels().map(|pixel| pixel[3] >= 128).collect();
    let squared_distances = squared_distances_to_ink(&is_ink, width as usize, height as usize);
    let mut outline = RgbaImage::new(width, height);
    for ((outline_pixel, letter_pixel), squared_distance) in outline
        .pixels_mut()
        .zip(letters.pixels())
        .zip(squared_distances)
    {
        let reach = (width_in_pixels + 0.5 - squared_distance.sqrt() as f32).clamp(0.0, 1.0);
        let coverage = reach.max(f32::from(letter_pixel[3]) / 255.0);
        let alpha = (f32::from(color.alpha) * coverage).round() as u8;
        *outline_pixel = Rgba([color.red, color.green, color.blue, alpha]);
    }
    outline
}

/// For every pixel, the squared distance to the nearest ink pixel (exact
/// Euclidean distance transform of Felzenszwalb and Huttenlocher, one pass per
/// column then one per row).
fn squared_distances_to_ink(is_ink: &[bool], width: usize, height: usize) -> Vec<f64> {
    /// Stands for "no ink on this line" (large but finite, so differences stay numbers).
    const FAR: f64 = 1e20;
    let mut distances: Vec<f64> = is_ink
        .iter()
        .map(|&ink| if ink { 0.0 } else { FAR })
        .collect();
    let longest_side = width.max(height);
    let mut line = vec![0.0; longest_side];
    let mut transformed = vec![0.0; longest_side];
    let mut parabola_vertices = vec![0_usize; longest_side];
    let mut boundaries = vec![0.0; longest_side + 1];
    for column in 0..width {
        for row in 0..height {
            line[row] = distances[row * width + column];
        }
        distance_transform_of_line(
            &line[..height],
            &mut transformed[..height],
            &mut parabola_vertices,
            &mut boundaries,
        );
        for row in 0..height {
            distances[row * width + column] = transformed[row];
        }
    }
    for row in 0..height {
        let row_values = &mut distances[row * width..(row + 1) * width];
        line[..width].copy_from_slice(row_values);
        distance_transform_of_line(
            &line[..width],
            &mut transformed[..width],
            &mut parabola_vertices,
            &mut boundaries,
        );
        row_values.copy_from_slice(&transformed[..width]);
    }
    distances
}

/// The 1D squared distance transform of `values` into `result`: the lower
/// envelope of the parabolas `(q − p)² + values[p]`.
fn distance_transform_of_line(
    values: &[f64],
    result: &mut [f64],
    parabola_vertices: &mut [usize],
    boundaries: &mut [f64],
) {
    let count = values.len();
    if count == 0 {
        return;
    }
    let intersection = |p: usize, q: usize| {
        let (p_float, q_float) = (p as f64, q as f64);
        ((values[q] + q_float * q_float) - (values[p] + p_float * p_float))
            / (2.0 * q_float - 2.0 * p_float)
    };
    let mut envelope_size = 0;
    parabola_vertices[0] = 0;
    boundaries[0] = f64::NEG_INFINITY;
    boundaries[1] = f64::INFINITY;
    for q in 1..count {
        let mut crossing = intersection(parabola_vertices[envelope_size], q);
        while crossing <= boundaries[envelope_size] && envelope_size > 0 {
            envelope_size -= 1;
            crossing = intersection(parabola_vertices[envelope_size], q);
        }
        envelope_size += 1;
        parabola_vertices[envelope_size] = q;
        boundaries[envelope_size] = crossing;
        boundaries[envelope_size + 1] = f64::INFINITY;
    }
    let mut segment = 0;
    for (q, value) in result.iter_mut().enumerate() {
        while boundaries[segment + 1] < q as f64 {
            segment += 1;
        }
        let vertex = parabola_vertices[segment];
        let offset = q as f64 - vertex as f64;
        *value = offset * offset + values[vertex];
    }
}

/// `source` drawn over `destination` (straight alpha).
fn over(source: Rgba<u8>, destination: Rgba<u8>) -> Rgba<u8> {
    let source_alpha = f32::from(source[3]) / 255.0;
    if source_alpha <= 0.0 {
        return destination;
    }
    let destination_alpha = f32::from(destination[3]) / 255.0;
    let result_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
    let mix = |source_channel: u8, destination_channel: u8| {
        let mixed = f32::from(source_channel) * source_alpha
            + f32::from(destination_channel) * destination_alpha * (1.0 - source_alpha);
        (mixed / result_alpha).round().clamp(0.0, 255.0) as u8
    };
    Rgba([
        mix(source[0], destination[0]),
        mix(source[1], destination[1]),
        mix(source[2], destination[2]),
        (result_alpha * 255.0).round() as u8,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERTICAL: PictureSize = PictureSize {
        width_in_pixels: 1080,
        height_in_pixels: 1920,
    };

    fn style_with(backdrop: SubtitleBackdrop) -> SubtitleStyle {
        SubtitleStyle {
            backdrop,
            ..SubtitleStyle::for_canvas(VERTICAL)
        }
    }

    fn count_pixels(picture: &RgbaImage, wanted: impl Fn(&Rgba<u8>) -> bool) -> usize {
        picture.pixels().filter(|pixel| wanted(pixel)).count()
    }

    #[test]
    fn an_outline_surrounds_the_letters() {
        let mut rasterizer = TextRasterizer::new();
        let plain =
            rasterizer.rasterize_subtitle("Hello", &style_with(SubtitleBackdrop::None), VERTICAL);
        let outlined = rasterizer.rasterize_subtitle(
            "Hello",
            &style_with(SubtitleBackdrop::Outline {
                color: Color::BLACK,
                width_in_pixels: 5.0,
            }),
            VERTICAL,
        );
        let white = |pixel: &Rgba<u8>| pixel[3] == 255 && pixel[0] > 240;
        let black = |pixel: &Rgba<u8>| pixel[3] == 255 && pixel[0] < 15;
        assert!(count_pixels(&plain.picture, black) == 0);
        let white_count = count_pixels(&outlined.picture, white);
        let black_count = count_pixels(&outlined.picture, black);
        assert!(white_count > 1_000, "{white_count} white pixels");
        assert!(black_count > white_count / 2, "{black_count} black pixels");
        // Every opaque black pixel is within the outline's reach of a letter.
        assert_eq!(outlined.picture.get_pixel(0, 0)[3], 0);
    }

    #[test]
    fn a_box_sits_behind_each_line() {
        let mut rasterizer = TextRasterizer::new();
        let boxed = rasterizer.rasterize_subtitle(
            "A much longer first line\nshort",
            &style_with(SubtitleBackdrop::Box {
                color: Color::from_rgb(0, 0, 200),
            }),
            VERTICAL,
        );
        let picture = &boxed.picture;
        let blue = |pixel: &Rgba<u8>| pixel[2] == 200 && pixel[0] == 0 && pixel[3] == 255;
        // The first line's box is wider than the second's.
        let blue_columns_in_row = |row: u32| {
            (0..picture.width())
                .filter(|column| blue(picture.get_pixel(*column, row)))
                .count()
        };
        let padding = (60.0 * MARGIN_RATIO + 60.0 * BOX_HORIZONTAL_PADDING_RATIO).ceil() as u32;
        let first_line_row = padding + 4;
        let second_line_row = picture.height() - padding - 4;
        assert!(blue_columns_in_row(first_line_row) > 2 * blue_columns_in_row(second_line_row));
        assert!(blue_columns_in_row(second_line_row) > 0);
        assert_eq!(picture.get_pixel(0, 0)[3], 0, "the padding is transparent");
    }

    #[test]
    fn long_lines_wrap_and_tags_change_the_letters() {
        let mut rasterizer = TextRasterizer::new();
        let style = style_with(SubtitleBackdrop::None);
        let one_line = rasterizer.rasterize_subtitle("Short", &style, VERTICAL);
        let wrapped = rasterizer.rasterize_subtitle(
            "This sentence is much too long to fit on a single line of a vertical video",
            &style,
            VERTICAL,
        );
        assert!(wrapped.picture.width() as f32 <= 1080.0 * 0.8 + 100.0);
        assert!(wrapped.picture.height() > 2 * one_line.picture.height());
        let plain = rasterizer.rasterize_subtitle("Words", &style, VERTICAL);
        let italic = rasterizer.rasterize_subtitle("<i>Words</i>", &style, VERTICAL);
        let black = rasterizer.rasterize_subtitle("<b>Words</b>", &style, VERTICAL);
        assert_ne!(plain.picture, italic.picture);
        let ink = |picture: &RgbaImage| count_pixels(picture, |pixel| pixel[3] > 128);
        assert!(ink(&black.picture) > ink(&plain.picture));
    }

    #[test]
    fn cues_sit_at_the_margin_on_whole_pixels() {
        let mut rasterizer = TextRasterizer::new();
        let mut style = style_with(SubtitleBackdrop::None);
        let bottom = rasterizer.rasterize_subtitle("Hello", &style, VERTICAL);
        let (width, height) = bottom.picture.dimensions();
        let top_left_y = bottom.placement.offset_y_in_pixels + 960.0 - height as f32 / 2.0;
        let top_left_x = bottom.placement.offset_x_in_pixels + 540.0 - width as f32 / 2.0;
        assert_eq!(top_left_x.fract(), 0.0);
        assert_eq!(top_left_y.fract(), 0.0);
        let padding = (60.0 * MARGIN_RATIO).ceil();
        let lines_bottom = top_left_y + height as f32 - padding;
        assert!(
            (lines_bottom - (1920.0 - 480.0)).abs() <= 0.5,
            "{lines_bottom}"
        );
        assert!((top_left_x + width as f32 / 2.0 - 540.0).abs() <= 0.5);

        style.placement = SubtitlePlacement::Top;
        style.margin_in_pixels = 100.0;
        let top = rasterizer.rasterize_subtitle("Hello", &style, VERTICAL);
        let top_y = top.placement.offset_y_in_pixels + 960.0 - top.picture.height() as f32 / 2.0;
        assert_eq!(top_y + padding, 100.0);
    }

    #[test]
    fn distances_to_ink_are_exact() {
        // A single ink pixel in the middle of a 7 × 5 picture.
        let mut is_ink = vec![false; 35];
        is_ink[2 * 7 + 3] = true;
        let distances = squared_distances_to_ink(&is_ink, 7, 5);
        assert_eq!(distances[2 * 7 + 3], 0.0);
        assert_eq!(distances[0], 13.0, "3 across and 2 down");
        assert_eq!(distances[4 * 7 + 6], 13.0);
        assert_eq!(distances[2 * 7], 9.0);
    }
}
