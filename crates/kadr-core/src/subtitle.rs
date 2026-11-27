//! Subtitles: cues on subtitle tracks, and the style they share.
//!
//! A **subtitle track** ([`TrackKind::Subtitle`](crate::timeline::TrackKind)) holds
//! **cues** ([`SubtitleCue`], clips whose content is
//! [`ClipContent::Subtitle`](crate::clip::ClipContent)): a few words shown from a
//! moment of the timeline for some time. Every cue of a track looks the same: the
//! track carries one [`SubtitleStyle`] (font, size, colour, outline or box,
//! position), so changing it restyles all its cues at once.
//!
//! Cues follow the **timeline**, like the times of an `.srt` file: cutting, moving
//! or trimming a video never moves them. Removing parts of a clip and closing the
//! gaps (silence removal) removes the same moments from the subtitle tracks, so the
//! words stay under the voice that says them (decision record 0056).
//!
//! A cue's text keeps the light markup of `.srt` files: line breaks, and `<i>` /
//! `<b>` for italic and bold words. [`parse_cue_markup`] reads it; any other text,
//! `<` included, is shown as it is.

use serde::{Deserialize, Serialize};

use crate::color::Color;
use crate::media_asset::PictureSize;
use crate::time::{TimeSpan, Timestamp};

/// A cue: words shown for some time. The track's [`SubtitleStyle`] says how.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubtitleCue {
    /// The words, with line breaks and `<i>` / `<b>` tags as in an `.srt` file.
    pub text: String,
    /// How long the cue stays on screen.
    pub duration: TimeSpan,
}

impl SubtitleCue {
    /// A cue showing `text` for `duration`.
    pub fn new(text: impl Into<String>, duration: TimeSpan) -> Self {
        Self {
            text: text.into(),
            duration,
        }
    }

    /// The words without their tags.
    pub fn plain_text(&self) -> String {
        plain_text_of_cue(&self.text)
    }
}

/// A cue with its place on the timeline: what an `.srt` file holds, and what
/// [`AddSubtitleCues`](crate::commands::AddSubtitleCues) adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedSubtitleCue {
    /// When it appears.
    pub start: Timestamp,
    /// When it disappears (after `start`).
    pub end: Timestamp,
    /// Its text, with line breaks and `<i>` / `<b>` tags.
    pub text: String,
}

/// What is drawn behind the letters so they read on any picture.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SubtitleBackdrop {
    /// Nothing: the letters alone.
    None,
    /// A line around every letter (the look of most short videos).
    Outline {
        /// The line's colour.
        color: Color,
        /// How far the line reaches beyond the letters, in canvas pixels.
        width_in_pixels: f32,
    },
    /// A box behind each line (the look of television and streaming subtitles).
    Box {
        /// The box's colour, usually partly transparent.
        color: Color,
    },
}

/// Which edge of the canvas the cues sit against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubtitlePlacement {
    /// Near the bottom (the usual place).
    #[default]
    Bottom,
    /// Near the top (to leave the bottom to something else).
    Top,
}

impl SubtitlePlacement {
    /// `"bottom"` or `"top"`, for messages.
    pub fn display_name(self) -> &'static str {
        match self {
            SubtitlePlacement::Bottom => "bottom",
            SubtitlePlacement::Top => "top",
        }
    }
}

/// How every cue of a subtitle track looks and where it sits.
///
/// Lines are centred. A line wider than [`maximum_line_width_fraction`](Self::maximum_line_width_fraction)
/// of the canvas is wrapped between words.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubtitleStyle {
    /// Font name, for example `"Inter"`.
    pub font_family: String,
    /// Font size, in canvas pixels.
    pub font_size_in_pixels: f32,
    /// Whether every word is bold (`<b>` makes single words bold).
    pub is_bold: bool,
    /// The letters' colour.
    pub text_color: Color,
    /// Outline, box, or nothing.
    pub backdrop: SubtitleBackdrop,
    /// Bottom or top of the canvas.
    pub placement: SubtitlePlacement,
    /// Distance from that edge of the canvas to the nearest line (its box or
    /// outline included), in canvas pixels.
    pub margin_in_pixels: f32,
    /// The widest a line may be, as a fraction of the canvas width (0.2 to 1).
    pub maximum_line_width_fraction: f32,
}

