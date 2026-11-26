//! What the interface can ask to change, and how each request becomes commands.
//!
//! One user gesture is one [`EditRequest`], applied as **one** undo step. The
//! rules a gesture follows ("add at the end of the first video track, creating it
//! if needed") live here, in Rust, where they are tested; the front end only
//! sends the gesture.
//!
//! Undo labels are **keys** (`addClip`, `importMedia`…) the front end translates;
//! steps made elsewhere (by Claude, in phase 7) keep the engine's English labels.

use kadr_core::commands::{
    AddClip, AddTrack, ChangeProjectSettings, CreatedObjectsExt, ImportMedia, TrackPosition,
};
use kadr_core::error::EditError;
use kadr_core::history::{EditSession, Transaction};
use kadr_core::ids::{ClipId, MediaAssetId, TrackId};
use kadr_core::media_asset::{MediaAssetDescription, MediaKind, PictureSize};
use kadr_core::project::{Project, ProjectSettings};
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_core::timeline::TrackKind;
use serde::{Deserialize, Serialize};

/// How long a still picture lasts when it is placed on the timeline.
pub const STILL_IMAGE_DURATION: TimeSpan = TimeSpan::from_whole_seconds(5);

/// The smallest and largest canvas sides the engine accepts (see `kadr_core::validation`).
const MINIMUM_CANVAS_SIDE_IN_PIXELS: u32 = 16;
const MAXIMUM_CANVAS_SIDE_IN_PIXELS: u32 = 8192;
/// The fastest frame rate the engine accepts.
const MAXIMUM_FRAMES_PER_SECOND: f64 = 240.0;

/// A change asked by the interface.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum EditRequest {
    /// Places a media file at the end of the first track of its kind (videos and
    /// pictures on the first video track, sounds on the first audio track),
    /// creating the track if there is none.
    AddMediaAtEnd {
        /// The imported file to place.
        media_id: MediaAssetId,
    },
}

/// Something the interface should tell the user after an edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum EditNotice {
    /// The project took the format of the first video ("Automatic" format).
    FormatAdopted {
        /// The video it came from.
        media_name: String,
        /// New canvas width.
        width: u32,
        /// New canvas height.
        height: u32,
    },
}

/// What an edit produced, besides the new project.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditOutcome {
    /// Clips created by the edit (the interface selects them).
    pub created_clip_ids: Vec<ClipId>,
    /// Messages for the user.
    pub notices: Vec<EditNotice>,
}

/// Applies `request` as one undo step.
///
/// `adopts_first_video_format` is the "Automatic format" flag: when set, the first
/// video placed sets the project format, and the flag is cleared.
///
/// # Errors
///
/// The engine's refusal; the project is then unchanged.
pub fn apply_edit_request(
    session: &mut EditSession,
    adopts_first_video_format: &mut bool,
    request: &EditRequest,
) -> Result<EditOutcome, EditError> {
    match request {
        EditRequest::AddMediaAtEnd { media_id } => {
            add_media_at_end(session, adopts_first_video_format, *media_id)
        }
    }
}

/// Imports already-probed files as one undo step and returns their identifiers,
/// in the same order. A file already in the project is not imported twice: its
/// existing identifier is returned.
///
/// # Errors
///
/// The engine's refusal; nothing is imported then.
pub fn import_media_files(
    session: &mut EditSession,
    descriptions: Vec<MediaAssetDescription>,
) -> Result<Vec<MediaAssetId>, EditError> {
    session.transaction("importMedia", |transaction| {
        let mut media_ids = Vec::new();
        for description in descriptions {
            let existing = transaction
                .project()
                .media_assets()
                .iter()
                .find(|media_asset| media_asset.description.file_path == description.file_path)
                .map(|media_asset| media_asset.id);
            let media_id = match existing {
                Some(media_id) => media_id,
                None => transaction
                    .execute(ImportMedia::new(description))?
                    .first_media_asset_id()
                    .ok_or_else(missing_created_object)?,
            };
            media_ids.push(media_id);
        }
        Ok(media_ids)
    })
}

fn add_media_at_end(
    session: &mut EditSession,
    adopts_first_video_format: &mut bool,
    media_id: MediaAssetId,
) -> Result<EditOutcome, EditError> {
    let description = media_description(session.project(), media_id)?;
    let should_adopt_format = *adopts_first_video_format && description.kind == MediaKind::Video;
    let outcome = session.transaction("addClip", |transaction| {
        let mut notices = Vec::new();
        if should_adopt_format
            && let Some(settings) = settings_adopted_from(transaction.project(), &description)
        {
            transaction.execute(ChangeProjectSettings::new(settings))?;
            notices.push(EditNotice::FormatAdopted {
                media_name: description.display_name.clone(),
                width: settings.canvas_size.width_in_pixels,
                height: settings.canvas_size.height_in_pixels,
            });
        }
        let track_kind = track_kind_for(description.kind);
        let track_id = first_track_of_kind_or_new(transaction, track_kind)?;
        let start = track_end(transaction.project(), track_id);
        let length = description.duration.unwrap_or(STILL_IMAGE_DURATION);
        let clip_id = transaction
            .execute(AddClip::media(
                track_id,
                media_id,
                Timestamp::ZERO,
                Timestamp::ZERO + length,
                start,
            ))?
            .first_clip_id()
            .ok_or_else(missing_created_object)?;
        Ok(EditOutcome {
            created_clip_ids: vec![clip_id],
            notices,
        })
    })?;
    if should_adopt_format {
        *adopts_first_video_format = false;
    }
    Ok(outcome)
}

