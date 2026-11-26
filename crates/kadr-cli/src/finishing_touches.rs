//! Optional touches `assemble` adds to the project it builds: a transition at
//! the start of every picture clip, and an animated title.
//!
//! They exist so transitions and text animations can be tried from the command
//! line before the desktop app; every change goes through the same undoable
//! commands the app and Claude use.

use kadr_core::clip::ClipContent;
use kadr_core::commands::{
    AddClip, AddTrack, ClipEdge, CreatedObjectsExt, SetClipTransition, SetTextAnimation,
    TrackPosition,
};
use kadr_core::error::EditError;
use kadr_core::history::EditSession;
use kadr_core::project::Project;
use kadr_core::text_animation::TextAnimation;
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_core::timeline::TrackKind;
use kadr_core::transition::Transition;

/// Gives every video and image clip of the bottom video track an entry
/// `transition`: the first one appears from the background, each following one
/// from the clip before it.
///
/// # Errors
///
/// Fails if the transition is refused (for example a zero duration).
pub fn add_transitions_at_every_cut(
    project: Project,
    transition: Transition,
) -> Result<Project, EditError> {
    let mut session = EditSession::new(project)?;
    let picture_clip_ids: Vec<_> = session
        .project()
        .timeline()
        .tracks()
        .iter()
        .find(|track| track.kind == TrackKind::Video)
        .map(|track| {
            track
                .clips()
                .iter()
                .filter(|clip| matches!(clip.content, ClipContent::Media(_)))
                .map(|clip| clip.id)
                .collect()
        })
        .unwrap_or_default();
    session.transaction("Add transitions", |transaction| {
        for clip_id in picture_clip_ids {
            transaction.execute(SetClipTransition::new(
                clip_id,
                ClipEdge::Start,
                Some(transition),
            ))?;
        }
        Ok(())
    })?;
    Ok(session.into_project())
}

/// Puts `title` on a new top video track for `duration` from the start of the
/// video, appearing and disappearing with `animation`.
///
/// # Errors
///
/// Fails if the title is refused (for example an empty text).
pub fn add_animated_title(
    project: Project,
    title: &str,
    duration: TimeSpan,
    animation: TextAnimation,
) -> Result<Project, EditError> {
    let mut session = EditSession::new(project)?;
    session.transaction("Add an animated title", |transaction| {
        let track_id = transaction
            .execute(AddTrack::new(TrackKind::Video, TrackPosition::Top))?
            .first_track_id()
            .ok_or_else(missing_created_object)?;
        let title_clip_id = transaction
            .execute(AddClip::text(track_id, title, duration, Timestamp::ZERO))?
            .first_clip_id()
            .ok_or_else(missing_created_object)?;
        for edge in [ClipEdge::Start, ClipEdge::End] {
            transaction.execute(SetTextAnimation::new(title_clip_id, edge, Some(animation)))?;
        }
        Ok(())
    })?;
    Ok(session.into_project())
}

fn missing_created_object() -> EditError {
    EditError::InternalInconsistency {
        details: "a command did not report the object it created".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kadr_core::commands::ImportMedia;
    use kadr_core::media_asset::{MediaAssetDescription, MediaKind, PictureSize};
    use kadr_core::project::ProjectSettings;
    use kadr_core::text_animation::TextAnimationStyle;
    use kadr_core::time::FrameRate;
    use kadr_core::transition::TransitionStyle;

    /// Two clips of a video placed end to end on a video track.
    fn two_clips_end_to_end() -> Project {
        let mut session =
            EditSession::new(Project::new(ProjectSettings::landscape_1080p())).unwrap();
        let video_id = session
            .execute(ImportMedia::new(MediaAssetDescription {
                file_path: "clip.mp4".into(),
                display_name: "clip.mp4".to_string(),
                kind: MediaKind::Video,
                duration: Some(TimeSpan::from_whole_seconds(4)),
                picture_size: Some(PictureSize {
                    width_in_pixels: 1920,
                    height_in_pixels: 1080,
                }),
                frame_rate: Some(FrameRate::FPS_30),
                audio_format: None,
            }))
            .unwrap()
            .first_media_asset_id()
            .unwrap();
        let track_id = session
            .execute(AddTrack::new(TrackKind::Video, TrackPosition::Top))
            .unwrap()
            .first_track_id()
            .unwrap();
        for start_second in [0, 4] {
            session
                .execute(AddClip::media(
                    track_id,
                    video_id,
                    Timestamp::ZERO,
                    Timestamp::from_whole_seconds(4),
                    Timestamp::from_whole_seconds(start_second),
                ))
                .unwrap();
        }
        session.into_project()
    }

    #[test]
    fn every_picture_clip_gets_the_entry_transition() {
        let fade = Transition {
            style: TransitionStyle::Fade,
            duration: TimeSpan::from_milliseconds(500),
        };
        let project = add_transitions_at_every_cut(two_clips_end_to_end(), fade).unwrap();
        let clips = project.timeline().tracks()[0].clips();
        assert_eq!(clips.len(), 2);
        assert!(clips.iter().all(|clip| clip.entry_transition == Some(fade)));
    }

    #[test]
    fn the_title_appears_and_disappears_with_its_animation() {
        let pop = TextAnimation {
            style: TextAnimationStyle::Pop,
            duration: TimeSpan::from_milliseconds(800),
        };
        let project = add_animated_title(
            two_clips_end_to_end(),
            "Hello",
            TimeSpan::from_whole_seconds(3),
            pop,
        )
        .unwrap();
        let title_track = project.timeline().tracks().last().unwrap();
        let ClipContent::Text(title) = &title_track.clips()[0].content else {
            panic!("the top track holds the title");
        };
        assert_eq!(
            (title.entry_animation, title.exit_animation),
            (Some(pop), Some(pop))
        );
    }
}
