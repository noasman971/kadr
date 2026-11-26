//! Commands that animate clip properties with keyframes.

use super::{ClipChangeRecord, CreatedObject, EditCommand};
use crate::animation::Keyframe;
use crate::clip::AnimatableProperty;
use crate::error::EditError;
use crate::ids::ClipId;
use crate::project::Project;
use crate::time::TimeSpan;

/// Adds a keyframe to a clip property, or replaces the one at the same moment.
#[derive(Debug)]
pub struct SetKeyframe {
    clip_id: ClipId,
    property: AnimatableProperty,
    keyframe: Keyframe,
    change_record: ClipChangeRecord,
}

impl SetKeyframe {
    /// Prepares the keyframe. Its time is measured from the start of the clip.
    pub fn new(clip_id: ClipId, property: AnimatableProperty, keyframe: Keyframe) -> Self {
        Self {
            clip_id,
            property,
            keyframe,
            change_record: ClipChangeRecord::default(),
        }
    }
}

impl EditCommand for SetKeyframe {
    fn label(&self) -> String {
        format!(
            "Set a {} keyframe on clip {} at {}",
            self.property, self.clip_id, self.keyframe.time_offset
        )
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let (property, keyframe) = (self.property, self.keyframe);
        self.change_record
            .replace_clip(project, self.clip_id, |clip| {
                let mut new_version = clip.clone();
                new_version
                    .animated_property_mut(property)
                    .set_keyframe(keyframe);
                Ok(new_version)
            })?;
        Ok(Vec::new())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let label = self.label();
        self.change_record.restore_previous_version(project, &label)
    }
}

/// Removes the keyframe of a clip property at an exact moment.
#[derive(Debug)]
pub struct RemoveKeyframe {
    clip_id: ClipId,
    property: AnimatableProperty,
    time_offset: TimeSpan,
    change_record: ClipChangeRecord,
}

impl RemoveKeyframe {
    /// Prepares the removal. `time_offset` is measured from the start of the clip.
    pub fn new(clip_id: ClipId, property: AnimatableProperty, time_offset: TimeSpan) -> Self {
        Self {
            clip_id,
            property,
            time_offset,
            change_record: ClipChangeRecord::default(),
        }
    }
}

impl EditCommand for RemoveKeyframe {
    fn label(&self) -> String {
        format!(
            "Remove the {} keyframe of clip {} at {}",
            self.property, self.clip_id, self.time_offset
        )
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let (property, time_offset) = (self.property, self.time_offset);
        self.change_record
            .replace_clip(project, self.clip_id, |clip| {
                let mut new_version = clip.clone();
                new_version
                    .animated_property_mut(property)
                    .remove_keyframe_at(time_offset)
                    .ok_or(EditError::KeyframeNotFound {
                        clip_id: clip.id,
                        property,
                        time_offset,
                    })?;
                Ok(new_version)
            })?;
        Ok(Vec::new())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let label = self.label();
        self.change_record.restore_previous_version(project, &label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Interpolation;
    use crate::commands::test_support::{apply_and_check_revert, apply_expecting_error};
    use crate::commands::{AddClip, AddTrack, CreatedObjectsExt, TrackPosition};
    use crate::project::ProjectSettings;
    use crate::time::Timestamp;
    use crate::timeline::TrackKind;

    fn project_with_text_clip() -> (Project, ClipId) {
        let project = Project::new(ProjectSettings::landscape_1080p());
        let (project, created) = apply_and_check_revert(
            &project,
            AddTrack::new(TrackKind::Video, TrackPosition::Top),
        );
        let track_id = created.first_track_id().unwrap();
        let (project, created) = apply_and_check_revert(
            &project,
            AddClip::text(
                track_id,
                "Fade",
                TimeSpan::from_whole_seconds(4),
                Timestamp::ZERO,
            ),
        );
        (project, created.first_clip_id().unwrap())
    }

    fn opacity_keyframe(at_second: u64, value: f32) -> Keyframe {
        Keyframe {
            time_offset: TimeSpan::from_whole_seconds(at_second),
            value,
            interpolation_to_next: Interpolation::Linear,
        }
    }

    #[test]
    fn setting_and_removing_keyframes_are_reversible() {
        let (project, clip_id) = project_with_text_clip();
        let (project, _) = apply_and_check_revert(
            &project,
            SetKeyframe::new(
                clip_id,
                AnimatableProperty::Opacity,
                opacity_keyframe(0, 0.0),
            ),
        );
        let (project, _) = apply_and_check_revert(
            &project,
            SetKeyframe::new(
                clip_id,
                AnimatableProperty::Opacity,
                opacity_keyframe(2, 1.0),
            ),
        );
        let opacity = &project.timeline().find_clip(clip_id).unwrap().1.opacity;
        assert_eq!(opacity.value_at(TimeSpan::from_whole_seconds(1)), 0.5);

        let (project, _) = apply_and_check_revert(
            &project,
            RemoveKeyframe::new(clip_id, AnimatableProperty::Opacity, TimeSpan::ZERO),
        );
        let opacity = &project.timeline().find_clip(clip_id).unwrap().1.opacity;
        assert_eq!(opacity.keyframes().len(), 1);
    }

    #[test]
    fn removing_a_missing_keyframe_fails() {
        let (project, clip_id) = project_with_text_clip();
        let error = apply_expecting_error(
            &project,
            RemoveKeyframe::new(
                clip_id,
                AnimatableProperty::Scale,
                TimeSpan::from_whole_seconds(1),
            ),
        );
        assert!(matches!(error, EditError::KeyframeNotFound { .. }));
    }
}
