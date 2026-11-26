//! Turning the words of a text clip into a picture.
//!
//! Text is laid out and rasterised by `cosmic-text` (shaping with HarfBuzz rules,
//! anti-aliased glyphs), into a transparent RGBA picture just large enough for
//! the text. The picture is then drawn like any other layer, at its native size
//! (one picture pixel per canvas pixel, before the clip's own scale).
//!
//! Kadr bundles the **Inter** font (SIL Open Font License 1.1, see
//! `assets/fonts/Inter-LICENSE.txt`), so the default style looks the same on
//! every machine. System fonts are loaded too: they serve other families the
//! user picks, and characters Inter lacks (emoji, many scripts).

use cosmic_text::{
    Align, Attrs, Buffer, Color as TextColor, Family, FontSystem, Metrics, Shaping, SwashCache,
};
use image::{Rgba, RgbaImage};
use kadr_core::clip::{TextAlignment, TextStyle};

/// The family name of the bundled font, the default for new text clips.
pub const BUNDLED_FONT_FAMILY: &str = "Inter";

/// The family name the bundled font file itself declares. Inter 4's variable
/// font calls itself "Inter Variable"; requests for "Inter" are redirected to it.
const BUNDLED_FONT_FILE_FAMILY: &str = "Inter Variable";

/// The bundled Inter variable font (all weights in one file).
static BUNDLED_FONT_DATA: &[u8] = include_bytes!("../assets/fonts/InterVariable.ttf");

/// Line height as a multiple of the font size.
const LINE_HEIGHT_RATIO: f32 = 1.2;
/// Empty space around the text, as a fraction of the font size, so glyphs that
/// overhang their advance (italics, accents) are not cut.
const MARGIN_RATIO: f32 = 0.15;

/// Lays out and rasterises texts. Creating one loads the fonts (system fonts
/// included), so keep it and reuse it.
pub struct TextRasterizer {
    font_system: FontSystem,
    glyph_cache: SwashCache,
}

impl Default for TextRasterizer {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for TextRasterizer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TextRasterizer")
    }
}

impl TextRasterizer {
    /// Loads the system fonts and the bundled Inter font.
    pub fn new() -> Self {
        let mut font_system = FontSystem::new();
        let font_database = font_system.db_mut();
        font_database.load_font_data(BUNDLED_FONT_DATA.to_vec());
        // Generic "sans-serif" requests also get Inter.
        font_database.set_sans_serif_family(BUNDLED_FONT_FILE_FAMILY);
        Self {
            font_system,
            glyph_cache: SwashCache::new(),
        }
    }

    /// Whether a font of this family is available (bundled or installed).
    pub fn has_font_family(&self, family: &str) -> bool {
        let requested_family = font_file_family_for(family);
        self.font_system.db().faces().any(|face| {
            face.families
                .iter()
                .any(|(face_family, _)| face_family.eq_ignore_ascii_case(requested_family))
        })
    }

    /// Draws `text` in `style` on a transparent picture just large enough for it
    /// (plus a small margin). Line breaks in `text` start new lines; lines are not
    /// wrapped.
    pub fn rasterize(&mut self, text: &str, style: &TextStyle) -> RgbaImage {
        let font_size = style.font_size_in_pixels.max(1.0);
        let mut buffer = Buffer::new(
            &mut self.font_system,
            Metrics::new(font_size, font_size * LINE_HEIGHT_RATIO),
        );
        let attributes =
            Attrs::new().family(Family::Name(font_file_family_for(&style.font_family)));
        let alignment = match style.alignment {
            TextAlignment::Left => Align::Left,
            TextAlignment::Center => Align::Center,
            TextAlignment::Right => Align::Right,
        };

        // First lay out without a width to measure the widest line, then again
        // with that width so centred and right-aligned lines line up with it.
        buffer.set_size(None, None);
        buffer.set_text(text, &attributes, Shaping::Advanced, Some(alignment));
        buffer.shape_until_scroll(&mut self.font_system, false);
        let (text_width, text_height) = measure(&buffer);
        buffer.set_size(Some(text_width), None);
        buffer.shape_until_scroll(&mut self.font_system, false);

        let margin = (font_size * MARGIN_RATIO).ceil();
        let picture_width = (text_width + 2.0 * margin).ceil().max(1.0) as u32;
        let picture_height = (text_height + 2.0 * margin).ceil().max(1.0) as u32;
        let mut picture = RgbaImage::new(picture_width, picture_height);
        let color = style.color;
        let text_color = TextColor::rgba(color.red, color.green, color.blue, color.alpha);
        let (margin_x, margin_y) = (margin as i32, margin as i32);
        buffer.draw(
            &mut self.font_system,
            &mut self.glyph_cache,
            text_color,
            |x, y, width, height, drawn_color| {
                for row in (y + margin_y)..(y + margin_y + height as i32) {
                    for column in (x + margin_x)..(x + margin_x + width as i32) {
                        blend_glyph_pixel(&mut picture, column, row, drawn_color);
                    }
                }
            },
        );
        picture
    }
}

/// The family to ask the font database for: the bundled file's own name when
/// the style asks for Inter.
fn font_file_family_for(requested_family: &str) -> &str {
    if requested_family.eq_ignore_ascii_case(BUNDLED_FONT_FAMILY) {
        BUNDLED_FONT_FILE_FAMILY
    } else {
        requested_family
    }
}

