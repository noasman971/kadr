//! Reading and writing `.srt` subtitle files (SubRip).
//!
//! An `.srt` file is a list of blocks separated by blank lines:
//!
//! ```text
//! 1
//! 00:00:01,000 --> 00:00:03,500
//! Hello <i>there</i>.
//! Second line.
//! ```
//!
//! [`parse_srt`] is forgiving where files in the wild differ, and precise where
//! a file cannot be read:
//!
//! - a byte-order mark, Windows (`\r\n`) or old Mac (`\r`) line endings;
//! - `,` or `.` before the milliseconds, one to three digits of them, hours left
//!   out (`01:02,500`), position coordinates after the times (ignored);
//! - missing numbers, numbers out of order or with gaps (numbers are ignored);
//! - blank lines inside a cue's text (the text goes on until the next number or
//!   timing line);
//! - cues out of order (sorted), overlapping cues (the earlier one is shortened,
//!   two cues starting together become one with both texts);
//! - `<i>` and `<b>` are kept; `<u>`, `<font …>`, `<s>` and `{\an8}`-style
//!   positions are removed with a warning.
//!
//! Errors give the line number (from 1) and never quote the file.
//!
//! [`decode_subtitle_file_bytes`] turns a file's bytes into text: UTF-8 with or
//! without a byte-order mark, UTF-16 with one, and Windows-1252 (the usual
//! encoding of older Western European files) when the bytes are not UTF-8.
//!
//! [`write_srt`] writes cues back: numbered from 1, `HH:MM:SS,mmm`, Windows line
//! endings, UTF-8.

use crate::subtitle::{TimedSubtitleCue, normalized_cue_text};
use crate::time::{TICKS_PER_SECOND, Timestamp};

/// What [`parse_srt`] read.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedSubtitleFile {
    /// The cues, sorted by start and not overlapping.
    pub cues: Vec<TimedSubtitleCue>,
    /// What was changed or left out to read the file, one sentence each.
    pub warnings: Vec<String>,
}

/// Why a subtitle file cannot be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SrtError {
    /// A line cannot be read.
    #[error("line {line_number}: {problem}")]
    AtLine {
        /// The line, counting from 1.
        line_number: usize,
        /// What is wrong, and what was expected.
        problem: String,
    },
    /// The file holds no cue at all.
    #[error(
        "the file holds no subtitle: an .srt file is a list of blocks like \"1\", then \"00:00:01,000 --> 00:00:03,500\", then the text"
    )]
    NoCue,
}

/// Milliseconds per second.
const MILLISECONDS_PER_SECOND: u64 = 1_000;
/// Ticks per millisecond (every millisecond is a whole number of ticks).
const TICKS_PER_MILLISECOND: u64 = TICKS_PER_SECOND / MILLISECONDS_PER_SECOND;

/// The example shown when a timing line cannot be read.
const TIMING_LINE_EXAMPLE: &str = "00:00:01,000 --> 00:00:03,500";

// ----- Bytes to text -----

/// The text of a subtitle file, and a warning when it was not UTF-8.
pub fn decode_subtitle_file_bytes(bytes: &[u8]) -> (String, Option<String>) {
    if let Some(utf16_text) = decoded_utf16_with_byte_order_mark(bytes) {
        return (utf16_text, None);
    }
    let without_mark = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match std::str::from_utf8(without_mark) {
        Ok(text) => (text.to_string(), None),
        Err(_) => (
            decoded_windows_1252(without_mark),
            Some(
                "the file is not UTF-8: it was read as Windows-1252 (Western European); if accents look wrong, save it as UTF-8 and import it again"
                    .to_string(),
            ),
        ),
    }
}

/// UTF-16 text, when the bytes start with a UTF-16 byte-order mark.
fn decoded_utf16_with_byte_order_mark(bytes: &[u8]) -> Option<String> {
    let (is_little_endian, rest) = match bytes {
        [0xFF, 0xFE, rest @ ..] => (true, rest),
        [0xFE, 0xFF, rest @ ..] => (false, rest),
        _ => return None,
    };
    let (pairs, _) = rest.as_chunks::<2>();
    let units = pairs.iter().map(|pair| {
        if is_little_endian {
            u16::from_le_bytes(*pair)
        } else {
            u16::from_be_bytes(*pair)
        }
    });
    Some(
        char::decode_utf16(units)
            .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect(),
    )
}

