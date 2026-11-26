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
//!
//! Type-on and word-by-word animations draw only part of the text
//! ([`TextReveal`]), on a picture the size of the whole text, so the visible
//! letters never move while the others appear.

use cosmic_text::{
    Align, Attrs, Buffer, Color as TextColor, Family, FontSystem, LayoutRun, Metrics, Shaping,
    SwashCache,
};
use image::{Rgba, RgbaImage};
use kadr_core::clip::{TextAlignment, TextStyle};

use crate::transitions::TextReveal;

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
        self.rasterize_revealed(text, style, TextReveal::Everything)
    }

    /// Like [`TextRasterizer::rasterize`], but only draws the part of the text
    /// that `reveal` shows. The picture has the size of the whole text, so the
    /// letters already visible stay in place.
    pub fn rasterize_revealed(
        &mut self,
        text: &str,
        style: &TextStyle,
        reveal: TextReveal,
    ) -> RgbaImage {
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
        let margin_in_pixels = margin as i32;
        let character_opacities = CharacterOpacities::of_laid_out_text(&buffer, reveal);
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let opacity = character_opacities.of_glyph_starting_at(&run, glyph.start);
                if opacity <= 0.0 {
                    continue;
                }
                let physical_glyph = glyph.physical((0.0, run.line_y), 1.0);
                let glyph_color = faded(glyph.color_opt.unwrap_or(text_color), opacity);
                self.glyph_cache.with_pixels(
                    &mut self.font_system,
                    physical_glyph.cache_key,
                    glyph_color,
                    |x, y, drawn_color| {
                        blend_glyph_pixel(
                            &mut picture,
                            physical_glyph.x + x + margin_in_pixels,
                            physical_glyph.y + y + margin_in_pixels,
                            drawn_color,
                        );
                    },
                );
            }
        }
        picture
    }
}

/// The opacity of every character of a laid-out text under a [`TextReveal`].
///
/// Characters are numbered across lines, in reading order of the source text,
/// line breaks excluded.
struct CharacterOpacities {
    /// The number of the first character of each source line.
    first_character_of_line: Vec<usize>,
    /// The opacity of each character, from 0 to 1; `None` when all are visible.
    opacities: Option<Vec<f32>>,
}

impl CharacterOpacities {
    fn of_laid_out_text(buffer: &Buffer, reveal: TextReveal) -> Self {
        let mut line_texts: Vec<String> = Vec::new();
        for run in buffer.layout_runs() {
            // A long line can be laid out in several runs; they share its text.
            if run.line_i >= line_texts.len() {
                line_texts.resize(run.line_i + 1, String::new());
                line_texts[run.line_i] = run.text.to_string();
            }
        }
        let mut first_character_of_line = Vec::with_capacity(line_texts.len());
        let mut line_lengths = Vec::with_capacity(line_texts.len());
        let mut characters: Vec<char> = Vec::new();
        for line_text in &line_texts {
            first_character_of_line.push(characters.len());
            let line_characters: Vec<char> = line_text.chars().collect();
            line_lengths.push(line_characters.len());
            characters.extend(line_characters);
        }
        let opacities = match reveal {
            TextReveal::Everything => None,
            TextReveal::Characters { shown_fraction } => {
                Some(opacities_by_character(characters.len(), shown_fraction))
            }
            TextReveal::Words { shown_fraction } => Some(opacities_by_word(
                &characters,
                &first_character_of_line,
                shown_fraction,
            )),
        };
        Self {
            first_character_of_line,
            opacities,
        }
    }

    /// The opacity of the glyph starting at byte `glyph_start` of `run`'s line.
    fn of_glyph_starting_at(&self, run: &LayoutRun<'_>, glyph_start: usize) -> f32 {
        let Some(opacities) = &self.opacities else {
            return 1.0;
        };
        let characters_before_in_line = run
            .text
            .get(..glyph_start)
            .map_or(0, |before| before.chars().count());
        let line_first_character = self
            .first_character_of_line
            .get(run.line_i)
            .copied()
            .unwrap_or(0);
        opacities
            .get(line_first_character + characters_before_in_line)
            .copied()
            .unwrap_or(1.0)
    }
}

