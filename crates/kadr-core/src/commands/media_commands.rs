//! Commands that add or remove media files from the project.

use super::{CreatedObject, EditCommand, IdAllocatorRecord, revert_without_apply};
use crate::error::EditError;
use crate::ids::MediaAssetId;
use crate::media_asset::{MediaAsset, MediaAssetDescription};
use crate::project::Project;

/// Adds a media file to the project's library. It does not place it on the
/// timeline: use [`AddClip`](super::AddClip) for that.
#[derive(Debug)]
pub struct ImportMedia {
    description: MediaAssetDescription,
    id_allocator_record: IdAllocatorRecord,
}

impl ImportMedia {
    /// Prepares the import of a file described by `kadr-media`.
    pub fn new(description: MediaAssetDescription) -> Self {
        Self {
            description,
            id_allocator_record: IdAllocatorRecord::default(),
        }
    }
}

impl EditCommand for ImportMedia {
    fn label(&self) -> String {
        format!("Import {}", self.description.display_name)
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        self.id_allocator_record.remember(project);
        let media_asset_id = MediaAssetId::from_raw(project.id_allocator_mut().allocate_raw_id());
        project.media_assets_mut().push(MediaAsset {
            id: media_asset_id,
            description: self.description.clone(),
        });
        Ok(vec![CreatedObject::MediaAsset(media_asset_id)])
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let label = self.label();
        project
            .media_assets_mut()
            .pop()
            .ok_or_else(|| revert_without_apply(&label))?;
        self.id_allocator_record.restore(project, &label)
    }
}

/// Removes a media file from the library. Refused while clips still show it.
#[derive(Debug)]
pub struct RemoveMedia {
    media_asset_id: MediaAssetId,
    removed_asset_and_position: Option<(usize, MediaAsset)>,
}

impl RemoveMedia {
    /// Prepares the removal.
    pub fn new(media_asset_id: MediaAssetId) -> Self {
        Self {
            media_asset_id,
            removed_asset_and_position: None,
        }
    }
}

impl EditCommand for RemoveMedia {
    fn label(&self) -> String {
        format!("Remove media {}", self.media_asset_id)
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        project.require_media_asset(self.media_asset_id)?;
        let used_by_clip_ids = project.clips_using_media_asset(self.media_asset_id);
        if !used_by_clip_ids.is_empty() {
            return Err(EditError::MediaAssetInUse {
                media_asset_id: self.media_asset_id,
                used_by_clip_ids,
            });
        }
        let media_assets = project.media_assets_mut();
        let position = media_assets
            .iter()
            .position(|asset| asset.id == self.media_asset_id)
            .ok_or_else(|| super::object_vanished(format!("media {}", self.media_asset_id)))?;
        let removed_asset = media_assets.remove(position);
        self.removed_asset_and_position = Some((position, removed_asset));
        Ok(Vec::new())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let (position, removed_asset) = self
            .removed_asset_and_position
            .take()
            .ok_or_else(|| revert_without_apply(&self.label()))?;
        project.media_assets_mut().insert(position, removed_asset);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CreatedObjectsExt;
    use crate::commands::test_support::{apply_and_check_revert, apply_expecting_error};
    use crate::media_asset::test_samples::{one_minute_music, ten_second_video};
    use crate::project::ProjectSettings;

    #[test]
    fn importing_gives_the_media_a_new_identifier() {
        let project = Project::new(ProjectSettings::landscape_1080p());
        let (project_with_video, created) =
            apply_and_check_revert(&project, ImportMedia::new(ten_second_video()));

        let media_asset_id = created.first_media_asset_id().unwrap();
        assert_eq!(
            project_with_video
                .media_asset(media_asset_id)
                .unwrap()
                .description,
            ten_second_video()
        );
    }

    #[test]
    fn removing_unused_media_is_reversible() {
        let project = Project::new(ProjectSettings::landscape_1080p());
        let (project, created) =
            apply_and_check_revert(&project, ImportMedia::new(ten_second_video()));
        let (project, _) = apply_and_check_revert(&project, ImportMedia::new(one_minute_music()));
        let video_id = created.first_media_asset_id().unwrap();

        let (project_without_video, _) =
            apply_and_check_revert(&project, RemoveMedia::new(video_id));
        assert_eq!(project_without_video.media_assets().len(), 1);
    }

    #[test]
    fn removing_missing_media_lists_existing_media() {
        let project = Project::new(ProjectSettings::landscape_1080p());
        let error = apply_expecting_error(&project, RemoveMedia::new(MediaAssetId::from_raw(4)));
        assert_eq!(
            error.to_string(),
            "media 4 does not exist; existing media: none"
        );
    }
}
