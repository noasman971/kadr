//! Removing parts of a clip and closing the gaps they leave ("ripple delete").

use super::subtitle_commands::{replace_track_clips, subtitle_cues_without_time};
use super::{CreatedObject, EditCommand, IdAllocatorRecord, object_vanished, revert_without_apply};
use crate::clip::{Clip, ClipContent};
use crate::error::EditError;
use crate::ids::{ClipId, TrackId};
use crate::project::Project;
use crate::time::{TimeRange, TimeSpan, Timestamp};

/// Cuts parts out of a clip and closes the gaps: what is left of the clip is
/// placed end to end from its start, and the later clips of its track move
/// earlier by the time removed. Clips on other tracks do not move, except
/// subtitle cues: the same moments are removed from every subtitle track, so
/// the words stay under the voice that says them (decision record 0056).
///
/// This is how silences are removed: one command, so one undo step, however
/// many parts. [`Clip::without_parts`] says what each part left keeps.
/// Subtitle cues themselves cannot lose parts (trim or split them instead).
#[derive(Debug)]
pub struct RemoveClipParts {
    clip_id: ClipId,
    removed_parts: Vec<TimeRange>,
    previous_clips_of_track: Option<(TrackId, Vec<Clip>)>,
    previous_subtitle_cues: Vec<(TrackId, Vec<Clip>)>,
    id_allocator_record: IdAllocatorRecord,
}

impl RemoveClipParts {
    /// Prepares removing `removed_parts` (timeline times inside the clip, in
    /// any order, possibly overlapping) from the clip.
    pub fn new(clip_id: ClipId, removed_parts: Vec<TimeRange>) -> Self {
        Self {
            clip_id,
            removed_parts,
            previous_clips_of_track: None,
            previous_subtitle_cues: Vec::new(),
            id_allocator_record: IdAllocatorRecord::default(),
        }
    }
}