/// Font size of the default style, as a fraction of the canvas's shorter side
/// (60 pixels for 1080p, landscape or vertical).
const DEFAULT_FONT_SIZE_FRACTION_OF_SHORTER_SIDE: f32 = 1.0 / 18.0;
/// Outline width of the default style, as a fraction of the font size.
const DEFAULT_OUTLINE_WIDTH_FRACTION_OF_FONT_SIZE: f32 = 0.08;
/// Bottom margin on a vertical canvas: TikTok, Reels and Shorts cover about the
/// bottom fifth with their captions and buttons.
const VERTICAL_CANVAS_MARGIN_FRACTION_OF_HEIGHT: f32 = 0.25;
/// Bottom margin on a landscape or square canvas, inside television's
/// "title safe" area.
const LANDSCAPE_CANVAS_MARGIN_FRACTION_OF_HEIGHT: f32 = 0.08;
/// Widest line of the default style, as a fraction of the canvas width (it also
/// keeps clear of the buttons along the right edge of vertical videos).
const DEFAULT_MAXIMUM_LINE_WIDTH_FRACTION: f32 = 0.8;

impl SubtitleStyle {
    /// Narrowest allowed line width, as a fraction of the canvas width.
    pub const MINIMUM_LINE_WIDTH_FRACTION: f32 = 0.2;
    /// Widest allowed outline, in canvas pixels.
    pub const MAXIMUM_OUTLINE_WIDTH_IN_PIXELS: f32 = 100.0;

    /// The default style for a canvas: white Inter letters with a black outline,
    /// sized from the canvas, clear of the interface of vertical-video apps on a
    /// vertical canvas, inside the title-safe area on the others.
    pub fn for_canvas(canvas_size: PictureSize) -> Self {
        let is_vertical = canvas_size.height_in_pixels > canvas_size.width_in_pixels;
        let font_size_in_pixels = default_font_size_for(canvas_size);
        Self {
            font_family: "Inter".to_string(),
            font_size_in_pixels,
            is_bold: is_vertical,
            text_color: Color::WHITE,
            backdrop: SubtitleBackdrop::Outline {
                color: Color::BLACK,
                width_in_pixels: default_outline_width_for(font_size_in_pixels),
            },
            placement: SubtitlePlacement::Bottom,
            margin_in_pixels: default_margin_for(canvas_size),
            maximum_line_width_fraction: DEFAULT_MAXIMUM_LINE_WIDTH_FRACTION,
        }
    }

    /// The style with every size multiplied by `factor` (an export at another
    /// size of the same shape). The line width is a fraction: it stays.
    pub fn scaled_by(&self, factor: f32) -> Self {
        let mut scaled = self.clone();
        scaled.font_size_in_pixels *= factor;
        scaled.margin_in_pixels *= factor;
        if let SubtitleBackdrop::Outline {
            width_in_pixels, ..
        } = &mut scaled.backdrop
        {
            *width_in_pixels *= factor;
        }
        scaled
    }

    /// The style refitted to a canvas of another shape or size: the font size,
    /// outline width and margin become those of [`SubtitleStyle::for_canvas`]
    /// for the new canvas, in the same proportion to the old canvas's defaults
    /// (a font made 20 % larger stays 20 % larger). Font, colours, backdrop kind,
    /// placement and line width stay.
    pub fn refitted_from_canvas_to_canvas(
        &self,
        old_canvas_size: PictureSize,
        new_canvas_size: PictureSize,
    ) -> Self {
        let ratio_to_default = |value: f32, old_default: f32| {
            if old_default > 0.0 && value.is_finite() {
                value / old_default
            } else {
                1.0
            }
        };
        let old_font_default = default_font_size_for(old_canvas_size);
        let new_font_default = default_font_size_for(new_canvas_size);
        let font_ratio = ratio_to_default(self.font_size_in_pixels, old_font_default);
        let margin_ratio =
            ratio_to_default(self.margin_in_pixels, default_margin_for(old_canvas_size));
        let mut refitted = self.clone();
        refitted.font_size_in_pixels = (new_font_default * font_ratio).round().max(1.0);
        refitted.margin_in_pixels = (default_margin_for(new_canvas_size) * margin_ratio).round();
        if let SubtitleBackdrop::Outline {
            width_in_pixels, ..
        } = &mut refitted.backdrop
        {
            let outline_ratio = ratio_to_default(
                *width_in_pixels,
                default_outline_width_for(self.font_size_in_pixels),
            );
            *width_in_pixels =
                default_outline_width_for(refitted.font_size_in_pixels) * outline_ratio;
        }
        refitted
    }

