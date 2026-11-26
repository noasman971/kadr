//! Commands that set the transitions of videos and images and the animations of
//! texts.
//!
//! Which clips may have a transition, and the allowed durations, are rules of
//! [`validate_project`](crate::validation::validate_project): a command that
//! breaks them is undone and refused by the edit session.

use super::{ClipChangeRecord, ClipEdge, CreatedObject, EditCommand};
use crate::clip::ClipContent;
use crate::error::EditError;
use crate::ids::ClipId;
use crate::project::Project;
use crate::text_animation::TextAnimation;
use crate::transition::Transition;

/// Sets, replaces or removes the transition at one edge of a video or image clip.
///
/// The entry transition (`ClipEdge::Start`) plays over the clip's first moments,
/// the exit transition (`ClipEdge::End`) over its last moments. To transition
/// from one clip to the next on the same track, give the second clip an entry
/// transition (or the first one an exit transition): the touching clip keeps
/// showing underneath.
#[derive(Debug)]
pub struct SetClipTransition {
    clip_id: ClipId,
    edge: ClipEdge,
    transition: Option<Transition>,
    change_record: ClipChangeRecord,
}

impl SetClipTransition {
    /// Prepares the change. `None` removes the transition at that edge.
    pub fn new(clip_id: ClipId, edge: ClipEdge, transition: Option<Transition>) -> Self {
        Self {
            clip_id,
            edge,
            transition,
            change_record: ClipChangeRecord::default(),
        }
    }
}