/// The characters Windows-1252 puts at 0x80 to 0x9F (the rest is Latin-1).
const WINDOWS_1252_0X80_TO_0X9F: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}',
    '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
];

fn decoded_windows_1252(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&byte| match byte {
            0x80..=0x9F => WINDOWS_1252_0X80_TO_0X9F[usize::from(byte - 0x80)],
            _ => char::from(byte),
        })
        .collect()
}

// ----- Reading -----

/// A cue as read, before sorting.
struct CueBeingRead {
    start: Timestamp,
    end: Timestamp,
    timing_line_number: usize,
    text_lines: Vec<String>,
    /// Blank lines seen after the last text line, kept if more text follows.
    pending_blank_lines: usize,
}

/// Tags and codes removed while reading, with the first line they were on.
#[derive(Default)]
struct RemovedFormatting {
    count: usize,
    first_line_number: Option<usize>,
}

/// Reads the text of an `.srt` file.
///
/// # Errors
///
/// [`SrtError::AtLine`] for a line that is neither a number, a timing line nor
/// a cue's text, a timing line that cannot be read, or a cue ending before it
/// starts; [`SrtError::NoCue`] when nothing is left.
pub fn parse_srt(text: &str) -> Result<ParsedSubtitleFile, SrtError> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let lines: Vec<&str> = text
        .split("\r\n")
        .flat_map(|line| line.split(['\r', '\n']))
        .collect();
    let mut cues_read: Vec<CueBeingRead> = Vec::new();
    let mut removed_formatting = RemovedFormatting::default();
    let mut index = 0;
    while index < lines.len() {
        let line_number = index + 1;
        let line = lines[index].trim();
        if line.is_empty() {
            if let Some(cue) = cues_read.last_mut()
                && !cue.text_lines.is_empty()
            {
                cue.pending_blank_lines += 1;
            }
            index += 1;
            continue;
        }
        if looks_like_timing_line(line) {
            cues_read.push(cue_starting_at(line, line_number)?);
            index += 1;
            continue;
        }
        let next_line = lines.get(index + 1).map(|next| next.trim());
        if is_cue_number(line)
            && let Some(next_line) = next_line
            && looks_like_timing_line(next_line)
        {
            cues_read.push(cue_starting_at(next_line, line_number + 1)?);
            index += 2;
            continue;
        }
        let Some(cue) = cues_read.last_mut() else {
            return Err(SrtError::AtLine {
                line_number,
                problem: format!(
                    "expected a subtitle number or a timing line such as {TIMING_LINE_EXAMPLE}, but found text"
                ),
            });
        };
        for _ in 0..std::mem::take(&mut cue.pending_blank_lines) {
            cue.text_lines.push(String::new());
        }
        cue.text_lines.push(without_unsupported_formatting(
            line,
            line_number,
            &mut removed_formatting,
        ));
        index += 1;
    }
    finish_reading(cues_read, &removed_formatting)
}

