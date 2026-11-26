//! Commands that place, move, cut and adjust clips.

use super::{
    ClipChangeRecord, CreatedObject, EditCommand, IdAllocatorRecord, object_vanished,
    revert_without_apply,
};
use crate::clip::{
    AnimatableProperty, Clip, ClipContent, CropInsets, MediaSegment, PlaybackSpeed, TextContent,
    TextStyle,
};
use crate::error::EditError;
use crate::ids::{ClipId, MediaAssetId, TrackId};
use crate::project::Project;
use crate::time::{TimeSpan, Timestamp};

/// Places a new clip on a track.
#[derive(Debug)]
pub struct AddClip {
    track_id: TrackId,
    content: ClipContent,
    timeline_start: Timestamp,
    id_allocator_record: IdAllocatorRecord,
    created_clip_id: Option<ClipId>,
}

impl AddClip {
    /// Prepares a clip with any content.
    pub fn new(track_id: TrackId, content: ClipContent, timeline_start: Timestamp) -> Self {
        Self {
            track_id,
            content,
            timeline_start,
            id_allocator_record: IdAllocatorRecord::default(),
            created_clip_id: None,
        }
    }

    /// Prepares a clip showing `source_start` to `source_end` of a media file at
    /// normal speed.
    pub fn media(
        track_id: TrackId,
        media_asset_id: MediaAssetId,
        source_start: Timestamp,
        source_end: Timestamp,
        timeline_start: Timestamp,
    ) -> Self {
        let segment = MediaSegment {
            media_asset_id,
            source_start,
            source_end,
            speed: PlaybackSpeed::NORMAL,
        };
        Self::new(track_id, ClipContent::Media(segment), timeline_start)
    }

    /// Prepares a text clip with the default style.
    pub fn text(
        track_id: TrackId,
        text: impl Into<String>,
        duration: TimeSpan,
        timeline_start: Timestamp,
    ) -> Self {
        let content = TextContent {
            text: text.into(),
            duration,
            style: TextStyle::default(),
        };
        Self::new(track_id, ClipContent::Text(content), timeline_start)
    }
}

impl EditCommand for AddClip {
    fn label(&self) -> String {
        let what = match &self.content {
            ClipContent::Media(segment) => format!("media {}", segment.media_asset_id),
            ClipContent::Text(_) => "text".to_string(),
        };
        format!(
            "Add {what} to track {} at {}",
            self.track_id, self.timeline_start
        )
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        project.require_track(self.track_id)?;
        if let ClipContent::Media(segment) = &self.content {
            project.require_media_asset(segment.media_asset_id)?;
        }
        self.id_allocator_record.remember(project);
        let clip_id = ClipId::from_raw(project.id_allocator_mut().allocate_raw_id());
        let clip = Clip::new(clip_id, self.content.clone(), self.timeline_start);
        project
            .timeline_mut()
            .track_mut(self.track_id)
            .ok_or_else(|| object_vanished(format!("track {}", self.track_id)))?
            .insert_clip(clip);
        self.created_clip_id = Some(clip_id);
        Ok(vec![CreatedObject::Clip(clip_id)])
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let label = self.label();
        let clip_id = self
            .created_clip_id
            .take()
            .ok_or_else(|| revert_without_apply(&label))?;
        remove_clip_from_its_track(project, clip_id)?;
        self.id_allocator_record.restore(project, &label)
    }
}

/// Removes a clip from wherever it is and returns it with its track.
fn remove_clip_from_its_track(
    project: &mut Project,
    clip_id: ClipId,
) -> Result<(TrackId, Clip), EditError> {
    let track = project
        .timeline_mut()
        .track_holding_clip_mut(clip_id)
        .ok_or_else(|| object_vanished(format!("clip {clip_id}")))?;
    let track_id = track.id;
    let clip = track
        .remove_clip(clip_id)
        .ok_or_else(|| object_vanished(format!("clip {clip_id}")))?;
    Ok((track_id, clip))
}

/// Puts a clip on a track.
fn insert_clip_on_track(
    project: &mut Project,
    track_id: TrackId,
    clip: Clip,
) -> Result<(), EditError> {
    project
        .timeline_mut()
        .track_mut(track_id)
        .ok_or_else(|| object_vanished(format!("track {track_id}")))?
        .insert_clip(clip);
    Ok(())
}

