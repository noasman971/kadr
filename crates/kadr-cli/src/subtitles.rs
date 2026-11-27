//! The `import-subtitles` and `export-subtitles` commands: an `.srt` file into a
//! new subtitle track of a project, and a subtitle track back to an `.srt` file.

use std::error::Error;
use std::path::{Path, PathBuf};

use kadr_core::history::EditSession;
use kadr_core::ids::TrackId;
use kadr_core::subtitle::{SubtitleBackdrop, SubtitlePlacement, SubtitleStyle};
use kadr_core::time::TimeSpan;
use kadr_core::timeline::Track;
use kadr_engine::project_files::{load_project_file, save_project_file};
use kadr_engine::subtitle_files::{
    add_subtitle_track_with_cues, export_subtitle_track, read_subtitle_file,
    subtitle_track_name_for,
};

use crate::arguments::{SubtitleBackdropChoice, SubtitlePositionChoice};

/// Everything the `import-subtitles` command was asked to do.
#[derive(Debug, Clone)]
pub struct SubtitleImportCommand {
    /// The `.kadr` project, changed in place.
    pub project: PathBuf,
    /// The `.srt` file.
    pub subtitles: PathBuf,
    /// Seconds every cue is moved later.
    pub offset_in_seconds: f64,
    /// What is behind the letters, if not the default.
    pub backdrop: Option<SubtitleBackdropChoice>,
    /// Bottom or top, if not the default.
    pub position: Option<SubtitlePositionChoice>,
}

/// Runs `import-subtitles`.
pub fn import_subtitles(command: &SubtitleImportCommand) -> Result<(), Box<dyn Error>> {
    let parsed = read_subtitle_file(&command.subtitles)?;
    let project = load_project_file(&command.project)?;
    let mut style = SubtitleStyle::for_canvas(project.settings().canvas_size);
    if let Some(backdrop) = command.backdrop {
        style.backdrop = match backdrop {
            SubtitleBackdropChoice::Outline => style.backdrop,
            SubtitleBackdropChoice::Box => SubtitleBackdrop::Box {
                color: kadr_core::color::Color::from_hex("#000000B3")?,
            },
            SubtitleBackdropChoice::None => SubtitleBackdrop::None,
        };
    }
    if let Some(position) = command.position {
        style.placement = match position {
            SubtitlePositionChoice::Bottom => SubtitlePlacement::Bottom,
            SubtitlePositionChoice::Top => SubtitlePlacement::Top,
        };
    }
    let offset = TimeSpan::from_seconds(command.offset_in_seconds)?;
    let mut session = EditSession::new(project)?;
    let added = session.transaction("Import subtitles", |transaction| {
        add_subtitle_track_with_cues(
            transaction,
            &subtitle_track_name_for(&command.subtitles),
            Some(style.clone()),
            &parsed.cues,
            offset,
        )
    })?;
    save_project_file(session.project(), &command.project)?;
    println!(
        "Imported {} subtitle cue(s) from {} into track {} of {}.",
        added.cue_ids.len(),
        command.subtitles.display(),
        added.track_id,
        command.project.display()
    );
    for warning in &parsed.warnings {
        println!("note: {warning}");
    }
    Ok(())
}

/// Runs `export-subtitles`.
pub fn export_subtitles(
    project_path: &Path,
    output: &Path,
    track: Option<u64>,
) -> Result<(), Box<dyn Error>> {
    let project = load_project_file(project_path)?;
    let subtitle_tracks: Vec<&Track> = project.timeline().subtitle_tracks().collect();
    let chosen = match track {
        Some(raw_track_id) => subtitle_tracks
            .iter()
            .find(|candidate| candidate.id == TrackId::from_raw(raw_track_id))
            .copied()
            .ok_or_else(|| {
                format!(
                    "track {raw_track_id} is not a subtitle track of {}; subtitle tracks: {}",
                    project_path.display(),
                    track_list(&subtitle_tracks)
                )
            })?,
        None => match subtitle_tracks.as_slice() {
            [only_track] => only_track,
            [] => return Err(format!("{} has no subtitle track", project_path.display()).into()),
            _ => {
                return Err(format!(
                    "{} has several subtitle tracks: choose one with --track ({})",
                    project_path.display(),
                    track_list(&subtitle_tracks)
                )
                .into());
            }
        },
    };
    let cue_count = export_subtitle_track(chosen, output)?;
    println!(
        "Wrote {cue_count} cue(s) of track {} (\"{}\") to {}.",
        chosen.id,
        chosen.name,
        output.display()
    );
    Ok(())
}

fn track_list(tracks: &[&Track]) -> String {
    if tracks.is_empty() {
        return "none".to_string();
    }
    tracks
        .iter()
        .map(|track| format!("{} (\"{}\")", track.id, track.name))
        .collect::<Vec<_>>()
        .join(", ")
}