impl EditCommand for SetClipTransition {
    fn label(&self) -> String {
        let edge_name = self.edge.display_name();
        match &self.transition {
            Some(transition) => format!(
                "Add a {} transition at the {edge_name} of clip {}",
                transition.style.display_name(),
                self.clip_id
            ),
            None => format!(
                "Remove the transition at the {edge_name} of clip {}",
                self.clip_id
            ),
        }
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let (edge, transition) = (self.edge, self.transition);
        self.change_record
            .replace_clip(project, self.clip_id, |clip| {
                let mut new_version = clip.clone();
                match edge {
                    ClipEdge::Start => new_version.entry_transition = transition,
                    ClipEdge::End => new_version.exit_transition = transition,
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

/// Sets, replaces or removes how a text clip appears (`ClipEdge::Start`) or
/// disappears (`ClipEdge::End`).
#[derive(Debug)]
pub struct SetTextAnimation {
    clip_id: ClipId,
    edge: ClipEdge,
    animation: Option<TextAnimation>,
    change_record: ClipChangeRecord,
}

impl SetTextAnimation {
    /// Prepares the change. `None` removes the animation at that edge.
    pub fn new(clip_id: ClipId, edge: ClipEdge, animation: Option<TextAnimation>) -> Self {
        Self {
            clip_id,
            edge,
            animation,
            change_record: ClipChangeRecord::default(),
        }
    }
}

impl EditCommand for SetTextAnimation {
    fn label(&self) -> String {
        let moment = match self.edge {
            ClipEdge::Start => "appears",
            ClipEdge::End => "disappears",
        };
        match &self.animation {
            Some(animation) => format!(
                "Animate how the text of clip {} {moment} ({})",
                self.clip_id,
                animation.style.display_name()
            ),
            None => format!(
                "Remove the animation when the text of clip {} {moment}",
                self.clip_id
            ),
        }
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let (edge, animation) = (self.edge, self.animation);
        self.change_record
            .replace_clip(project, self.clip_id, |clip| {
                let mut new_version = clip.clone();
                let ClipContent::Text(text_content) = &mut new_version.content else {
                    return Err(EditError::NotATextClip { clip_id: clip.id });
                };
                match edge {
                    ClipEdge::Start => text_content.entry_animation = animation,
                    ClipEdge::End => text_content.exit_animation = animation,
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
    use crate::commands::{AddClip, AddTrack, CreatedObjectsExt, ImportMedia, TrackPosition};
    use crate::history::EditSession;
    use crate::media_asset::test_samples::ten_second_video;
    use crate::project::ProjectSettings;
    use crate::text_animation::TextAnimationStyle;
    use crate::time::{TimeSpan, Timestamp};
    use crate::timeline::TrackKind;
    use crate::transition::{SlideDirection, TransitionStyle};

    /// A project with a video clip (0–4 s) and a text clip (4–6 s) on one track.
    struct Fixture {
        project: Project,
        video_clip_id: ClipId,
        text_clip_id: ClipId,
    }

    fn fixture() -> Fixture {
        let mut session =
            EditSession::new(Project::new(ProjectSettings::landscape_1080p())).unwrap();
        let media_asset_id = session
            .execute(ImportMedia::new(ten_second_video()))
            .unwrap()
            .first_media_asset_id()
            .unwrap();
        let track_id = session
            .execute(AddTrack::new(TrackKind::Video, TrackPosition::Top))
            .unwrap()
            .first_track_id()
            .unwrap();
        let video_clip_id = session
            .execute(AddClip::media(
                track_id,
                media_asset_id,
                Timestamp::ZERO,
                Timestamp::from_whole_seconds(4),
                Timestamp::ZERO,
            ))
            .unwrap()
            .first_clip_id()
            .unwrap();
        let text_clip_id = session
            .execute(AddClip::text(
                track_id,
                "Title",
                TimeSpan::from_whole_seconds(2),
                Timestamp::from_whole_seconds(4),
            ))
            .unwrap()
            .first_clip_id()
            .unwrap();
        Fixture {
            project: session.into_project(),
            video_clip_id,
            text_clip_id,
        }
    }

    fn half_second_slide() -> Transition {
        Transition {
            style: TransitionStyle::Slide {
                direction: SlideDirection::Left,
            },
            duration: TimeSpan::from_milliseconds(500),
        }
    }

    #[test]
    fn transitions_are_set_on_either_edge_and_removed() {
        let fixture = fixture();
        let (project, _) = apply_and_check_revert(
            &fixture.project,
            SetClipTransition::new(
                fixture.video_clip_id,
                ClipEdge::Start,
                Some(half_second_slide()),
            ),
        );
        let (project, _) = apply_and_check_revert(
            &project,
            SetClipTransition::new(
                fixture.video_clip_id,
                ClipEdge::End,
                Some(half_second_slide()),
            ),
        );
        let (_, clip) = project.require_clip(fixture.video_clip_id).unwrap();
        assert_eq!(clip.entry_transition, Some(half_second_slide()));
        assert_eq!(clip.exit_transition, Some(half_second_slide()));

        let (project, _) = apply_and_check_revert(
            &project,
            SetClipTransition::new(fixture.video_clip_id, ClipEdge::Start, None),
        );
        let (_, clip) = project.require_clip(fixture.video_clip_id).unwrap();
        assert_eq!(clip.entry_transition, None);
        assert_eq!(clip.exit_transition, Some(half_second_slide()));
    }

    #[test]
    fn text_animations_only_apply_to_texts() {
        let fixture = fixture();
        let type_on = TextAnimation {
            style: TextAnimationStyle::TypeOn,
            duration: TimeSpan::from_whole_seconds(1),
        };
        let (project, _) = apply_and_check_revert(
            &fixture.project,
            SetTextAnimation::new(fixture.text_clip_id, ClipEdge::Start, Some(type_on)),
        );
        let (_, clip) = project.require_clip(fixture.text_clip_id).unwrap();
        let ClipContent::Text(text_content) = &clip.content else {
            panic!("expected a text clip");
        };
        assert_eq!(text_content.entry_animation, Some(type_on));

        let error = apply_expecting_error(
            &fixture.project,
            SetTextAnimation::new(fixture.video_clip_id, ClipEdge::Start, Some(type_on)),
        );
        assert!(matches!(error, EditError::NotATextClip { .. }));
    }

    #[test]
    fn labels_describe_the_change() {
        let clip_id = ClipId::from_raw(4);
        assert_eq!(
            SetClipTransition::new(clip_id, ClipEdge::End, Some(half_second_slide())).label(),
            "Add a slide transition at the end of clip 4"
        );
        assert_eq!(
            SetTextAnimation::new(clip_id, ClipEdge::End, None).label(),
            "Remove the animation when the text of clip 4 disappears"
        );
    }
}