/// The description of an imported file.
fn media_description(
    project: &Project,
    media_id: MediaAssetId,
) -> Result<MediaAssetDescription, EditError> {
    project
        .media_asset(media_id)
        .map(|media_asset| media_asset.description.clone())
        .ok_or_else(|| EditError::MediaAssetNotFound {
            media_asset_id: media_id,
            existing_media_asset_ids: project
                .media_assets()
                .iter()
                .map(|media_asset| media_asset.id)
                .collect(),
        })
}

/// Sounds go on audio tracks, everything else on video tracks.
pub fn track_kind_for(media_kind: MediaKind) -> TrackKind {
    if media_kind == MediaKind::Audio {
        TrackKind::Audio
    } else {
        TrackKind::Video
    }
}

/// The lowest track of `kind` (the main video track, or the first audio track),
/// created at the bottom of the stack if there is none.
fn first_track_of_kind_or_new(
    transaction: &mut Transaction<'_>,
    kind: TrackKind,
) -> Result<TrackId, EditError> {
    let existing = transaction
        .project()
        .timeline()
        .tracks()
        .iter()
        .find(|track| track.kind == kind)
        .map(|track| track.id);
    match existing {
        Some(track_id) => Ok(track_id),
        None => transaction
            .execute(AddTrack::new(kind, TrackPosition::Bottom))?
            .first_track_id()
            .ok_or_else(missing_created_object),
    }
}

/// Where the last clip of a track ends (the start of the timeline for an empty track).
fn track_end(project: &Project, track_id: TrackId) -> Timestamp {
    project
        .timeline()
        .track(track_id)
        .map_or(Timestamp::ZERO, |track| track.end())
}

/// The project settings matching a video: its picture size (made even and kept
/// within the engine's limits) and its frame rate. `None` when the video does not
/// say, or the format would not change.
fn settings_adopted_from(
    project: &Project,
    description: &MediaAssetDescription,
) -> Option<ProjectSettings> {
    let picture_size = description.picture_size?;
    let current = *project.settings();
    let frame_rate = description
        .frame_rate
        .filter(|frame_rate| frame_rate.as_frames_per_second() <= MAXIMUM_FRAMES_PER_SECOND)
        .unwrap_or(current.frame_rate);
    let adopted = ProjectSettings {
        canvas_size: PictureSize {
            width_in_pixels: usable_canvas_side(picture_size.width_in_pixels),
            height_in_pixels: usable_canvas_side(picture_size.height_in_pixels),
        },
        frame_rate,
        ..current
    };
    (adopted != current).then_some(adopted)
}

/// `side` rounded down to an even number (H.264 needs it) within the engine's limits.
fn usable_canvas_side(side: u32) -> u32 {
    let clamped = side.clamp(MINIMUM_CANVAS_SIDE_IN_PIXELS, MAXIMUM_CANVAS_SIDE_IN_PIXELS);
    clamped - clamped % 2
}

