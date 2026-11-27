//! Errors raised when a change to a project is refused.
//!
//! Messages are written for people **and** for Claude: they say what went wrong
//! and, when something is missing, what exists instead
//! ("clip 12 does not exist; existing clips: 3, 5, 8").

use std::fmt;

use crate::clip::AnimatableProperty;
use crate::ids::{ClipId, EffectId, MediaAssetId, TrackId};
use crate::time::{TimeRange, TimeSpan, Timestamp};
use crate::validation::ValidationError;

/// Writes a list of identifiers as `3, 5, 8`, or `none` when empty.
pub(crate) struct IdentifierList<'a, Identifier>(pub(crate) &'a [Identifier]);

impl<Identifier: fmt::Display> fmt::Display for IdentifierList<'_, Identifier> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return formatter.write_str("none");
        }
        for (index, identifier) in self.0.iter().enumerate() {
            if index > 0 {
                formatter.write_str(", ")?;
            }
            write!(formatter, "{identifier}")?;
        }
        Ok(())
    }
}

/// Why a change to a project was refused. The project is left unchanged.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EditError {
    /// No media file has this identifier.
    #[error(
        "media {media_asset_id} does not exist; existing media: {}",
        IdentifierList(existing_media_asset_ids)
    )]
    MediaAssetNotFound {
        /// The identifier asked for.
        media_asset_id: MediaAssetId,
        /// The identifiers that do exist.
        existing_media_asset_ids: Vec<MediaAssetId>,
    },

    /// No track has this identifier.
    #[error(
        "track {track_id} does not exist; existing tracks: {}",
        IdentifierList(existing_track_ids)
    )]
    TrackNotFound {
        /// The identifier asked for.
        track_id: TrackId,
        /// The identifiers that do exist.
        existing_track_ids: Vec<TrackId>,
    },

    /// No clip has this identifier.
    #[error(
        "clip {clip_id} does not exist; existing clips: {}",
        IdentifierList(existing_clip_ids)
    )]
    ClipNotFound {
        /// The identifier asked for.
        clip_id: ClipId,
        /// The identifiers that do exist.
        existing_clip_ids: Vec<ClipId>,
    },

    /// The clip has no effect with this identifier.
    #[error(
        "clip {clip_id} has no effect {effect_id}; its effects: {}",
        IdentifierList(existing_effect_ids)
    )]
    EffectNotFound {
        /// The clip searched.
        clip_id: ClipId,
        /// The identifier asked for.
        effect_id: EffectId,
        /// The effects the clip does have.
        existing_effect_ids: Vec<EffectId>,
    },

    /// The property has no keyframe at this moment.
    #[error(
        "the {property} of clip {clip_id} has no keyframe at {time_offset} from the clip start"
    )]
    KeyframeNotFound {
        /// The clip searched.
        clip_id: ClipId,
        /// The property searched.
        property: AnimatableProperty,
        /// The moment asked for, from the clip start.
        time_offset: TimeSpan,
    },

    /// A keyframe would be placed outside its clip.
    #[error(
        "a keyframe at {time_offset} from the start of clip {clip_id} would be outside the clip, which lasts {clip_duration}"
    )]
    KeyframeOutsideClip {
        /// The clip.
        clip_id: ClipId,
        /// The moment asked for, from the clip start.
        time_offset: TimeSpan,
        /// How long the clip lasts.
        clip_duration: TimeSpan,
    },

    /// A keyframe cannot move onto a moment that already has one.
    #[error(
        "the {property} of clip {clip_id} already has a keyframe at {time_offset} from the clip start; remove it first, or move this one elsewhere"
    )]
    KeyframeAlreadyThere {
        /// The clip.
        clip_id: ClipId,
        /// The property.
        property: AnimatableProperty,
        /// The moment already taken, from the clip start.
        time_offset: TimeSpan,
    },

    /// A media file cannot be removed while clips still show it.
    #[error(
        "media {media_asset_id} is still used by clips {}; remove those clips first",
        IdentifierList(used_by_clip_ids)
    )]
    MediaAssetInUse {
        /// The media file.
        media_asset_id: MediaAssetId,
        /// The clips showing it.
        used_by_clip_ids: Vec<ClipId>,
    },

    /// A time that must be strictly inside a clip is not.
    #[error(
        "{requested_time} is not strictly inside clip {clip_id}, which runs from {clip_start} to {clip_end}"
    )]
    TimeOutsideClip {
        /// The clip.
        clip_id: ClipId,
        /// The rejected time on the timeline.
        requested_time: Timestamp,
        /// Where the clip starts.
        clip_start: Timestamp,
        /// Where the clip ends.
        clip_end: Timestamp,
    },

    /// Trimming would leave nothing of the clip.
    #[error(
        "moving an edge of clip {clip_id} to {requested_time} would leave the clip empty; to delete it, remove it instead"
    )]
    TrimWouldEmptyClip {
        /// The clip.
        clip_id: ClipId,
        /// The rejected edge position.
        requested_time: Timestamp,
    },

    /// A part to remove from a clip is not inside the clip.
    #[error(
        "the part {part} cannot be removed from clip {clip_id}, which runs from {clip_start} to {clip_end}: parts to remove must lie inside the clip"
    )]
    PartOutsideClip {
        /// The clip.
        clip_id: ClipId,
        /// The rejected part, on the timeline.
        part: TimeRange,
        /// Where the clip starts.
        clip_start: Timestamp,
        /// Where the clip ends.
        clip_end: Timestamp,
    },

    /// No part of a clip was given to remove.
    #[error("no part of clip {clip_id} was given to remove")]
    NoPartToRemove {
        /// The clip.
        clip_id: ClipId,
    },

    /// Removing parts would leave nothing of the clip.
    #[error(
        "removing these parts would leave nothing of clip {clip_id}; to delete it, remove it instead"
    )]
    RemovalWouldEmptyClip {
        /// The clip.
        clip_id: ClipId,
    },

    /// Extending a clip's start would go before the beginning of its source file.
    #[error(
        "clip {clip_id} cannot start earlier: it already starts at the beginning of its media file"
    )]
    ExtendsBeforeSourceStart {
        /// The clip.
        clip_id: ClipId,
    },

    /// A constant value was set on a property animated by keyframes.
    #[error(
        "the {property} of clip {clip_id} is animated with keyframes; set a keyframe instead, or remove the keyframes first"
    )]
    PropertyIsAnimated {
        /// The clip.
        clip_id: ClipId,
        /// The animated property.
        property: AnimatableProperty,
    },

    /// The operation only applies to clips showing a media file.
    #[error(
        "clip {clip_id} shows no media file (it is a text or a subtitle cue); this only applies to clips showing a video, a sound or an image"
    )]
    NotAMediaClip {
        /// The clip.
        clip_id: ClipId,
    },

    /// The operation only applies to text clips.
    #[error("clip {clip_id} is not a text clip")]
    NotATextClip {
        /// The clip.
        clip_id: ClipId,
    },

    /// The operation only applies to subtitle cues.
    #[error("clip {clip_id} is not a subtitle cue")]
    NotASubtitleCue {
        /// The clip.
        clip_id: ClipId,
    },

    /// The operation only applies to subtitle tracks.
    #[error(
        "track {track_id} is not a subtitle track; subtitle tracks: {}",
        IdentifierList(subtitle_track_ids)
    )]
    NotASubtitleTrack {
        /// The track asked for.
        track_id: TrackId,
        /// The subtitle tracks that exist.
        subtitle_track_ids: Vec<TrackId>,
    },

    /// A subtitle cue would end at or before its start.
    #[error(
        "a subtitle cue must end after it starts, but this one would start at {start} and end at {end}"
    )]
    CueEndsBeforeItStarts {
        /// The requested start.
        start: Timestamp,
        /// The requested end.
        end: Timestamp,
    },

    /// Two cues to merge are not neighbours on one subtitle track.
    #[error("subtitle cues {first_clip_id} and {second_clip_id} cannot be merged: {reason}")]
    CuesCannotBeMerged {
        /// The first cue asked for.
        first_clip_id: ClipId,
        /// The second cue asked for.
        second_clip_id: ClipId,
        /// Why not.
        reason: String,
    },

    /// Parts cannot be cut out of a subtitle cue: subtitle tracks follow cuts
    /// made in the clips they describe.
    #[error(
        "clip {clip_id} is a subtitle cue: parts are cut out of videos and sounds, and the subtitle tracks follow (trim or split the cue to change it)"
    )]
    PartsOfSubtitleCue {
        /// The cue.
        clip_id: ClipId,
    },

    /// The change was applied, but the resulting project would be invalid, so it
    /// was undone.
    #[error("the change was refused: {0}")]
    WouldMakeProjectInvalid(#[from] ValidationError),

    /// A step of a multi-step change failed; every earlier step was undone.
    #[error("step {step_number} ({step_label}) failed, so the whole change was cancelled: {cause}")]
    StepFailed {
        /// Position of the failed step, starting at 1.
        step_number: usize,
        /// Description of the failed step.
        step_label: String,
        /// Why it failed.
        cause: Box<EditError>,
    },

    /// There is nothing to undo.
    #[error("there is nothing to undo")]
    NothingToUndo,

    /// There is nothing to redo.
    #[error("there is nothing to redo")]
    NothingToRedo,

    /// An internal inconsistency: a bug in Kadr, never the user's fault.
    #[error("internal error in Kadr, please report it: {details}")]
    InternalInconsistency {
        /// What was inconsistent.
        details: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_objects_list_what_exists() {
        let error = EditError::ClipNotFound {
            clip_id: ClipId::from_raw(12),
            existing_clip_ids: vec![
                ClipId::from_raw(3),
                ClipId::from_raw(5),
                ClipId::from_raw(8),
            ],
        };
        assert_eq!(
            error.to_string(),
            "clip 12 does not exist; existing clips: 3, 5, 8"
        );

        let empty_project_error = EditError::TrackNotFound {
            track_id: TrackId::from_raw(1),
            existing_track_ids: Vec::new(),
        };
        assert_eq!(
            empty_project_error.to_string(),
            "track 1 does not exist; existing tracks: none"
        );
    }
}