/// Sorts, removes empty cues and overlaps, and gathers the warnings.
fn finish_reading(
    cues_read: Vec<CueBeingRead>,
    removed_formatting: &RemovedFormatting,
) -> Result<ParsedSubtitleFile, SrtError> {
    let mut warnings = Vec::new();
    let mut cues: Vec<(TimedSubtitleCue, usize)> = Vec::with_capacity(cues_read.len());
    let mut empty_cue_lines: Vec<usize> = Vec::new();
    let mut instant_cue_lines: Vec<usize> = Vec::new();
    for cue in cues_read {
        let text = normalized_cue_text(&cue.text_lines.join("\n"));
        if text.is_empty() {
            empty_cue_lines.push(cue.timing_line_number);
        } else if cue.end == cue.start {
            instant_cue_lines.push(cue.timing_line_number);
        } else {
            cues.push((
                TimedSubtitleCue {
                    start: cue.start,
                    end: cue.end,
                    text,
                },
                cue.timing_line_number,
            ));
        }
    }
    if let Some(warning) = lines_warning(&empty_cue_lines, "had no text and was left out") {
        warnings.push(warning);
    }
    if let Some(warning) = lines_warning(&instant_cue_lines, "lasted no time and was left out") {
        warnings.push(warning);
    }
    if removed_formatting.count > 0 {
        warnings.push(format!(
            "{} underline, font, colour or position code(s) were removed (the first on line {}): Kadr keeps only <i> and <b>, and the subtitle style sets the rest",
            removed_formatting.count,
            removed_formatting.first_line_number.unwrap_or(1)
        ));
    }
    let was_sorted = cues
        .windows(2)
        .all(|pair| pair[0].0.start <= pair[1].0.start);
    if !was_sorted {
        cues.sort_by_key(|(cue, _)| cue.start);
        warnings.push("the subtitles were not in time order: they were sorted".to_string());
    }
    let cues = without_overlaps(cues, &mut warnings);
    if cues.is_empty() {
        return Err(SrtError::NoCue);
    }
    Ok(ParsedSubtitleFile { cues, warnings })
}

/// "the subtitle on line 4 …" or "3 subtitles (the first on line 4) …".
fn lines_warning(line_numbers: &[usize], what_happened: &str) -> Option<String> {
    match line_numbers {
        [] => None,
        [line_number] => Some(format!(
            "the subtitle timed on line {line_number} {what_happened}"
        )),
        [first, ..] => Some(format!(
            "{} subtitles (the first timed on line {first}) each {what_happened}",
            line_numbers.len()
        )),
    }
}

/// Cues that follow each other: an earlier cue ending after the next one starts
/// ends there; cues starting at the same moment become one, with both texts.
fn without_overlaps(
    sorted_cues: Vec<(TimedSubtitleCue, usize)>,
    warnings: &mut Vec<String>,
) -> Vec<TimedSubtitleCue> {
    let mut result: Vec<TimedSubtitleCue> = Vec::with_capacity(sorted_cues.len());
    let (mut shortened_count, mut combined_count) = (0, 0);
    for (cue, _) in sorted_cues {
        let Some(previous) = result.last_mut() else {
            result.push(cue);
            continue;
        };
        if cue.start == previous.start {
            previous.text = format!("{}\n{}", previous.text, cue.text);
            previous.end = previous.end.max(cue.end);
            combined_count += 1;
            continue;
        }
        if previous.end > cue.start {
            previous.end = cue.start;
            shortened_count += 1;
        }
        result.push(cue);
    }
    if shortened_count > 0 {
        warnings.push(format!(
            "{shortened_count} subtitle(s) overlapped the next one and now end when it starts (a subtitle track shows one subtitle at a time)"
        ));
    }
    if combined_count > 0 {
        warnings.push(format!(
            "{combined_count} subtitle(s) started at the same moment as another one and were combined with it, one under the other"
        ));
    }
    result
}

/// Whether a line is a cue number.
fn is_cue_number(line: &str) -> bool {
    !line.is_empty() && line.chars().all(|character| character.is_ascii_digit())
}

/// Whether a line is meant as a timing line (it may still be malformed).
fn looks_like_timing_line(line: &str) -> bool {
    line.contains("-->") || {
        let starts_with_time = line
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_digit())
            && line.contains(':');
        starts_with_time && (line.contains("->") || line.contains('→'))
    }
}

/// A new cue from its timing line.
fn cue_starting_at(line: &str, line_number: usize) -> Result<CueBeingRead, SrtError> {
    let at_line = |problem: String| SrtError::AtLine {
        line_number,
        problem,
    };
    let Some((start_text, rest)) = line.split_once("-->") else {
        return Err(at_line(format!(
            "this timing line cannot be read: the two times must be separated by \"-->\", as in {TIMING_LINE_EXAMPLE}"
        )));
    };
    // Coordinates may follow the end time ("X1:100 X2:200 …"): ignored.
    let end_text = rest.split_whitespace().next().unwrap_or("");
    let start = parse_time(start_text.trim())
        .map_err(|problem| at_line(format!("the start time cannot be read: {problem}")))?;
    let end = parse_time(end_text)
        .map_err(|problem| at_line(format!("the end time cannot be read: {problem}")))?;
    if end < start {
        return Err(at_line(format!(
            "this subtitle ends ({}) before it starts ({})",
            written_time(end),
            written_time(start)
        )));
    }
    Ok(CueBeingRead {
        start,
        end,
        timing_line_number: line_number,
        text_lines: Vec::new(),
        pending_blank_lines: 0,
    })
}

