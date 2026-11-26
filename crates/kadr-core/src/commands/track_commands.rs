//! Commands that add, remove, mute or hide tracks.

use super::{CreatedObject, EditCommand, IdAllocatorRecord, object_vanished, revert_without_apply};
use crate::error::EditError;
use crate::ids::TrackId;
use crate::project::Project;
use crate::timeline::{Track, TrackKind};

/// Where a new track goes in the stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackPosition {
    /// Above every existing track (drawn on top).
    Top,
    /// Below every existing track (drawn underneath).
    Bottom,
    /// At this index, 0 being the bottom. Larger indexes go to the top.
    AtIndex(usize),
}

/// Adds an empty track.
#[derive(Debug)]
pub struct AddTrack {
    kind: TrackKind,
    requested_name: Option<String>,
    position: TrackPosition,
    id_allocator_record: IdAllocatorRecord,
    created_track_id: Option<TrackId>,
}

impl AddTrack {
    /// Prepares a new track. Without a name, it is called "Video 2", "Audio 1"…
    pub fn new(kind: TrackKind, position: TrackPosition) -> Self {
        Self {
            kind,
            requested_name: None,
            position,
            id_allocator_record: IdAllocatorRecord::default(),
            created_track_id: None,
        }
    }

    /// Gives the new track a name.
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.requested_name = Some(name.into());
        self
    }

    /// "Video 3" when there are already two video tracks.
    fn default_name(&self, project: &Project) -> String {
        let existing_tracks_of_same_kind = project
            .timeline()
            .tracks()
            .iter()
            .filter(|track| track.kind == self.kind)
            .count();
        let kind_name = match self.kind {
            TrackKind::Video => "Video",
            TrackKind::Audio => "Audio",
        };
        format!("{kind_name} {}", existing_tracks_of_same_kind + 1)
    }
}

impl EditCommand for AddTrack {
    fn label(&self) -> String {
        format!("Add {} track", self.kind.display_name())
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let name = self
            .requested_name
            .clone()
            .unwrap_or_else(|| self.default_name(project));
        let index = match self.position {
            TrackPosition::Top => usize::MAX,
            TrackPosition::Bottom => 0,
            TrackPosition::AtIndex(index) => index,
        };
        self.id_allocator_record.remember(project);
        let track_id = TrackId::from_raw(project.id_allocator_mut().allocate_raw_id());
        project
            .timeline_mut()
            .insert_track(index, Track::new(track_id, self.kind, name));
        self.created_track_id = Some(track_id);
        Ok(vec![CreatedObject::Track(track_id)])
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let label = self.label();
        let track_id = self
            .created_track_id
            .take()
            .ok_or_else(|| revert_without_apply(&label))?;
        project
            .timeline_mut()
            .remove_track(track_id)
            .ok_or_else(|| object_vanished(format!("track {track_id}")))?;
        self.id_allocator_record.restore(project, &label)
    }
}

/// Removes a track **with all its clips**.
#[derive(Debug)]
pub struct RemoveTrack {
    track_id: TrackId,
    removed_track_and_index: Option<(usize, Track)>,
}

impl RemoveTrack {
    /// Prepares the removal.
    pub fn new(track_id: TrackId) -> Self {
        Self {
            track_id,
            removed_track_and_index: None,
        }
    }
}

impl EditCommand for RemoveTrack {
    fn label(&self) -> String {
        format!("Remove track {}", self.track_id)
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        project.require_track(self.track_id)?;
        let removed_track_and_index = project
            .timeline_mut()
            .remove_track(self.track_id)
            .ok_or_else(|| object_vanished(format!("track {}", self.track_id)))?;
        self.removed_track_and_index = Some(removed_track_and_index);
        Ok(Vec::new())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let (index, removed_track) = self
            .removed_track_and_index
            .take()
            .ok_or_else(|| revert_without_apply(&self.label()))?;
        project.timeline_mut().insert_track(index, removed_track);
        Ok(())
    }
}

/// Changes one on/off setting of a track and remembers the previous value.
fn set_track_flag(
    project: &mut Project,
    track_id: TrackId,
    new_value: bool,
    select_flag: fn(&mut Track) -> &mut bool,
) -> Result<bool, EditError> {
    project.require_track(track_id)?;
    let track = project
        .timeline_mut()
        .track_mut(track_id)
        .ok_or_else(|| object_vanished(format!("track {track_id}")))?;
    let flag = select_flag(track);
    let previous_value = *flag;
    *flag = new_value;
    Ok(previous_value)
}

/// Mutes or unmutes a track.
#[derive(Debug)]
pub struct SetTrackMuted {
    track_id: TrackId,
    is_muted: bool,
    previous_value: Option<bool>,
}