/// Deletes a clip from the timeline.
#[derive(Debug)]
pub struct RemoveClip {
    clip_id: ClipId,
    removed_clip_and_track: Option<(TrackId, Clip)>,
}

impl RemoveClip {
    /// Prepares the removal.
    pub fn new(clip_id: ClipId) -> Self {
        Self {
            clip_id,
            removed_clip_and_track: None,
        }
    }
}

impl EditCommand for RemoveClip {
    fn label(&self) -> String {
        format!("Remove clip {}", self.clip_id)
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        project.require_clip(self.clip_id)?;
        self.removed_clip_and_track = Some(remove_clip_from_its_track(project, self.clip_id)?);
        Ok(Vec::new())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let (track_id, clip) = self
            .removed_clip_and_track
            .take()
            .ok_or_else(|| revert_without_apply(&self.label()))?;
        insert_clip_on_track(project, track_id, clip)
    }
}

/// Moves a clip to another time, and optionally to another track.
#[derive(Debug)]
pub struct MoveClip {
    clip_id: ClipId,
    destination_track_id: Option<TrackId>,
    new_timeline_start: Timestamp,
    original_clip_and_track: Option<(TrackId, Clip)>,
}

impl MoveClip {
    /// Moves the clip along its own track.
    pub fn along_track(clip_id: ClipId, new_timeline_start: Timestamp) -> Self {
        Self {
            clip_id,
            destination_track_id: None,
            new_timeline_start,
            original_clip_and_track: None,
        }
    }

    /// Moves the clip to another track.
    pub fn to_track(
        clip_id: ClipId,
        destination_track_id: TrackId,
        new_timeline_start: Timestamp,
    ) -> Self {
        Self {
            destination_track_id: Some(destination_track_id),
            ..Self::along_track(clip_id, new_timeline_start)
        }
    }
}

impl EditCommand for MoveClip {
    fn label(&self) -> String {
        match self.destination_track_id {
            Some(track_id) => format!(
                "Move clip {} to track {track_id} at {}",
                self.clip_id, self.new_timeline_start
            ),
            None => format!("Move clip {} to {}", self.clip_id, self.new_timeline_start),
        }
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let (source_track, _) = project.require_clip(self.clip_id)?;
        let destination_track_id = self.destination_track_id.unwrap_or(source_track.id);
        project.require_track(destination_track_id)?;

        let (source_track_id, original_clip) = remove_clip_from_its_track(project, self.clip_id)?;
        let mut moved_clip = original_clip.clone();
        moved_clip.timeline_start = self.new_timeline_start;
        insert_clip_on_track(project, destination_track_id, moved_clip)?;
        self.original_clip_and_track = Some((source_track_id, original_clip));
        Ok(Vec::new())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let (source_track_id, original_clip) = self
            .original_clip_and_track
            .take()
            .ok_or_else(|| revert_without_apply(&self.label()))?;
        remove_clip_from_its_track(project, self.clip_id)?;
        insert_clip_on_track(project, source_track_id, original_clip)
    }
}

/// Which edge of a clip a trim moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipEdge {
    /// The beginning: the end stays in place.
    Start,
    /// The end: the beginning stays in place.
    End,
}

/// Moves one edge of a clip, shortening or lengthening it.
#[derive(Debug)]
pub struct TrimClip {
    clip_id: ClipId,
    edge: ClipEdge,
    new_edge_time: Timestamp,
    change_record: ClipChangeRecord,
}

impl TrimClip {
    /// Prepares moving `edge` of the clip to `new_edge_time` on the timeline.
    pub fn new(clip_id: ClipId, edge: ClipEdge, new_edge_time: Timestamp) -> Self {
        Self {
            clip_id,
            edge,
            new_edge_time,
            change_record: ClipChangeRecord::default(),
        }
    }
}

impl EditCommand for TrimClip {
    fn label(&self) -> String {
        let edge_name = match self.edge {
            ClipEdge::Start => "start",
            ClipEdge::End => "end",
        };
        format!(
            "Move the {edge_name} of clip {} to {}",
            self.clip_id, self.new_edge_time
        )
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let (edge, new_edge_time) = (self.edge, self.new_edge_time);
        self.change_record
            .replace_clip(project, self.clip_id, |clip| match edge {
                ClipEdge::Start => clip.with_start_moved_to(new_edge_time),
                ClipEdge::End => clip.with_end_moved_to(new_edge_time),
            })?;
        Ok(Vec::new())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let label = self.label();
        self.change_record.restore_previous_version(project, &label)
    }
}