/// Reads `HH:MM:SS,mmm` (or `.mmm`, or `MM:SS,mmm`).
fn parse_time(text: &str) -> Result<Timestamp, String> {
    let expected = || {
        format!("\"hours:minutes:seconds,milliseconds\" is expected, as in {TIMING_LINE_EXAMPLE}")
    };
    if text.is_empty() {
        return Err(expected());
    }
    let (clock_text, fraction_text) = match text.find([',', '.']) {
        Some(position) => (&text[..position], Some(&text[position + 1..])),
        None => (text, None),
    };
    let fields: Vec<&str> = clock_text.split(':').collect();
    let numbers: Option<Vec<u64>> = fields
        .iter()
        .map(|field| {
            (!field.is_empty() && field.chars().all(|character| character.is_ascii_digit()))
                .then(|| field.parse::<u64>().ok())
                .flatten()
        })
        .collect();
    let Some(numbers) = numbers else {
        return Err(expected());
    };
    let (hours, minutes, seconds) = match numbers.as_slice() {
        [hours, minutes, seconds] => (*hours, *minutes, *seconds),
        [minutes, seconds] => (0, *minutes, *seconds),
        _ => return Err(expected()),
    };
    if minutes >= 60 || seconds >= 60 {
        return Err(format!(
            "minutes and seconds go from 00 to 59 ({} is not a time)",
            text.chars()
                .filter(|character| character.is_ascii_digit() || ":,.".contains(*character))
                .collect::<String>()
        ));
    }
    let milliseconds = match fraction_text {
        None => 0,
        Some(digits)
            if (1..=3).contains(&digits.len())
                && digits.chars().all(|character| character.is_ascii_digit()) =>
        {
            // "5" is half a second, "05" five hundredths.
            let padded = format!("{digits:0<3}");
            padded.parse::<u64>().map_err(|_| expected())?
        }
        Some(_) => {
            return Err("the milliseconds must be one to three digits after the comma".to_string());
        }
    };
    let total_milliseconds =
        ((hours * 60 + minutes) * 60 + seconds) * MILLISECONDS_PER_SECOND + milliseconds;
    total_milliseconds
        .checked_mul(TICKS_PER_MILLISECOND)
        .map(Timestamp::from_ticks)
        .ok_or_else(|| "this time is too large".to_string())
}

/// `line` without the formatting Kadr does not keep: `<u>`, `<s>`, `<font …>`
/// and their closing tags, and `{\…}` codes. Counted in `removed`.
fn without_unsupported_formatting(
    line: &str,
    line_number: usize,
    removed: &mut RemovedFormatting,
) -> String {
    let mut kept = String::with_capacity(line.len());
    let mut remaining = line;
    while let Some(character) = remaining.chars().next() {
        if let Some(length) = unsupported_code_length_at_start_of(remaining) {
            removed.count += 1;
            removed.first_line_number.get_or_insert(line_number);
            remaining = &remaining[length..];
            continue;
        }
        kept.push(character);
        remaining = &remaining[character.len_utf8()..];
    }
    kept
}

/// The length of a removable code at the start of `text`, if one is there.
fn unsupported_code_length_at_start_of(text: &str) -> Option<usize> {
    if text.starts_with("{\\") {
        return text.find('}').map(|end| end + 1);
    }
    if !text.starts_with('<') {
        return None;
    }
    let end = text.find('>')?;
    let inside = text[1..end].trim().to_ascii_lowercase();
    let name = inside.trim_start_matches('/');
    let tag_name = name.split_whitespace().next().unwrap_or("");
    let is_removable = matches!(tag_name, "u" | "s" | "font")
        || tag_name.starts_with("font ")
        || name.starts_with("font");
    is_removable.then_some(end + 1)
}

// ----- Writing -----

