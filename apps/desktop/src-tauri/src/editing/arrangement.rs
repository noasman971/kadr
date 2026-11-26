//! Arranging clips on the timeline: moving, trimming, splitting, deleting, and
//! muting, hiding or removing tracks. Each function is one undo step.

use kadr_core::commands::{
    AddTrack, ClipEdge, CreatedObjectsExt, MoveClip, RemoveClip, RemoveTrack, SetTrackHidden,
    SetTrackMuted, SplitClip, TrackPosition, TrimClip,
};
use kadr_core::error::EditError;
use kadr_core::history::EditSession;
use kadr_core::ids::{ClipId, TrackId};
use kadr_core::time::Timestamp;
use serde::Deserialize;

use super::{EditOutcome, clip_track_kind, missing_created_object};

/// Which edge of a clip a trim moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrimEdge {
    /// The beginning (the end stays in place).
    Start,
    /// The end (the beginning stays in place).
    End,
}

/// Moves a clip to `start` on `track_id`, or on a new track of its kind added on
/// top when `track_id` is `None`.
///
/// # Errors
///
/// When the clip or track does not exist, or the clip would overlap another one.
pub fn move_clip(
    session: &mut EditSession,
    clip_id: ClipId,
    track_id: Option<TrackId>,
    start: Timestamp,
) -> Result<EditOutcome, EditError> {
    session.transaction("moveClip", |transaction| {
        let destination = match track_id {
            Some(track_id) => track_id,
            None => {
                let kind = clip_track_kind(transaction.project(), clip_id)?;
                transaction
                    .execute(AddTrack::new(kind, TrackPosition::Top))?
                    .first_track_id()
                    .ok_or_else(missing_created_object)?
            }
        };
        transaction.execute(MoveClip::to_track(clip_id, destination, start))?;
        Ok(EditOutcome::default())
    })
}

/// Moves one edge of a clip to `time` on the timeline.
///
/// # Errors
///
/// When the clip does not exist, would go past its file or overlap another clip.
pub fn trim_clip(
    session: &mut EditSession,
    clip_id: ClipId,
    edge: TrimEdge,
    time: Timestamp,
) -> Result<EditOutcome, EditError> {
    let edge = match edge {
        TrimEdge::Start => ClipEdge::Start,
        TrimEdge::End => ClipEdge::End,
    };
    session.transaction("trimClip", |transaction| {
        transaction.execute(TrimClip::new(clip_id, edge, time))?;
        Ok(EditOutcome::default())
    })
}

/// Cuts clips in two at `time`: the given clips, or every clip under `time` when
/// the list is empty. Clips that do not contain `time` strictly inside are skipped.
/// Returns the new second halves.
///
/// # Errors
///
/// When a cut fails; nothing is cut then.
pub fn split_clips(
    session: &mut EditSession,
    clip_ids: &[ClipId],
    time: Timestamp,
) -> Result<EditOutcome, EditError> {
    let clips_to_split: Vec<ClipId> = session
        .project()
        .timeline()
        .all_clips()
        .filter(|(_, clip)| clip_ids.is_empty() || clip_ids.contains(&clip.id))
        .filter(|(_, clip)| clip.timeline_start < time && time < clip.timeline_end())
        .map(|(_, clip)| clip.id)
        .collect();
    session.transaction("splitClip", |transaction| {
        let mut created_clip_ids = Vec::new();
        for clip_id in clips_to_split {
            if let Some(second_half) = transaction
                .execute(SplitClip::new(clip_id, time))?
                .first_clip_id()
            {
                created_clip_ids.push(second_half);
            }
        }
        Ok(EditOutcome {
            created_clip_ids,
            notices: Vec::new(),
        })
    })
}

/// Deletes clips (the gaps they leave stay).
///
/// # Errors
///
/// When a clip does not exist; nothing is deleted then.
pub fn remove_clips(
    session: &mut EditSession,
    clip_ids: &[ClipId],
) -> Result<EditOutcome, EditError> {
    session.transaction("removeClips", |transaction| {
        for clip_id in clip_ids {
            transaction.execute(RemoveClip::new(*clip_id))?;
        }
        Ok(EditOutcome::default())
    })
}

/// Turns a track's sound off or on.
///
/// # Errors
///
/// When the track does not exist.
pub fn set_track_muted(
    session: &mut EditSession,
    track_id: TrackId,
    is_muted: bool,
) -> Result<EditOutcome, EditError> {
    let label = if is_muted { "muteTrack" } else { "unmuteTrack" };
    session.transaction(label, |transaction| {
        transaction.execute(SetTrackMuted::new(track_id, is_muted))?;
        Ok(EditOutcome::default())
    })
}

/// Hides or shows a track's pictures.
///
/// # Errors
///
/// When the track does not exist.
pub fn set_track_hidden(
    session: &mut EditSession,
    track_id: TrackId,
    is_hidden: bool,
) -> Result<EditOutcome, EditError> {
    let label = if is_hidden { "hideTrack" } else { "showTrack" };
    session.transaction(label, |transaction| {
        transaction.execute(SetTrackHidden::new(track_id, is_hidden))?;
        Ok(EditOutcome::default())
    })
}

/// Removes a track (the engine refuses a track that still holds clips).
///
/// # Errors
///
/// When the track does not exist or is not empty.
pub fn remove_track(session: &mut EditSession, track_id: TrackId) -> Result<EditOutcome, EditError> {
    session.transaction("removeTrack", |transaction| {
        transaction.execute(RemoveTrack::new(track_id))?;
        Ok(EditOutcome::default())
    })
}