    /// The widest a line may be on a canvas `canvas_width_in_pixels` wide.
    pub fn maximum_line_width_in_pixels(&self, canvas_width_in_pixels: u32) -> f32 {
        canvas_width_in_pixels as f32 * self.maximum_line_width_fraction
    }
}

fn default_font_size_for(canvas_size: PictureSize) -> f32 {
    let shorter_side = canvas_size
        .width_in_pixels
        .min(canvas_size.height_in_pixels);
    (shorter_side as f32 * DEFAULT_FONT_SIZE_FRACTION_OF_SHORTER_SIDE)
        .round()
        .max(1.0)
}

fn default_outline_width_for(font_size_in_pixels: f32) -> f32 {
    (font_size_in_pixels * DEFAULT_OUTLINE_WIDTH_FRACTION_OF_FONT_SIZE * 10.0).round() / 10.0
}

fn default_margin_for(canvas_size: PictureSize) -> f32 {
    let fraction = if canvas_size.height_in_pixels > canvas_size.width_in_pixels {
        VERTICAL_CANVAS_MARGIN_FRACTION_OF_HEIGHT
    } else {
        LANDSCAPE_CANVAS_MARGIN_FRACTION_OF_HEIGHT
    };
    (canvas_size.height_in_pixels as f32 * fraction).round()
}

// ----- Markup: line breaks, <i> and <b> -----

/// A piece of a cue's text in one style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CueTextRun {
    /// The characters, line breaks included.
    pub text: String,
    /// Inside `<i>…</i>`.
    pub is_italic: bool,
    /// Inside `<b>…</b>`.
    pub is_bold: bool,
}

/// A tag of the cue markup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CueTag {
    Italic,
    Bold,
}

impl CueTag {
    fn opening(self) -> &'static str {
        match self {
            CueTag::Italic => "<i>",
            CueTag::Bold => "<b>",
        }
    }

    fn closing(self) -> &'static str {
        match self {
            CueTag::Italic => "</i>",
            CueTag::Bold => "</b>",
        }
    }
}

/// The tag at the start of `text`, with whether it closes and its length.
fn tag_at_start_of(text: &str) -> Option<(CueTag, bool, usize)> {
    let candidates = [
        ("<i>", CueTag::Italic, false),
        ("</i>", CueTag::Italic, true),
        ("<b>", CueTag::Bold, false),
        ("</b>", CueTag::Bold, true),
    ];
    candidates
        .into_iter()
        .find(|(written, _, _)| {
            text.get(..written.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(written))
        })
        .map(|(written, tag, is_closing)| (tag, is_closing, written.len()))
}

/// Reads a cue's text into runs of one style. Only `<i>`, `</i>`, `<b>` and
/// `</b>` (in any case) are tags; anything else is text. An unclosed tag runs
/// to the end, a closing tag without its opening one is ignored. Neighbouring
/// runs of the same style are joined, and empty runs left out.
pub fn parse_cue_markup(text: &str) -> Vec<CueTextRun> {
    let mut runs: Vec<CueTextRun> = Vec::new();
    let (mut italic_depth, mut bold_depth) = (0_u32, 0_u32);
    let mut current = String::new();
    let mut remaining = text;
    while let Some(character) = remaining.chars().next() {
        if character == '<'
            && let Some((tag, is_closing, length)) = tag_at_start_of(remaining)
        {
            push_run(&mut runs, &mut current, italic_depth > 0, bold_depth > 0);
            let depth = match tag {
                CueTag::Italic => &mut italic_depth,
                CueTag::Bold => &mut bold_depth,
            };
            *depth = if is_closing {
                depth.saturating_sub(1)
            } else {
                depth.saturating_add(1)
            };
            remaining = &remaining[length..];
            continue;
        }
        current.push(character);
        remaining = &remaining[character.len_utf8()..];
    }
    push_run(&mut runs, &mut current, italic_depth > 0, bold_depth > 0);
    runs
}

/// Adds `text` as a run (joined to the last one if it has the same style).
fn push_run(runs: &mut Vec<CueTextRun>, text: &mut String, is_italic: bool, is_bold: bool) {
    if text.is_empty() {
        return;
    }
    let text = std::mem::take(text);
    match runs.last_mut() {
        Some(last) if last.is_italic == is_italic && last.is_bold == is_bold => {
            last.text.push_str(&text);
        }
        _ => runs.push(CueTextRun {
            text,
            is_italic,
            is_bold,
        }),
    }
}

