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
///
/// Once the last keyframe is gone, the property takes its constant value again:
/// the one it had before it was animated, unless
/// [`RemoveKeyframe::keeping_the_value_when_last`] was asked.
#[derive(Debug)]
pub struct RemoveKeyframe {
    clip_id: ClipId,
    property: AnimatableProperty,
    time_offset: TimeSpan,
    keeps_the_value_when_last: bool,
    change_record: ClipChangeRecord,
}

impl RemoveKeyframe {
    /// Prepares the removal. `time_offset` is measured from the start of the clip.
    pub fn new(clip_id: ClipId, property: AnimatableProperty, time_offset: TimeSpan) -> Self {
        Self {
            clip_id,
            property,
            time_offset,
            keeps_the_value_when_last: false,
            change_record: ClipChangeRecord::default(),
        }
    }

    /// When this removes the property's last keyframe, the property keeps that
    /// keyframe's value instead of going back to its value from before the
    /// animation: what the person sees does not jump.
    pub fn keeping_the_value_when_last(mut self) -> Self {
        self.keeps_the_value_when_last = true;
        self
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
        let keeps_the_value_when_last = self.keeps_the_value_when_last;
        self.change_record
            .replace_clip(project, self.clip_id, |clip| {
                let mut new_version = clip.clone();
                let animation = new_version.animated_property_mut(property);
                let removed_keyframe = animation.remove_keyframe_at(time_offset).ok_or(
                    EditError::KeyframeNotFound {
                        clip_id: clip.id,
                        property,
                        time_offset,
                    },
                )?;
                if keeps_the_value_when_last && !animation.is_animated() {
                    animation.constant_value = removed_keyframe.value;
                }
                Ok(new_version)
            })?;
        Ok(Vec::new())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let label = self.label();
        self.change_record.restore_previous_version(project, &label)
    }
}

/// Moves the keyframe of a clip property to another moment, keeping its value
/// and the curve leading to the next keyframe. It may pass other keyframes, but
/// not land on one.
#[derive(Debug)]
pub struct MoveKeyframe {
    clip_id: ClipId,
    property: AnimatableProperty,
    from_offset: TimeSpan,
    to_offset: TimeSpan,
    change_record: ClipChangeRecord,
}

impl MoveKeyframe {
    /// Prepares the move. Both times are measured from the start of the clip.
    pub fn new(
        clip_id: ClipId,
        property: AnimatableProperty,
        from_offset: TimeSpan,
        to_offset: TimeSpan,
    ) -> Self {
        Self {
            clip_id,
            property,
            from_offset,
            to_offset,
            change_record: ClipChangeRecord::default(),
        }
    }
}