impl SetTrackMuted {
    /// Prepares the change.
    pub fn new(track_id: TrackId, is_muted: bool) -> Self {
        Self {
            track_id,
            is_muted,
            previous_value: None,
        }
    }
}

impl EditCommand for SetTrackMuted {
    fn label(&self) -> String {
        let action = if self.is_muted { "Mute" } else { "Unmute" };
        format!("{action} track {}", self.track_id)
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let previous_value = set_track_flag(project, self.track_id, self.is_muted, |track| {
            &mut track.is_muted
        })?;
        self.previous_value = Some(previous_value);
        Ok(Vec::new())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let previous_value = self
            .previous_value
            .take()
            .ok_or_else(|| revert_without_apply(&self.label()))?;
        set_track_flag(project, self.track_id, previous_value, |track| {
            &mut track.is_muted
        })?;
        Ok(())
    }
}

/// Hides or shows a track.
#[derive(Debug)]
pub struct SetTrackHidden {
    track_id: TrackId,
    is_hidden: bool,
    previous_value: Option<bool>,
}

impl SetTrackHidden {
    /// Prepares the change.
    pub fn new(track_id: TrackId, is_hidden: bool) -> Self {
        Self {
            track_id,
            is_hidden,
            previous_value: None,
        }
    }
}

impl EditCommand for SetTrackHidden {
    fn label(&self) -> String {
        let action = if self.is_hidden { "Hide" } else { "Show" };
        format!("{action} track {}", self.track_id)
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let previous_value = set_track_flag(project, self.track_id, self.is_hidden, |track| {
            &mut track.is_hidden
        })?;
        self.previous_value = Some(previous_value);
        Ok(Vec::new())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let previous_value = self
            .previous_value
            .take()
            .ok_or_else(|| revert_without_apply(&self.label()))?;
        set_track_flag(project, self.track_id, previous_value, |track| {
            &mut track.is_hidden
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CreatedObjectsExt;
    use crate::commands::test_support::{apply_and_check_revert, apply_expecting_error};
    use crate::project::ProjectSettings;

    fn empty_project() -> Project {
        Project::new(ProjectSettings::landscape_1080p())
    }

    #[test]
    fn tracks_get_default_names_and_requested_positions() {
        let project = empty_project();
        let (project, first) = apply_and_check_revert(
            &project,
            AddTrack::new(TrackKind::Video, TrackPosition::Top),
        );
        let (project, second) = apply_and_check_revert(
            &project,
            AddTrack::new(TrackKind::Video, TrackPosition::Bottom),
        );
        let (project, _) = apply_and_check_revert(
            &project,
            AddTrack::new(TrackKind::Audio, TrackPosition::AtIndex(1)).named("Music"),
        );

        let names: Vec<&str> = project
            .timeline()
            .tracks()
            .iter()
            .map(|track| track.name.as_str())
            .collect();
        assert_eq!(names, vec!["Video 2", "Music", "Video 1"]);
        assert_eq!(
            project.timeline().tracks()[0].id,
            second.first_track_id().unwrap()
        );
        assert_eq!(
            project.timeline().tracks()[2].id,
            first.first_track_id().unwrap()
        );
    }

    #[test]
    fn removing_a_track_is_reversible() {
        let (project, created) = apply_and_check_revert(
            &empty_project(),
            AddTrack::new(TrackKind::Video, TrackPosition::Top),
        );
        let track_id = created.first_track_id().unwrap();

        let (project_without_track, _) =
            apply_and_check_revert(&project, RemoveTrack::new(track_id));
        assert!(project_without_track.timeline().tracks().is_empty());
    }

    #[test]
    fn muting_and_hiding_are_reversible() {
        let (project, created) = apply_and_check_revert(
            &empty_project(),
            AddTrack::new(TrackKind::Video, TrackPosition::Top),
        );
        let track_id = created.first_track_id().unwrap();

        let (muted_project, _) =
            apply_and_check_revert(&project, SetTrackMuted::new(track_id, true));
        assert!(muted_project.timeline().track(track_id).unwrap().is_muted);
        let (hidden_project, _) =
            apply_and_check_revert(&project, SetTrackHidden::new(track_id, true));
        assert!(hidden_project.timeline().track(track_id).unwrap().is_hidden);
    }

    #[test]
    fn unknown_track_is_reported() {
        let error = apply_expecting_error(
            &empty_project(),
            SetTrackMuted::new(TrackId::from_raw(9), true),
        );
        assert!(matches!(error, EditError::TrackNotFound { .. }));
    }
}