fn missing_created_object() -> EditError {
    EditError::InternalInconsistency {
        details: "a command did not report the object it created".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kadr_core::media_asset::AudioFormat;
    use kadr_core::time::FrameRate;
    use std::path::PathBuf;

    fn video(name: &str, width: u32, height: u32, seconds: u64) -> MediaAssetDescription {
        MediaAssetDescription {
            file_path: PathBuf::from(format!("C:/videos/{name}")),
            display_name: name.to_string(),
            kind: MediaKind::Video,
            duration: Some(TimeSpan::from_whole_seconds(seconds)),
            picture_size: Some(PictureSize {
                width_in_pixels: width,
                height_in_pixels: height,
            }),
            frame_rate: Some(FrameRate::FPS_25),
            audio_format: Some(AudioFormat {
                sample_rate_in_hertz: 48_000,
                channel_count: 2,
            }),
        }
    }

    fn sound(name: &str, seconds: u64) -> MediaAssetDescription {
        MediaAssetDescription {
            file_path: PathBuf::from(format!("C:/music/{name}")),
            display_name: name.to_string(),
            kind: MediaKind::Audio,
            duration: Some(TimeSpan::from_whole_seconds(seconds)),
            picture_size: None,
            frame_rate: None,
            audio_format: Some(AudioFormat {
                sample_rate_in_hertz: 44_100,
                channel_count: 2,
            }),
        }
    }

    fn picture(name: &str) -> MediaAssetDescription {
        MediaAssetDescription {
            file_path: PathBuf::from(format!("C:/pictures/{name}")),
            display_name: name.to_string(),
            kind: MediaKind::Image,
            duration: None,
            picture_size: Some(PictureSize {
                width_in_pixels: 4000,
                height_in_pixels: 3000,
            }),
            frame_rate: None,
            audio_format: None,
        }
    }

    fn session_with(descriptions: Vec<MediaAssetDescription>) -> (EditSession, Vec<MediaAssetId>) {
        let mut session =
            EditSession::new(Project::new(ProjectSettings::landscape_1080p())).unwrap();
        let media_ids = import_media_files(&mut session, descriptions).unwrap();
        (session, media_ids)
    }

    fn add_at_end(
        session: &mut EditSession,
        adopts: &mut bool,
        media_id: MediaAssetId,
    ) -> EditOutcome {
        apply_edit_request(session, adopts, &EditRequest::AddMediaAtEnd { media_id }).unwrap()
    }

    #[test]
    fn importing_twice_reuses_the_media_and_is_one_undo_step() {
        let (mut session, first_ids) =
            session_with(vec![video("a.mp4", 1920, 1080, 3), sound("b.mp3", 9)]);
        let second_ids = import_media_files(
            &mut session,
            vec![video("a.mp4", 1920, 1080, 3), picture("c.png")],
        )
        .unwrap();
        assert_eq!(second_ids[0], first_ids[0]);
        assert_eq!(session.project().media_assets().len(), 3);
        assert_eq!(session.next_undo_label(), Some("importMedia"));
        session.undo().unwrap();
        assert_eq!(session.project().media_assets().len(), 2);
    }

    #[test]
    fn media_go_end_to_end_on_the_first_track_of_their_kind() {
        let (mut session, ids) = session_with(vec![
            video("a.mp4", 1920, 1080, 3),
            sound("music.mp3", 9),
            picture("logo.png"),
        ]);
        let mut adopts = false;
        add_at_end(&mut session, &mut adopts, ids[0]);
        add_at_end(&mut session, &mut adopts, ids[1]);
        let outcome = add_at_end(&mut session, &mut adopts, ids[2]);

        let timeline = session.project().timeline();
        assert_eq!(timeline.tracks().len(), 2);
        let video_track = timeline
            .tracks()
            .iter()
            .find(|track| track.kind == TrackKind::Video)
            .unwrap();
        let clips = video_track.clips();
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[1].timeline_start, Timestamp::from_whole_seconds(3));
        assert_eq!(clips[1].timeline_duration(), STILL_IMAGE_DURATION);
        assert_eq!(outcome.created_clip_ids, vec![clips[1].id]);
        assert_eq!(session.next_undo_label(), Some("addClip"));
    }

    #[test]
    fn the_first_video_sets_the_automatic_format_once_in_the_same_undo_step() {
        let (mut session, ids) = session_with(vec![
            picture("logo.png"),
            video("phone.mp4", 1081, 1921, 4),
            video("wide.mp4", 3840, 2160, 2),
        ]);
        let mut adopts = true;

        // A picture does not decide the format.
        assert!(
            add_at_end(&mut session, &mut adopts, ids[0])
                .notices
                .is_empty()
        );
        assert!(adopts);

        let outcome = add_at_end(&mut session, &mut adopts, ids[1]);
        assert_eq!(
            outcome.notices,
            vec![EditNotice::FormatAdopted {
                media_name: "phone.mp4".to_string(),
                width: 1080,
                height: 1920,
            }]
        );
        assert!(!adopts);
        let settings = session.project().settings();
        assert_eq!(settings.canvas_size.width_in_pixels, 1080);
        assert_eq!(settings.frame_rate, FrameRate::FPS_25);

        // Later videos do not change it.
        assert!(
            add_at_end(&mut session, &mut adopts, ids[2])
                .notices
                .is_empty()
        );
        assert_eq!(
            session.project().settings().canvas_size.height_in_pixels,
            1920
        );

        // One undo removes the clip and restores the format.
        session.undo().unwrap();
        session.undo().unwrap();
        assert_eq!(
            session.project().settings(),
            &ProjectSettings::landscape_1080p()
        );
    }

    #[test]
    fn an_unknown_media_is_refused_without_change() {
        let (mut session, _) = session_with(vec![]);
        let mut adopts = true;
        let error = apply_edit_request(
            &mut session,
            &mut adopts,
            &EditRequest::AddMediaAtEnd {
                media_id: MediaAssetId::from_raw(99),
            },
        )
        .unwrap_err();
        assert!(matches!(error, EditError::MediaAssetNotFound { .. }));
        assert!(adopts);
        assert!(session.project().timeline().tracks().is_empty());
    }

    #[test]
    fn requests_read_from_the_front_end_json() {
        let request: EditRequest =
            serde_json::from_str(r#"{ "type": "addMediaAtEnd", "mediaId": 7 }"#).unwrap();
        assert_eq!(
            request,
            EditRequest::AddMediaAtEnd {
                media_id: MediaAssetId::from_raw(7)
            }
        );
    }
}
