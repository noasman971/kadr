//! Commands that add, change and remove clip effects.

use super::{ClipChangeRecord, CreatedObject, EditCommand, IdAllocatorRecord};
use crate::clip::Clip;
use crate::effect::{Effect, EffectKind};
use crate::error::EditError;
use crate::ids::{ClipId, EffectId};
use crate::project::Project;

/// The position of an effect in its clip, or an error listing the clip's effects.
fn find_effect_index(clip: &Clip, effect_id: EffectId) -> Result<usize, EditError> {
    clip.effects
        .iter()
        .position(|effect| effect.id == effect_id)
        .ok_or_else(|| EditError::EffectNotFound {
            clip_id: clip.id,
            effect_id,
            existing_effect_ids: clip.effects.iter().map(|effect| effect.id).collect(),
        })
}

/// Adds an enabled effect at the end of a clip's effect list.
#[derive(Debug)]
pub struct AddEffect {
    clip_id: ClipId,
    kind: EffectKind,
    change_record: ClipChangeRecord,
    id_allocator_record: IdAllocatorRecord,
}

impl AddEffect {
    /// Prepares the effect.
    pub fn new(clip_id: ClipId, kind: EffectKind) -> Self {
        Self {
            clip_id,
            kind,
            change_record: ClipChangeRecord::default(),
            id_allocator_record: IdAllocatorRecord::default(),
        }
    }
}

impl EditCommand for AddEffect {
    fn label(&self) -> String {
        format!(
            "Add a {} to clip {}",
            self.kind.display_name(),
            self.clip_id
        )
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        project.require_clip(self.clip_id)?;
        self.id_allocator_record.remember(project);
        let effect_id = EffectId::from_raw(project.id_allocator_mut().allocate_raw_id());
        let new_effect = Effect {
            id: effect_id,
            is_enabled: true,
            kind: self.kind.clone(),
        };
        self.change_record
            .replace_clip(project, self.clip_id, |clip| {
                let mut new_version = clip.clone();
                new_version.effects.push(new_effect);
                Ok(new_version)
            })?;
        Ok(vec![CreatedObject::Effect(effect_id)])
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let label = self.label();
        self.change_record
            .restore_previous_version(project, &label)?;
        self.id_allocator_record.restore(project, &label)
    }
}

/// Removes an effect from a clip.
#[derive(Debug)]
pub struct RemoveEffect {
    clip_id: ClipId,
    effect_id: EffectId,
    change_record: ClipChangeRecord,
}

impl RemoveEffect {
    /// Prepares the removal.
    pub fn new(clip_id: ClipId, effect_id: EffectId) -> Self {
        Self {
            clip_id,
            effect_id,
            change_record: ClipChangeRecord::default(),
        }
    }
}

impl EditCommand for RemoveEffect {
    fn label(&self) -> String {
        format!(
            "Remove effect {} from clip {}",
            self.effect_id, self.clip_id
        )
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let effect_id = self.effect_id;
        self.change_record
            .replace_clip(project, self.clip_id, |clip| {
                let effect_index = find_effect_index(clip, effect_id)?;
                let mut new_version = clip.clone();
                new_version.effects.remove(effect_index);
                Ok(new_version)
            })?;
        Ok(Vec::new())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let label = self.label();
        self.change_record.restore_previous_version(project, &label)
    }
}

/// Replaces the settings of an effect, keeping its identifier and position.
/// Used for example to store the regions computed by the face tracker.
#[derive(Debug)]
pub struct ReplaceEffect {
    clip_id: ClipId,
    effect_id: EffectId,
    new_kind: EffectKind,
    change_record: ClipChangeRecord,
}

impl ReplaceEffect {
    /// Prepares the change.
    pub fn new(clip_id: ClipId, effect_id: EffectId, new_kind: EffectKind) -> Self {
        Self {
            clip_id,
            effect_id,
            new_kind,
            change_record: ClipChangeRecord::default(),
        }
    }
}

impl EditCommand for ReplaceEffect {
    fn label(&self) -> String {
        format!("Change effect {} of clip {}", self.effect_id, self.clip_id)
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let (effect_id, new_kind) = (self.effect_id, self.new_kind.clone());
        self.change_record
            .replace_clip(project, self.clip_id, |clip| {
                let effect_index = find_effect_index(clip, effect_id)?;
                let mut new_version = clip.clone();
                new_version.effects[effect_index].kind = new_kind;
                Ok(new_version)
            })?;
        Ok(Vec::new())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let label = self.label();
        self.change_record.restore_previous_version(project, &label)
    }
}

