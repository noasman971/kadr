//! Commands: the only way to change a project.
//!
//! Each change (add a clip, split it, set its opacity…) is a small struct that
//! implements [`EditCommand`]: it knows how to **apply** itself and how to
//! **revert** what it did. An [`EditSession`](crate::history::EditSession)
//! applies commands, checks the project is still valid, and keeps them for
//! undo and redo.
//!
//! ## Contract every command follows
//!
//! - `apply` either succeeds, or fails **without changing anything**: all checks
//!   happen before the first modification.
//! - `revert` puts the project back exactly as it was before `apply`, including
//!   the identifier allocator, so that redoing gives the same identifiers.
//! - `apply` may be called again after `revert` (that is what redo does).

mod clip_commands;
mod clip_part_commands;
mod effect_commands;
mod keyframe_commands;
mod media_commands;
mod project_commands;
mod track_commands;
mod transition_commands;

use std::fmt;

pub use clip_commands::{
    AddClip, ClipEdge, MoveClip, RemoveClip, SetClipCrop, SetClipPropertyValue, SetClipSpeed,
    SetClipText, SplitClip, TrimClip,
};
pub use clip_part_commands::RemoveClipParts;
pub use effect_commands::{AddEffect, RemoveEffect, ReplaceEffect, SetEffectEnabled};
pub use keyframe_commands::{MoveKeyframe, RemoveKeyframe, SetKeyframe};
pub use media_commands::{ImportMedia, RemoveMedia};
pub use project_commands::ChangeProjectSettings;
pub use track_commands::{AddTrack, RemoveTrack, SetTrackHidden, SetTrackMuted, TrackPosition};
pub use transition_commands::{SetClipTransition, SetTextAnimation};

use crate::clip::Clip;
use crate::error::EditError;
use crate::ids::{ClipId, EffectId, IdAllocator, MediaAssetId, TrackId};
use crate::project::Project;

/// A reversible change to a project.
pub trait EditCommand: fmt::Debug + Send {
    /// A short description for the history, for example `"Split clip 4 at 12.000 s"`.
    fn label(&self) -> String;

    /// Makes the change. On error, the project is left untouched.
    ///
    /// Returns the objects created by the change (for example the new clip of a
    /// split), so the caller can refer to them.
    ///
    /// # Errors
    ///
    /// Returns why the change is impossible (missing object, invalid time…).
    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError>;

    /// Undoes the change made by the last successful [`EditCommand::apply`].
    ///
    /// # Errors
    ///
    /// Only fails on an internal inconsistency (a bug), reported as
    /// [`EditError::InternalInconsistency`].
    fn revert(&mut self, project: &mut Project) -> Result<(), EditError>;
}

/// An object created by a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CreatedObject {
    /// A media file was imported.
    MediaAsset(MediaAssetId),
    /// A track was added.
    Track(TrackId),
    /// A clip was added (or produced by a split).
    Clip(ClipId),
    /// An effect was added.
    Effect(EffectId),
}

/// Helpers to pick one kind of object out of a command's result.
pub trait CreatedObjectsExt {
    /// The first clip created, if any.
    fn first_clip_id(&self) -> Option<ClipId>;
    /// The first track created, if any.
    fn first_track_id(&self) -> Option<TrackId>;
    /// The first media file imported, if any.
    fn first_media_asset_id(&self) -> Option<MediaAssetId>;
    /// The first effect created, if any.
    fn first_effect_id(&self) -> Option<EffectId>;
}

impl CreatedObjectsExt for [CreatedObject] {
    fn first_clip_id(&self) -> Option<ClipId> {
        self.iter().find_map(|created| match created {
            CreatedObject::Clip(clip_id) => Some(*clip_id),
            _ => None,
        })
    }

    fn first_track_id(&self) -> Option<TrackId> {
        self.iter().find_map(|created| match created {
            CreatedObject::Track(track_id) => Some(*track_id),
            _ => None,
        })
    }

    fn first_media_asset_id(&self) -> Option<MediaAssetId> {
        self.iter().find_map(|created| match created {
            CreatedObject::MediaAsset(media_asset_id) => Some(*media_asset_id),
            _ => None,
        })
    }

    fn first_effect_id(&self) -> Option<EffectId> {
        self.iter().find_map(|created| match created {
            CreatedObject::Effect(effect_id) => Some(*effect_id),
            _ => None,
        })
    }
}

// ----- Building blocks shared by the commands -----