impl EditCommand for RemoveClipParts {
    fn label(&self) -> String {
        format!(
            "Remove {} part(s) of clip {} and close the gaps",
            self.removed_parts.len(),
            self.clip_id
        )
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let (track, clip) = project.require_clip(self.clip_id)?;
        if matches!(clip.content, ClipContent::Subtitle(_)) {
            return Err(EditError::PartsOfSubtitleCue {
                clip_id: self.clip_id,
            });
        }
        let track_id = track.id;
        let mut id_allocator_after_cuts = project.id_allocator();
        let pieces = clip.without_parts(&self.removed_parts, &mut id_allocator_after_cuts)?;
        let kept_duration = pieces
            .iter()
            .map(Clip::timeline_duration)
            .fold(TimeSpan::ZERO, |total, duration| total + duration);
        let removed_duration = clip
            .timeline_duration()
            .checked_sub(kept_duration)
            .unwrap_or(TimeSpan::ZERO);
        let clip_end = clip.timeline_end();
        let created_objects: Vec<CreatedObject> = pieces
            .iter()
            .skip(1)
            .map(|piece| CreatedObject::Clip(piece.id))
            .collect();

        let mut new_clips_of_track: Vec<Clip> = track
            .clips()
            .iter()
            .filter(|other| other.id != self.clip_id)
            .map(|other| {
                let mut placed = other.clone();
                if other.timeline_start >= clip_end {
                    // A later clip starts after the whole removed duration.
                    placed.timeline_start = other
                        .timeline_start
                        .checked_sub(removed_duration)
                        .unwrap_or(Timestamp::ZERO);
                }
                placed
            })
            .collect();
        new_clips_of_track.extend(pieces);
        let new_subtitle_cues = subtitle_cues_without_time(project, &self.removed_parts);

        self.id_allocator_record.remember(project);
        *project.id_allocator_mut() = id_allocator_after_cuts;
        let track = project
            .timeline_mut()
            .track_mut(track_id)
            .ok_or_else(|| object_vanished(format!("track {track_id}")))?;
        let previous_clips = track.replace_all_clips(new_clips_of_track);
        self.previous_clips_of_track = Some((track_id, previous_clips));
        self.previous_subtitle_cues = replace_track_clips(project, new_subtitle_cues)?;
        Ok(created_objects)
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let label = self.label();
        let (track_id, previous_clips) = self
            .previous_clips_of_track
            .take()
            .ok_or_else(|| revert_without_apply(&label))?;
        project
            .timeline_mut()
            .track_mut(track_id)
            .ok_or_else(|| object_vanished(format!("track {track_id}")))?
            .replace_all_clips(previous_clips);
        replace_track_clips(project, std::mem::take(&mut self.previous_subtitle_cues))?;
        self.id_allocator_record.restore(project, &label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::{apply_and_check_revert, apply_expecting_error};
    use crate::commands::{AddClip, AddTrack, CreatedObjectsExt, ImportMedia, TrackPosition};
    use crate::media_asset::test_samples::ten_second_video;
    use crate::project::ProjectSettings;
    use crate::timeline::TrackKind;

    fn seconds(value: u64) -> Timestamp {
        Timestamp::from_whole_seconds(value)
    }

    fn range(start_second: u64, end_second: u64) -> TimeRange {
        TimeRange::new(seconds(start_second), seconds(end_second)).unwrap()
    }

    /// A project with, on one video track, seconds 0–6 of a video at 0 s and a
    /// title from 10 s to 14 s; on a second track, a title from 2 s to 4 s.
    struct Fixture {
        project: Project,
        video_clip_id: ClipId,
        later_title_id: ClipId,
        other_track_title_id: ClipId,
    }

    fn fixture() -> Fixture {
        let project = Project::new(ProjectSettings::landscape_1080p());
        let (project, created) =
            apply_and_check_revert(&project, ImportMedia::new(ten_second_video()));
        let media_asset_id = created.first_media_asset_id().unwrap();
        let add_track = || AddTrack::new(TrackKind::Video, TrackPosition::Top);
        let (project, created) = apply_and_check_revert(&project, add_track());
        let track_id = created.first_track_id().unwrap();
        let (project, created) = apply_and_check_revert(&project, add_track());
        let other_track_id = created.first_track_id().unwrap();
        let (project, created) = apply_and_check_revert(
            &project,
            AddClip::media(track_id, media_asset_id, seconds(0), seconds(6), seconds(0)),
        );
        let video_clip_id = created.first_clip_id().unwrap();
        let title = |track_id, start_second| {
            AddClip::text(
                track_id,
                "Title",
                TimeSpan::from_whole_seconds(2),
                seconds(start_second),
            )
        };
        let (project, created) = apply_and_check_revert(&project, title(track_id, 10));
        let later_title_id = created.first_clip_id().unwrap();
        let (project, created) = apply_and_check_revert(&project, title(other_track_id, 2));
        let other_track_title_id = created.first_clip_id().unwrap();
        Fixture {
            project,
            video_clip_id,
            later_title_id,
            other_track_title_id,
        }
    }

    fn start_of(project: &Project, clip_id: ClipId) -> Timestamp {
        project
            .timeline()
            .find_clip(clip_id)
            .unwrap()
            .1
            .timeline_start
    }

    #[test]
    fn removed_parts_close_up_and_later_clips_of_the_track_follow() {
        let fixture = fixture();

        let (project, created) = apply_and_check_revert(
            &fixture.project,
            RemoveClipParts::new(fixture.video_clip_id, vec![range(4, 5), range(1, 2)]),
        );

        let (track, _) = project.timeline().find_clip(fixture.video_clip_id).unwrap();
        let spans: Vec<(f64, f64)> = track
            .clips()
            .iter()
            .map(|clip| {
                (
                    clip.timeline_start.as_seconds(),
                    clip.timeline_end().as_seconds(),
                )
            })
            .collect();
        assert_eq!(
            spans,
            vec![(0.0, 1.0), (1.0, 3.0), (3.0, 4.0), (8.0, 10.0)],
            "three pieces end to end, then the title 2 s earlier"
        );
        assert_eq!(created.len(), 2);
        assert_eq!(start_of(&project, fixture.later_title_id), seconds(8));
        assert_eq!(
            start_of(&project, fixture.other_track_title_id),
            seconds(2),
            "other tracks do not move"
        );
    }

    #[test]
    fn parts_outside_the_clip_or_covering_it_are_refused_without_change() {
        let fixture = fixture();
        assert!(matches!(
            apply_expecting_error(
                &fixture.project,
                RemoveClipParts::new(fixture.video_clip_id, vec![range(5, 8)])
            ),
            EditError::PartOutsideClip { .. }
        ));
        assert!(matches!(
            apply_expecting_error(
                &fixture.project,
                RemoveClipParts::new(fixture.video_clip_id, vec![range(0, 6)])
            ),
            EditError::RemovalWouldEmptyClip { .. }
        ));
        assert!(matches!(
            apply_expecting_error(
                &fixture.project,
                RemoveClipParts::new(ClipId::from_raw(99), vec![range(0, 1)])
            ),
            EditError::ClipNotFound { .. }
        ));
    }
}