/// Writes runs back as cue markup, with properly nested tags (`<b>` outside
/// `<i>` when both start together).
pub fn write_cue_markup(runs: &[CueTextRun]) -> String {
    let mut written = String::new();
    let mut open_tags: Vec<CueTag> = Vec::new();
    for run in runs {
        if run.text.is_empty() {
            continue;
        }
        let wants = |tag: CueTag| match tag {
            CueTag::Italic => run.is_italic,
            CueTag::Bold => run.is_bold,
        };
        // Close back to the outermost tag this run does not want.
        if let Some(first_unwanted) = open_tags.iter().position(|tag| !wants(*tag)) {
            for tag in open_tags.drain(first_unwanted..).rev() {
                written.push_str(tag.closing());
            }
        }
        for tag in [CueTag::Bold, CueTag::Italic] {
            if wants(tag) && !open_tags.contains(&tag) {
                written.push_str(tag.opening());
                open_tags.push(tag);
            }
        }
        written.push_str(&run.text);
    }
    for tag in open_tags.into_iter().rev() {
        written.push_str(tag.closing());
    }
    written
}

/// The words of a cue without their tags.
pub fn plain_text_of_cue(text: &str) -> String {
    parse_cue_markup(text)
        .into_iter()
        .map(|run| run.text)
        .collect()
}

/// A cue's text tidied: tags in lower case and properly nested, spaces around
/// each line removed, empty lines at the start and end removed.
pub fn normalized_cue_text(text: &str) -> String {
    let rewritten = write_cue_markup(&parse_cue_markup(text));
    let lines: Vec<&str> = rewritten.lines().map(str::trim).collect();
    let first = lines.iter().position(|line| !is_blank_markup(line));
    let last = lines.iter().rposition(|line| !is_blank_markup(line));
    match (first, last) {
        (Some(first), Some(last)) => lines[first..=last].join("\n"),
        _ => String::new(),
    }
}

/// Whether a line of markup shows no character (only tags and spaces).
fn is_blank_markup(line: &str) -> bool {
    plain_text_of_cue(line).trim().is_empty()
}

/// A cue's text cut in two for a cut at `fraction` (0 to 1) of its duration:
/// at the space or line break nearest to that fraction of its characters, so
/// that each part keeps whole words. Tags open at the cut are closed at the end
/// of the first part and opened again at the start of the second.
///
/// A text without a space or line break cannot be cut: both parts get all of it.
pub fn split_cue_text_at_fraction(text: &str, fraction: f64) -> (String, String) {
    let characters: Vec<(char, bool, bool)> = parse_cue_markup(text)
        .iter()
        .flat_map(|run| {
            run.text
                .chars()
                .map(move |character| (character, run.is_italic, run.is_bold))
        })
        .collect();
    let target = fraction.clamp(0.0, 1.0) * characters.len() as f64;
    let best_break = characters
        .iter()
        .enumerate()
        .filter(|(index, (character, _, _))| {
            character.is_whitespace() && *index > 0 && index + 1 < characters.len()
        })
        .min_by(|(first, _), (second, _)| {
            let distance = |index: usize| (index as f64 + 0.5 - target).abs();
            distance(*first).total_cmp(&distance(*second))
        })
        .map(|(index, _)| index);
    let Some(break_index) = best_break else {
        return (text.to_string(), text.to_string());
    };
    let part_text = |part: &[(char, bool, bool)]| {
        let mut runs: Vec<CueTextRun> = Vec::new();
        for (character, is_italic, is_bold) in part {
            let mut piece = character.to_string();
            push_run(&mut runs, &mut piece, *is_italic, *is_bold);
        }
        normalized_cue_text(&write_cue_markup(&runs))
    };
    let first = part_text(&characters[..break_index]);
    let second = part_text(&characters[break_index + 1..]);
    if is_blank_markup(&first) || is_blank_markup(&second) {
        return (text.to_string(), text.to_string());
    }
    (first, second)
}

