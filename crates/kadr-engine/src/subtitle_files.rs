//! `.srt` subtitle files on disk: reading one into a new subtitle track, and
//! writing a subtitle track back.
//!
//! The format itself (and its forgiving reader) is in [`kadr_core::srt`]; this
//! module reads and writes the files (atomically), and adds an imported file's
//! cues to a project in one undo step. The desktop app, Claude's tools and the
//! command line all go through it.

use std::fs;
use std::path::{Path, PathBuf};

use kadr_core::commands::{
    AddSubtitleCues, AddTrack, CreatedObject, CreatedObjectsExt, TrackPosition,
};
use kadr_core::error::EditError;
use kadr_core::history::Transaction;
use kadr_core::ids::{ClipId, TrackId};
use kadr_core::srt::{
    ParsedSubtitleFile, SrtError, decode_subtitle_file_bytes, parse_srt, write_srt,
};
use kadr_core::subtitle::{SubtitleStyle, TimedSubtitleCue, timed_cues_of};
use kadr_core::time::TimeSpan;
use kadr_core::timeline::{Track, TrackKind};

/// The extension of subtitle files, without the dot.
pub const SUBTITLE_FILE_EXTENSION: &str = "srt";

/// Subtitle files larger than this are refused before being read (a two-hour
/// film's subtitles weigh about 100 kB).
pub const MAXIMUM_SUBTITLE_FILE_SIZE_IN_BYTES: u64 = 16 * 1024 * 1024;

/// Why a subtitle file could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum SubtitleFileError {
    /// The file could not be read.
    #[error("could not read \"{}\": {reason}", file_path.display())]
    CannotRead {
        /// The file.
        file_path: PathBuf,
        /// What the system said.
        reason: String,
    },
    /// The file is too large to be subtitles.
    #[error(
        "\"{}\" is {size_in_bytes} bytes long, too large for a subtitle file (at most {MAXIMUM_SUBTITLE_FILE_SIZE_IN_BYTES})",
        file_path.display()
    )]
    TooLarge {
        /// The file.
        file_path: PathBuf,
        /// Its size.
        size_in_bytes: u64,
    },
    /// The file was read but is not a usable `.srt` file.
    #[error("\"{}\" cannot be read as subtitles: {source}", file_path.display())]
    InvalidContent {
        /// The file.
        file_path: PathBuf,
        /// What is wrong, with the line.
        source: SrtError,
    },
    /// The file could not be written.
    #[error("could not save \"{}\": {reason}", file_path.display())]
    CannotWrite {
        /// The file.
        file_path: PathBuf,
        /// What the system said.
        reason: String,
    },
}

/// Whether `file_path` is named like a subtitle file (`.srt`, any case).
pub fn is_subtitle_file_path(file_path: &Path) -> bool {
    file_path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(SUBTITLE_FILE_EXTENSION))
}

/// Reads an `.srt` file: its cues, sorted and without overlaps, and what was
/// changed to read it.
///
/// # Errors
///
/// The file cannot be read, is too large, or is not an `.srt` file (the error
/// names the line).
pub fn read_subtitle_file(file_path: &Path) -> Result<ParsedSubtitleFile, SubtitleFileError> {
    let cannot_read = |reason: String| SubtitleFileError::CannotRead {
        file_path: file_path.to_path_buf(),
        reason,
    };
    let size_in_bytes = fs::metadata(file_path)
        .map_err(|error| cannot_read(error.to_string()))?
        .len();
    if size_in_bytes > MAXIMUM_SUBTITLE_FILE_SIZE_IN_BYTES {
        return Err(SubtitleFileError::TooLarge {
            file_path: file_path.to_path_buf(),
            size_in_bytes,
        });
    }
    let bytes = fs::read(file_path).map_err(|error| cannot_read(error.to_string()))?;
    let (text, encoding_warning) = decode_subtitle_file_bytes(&bytes);
    let mut parsed = parse_srt(&text).map_err(|source| SubtitleFileError::InvalidContent {
        file_path: file_path.to_path_buf(),
        source,
    })?;
    if let Some(warning) = encoding_warning {
        parsed.warnings.insert(0, warning);
    }
    Ok(parsed)
}

/// Writes `cues` as an `.srt` file at `file_path`, replacing it if it exists,
/// atomically (a temporary file renamed over it).
///
/// # Errors
///
/// The file cannot be written.
pub fn write_subtitle_file(
    file_path: &Path,
    cues: &[TimedSubtitleCue],
) -> Result<(), SubtitleFileError> {
    let cannot_write = |reason: String| SubtitleFileError::CannotWrite {
        file_path: file_path.to_path_buf(),
        reason,
    };
    let mut temporary_name = file_path.as_os_str().to_owned();
    temporary_name.push(".saving");
    let temporary_path = PathBuf::from(temporary_name);
    fs::write(&temporary_path, write_srt(cues)).map_err(|error| cannot_write(error.to_string()))?;
    fs::rename(&temporary_path, file_path).map_err(|error| {
        // Best effort: do not leave the temporary file behind.
        let _ = fs::remove_file(&temporary_path);
        cannot_write(error.to_string())
    })
}