/// The error returned when `revert` is called without a successful `apply`.
fn revert_without_apply(command_label: &str) -> EditError {
    EditError::InternalInconsistency {
        details: format!("\"{command_label}\" was reverted without having been applied"),
    }
}

/// The error returned when an object that was just found disappears.
fn object_vanished(description: String) -> EditError {
    EditError::InternalInconsistency {
        details: format!("{description} disappeared during a change"),
    }
}

/// Undo support for commands that replace one clip by a new version of itself,
/// on the same track.
#[derive(Debug, Default)]
struct ClipChangeRecord {
    previous_version: Option<Clip>,
}

impl ClipChangeRecord {
    /// Computes the new version of the clip with `make_new_version` and puts it
    /// in place of the current one, remembering the current one for undo.
    fn replace_clip(
        &mut self,
        project: &mut Project,
        clip_id: ClipId,
        make_new_version: impl FnOnce(&Clip) -> Result<Clip, EditError>,
    ) -> Result<(), EditError> {
        let (_, current_version) = project.require_clip(clip_id)?;
        let new_version = make_new_version(current_version)?;
        let track = project
            .timeline_mut()
            .track_holding_clip_mut(clip_id)
            .ok_or_else(|| object_vanished(format!("the track of clip {clip_id}")))?;
        let previous_version = track
            .replace_clip(new_version)
            .ok_or_else(|| object_vanished(format!("clip {clip_id}")))?;
        self.previous_version = Some(previous_version);
        Ok(())
    }

    /// Puts the remembered version back.
    fn restore_previous_version(
        &mut self,
        project: &mut Project,
        command_label: &str,
    ) -> Result<(), EditError> {
        let previous_version = self
            .previous_version
            .take()
            .ok_or_else(|| revert_without_apply(command_label))?;
        let clip_id = previous_version.id;
        let track = project
            .timeline_mut()
            .track_holding_clip_mut(clip_id)
            .ok_or_else(|| object_vanished(format!("the track of clip {clip_id}")))?;
        track
            .replace_clip(previous_version)
            .ok_or_else(|| object_vanished(format!("clip {clip_id}")))?;
        Ok(())
    }
}

/// Undo support for commands that hand out new identifiers.
#[derive(Debug, Default)]
struct IdAllocatorRecord {
    state_before_apply: Option<IdAllocator>,
}

impl IdAllocatorRecord {
    /// Remembers the allocator before the command allocates anything.
    fn remember(&mut self, project: &Project) {
        self.state_before_apply = Some(project.id_allocator());
    }

    /// Puts the allocator back, so that redo hands out the same identifiers.
    fn restore(&mut self, project: &mut Project, command_label: &str) -> Result<(), EditError> {
        let earlier_state = self
            .state_before_apply
            .take()
            .ok_or_else(|| revert_without_apply(command_label))?;
        project.restore_id_allocator(earlier_state);
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Checks shared by the command tests.

    use super::*;
    use crate::validation::validate_project;

    /// Applies `command`, checks the project is still valid, reverts it, and
    /// checks the project is exactly as before. Then applies it again (redo) and
    /// checks the result is identical to the first application.
    ///
    /// Returns the project after the command and the created objects.
    pub(crate) fn apply_and_check_revert(
        project: &Project,
        mut command: impl EditCommand,
    ) -> (Project, Vec<CreatedObject>) {
        let mut working_project = project.clone();

        let created_objects = command
            .apply(&mut working_project)
            .unwrap_or_else(|error| panic!("{} failed: {error}", command.label()));
        validate_project(&working_project).unwrap_or_else(|error| {
            panic!("{} made the project invalid: {error}", command.label())
        });
        let project_after_first_apply = working_project.clone();

        command.revert(&mut working_project).unwrap();
        assert_eq!(&working_project, project, "revert of {}", command.label());

        let created_on_redo = command.apply(&mut working_project).unwrap();
        assert_eq!(
            working_project,
            project_after_first_apply,
            "redo of {}",
            command.label()
        );
        assert_eq!(created_on_redo, created_objects);

        (working_project, created_objects)
    }

    /// Applies `command`, expects it to fail, and checks nothing changed.
    pub(crate) fn apply_expecting_error(
        project: &Project,
        mut command: impl EditCommand,
    ) -> EditError {
        let mut working_project = project.clone();
        let error = command
            .apply(&mut working_project)
            .expect_err("the command should have failed");
        assert_eq!(
            &working_project, project,
            "a failed command must not change anything"
        );
        error
    }
}