/// The text of two cues merged into one: the first's words, a space, then the
/// second's.
pub fn joined_cue_texts(first: &str, second: &str) -> String {
    normalized_cue_text(&format!("{} {}", first.trim_end(), second.trim_start()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, is_italic: bool, is_bold: bool) -> CueTextRun {
        CueTextRun {
            text: text.to_string(),
            is_italic,
            is_bold,
        }
    }

    #[test]
    fn markup_is_read_into_runs_of_one_style() {
        assert_eq!(
            parse_cue_markup("Hello <i>dear</i>\n<B>friend</B>!"),
            vec![
                run("Hello ", false, false),
                run("dear", true, false),
                run("\n", false, false),
                run("friend", false, true),
                run("!", false, false),
            ]
        );
        assert_eq!(
            parse_cue_markup("<b>both <i>styles</i></b>"),
            vec![run("both ", false, true), run("styles", true, true)]
        );
    }

    #[test]
    fn anything_that_is_not_one_of_the_four_tags_is_text() {
        assert_eq!(
            parse_cue_markup("1 < 2 and <u>under</u> </i>"),
            vec![run("1 < 2 and <u>under</u> ", false, false)]
        );
        assert_eq!(
            parse_cue_markup("<i>never closed"),
            vec![run("never closed", true, false)]
        );
        assert_eq!(plain_text_of_cue("<i>a</i> <b>b</b>"), "a b");
    }

    #[test]
    fn markup_is_written_back_properly_nested() {
        let runs = vec![
            run("plain ", false, false),
            run("bold ", false, true),
            run("both", true, true),
            run(" italic", true, false),
        ];
        let written = write_cue_markup(&runs);
        assert_eq!(written, "plain <b>bold <i>both</i></b><i> italic</i>");
        assert_eq!(parse_cue_markup(&written), runs);
    }

    #[test]
    fn normalizing_tidies_tags_and_spaces() {
        assert_eq!(
            normalized_cue_text("  \n <I>Hello</I>  \n  world \n\n"),
            "<i>Hello</i>\nworld"
        );
        assert_eq!(normalized_cue_text("<i> </i>"), "");
    }

    #[test]
    fn a_cut_falls_between_words_near_the_fraction() {
        assert_eq!(
            split_cue_text_at_fraction("one two three four", 0.5),
            ("one two".to_string(), "three four".to_string())
        );
        assert_eq!(
            split_cue_text_at_fraction("first line\nsecond line", 0.5),
            ("first line".to_string(), "second line".to_string())
        );
        assert_eq!(
            split_cue_text_at_fraction("one two three four", 0.1),
            ("one".to_string(), "two three four".to_string())
        );
        assert_eq!(
            split_cue_text_at_fraction("<i>one two</i>", 0.5),
            ("<i>one</i>".to_string(), "<i>two</i>".to_string())
        );
        assert_eq!(
            split_cue_text_at_fraction("Unbreakable", 0.5),
            ("Unbreakable".to_string(), "Unbreakable".to_string())
        );
    }

    #[test]
    fn merged_cues_are_joined_with_a_space() {
        assert_eq!(
            joined_cue_texts("I think that ", " we should go."),
            "I think that we should go."
        );
        assert_eq!(joined_cue_texts("<i>one</i>", "two"), "<i>one</i> two");
    }

    #[test]
    fn the_default_style_fits_vertical_and_landscape_canvases() {
        let vertical = SubtitleStyle::for_canvas(PictureSize {
            width_in_pixels: 1080,
            height_in_pixels: 1920,
        });
        let landscape = SubtitleStyle::for_canvas(PictureSize {
            width_in_pixels: 1920,
            height_in_pixels: 1080,
        });
        assert_eq!(vertical.font_size_in_pixels, 60.0);
        assert_eq!(landscape.font_size_in_pixels, 60.0);
        assert_eq!(vertical.margin_in_pixels, 480.0);
        assert_eq!(landscape.margin_in_pixels, 86.0);
        assert!(vertical.is_bold && !landscape.is_bold);

        let refitted = landscape.refitted_from_canvas_to_canvas(
            PictureSize {
                width_in_pixels: 1920,
                height_in_pixels: 1080,
            },
            PictureSize {
                width_in_pixels: 1080,
                height_in_pixels: 1920,
            },
        );
        assert_eq!(refitted.margin_in_pixels, 480.0);
        assert_eq!(refitted.font_size_in_pixels, 60.0);

        let doubled = landscape.scaled_by(2.0);
        assert_eq!(doubled.font_size_in_pixels, 120.0);
        assert_eq!(doubled.margin_in_pixels, 172.0);
    }
}