/// Cuts a clip in two at a moment of the timeline.
#[derive(Debug)]
pub struct SplitClip {
    clip_id: ClipId,
    split_time: Timestamp,
    change_record: ClipChangeRecord,
    id_allocator_record: IdAllocatorRecord,
    created_clip_id: Option<ClipId>,
}

impl SplitClip {
    /// Prepares the cut. The part after the cut becomes a new clip.
    pub fn new(clip_id: ClipId, split_time: Timestamp) -> Self {
        Self {
            clip_id,
            split_time,
            change_record: ClipChangeRecord::default(),
            id_allocator_record: IdAllocatorRecord::default(),
            created_clip_id: None,
        }
    }
}

impl EditCommand for SplitClip {
    fn label(&self) -> String {
        format!("Split clip {} at {}", self.clip_id, self.split_time)
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let (track, clip) = project.require_clip(self.clip_id)?;
        let track_id = track.id;
        let mut id_allocator_after_split = project.id_allocator();
        let (first_part, second_part) =
            clip.split_at(self.split_time, &mut id_allocator_after_split)?;
        let second_part_id = second_part.id;

        self.id_allocator_record.remember(project);
        *project.id_allocator_mut() = id_allocator_after_split;
        self.change_record
            .replace_clip(project, self.clip_id, |_| Ok(first_part))?;
        insert_clip_on_track(project, track_id, second_part)?;
        self.created_clip_id = Some(second_part_id);
        Ok(vec![CreatedObject::Clip(second_part_id)])
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let label = self.label();
        let second_part_id = self
            .created_clip_id
            .take()
            .ok_or_else(|| revert_without_apply(&label))?;
        remove_clip_from_its_track(project, second_part_id)?;
        self.change_record
            .restore_previous_version(project, &label)?;
        self.id_allocator_record.restore(project, &label)
    }
}

/// Changes how fast a media clip plays. Its start stays in place; its end moves.
#[derive(Debug)]
pub struct SetClipSpeed {
    clip_id: ClipId,
    speed: PlaybackSpeed,
    change_record: ClipChangeRecord,
}

impl SetClipSpeed {
    /// Prepares the change.
    pub fn new(clip_id: ClipId, speed: PlaybackSpeed) -> Self {
        Self {
            clip_id,
            speed,
            change_record: ClipChangeRecord::default(),
        }
    }
}