/// Writes the cues of a subtitle track as an `.srt` file. Returns how many.
///
/// # Errors
///
/// The file cannot be written.
pub fn export_subtitle_track(track: &Track, file_path: &Path) -> Result<usize, SubtitleFileError> {
    let cues = timed_cues_of(track);
    write_subtitle_file(file_path, &cues)?;
    Ok(cues.len())
}

/// The name of the track a subtitle file is imported into: the file's name
/// without its extension ("Subtitles" when it has none).
pub fn subtitle_track_name_for(file_path: &Path) -> String {
    file_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().trim().to_string())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "Subtitles".to_string())
}

/// A new subtitle track on top, and its cues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedSubtitleTrack {
    /// The new track.
    pub track_id: TrackId,
    /// Its cues, in time order.
    pub cue_ids: Vec<ClipId>,
}

/// Adds a subtitle track on top named `track_name`, with `style` (the canvas's
/// default style when `None`), holding `cues` moved later by `offset`. Call it
/// inside a transaction, so the import is one undo step.
///
/// # Errors
///
/// The engine's refusal (a cue without words, say).
pub fn add_subtitle_track_with_cues(
    transaction: &mut Transaction<'_>,
    track_name: &str,
    style: Option<SubtitleStyle>,
    cues: &[TimedSubtitleCue],
    offset: TimeSpan,
) -> Result<AddedSubtitleTrack, EditError> {
    let mut add_track = AddTrack::new(TrackKind::Subtitle, TrackPosition::Top).named(track_name);
    if let Some(style) = style {
        add_track = add_track.with_subtitle_style(style);
    }
    let track_id = transaction
        .execute(add_track)?
        .first_track_id()
        .ok_or_else(|| EditError::InternalInconsistency {
            details: "adding a subtitle track created no track".to_string(),
        })?;
    let moved_cues: Vec<TimedSubtitleCue> = cues
        .iter()
        .map(|cue| TimedSubtitleCue {
            start: cue.start + offset,
            end: cue.end + offset,
            text: cue.text.clone(),
        })
        .collect();
    let created = transaction.execute(AddSubtitleCues::new(track_id, moved_cues))?;
    let mut cue_ids: Vec<ClipId> = created
        .iter()
        .filter_map(|object| match object {
            CreatedObject::Clip(clip_id) => Some(*clip_id),
            _ => None,
        })
        .collect();
    let project = transaction.project();
    cue_ids.sort_by_key(|clip_id| {
        project
            .timeline()
            .find_clip(*clip_id)
            .map(|(_, clip)| clip.timeline_start)
    });
    Ok(AddedSubtitleTrack { track_id, cue_ids })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kadr_core::history::EditSession;
    use kadr_core::project::{Project, ProjectSettings};
    use kadr_core::time::Timestamp;

    #[test]
    fn a_file_is_read_imported_and_written_back() {
        let directory = tempfile::tempdir().unwrap();
        let input_path = directory.path().join("interview.SRT");
        fs::write(
            &input_path,
            b"\xEF\xBB\xBF1\r\n00:00:01,000 --> 00:00:02,500\r\nHello <i>there</i>\r\n\r\n3\r\n00:00:03,000 --> 00:00:04,000\r\nSecond\r\n",
        )
        .unwrap();
        assert!(is_subtitle_file_path(&input_path));
        let parsed = read_subtitle_file(&input_path).unwrap();
        assert_eq!(parsed.cues.len(), 2);

        let mut session =
            EditSession::new(Project::new(ProjectSettings::landscape_1080p())).unwrap();
        let added = session
            .transaction("Import subtitles", |transaction| {
                add_subtitle_track_with_cues(
                    transaction,
                    &subtitle_track_name_for(&input_path),
                    None,
                    &parsed.cues,
                    TimeSpan::from_whole_seconds(10),
                )
            })
            .unwrap();
        assert_eq!(added.cue_ids.len(), 2);
        let track = session.project().timeline().track(added.track_id).unwrap();
        assert_eq!(track.name, "interview");
        assert_eq!(
            track.clips()[0].timeline_start,
            Timestamp::from_milliseconds(11_000)
        );

        let output_path = directory.path().join("corrected.srt");
        assert_eq!(export_subtitle_track(track, &output_path).unwrap(), 2);
        let written = fs::read_to_string(&output_path).unwrap();
        assert!(
            written.starts_with("1\r\n00:00:11,000 --> 00:00:12,500\r\nHello <i>there</i>\r\n")
        );
        assert!(!directory.path().join("corrected.srt.saving").exists());
    }

    #[test]
    fn errors_name_the_file_and_the_line() {
        let directory = tempfile::tempdir().unwrap();
        let broken_path = directory.path().join("broken.srt");
        fs::write(&broken_path, "1\n00:00:01,000 -> 00:00:02,000\nText\n").unwrap();
        let error = read_subtitle_file(&broken_path).unwrap_err().to_string();
        assert!(error.contains("broken.srt"), "{error}");
        assert!(error.contains("line 2"), "{error}");
        let missing = read_subtitle_file(&directory.path().join("missing.srt")).unwrap_err();
        assert!(matches!(missing, SubtitleFileError::CannotRead { .. }));
    }
}