/// Writes cues as an `.srt` file: numbered from 1, Windows line endings, times
/// rounded to the millisecond (a cue never lasts less than one).
pub fn write_srt(cues: &[TimedSubtitleCue]) -> String {
    let mut written = String::new();
    for (index, cue) in cues.iter().enumerate() {
        let start_milliseconds = rounded_milliseconds(cue.start);
        let end_milliseconds = rounded_milliseconds(cue.end).max(start_milliseconds + 1);
        written.push_str(&format!(
            "{}\r\n{} --> {}\r\n",
            index + 1,
            written_milliseconds(start_milliseconds),
            written_milliseconds(end_milliseconds)
        ));
        for line in cue.text.lines() {
            written.push_str(line);
            written.push_str("\r\n");
        }
        written.push_str("\r\n");
    }
    written
}

fn rounded_milliseconds(time: Timestamp) -> u64 {
    (time.ticks() + TICKS_PER_MILLISECOND / 2) / TICKS_PER_MILLISECOND
}

/// `HH:MM:SS,mmm`.
pub fn written_time(time: Timestamp) -> String {
    written_milliseconds(rounded_milliseconds(time))
}

fn written_milliseconds(total_milliseconds: u64) -> String {
    let milliseconds = total_milliseconds % MILLISECONDS_PER_SECOND;
    let total_seconds = total_milliseconds / MILLISECONDS_PER_SECOND;
    let (hours, minutes, seconds) = (
        total_seconds / 3_600,
        (total_seconds / 60) % 60,
        total_seconds % 60,
    );
    format!("{hours:02}:{minutes:02}:{seconds:02},{milliseconds:03}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn milliseconds(value: u64) -> Timestamp {
        Timestamp::from_milliseconds(value)
    }

    fn cue(start: u64, end: u64, text: &str) -> TimedSubtitleCue {
        TimedSubtitleCue {
            start: milliseconds(start),
            end: milliseconds(end),
            text: text.to_string(),
        }
    }

    #[test]
    fn a_plain_file_is_read() {
        let parsed = parse_srt(
            "1\n00:00:01,000 --> 00:00:03,500\nHello.\n\n2\n00:00:04,000 --> 00:00:06,000\nSecond <i>one</i>\non two lines\n",
        )
        .unwrap();
        assert_eq!(
            parsed.cues,
            vec![
                cue(1_000, 3_500, "Hello."),
                cue(4_000, 6_000, "Second <i>one</i>\non two lines"),
            ]
        );
        assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    }

    #[test]
    fn byte_order_marks_windows_line_endings_and_dots_are_accepted() {
        let parsed = parse_srt(
            "\u{FEFF}1\r\n00:00:01.5 --> 00:01:02.25 X1:10 X2:20\r\nA\r\n\r\n7\r\n01:02:03,004 --> 01:02:04,000\r\nB\r\n",
        )
        .unwrap();
        assert_eq!(
            parsed.cues,
            vec![cue(1_500, 62_250, "A"), cue(3_723_004, 3_724_000, "B"),]
        );
    }

    #[test]
    fn numbering_gaps_missing_numbers_and_blank_lines_in_text_are_accepted() {
        let parsed = parse_srt(
            "\n\n10\n00:00:01,000 --> 00:00:02,000\nFirst\n\nstill the first\n\n00:00:03,000 --> 00:00:04,000\nSecond\n\n\n\n99\n00:00:05,000 --> 00:00:06,000\nThird\n",
        )
        .unwrap();
        assert_eq!(
            parsed.cues,
            vec![
                cue(1_000, 2_000, "First\n\nstill the first"),
                cue(3_000, 4_000, "Second"),
                cue(5_000, 6_000, "Third"),
            ]
        );
    }

    #[test]
    fn unsupported_codes_are_removed_with_a_warning() {
        let parsed = parse_srt(
            "1\n00:00:01,000 --> 00:00:02,000\n{\\an8}<font color=\"#ff0000\">Red</font> <u>under</u> <B>bold</B>\n",
        )
        .unwrap();
        assert_eq!(parsed.cues[0].text, "Red under <b>bold</b>");
        assert_eq!(parsed.warnings.len(), 1);
        assert!(
            parsed.warnings[0].contains("5 underline"),
            "{:?}",
            parsed.warnings
        );
        assert!(parsed.warnings[0].contains("line 3"));
    }

    #[test]
    fn overlapping_and_unsorted_cues_are_fixed_with_warnings() {
        let parsed = parse_srt(
            "1\n00:00:05,000 --> 00:00:06,000\nLast\n\n2\n00:00:01,000 --> 00:00:03,000\nFirst\n\n3\n00:00:02,000 --> 00:00:04,000\nSecond\n\n4\n00:00:02,000 --> 00:00:03,000\nAlso second\n",
        )
        .unwrap();
        assert_eq!(
            parsed.cues,
            vec![
                cue(1_000, 2_000, "First"),
                cue(2_000, 4_000, "Second\nAlso second"),
                cue(5_000, 6_000, "Last"),
            ]
        );
        assert_eq!(parsed.warnings.len(), 3, "{:?}", parsed.warnings);
    }

    #[test]
    fn empty_and_instant_cues_are_left_out() {
        let parsed = parse_srt(
            "1\n00:00:01,000 --> 00:00:02,000\n<i> </i>\n\n2\n00:00:03,000 --> 00:00:03,000\nInstant\n\n3\n00:00:04,000 --> 00:00:05,000\nKept\n",
        )
        .unwrap();
        assert_eq!(parsed.cues, vec![cue(4_000, 5_000, "Kept")]);
        assert_eq!(parsed.warnings.len(), 2);
        assert!(parsed.warnings[0].contains("line 2"));
        assert!(parsed.warnings[1].contains("line 6"));
    }

    #[test]
    fn errors_name_the_line() {
        let error = |text: &str| parse_srt(text).unwrap_err().to_string();
        assert_eq!(
            error("Hello\n00:00:01,000 --> 00:00:02,000\nText\n"),
            "line 1: expected a subtitle number or a timing line such as 00:00:01,000 --> 00:00:03,500, but found text"
        );
        assert!(
            error("1\n00:00:01,000 --> 00:00:02,000\nA\n\n2\n00:00:01,000 -> 00:00:02,000\nB\n")
                .starts_with("line 6: this timing line cannot be read")
        );
        assert!(
            error("1\n00:00:01,000 --> 00:00:0x,000\nA\n")
                .starts_with("line 2: the end time cannot be read")
        );
        assert!(
            error("1\n00:00:61,000 --> 00:01:02,000\nA\n").starts_with("line 2: the start time")
        );
        assert!(error("1\n00:00:01,0000 --> 00:00:02,000\nA\n").contains("one to three digits"));
        assert_eq!(
            error("1\n00:00:05,000 --> 00:00:02,000\nA\n"),
            "line 2: this subtitle ends (00:00:02,000) before it starts (00:00:05,000)"
        );
        assert_eq!(parse_srt("\n \n").unwrap_err(), SrtError::NoCue);
    }

    #[test]
    fn bytes_in_utf8_utf16_and_windows_1252_are_decoded() {
        let (text, warning) = decode_subtitle_file_bytes("\u{FEFF}Été".as_bytes());
        assert_eq!((text.as_str(), warning), ("Été", None));
        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain("Ça va".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        assert_eq!(decode_subtitle_file_bytes(&utf16).0, "Ça va");
        let (text, warning) = decode_subtitle_file_bytes(b"\xC7a co\xFBte 5 \x80");
        assert_eq!(text, "Ça coûte 5 €");
        assert!(warning.unwrap().contains("Windows-1252"));
    }

    #[test]
    fn cues_are_written_back_and_read_again() {
        let cues = vec![
            cue(1_000, 3_500, "Hello <i>there</i>"),
            cue(3_723_004, 3_724_000, "Two\nlines"),
        ];
        let written = write_srt(&cues);
        assert_eq!(
            written,
            "1\r\n00:00:01,000 --> 00:00:03,500\r\nHello <i>there</i>\r\n\r\n2\r\n01:02:03,004 --> 01:02:04,000\r\nTwo\r\nlines\r\n\r\n"
        );
        assert_eq!(parse_srt(&written).unwrap().cues, cues);
    }
}