impl EditCommand for SetClipSpeed {
    fn label(&self) -> String {
        format!("Set the speed of clip {} to {}", self.clip_id, self.speed)
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let new_speed = self.speed;
        self.change_record
            .replace_clip(project, self.clip_id, |clip| {
                let mut new_version = clip.clone();
                match &mut new_version.content {
                    ClipContent::Media(segment) => segment.speed = new_speed,
                    ClipContent::Text(_) => {
                        return Err(EditError::NotAMediaClip { clip_id: clip.id });
                    }
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

/// Sets a constant value for a clip property (volume, opacity, position, scale,
/// rotation). Refused if the property is animated: set a keyframe instead.
#[derive(Debug)]
pub struct SetClipPropertyValue {
    clip_id: ClipId,
    property: AnimatableProperty,
    value: f32,
    change_record: ClipChangeRecord,
}

impl SetClipPropertyValue {
    /// Prepares the change. The value is checked against the property's range.
    pub fn new(clip_id: ClipId, property: AnimatableProperty, value: f32) -> Self {
        Self {
            clip_id,
            property,
            value,
            change_record: ClipChangeRecord::default(),
        }
    }
}

impl EditCommand for SetClipPropertyValue {
    fn label(&self) -> String {
        format!(
            "Set the {} of clip {} to {}",
            self.property, self.clip_id, self.value
        )
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let (property, value) = (self.property, self.value);
        self.change_record
            .replace_clip(project, self.clip_id, |clip| {
                if clip.animated_property(property).is_animated() {
                    return Err(EditError::PropertyIsAnimated {
                        clip_id: clip.id,
                        property,
                    });
                }
                let mut new_version = clip.clone();
                new_version.animated_property_mut(property).constant_value = value;
                Ok(new_version)
            })?;
        Ok(Vec::new())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let label = self.label();
        self.change_record.restore_previous_version(project, &label)
    }
}

/// Sets how much of each edge of a clip's picture is cut away.
#[derive(Debug)]
pub struct SetClipCrop {
    clip_id: ClipId,
    crop: CropInsets,
    change_record: ClipChangeRecord,
}

impl SetClipCrop {
    /// Prepares the change.
    pub fn new(clip_id: ClipId, crop: CropInsets) -> Self {
        Self {
            clip_id,
            crop,
            change_record: ClipChangeRecord::default(),
        }
    }
}

impl EditCommand for SetClipCrop {
    fn label(&self) -> String {
        format!("Crop clip {}", self.clip_id)
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let crop = self.crop;
        self.change_record
            .replace_clip(project, self.clip_id, |clip| {
                let mut new_version = clip.clone();
                new_version.transform.crop = crop;
                Ok(new_version)
            })?;
        Ok(Vec::new())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let label = self.label();
        self.change_record.restore_previous_version(project, &label)
    }
}

/// Changes the words (and optionally the style) of a text clip.
#[derive(Debug)]
pub struct SetClipText {
    clip_id: ClipId,
    new_text: String,
    new_style: Option<TextStyle>,
    change_record: ClipChangeRecord,
}

impl SetClipText {
    /// Prepares new words, keeping the current style.
    pub fn new(clip_id: ClipId, new_text: impl Into<String>) -> Self {
        Self {
            clip_id,
            new_text: new_text.into(),
            new_style: None,
            change_record: ClipChangeRecord::default(),
        }
    }

    /// Also changes the style.
    pub fn with_style(mut self, new_style: TextStyle) -> Self {
        self.new_style = Some(new_style);
        self
    }
}

impl EditCommand for SetClipText {
    fn label(&self) -> String {
        format!("Edit the text of clip {}", self.clip_id)
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let (new_text, new_style) = (self.new_text.clone(), self.new_style.clone());
        self.change_record
            .replace_clip(project, self.clip_id, |clip| {
                let mut new_version = clip.clone();
                let ClipContent::Text(text_content) = &mut new_version.content else {
                    return Err(EditError::NotATextClip { clip_id: clip.id });
                };
                text_content.text = new_text;
                if let Some(style) = new_style {
                    text_content.style = style;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::{apply_and_check_revert, apply_expecting_error};
    use crate::commands::{AddTrack, CreatedObjectsExt, ImportMedia, TrackPosition};
    use crate::media_asset::test_samples::ten_second_video;
    use crate::project::ProjectSettings;
    use crate::timeline::TrackKind;

    fn seconds(value: u64) -> Timestamp {
        Timestamp::from_whole_seconds(value)
    }

    /// A project with one video track holding clip `0–6 s of the video` at 0 s
    /// and a text clip from 10 s to 14 s.
    struct Fixture {
        project: Project,
        track_id: TrackId,
        video_clip_id: ClipId,
        text_clip_id: ClipId,
    }

    fn fixture() -> Fixture {
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
            AddClip::media(track_id, media_asset_id, seconds(0), seconds(6), seconds(0)),
        );
        let video_clip_id = created.first_clip_id().unwrap();
        let (project, created) = apply_and_check_revert(
            &project,
            AddClip::text(
                track_id,
                "Title",
                TimeSpan::from_whole_seconds(4),
                seconds(10),
            ),
        );
        let text_clip_id = created.first_clip_id().unwrap();
        Fixture {
            project,
            track_id,
            video_clip_id,
            text_clip_id,
        }
    }

    fn clip(project: &Project, clip_id: ClipId) -> &Clip {
        project.timeline().find_clip(clip_id).unwrap().1
    }

    #[test]
    fn adding_a_clip_to_a_missing_track_or_media_fails() {
        let fixture = fixture();
        let missing_track = AddClip::text(
            TrackId::from_raw(99),
            "x",
            TimeSpan::from_whole_seconds(1),
            seconds(0),
        );
        assert!(matches!(
            apply_expecting_error(&fixture.project, missing_track),
            EditError::TrackNotFound { .. }
        ));
        let missing_media = AddClip::media(
            fixture.track_id,
            MediaAssetId::from_raw(99),
            seconds(0),
            seconds(1),
            seconds(20),
        );
        assert!(matches!(
            apply_expecting_error(&fixture.project, missing_media),
            EditError::MediaAssetNotFound { .. }
        ));
    }

    #[test]
    fn removing_and_moving_clips_are_reversible() {
        let fixture = fixture();
        let (project, _) =
            apply_and_check_revert(&fixture.project, RemoveClip::new(fixture.video_clip_id));
        assert!(
            project
                .timeline()
                .find_clip(fixture.video_clip_id)
                .is_none()
        );

        let (project, _) = apply_and_check_revert(
            &fixture.project,
            MoveClip::along_track(fixture.text_clip_id, seconds(20)),
        );
        assert_eq!(
            clip(&project, fixture.text_clip_id).timeline_start,
            seconds(20)
        );
    }

    #[test]
    fn moving_a_clip_to_another_track_is_reversible() {
        let fixture = fixture();
        let (project, created) = apply_and_check_revert(
            &fixture.project,
            AddTrack::new(TrackKind::Video, TrackPosition::Top),
        );
        let second_track_id = created.first_track_id().unwrap();

        let (moved_project, _) = apply_and_check_revert(
            &project,
            MoveClip::to_track(fixture.text_clip_id, second_track_id, seconds(1)),
        );
        let (track, _) = moved_project
            .timeline()
            .find_clip(fixture.text_clip_id)
            .unwrap();
        assert_eq!(track.id, second_track_id);
    }

    #[test]
    fn trimming_and_splitting_are_reversible() {
        let fixture = fixture();
        let (trimmed_project, _) = apply_and_check_revert(
            &fixture.project,
            TrimClip::new(fixture.video_clip_id, ClipEdge::End, seconds(4)),
        );
        assert_eq!(
            clip(&trimmed_project, fixture.video_clip_id).timeline_end(),
            seconds(4)
        );

        let (split_project, created) = apply_and_check_revert(
            &fixture.project,
            SplitClip::new(fixture.video_clip_id, seconds(2)),
        );
        let second_part_id = created.first_clip_id().unwrap();
        assert_eq!(
            clip(&split_project, second_part_id).timeline_start,
            seconds(2)
        );
        assert_eq!(
            clip(&split_project, fixture.video_clip_id).timeline_end(),
            seconds(2)
        );
    }

    #[test]
    fn speed_property_crop_and_text_changes_are_reversible() {
        let fixture = fixture();
        let double_speed = PlaybackSpeed::from_fraction(2, 1).unwrap();
        let (project, _) = apply_and_check_revert(
            &fixture.project,
            SetClipSpeed::new(fixture.video_clip_id, double_speed),
        );
        assert_eq!(
            clip(&project, fixture.video_clip_id).timeline_end(),
            seconds(3)
        );

        let (project, _) = apply_and_check_revert(
            &fixture.project,
            SetClipPropertyValue::new(fixture.video_clip_id, AnimatableProperty::Opacity, 0.5),
        );
        assert_eq!(
            clip(&project, fixture.video_clip_id).opacity.constant_value,
            0.5
        );

        let crop = CropInsets {
            left_fraction: 0.1,
            ..CropInsets::default()
        };
        let (project, _) = apply_and_check_revert(
            &fixture.project,
            SetClipCrop::new(fixture.video_clip_id, crop),
        );
        assert_eq!(clip(&project, fixture.video_clip_id).transform.crop, crop);

        let (project, _) = apply_and_check_revert(
            &fixture.project,
            SetClipText::new(fixture.text_clip_id, "New title"),
        );
        let ClipContent::Text(text_content) = &clip(&project, fixture.text_clip_id).content else {
            panic!("expected a text clip");
        };
        assert_eq!(text_content.text, "New title");
    }

    #[test]
    fn clip_kind_specific_commands_refuse_the_wrong_kind() {
        let fixture = fixture();
        assert!(matches!(
            apply_expecting_error(
                &fixture.project,
                SetClipSpeed::new(fixture.text_clip_id, PlaybackSpeed::NORMAL)
            ),
            EditError::NotAMediaClip { .. }
        ));
        assert!(matches!(
            apply_expecting_error(
                &fixture.project,
                SetClipText::new(fixture.video_clip_id, "x")
            ),
            EditError::NotATextClip { .. }
        ));
    }
}
