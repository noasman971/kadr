//! Building a simple project from a list of files, end to end.
//!
//! Until the desktop app exists, this is the quickest way to get a project to
//! export: videos and images go one after another on a video track, sound files
//! one after another on an audio track.

use std::path::PathBuf;

use kadr_core::commands::{AddClip, AddTrack, CreatedObjectsExt, ImportMedia, TrackPosition};
use kadr_core::error::EditError;
use kadr_core::history::EditSession;
use kadr_core::ids::TrackId;
use kadr_core::media_asset::MediaKind;
use kadr_core::project::{Project, ProjectSettings};
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_core::timeline::TrackKind;
use kadr_core::validation::ValidationError;
use kadr_media::{MediaBackend, MediaError};

/// Why a project could not be assembled.
#[derive(Debug, thiserror::Error)]
pub enum AssemblyError {
    /// No file was given.
    #[error("give at least one video, image or sound file")]
    NoFiles,
    /// A file could not be read.
    #[error(transparent)]
    Media(#[from] MediaError),
    /// A file could not be placed on the timeline.
    #[error(transparent)]
    Edit(#[from] EditError),
    /// The settings are not valid.
    #[error(transparent)]
    InvalidSettings(#[from] ValidationError),
}

/// Places every file end to end: pictures on a video track, sound files on an
/// audio track. Still images last `still_image_duration`.
///
/// # Errors
///
/// See [`AssemblyError`].
pub fn assemble_project_from_files(
    backend: &dyn MediaBackend,
    file_paths: &[PathBuf],
    settings: ProjectSettings,
    still_image_duration: TimeSpan,
) -> Result<Project, AssemblyError> {
    if file_paths.is_empty() {
        return Err(AssemblyError::NoFiles);
    }
    let mut session = EditSession::new(Project::new(settings))?;
    let mut video_track: Option<(TrackId, Timestamp)> = None;
    let mut audio_track: Option<(TrackId, Timestamp)> = None;

    for file_path in file_paths {
        let description = backend.probe(file_path)?;
        let kind = description.kind;
        let length = description.duration.unwrap_or(still_image_duration);
        let media_asset_id = session
            .execute(ImportMedia::new(description))?
            .first_media_asset_id()
            .ok_or_else(missing_created_object)?;

        let track_kind = if kind == MediaKind::Audio {
            TrackKind::Audio
        } else {
            TrackKind::Video
        };
        let track_slot = if kind == MediaKind::Audio {
            &mut audio_track
        } else {
            &mut video_track
        };
        let (track_id, next_free_time) = match *track_slot {
            Some(existing) => existing,
            None => (add_track(&mut session, track_kind)?, Timestamp::ZERO),
        };
        session.execute(AddClip::media(
            track_id,
            media_asset_id,
            Timestamp::ZERO,
            Timestamp::ZERO + length,
            next_free_time,
        ))?;
        *track_slot = Some((track_id, next_free_time + length));
    }
    Ok(session.into_project())
}

fn add_track(session: &mut EditSession, kind: TrackKind) -> Result<TrackId, AssemblyError> {
    session
        .execute(AddTrack::new(kind, TrackPosition::Top))?
        .first_track_id()
        .ok_or_else(missing_created_object)
}

fn missing_created_object() -> AssemblyError {
    AssemblyError::Edit(EditError::InternalInconsistency {
        details: "a command did not report the object it created".to_string(),
    })
}