/// Turns an effect on or off without deleting it.
#[derive(Debug)]
pub struct SetEffectEnabled {
    clip_id: ClipId,
    effect_id: EffectId,
    is_enabled: bool,
    change_record: ClipChangeRecord,
}

impl SetEffectEnabled {
    /// Prepares the change.
    pub fn new(clip_id: ClipId, effect_id: EffectId, is_enabled: bool) -> Self {
        Self {
            clip_id,
            effect_id,
            is_enabled,
            change_record: ClipChangeRecord::default(),
        }
    }
}

impl EditCommand for SetEffectEnabled {
    fn label(&self) -> String {
        let action = if self.is_enabled { "Enable" } else { "Disable" };
        format!(
            "{action} effect {} of clip {}",
            self.effect_id, self.clip_id
        )
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let (effect_id, is_enabled) = (self.effect_id, self.is_enabled);
        self.change_record
            .replace_clip(project, self.clip_id, |clip| {
                let effect_index = find_effect_index(clip, effect_id)?;
                let mut new_version = clip.clone();
                new_version.effects[effect_index].is_enabled = is_enabled;
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
    use crate::commands::test_support::{apply_and_check_revert, apply_expecting_error};
    use crate::commands::{AddClip, AddTrack, CreatedObjectsExt, ImportMedia, TrackPosition};
    use crate::effect::{ColorAdjustment, TrackedBlur};
    use crate::media_asset::test_samples::ten_second_video;
    use crate::project::ProjectSettings;
    use crate::time::Timestamp;
    use crate::timeline::TrackKind;

    fn project_with_video_clip() -> (Project, ClipId) {
        let project = Project::new(ProjectSettings::landscape_1080p());
        let (project, created) =
            apply_and_check_revert(&project, ImportMedia::new(ten_second_video()));
        let media_asset_id = created.first_media_asset_id().unwrap();
        let (project, created) = apply_and_check_revert(
            &project,
            AddTrack::new(TrackKind::Video, TrackPosition::Top),
        );
        let track_id = created.first_track_id().unwrap();
        let (project, created) = apply_and_check_revert(
            &project,
            AddClip::media(
                track_id,
                media_asset_id,
                Timestamp::ZERO,
                Timestamp::from_whole_seconds(5),
                Timestamp::ZERO,
            ),
        );
        (project, created.first_clip_id().unwrap())
    }

    fn effects_of(project: &Project, clip_id: ClipId) -> &[Effect] {
        &project.timeline().find_clip(clip_id).unwrap().1.effects
    }

    #[test]
    fn effects_can_be_added_changed_disabled_and_removed() {
        let (project, clip_id) = project_with_video_clip();
        let (project, created) = apply_and_check_revert(
            &project,
            AddEffect::new(
                clip_id,
                EffectKind::TrackedBlur(TrackedBlur::with_default_style()),
            ),
        );
        let effect_id = created.first_effect_id().unwrap();

        let brighter = ColorAdjustment {
            exposure_in_stops: 1.0,
            ..ColorAdjustment::NEUTRAL
        };
        let (changed_project, _) = apply_and_check_revert(
            &project,
            ReplaceEffect::new(clip_id, effect_id, EffectKind::ColorAdjustment(brighter)),
        );
        assert_eq!(
            effects_of(&changed_project, clip_id)[0].kind,
            EffectKind::ColorAdjustment(brighter)
        );

        let (disabled_project, _) =
            apply_and_check_revert(&project, SetEffectEnabled::new(clip_id, effect_id, false));
        assert!(!effects_of(&disabled_project, clip_id)[0].is_enabled);

        let (project_without_effect, _) =
            apply_and_check_revert(&project, RemoveEffect::new(clip_id, effect_id));
        assert!(effects_of(&project_without_effect, clip_id).is_empty());
    }

    #[test]
    fn unknown_effect_lists_the_clip_effects() {
        let (project, clip_id) = project_with_video_clip();
        let error =
            apply_expecting_error(&project, RemoveEffect::new(clip_id, EffectId::from_raw(77)));
        assert_eq!(
            error.to_string(),
            format!("clip {clip_id} has no effect 77; its effects: none")
        );
    }
}