/// Width of the widest line and total height of the laid-out text.
fn measure(buffer: &Buffer) -> (f32, f32) {
    buffer
        .layout_runs()
        .fold((0.0_f32, 0.0_f32), |(width, height), run| {
            (
                width.max(run.line_w),
                height.max(run.line_top + run.line_height),
            )
        })
}

/// Blends one glyph pixel (straight alpha) over the picture, ignoring pixels
/// outside it.
fn blend_glyph_pixel(picture: &mut RgbaImage, column: i32, row: i32, color: TextColor) {
    let (Ok(column), Ok(row)) = (u32::try_from(column), u32::try_from(row)) else {
        return;
    };
    if column >= picture.width() || row >= picture.height() || color.a() == 0 {
        return;
    }
    let destination = picture.get_pixel_mut(column, row);
    let source_alpha = f32::from(color.a()) / 255.0;
    let destination_alpha = f32::from(destination[3]) / 255.0;
    let result_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
    let blend_channel = |source: u8, destination: u8| {
        let mixed = f32::from(source) * source_alpha
            + f32::from(destination) * destination_alpha * (1.0 - source_alpha);
        (mixed / result_alpha).round().clamp(0.0, 255.0) as u8
    };
    *destination = Rgba([
        blend_channel(color.r(), destination[0]),
        blend_channel(color.g(), destination[1]),
        blend_channel(color.b(), destination[2]),
        (result_alpha * 255.0).round() as u8,
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use kadr_core::color::Color;

    fn style(font_family: &str, alignment: TextAlignment) -> TextStyle {
        TextStyle {
            font_family: font_family.to_string(),
            font_size_in_pixels: 64.0,
            color: Color::WHITE,
            alignment,
        }
    }

    /// The columns and rows covered by visible pixels: (left, right, top, bottom).
    fn visible_bounds(
        picture: &RgbaImage,
        rows: std::ops::Range<u32>,
    ) -> Option<(u32, u32, u32, u32)> {
        let mut bounds: Option<(u32, u32, u32, u32)> = None;
        for (column, row, pixel) in picture.enumerate_pixels() {
            if pixel[3] > 128 && rows.contains(&row) {
                bounds = Some(match bounds {
                    None => (column, column, row, row),
                    Some((left, right, top, bottom)) => (
                        left.min(column),
                        right.max(column),
                        top.min(row),
                        bottom.max(row),
                    ),
                });
            }
        }
        bounds
    }

    #[test]
    fn text_is_drawn_opaque_on_a_transparent_picture() {
        let mut rasterizer = TextRasterizer::new();
        let picture =
            rasterizer.rasterize("Kadr", &style(BUNDLED_FONT_FAMILY, TextAlignment::Center));

        assert!(
            picture.width() > picture.height(),
            "{:?}",
            picture.dimensions()
        );
        assert!(picture.height() >= 64);
        assert_eq!(picture.get_pixel(0, 0)[3], 0, "the margin is transparent");
        let opaque_pixels = picture.pixels().filter(|pixel| pixel[3] == 255).count();
        assert!(opaque_pixels > 500, "{opaque_pixels} opaque pixels");
        assert!(
            picture
                .pixels()
                .filter(|pixel| pixel[3] > 0)
                .all(|pixel| pixel[0] == 255)
        );
    }

    #[test]
    fn the_same_text_always_gives_the_same_picture() {
        let mut rasterizer = TextRasterizer::new();
        let text_style = style(BUNDLED_FONT_FAMILY, TextAlignment::Left);
        assert_eq!(
            rasterizer.rasterize("Same", &text_style),
            rasterizer.rasterize("Same", &text_style)
        );
    }

    #[test]
    fn a_short_centred_line_sits_in_the_middle_of_a_long_one() {
        let mut rasterizer = TextRasterizer::new();
        let picture = rasterizer.rasterize(
            "A much longer first line\nshort",
            &style(BUNDLED_FONT_FAMILY, TextAlignment::Center),
        );
        let line_height = (64.0 * LINE_HEIGHT_RATIO) as u32;
        let second_line_rows = picture.height() - line_height..picture.height();
        let (left, right, _, _) = visible_bounds(&picture, second_line_rows).unwrap();
        let line_middle = (left + right) / 2;
        let picture_middle = picture.width() / 2;
        assert!(
            line_middle.abs_diff(picture_middle) < 10,
            "line middle {line_middle}, picture middle {picture_middle}"
        );
        assert!(left > 100, "the short line must not start at the left edge");
    }

    #[test]
    fn the_default_style_uses_the_bundled_inter_font() {
        let mut rasterizer = TextRasterizer::new();
        assert!(rasterizer.has_font_family(BUNDLED_FONT_FAMILY));
        assert!(rasterizer.has_font_family("inter"));
        assert!(!rasterizer.has_font_family("No Such Font Family"));
        // Inter and the fallback font draw the same word differently.
        let inter = rasterizer.rasterize("Kadr", &style(BUNDLED_FONT_FAMILY, TextAlignment::Left));
        let fallback =
            rasterizer.rasterize("Kadr", &style("No Such Font Family", TextAlignment::Left));
        assert_ne!(inter, fallback);
    }

    #[test]
    fn an_unknown_font_falls_back_to_a_font_that_exists() {
        let mut rasterizer = TextRasterizer::new();
        let picture = rasterizer.rasterize(
            "Fallback",
            &style("No Such Font Family", TextAlignment::Left),
        );
        assert!(picture.pixels().any(|pixel| pixel[3] == 255));
    }
}