/// Characters appear one after another: with `shown_fraction` of `character_count`
/// characters shown, character `k` has opacity `shown_fraction × count − k`,
/// between 0 and 1, so the one being typed fades in.
fn opacities_by_character(character_count: usize, shown_fraction: f32) -> Vec<f32> {
    let shown_count = shown_fraction.clamp(0.0, 1.0) * character_count as f32;
    (0..character_count)
        .map(|index| (shown_count - index as f32).clamp(0.0, 1.0))
        .collect()
}

/// Words appear one after another, the same way characters do. A word is a run
/// of non-space characters; a line start also starts a new word.
fn opacities_by_word(
    characters: &[char],
    first_character_of_line: &[usize],
    shown_fraction: f32,
) -> Vec<f32> {
    let mut word_of_character: Vec<Option<usize>> = Vec::with_capacity(characters.len());
    let mut word_count = 0;
    let mut is_inside_word = false;
    for (index, character) in characters.iter().enumerate() {
        if first_character_of_line.contains(&index) {
            is_inside_word = false;
        }
        if character.is_whitespace() {
            is_inside_word = false;
            word_of_character.push(None);
            continue;
        }
        if !is_inside_word {
            word_count += 1;
            is_inside_word = true;
        }
        word_of_character.push(Some(word_count - 1));
    }
    let shown_count = shown_fraction.clamp(0.0, 1.0) * word_count as f32;
    word_of_character
        .into_iter()
        .map(|word| word.map_or(0.0, |index| (shown_count - index as f32).clamp(0.0, 1.0)))
        .collect()
}

/// `color` with its alpha multiplied by `opacity`.
fn faded(color: TextColor, opacity: f32) -> TextColor {
    let alpha = (f32::from(color.a()) * opacity.clamp(0.0, 1.0)).round() as u8;
    TextColor::rgba(color.r(), color.g(), color.b(), alpha)
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
    fn a_typed_on_text_shows_its_first_letters_in_place() {
        let mut rasterizer = TextRasterizer::new();
        let text_style = style(BUNDLED_FONT_FAMILY, TextAlignment::Left);
        let whole = rasterizer.rasterize("AAAA", &text_style);
        let half = rasterizer.rasterize_revealed(
            "AAAA",
            &text_style,
            TextReveal::Characters {
                shown_fraction: 0.5,
            },
        );
        let nothing = rasterizer.rasterize_revealed(
            "AAAA",
            &text_style,
            TextReveal::Characters {
                shown_fraction: 0.0,
            },
        );
        assert_eq!(
            half.dimensions(),
            whole.dimensions(),
            "the layout never moves"
        );
        assert!(nothing.pixels().all(|pixel| pixel[3] == 0));
        // The two letters shown are exactly those of the whole text, on the left.
        let all_rows = 0..whole.height();
        let (_, whole_right, _, _) = visible_bounds(&whole, all_rows.clone()).unwrap();
        let (half_left, half_right, _, _) = visible_bounds(&half, all_rows.clone()).unwrap();
        assert_eq!(half_left, visible_bounds(&whole, all_rows).unwrap().0);
        assert!(
            half_right < whole_right * 6 / 10,
            "{half_right} vs {whole_right}"
        );
        for (shown, full) in half.pixels().zip(whole.pixels()) {
            assert!(shown[3] == 0 || shown == full);
        }
    }

    #[test]
    fn words_appear_one_after_another_across_lines() {
        let mut rasterizer = TextRasterizer::new();
        let text_style = style(BUNDLED_FONT_FAMILY, TextAlignment::Left);
        let visible_pixels = |shown_fraction| {
            rasterizer
                .rasterize_revealed(
                    "one two
three four",
                    &text_style,
                    TextReveal::Words { shown_fraction },
                )
                .pixels()
                .filter(|pixel| pixel[3] > 0)
                .count()
        };
        let counts = [0.0, 0.25, 0.5, 0.75, 1.0].map(visible_pixels);
        assert_eq!(counts[0], 0);
        assert!(
            counts.windows(2).all(|pair| pair[0] < pair[1]),
            "{counts:?}"
        );
        // Half the words: the first line only.
        let first_line_only = rasterizer.rasterize_revealed(
            "one two
three four",
            &text_style,
            TextReveal::Words {
                shown_fraction: 0.5,
            },
        );
        let line_height = (64.0 * LINE_HEIGHT_RATIO) as u32;
        assert!(visible_bounds(&first_line_only, line_height..first_line_only.height()).is_none());
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