impl EditCommand for MoveKeyframe {
    fn label(&self) -> String {
        format!(
            "Move the {} keyframe of clip {} from {} to {}",
            self.property, self.clip_id, self.from_offset, self.to_offset
        )
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let (property, from_offset, to_offset) = (self.property, self.from_offset, self.to_offset);
        self.change_record
            .replace_clip(project, self.clip_id, |clip| {
                let mut new_version = clip.clone();
                let animation = new_version.animated_property_mut(property);
                if to_offset != from_offset && animation.keyframe_at(to_offset).is_some() {
                    return Err(EditError::KeyframeAlreadyThere {
                        clip_id: clip.id,
                        property,
                        time_offset: to_offset,
                    });
                }
                let moved_keyframe = animation.remove_keyframe_at(from_offset).ok_or(
                    EditError::KeyframeNotFound {
                        clip_id: clip.id,
                        property,
                        time_offset: from_offset,
                    },
                )?;
                animation.set_keyframe(Keyframe {
                    time_offset: to_offset,
                    ..moved_keyframe
                });
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
    use crate::animation::{AnimatedValue, Interpolation};
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

    /// The text clip with opacity keyframes at 0 s (0, ease out) and 2 s (1).
    fn project_with_a_fade() -> (Project, ClipId) {
        let (project, clip_id) = project_with_text_clip();
        let (project, _) = apply_and_check_revert(
            &project,
            SetKeyframe::new(
                clip_id,
                AnimatableProperty::Opacity,
                Keyframe {
                    interpolation_to_next: Interpolation::EaseOut,
                    ..opacity_keyframe(0, 0.0)
                },
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
        (project, clip_id)
    }

    fn opacity_of(project: &Project, clip_id: ClipId) -> &AnimatedValue {
        &project.timeline().find_clip(clip_id).unwrap().1.opacity
    }

    #[test]
    fn moving_a_keyframe_keeps_its_value_and_curve() {
        let (project, clip_id) = project_with_a_fade();
        let (project, _) = apply_and_check_revert(
            &project,
            MoveKeyframe::new(
                clip_id,
                AnimatableProperty::Opacity,
                TimeSpan::ZERO,
                TimeSpan::from_milliseconds(500),
            ),
        );
        let first_keyframe = opacity_of(&project, clip_id).keyframes()[0];
        assert_eq!(first_keyframe.time_offset, TimeSpan::from_milliseconds(500));
        assert_eq!(first_keyframe.value, 0.0);
        assert_eq!(first_keyframe.interpolation_to_next, Interpolation::EaseOut);

        // Past the other keyframe: the order changes, each keeps its own curve.
        let (project, _) = apply_and_check_revert(
            &project,
            MoveKeyframe::new(
                clip_id,
                AnimatableProperty::Opacity,
                TimeSpan::from_milliseconds(500),
                TimeSpan::from_whole_seconds(3),
            ),
        );
        let keyframes = opacity_of(&project, clip_id).keyframes();
        assert_eq!(keyframes[0].value, 1.0);
        assert_eq!(keyframes[1].time_offset, TimeSpan::from_whole_seconds(3));
        assert_eq!(keyframes[1].interpolation_to_next, Interpolation::EaseOut);
    }

    #[test]
    fn a_keyframe_cannot_land_on_another_one() {
        let (project, clip_id) = project_with_a_fade();
        let error = apply_expecting_error(
            &project,
            MoveKeyframe::new(
                clip_id,
                AnimatableProperty::Opacity,
                TimeSpan::ZERO,
                TimeSpan::from_whole_seconds(2),
            ),
        );
        assert!(matches!(error, EditError::KeyframeAlreadyThere { .. }));
        let error = apply_expecting_error(
            &project,
            MoveKeyframe::new(
                clip_id,
                AnimatableProperty::Opacity,
                TimeSpan::from_whole_seconds(1),
                TimeSpan::from_whole_seconds(3),
            ),
        );
        assert!(matches!(error, EditError::KeyframeNotFound { .. }));
    }

    #[test]
    fn removing_the_last_keyframe_can_keep_its_value() {
        let (project, clip_id) = project_with_a_fade();
        let (project, _) = apply_and_check_revert(
            &project,
            RemoveKeyframe::new(clip_id, AnimatableProperty::Opacity, TimeSpan::ZERO),
        );

        // By default the value from before the animation (1, the text's default)
        // comes back.
        let (back_to_before, _) = apply_and_check_revert(
            &project,
            RemoveKeyframe::new(
                clip_id,
                AnimatableProperty::Opacity,
                TimeSpan::from_whole_seconds(2),
            ),
        );
        assert_eq!(
            *opacity_of(&back_to_before, clip_id),
            AnimatedValue::constant(1.0)
        );

        let (project, _) = apply_and_check_revert(
            &project,
            SetKeyframe::new(
                clip_id,
                AnimatableProperty::Opacity,
                opacity_keyframe(2, 0.4),
            ),
        );
        let (kept, _) = apply_and_check_revert(
            &project,
            RemoveKeyframe::new(
                clip_id,
                AnimatableProperty::Opacity,
                TimeSpan::from_whole_seconds(2),
            )
            .keeping_the_value_when_last(),
        );
        assert_eq!(*opacity_of(&kept, clip_id), AnimatedValue::constant(0.4));
    }
}
