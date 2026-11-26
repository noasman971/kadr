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

mod arrangement;

pub use arrangement::TrimEdge;

/// How long a still picture lasts when it is placed on the timeline.
pub const STILL_IMAGE_DURATION: TimeSpan = TimeSpan::from_whole_seconds(5);

/// Undo label of the step that placed the first video in an "Automatic" format
/// project. Undoing it makes the format automatic again; redoing it does not.
pub const ADD_FIRST_VIDEO_UNDO_LABEL: &str = "addFirstVideo";
/// Undo label of the other steps that place a media file.
const ADD_CLIP_UNDO_LABEL: &str = "addClip";

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
    /// Places a media file at `start_ticks` on a track, or on a new track of its
    /// kind added on top when `track_id` is `None` (a drop from the library).
    AddMediaAt {
        /// The imported file to place.
        media_id: MediaAssetId,
        /// The destination track, or `None` for a new one.
        track_id: Option<TrackId>,
        /// Where the clip starts on the timeline.
        start_ticks: u64,
    },
    /// Moves a clip to `start_ticks` on a track, or on a new track of its kind
    /// added on top when `track_id` is `None`.
    MoveClip {
        /// The clip to move.
        clip_id: ClipId,
        /// The destination track, or `None` for a new one.
        track_id: Option<TrackId>,
        /// Where the clip starts after the move.
        start_ticks: u64,
    },
    /// Moves one edge of a clip to `time_ticks` on the timeline.
    TrimClip {
        /// The clip to trim.
        clip_id: ClipId,
        /// The edge that moves.
        edge: TrimEdge,
        /// Where that edge goes.
        time_ticks: u64,
    },
    /// Cuts clips in two at `time_ticks`: the given clips, or every clip under
    /// that time when the list is empty.
    SplitClips {
        /// The clips to cut (empty: every clip under the time).
        clip_ids: Vec<ClipId>,
        /// Where to cut.
        time_ticks: u64,
    },
    /// Deletes clips; the gaps they leave stay.
    RemoveClips {
        /// The clips to delete.
        clip_ids: Vec<ClipId>,
    },
    /// Turns a track's sound off or on.
    SetTrackMuted {
        /// The track.
        track_id: TrackId,
        /// `true` to mute.
        is_muted: bool,
    },
    /// Hides or shows a track's pictures.
    SetTrackHidden {
        /// The track.
        track_id: TrackId,
        /// `true` to hide.
        is_hidden: bool,
    },
    /// Removes an empty track.
    RemoveTrack {
        /// The track.
        track_id: TrackId,
    },
}

/// Where a media file goes on the timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MediaPlacement {
    /// After the last clip of the first track of its kind (created if needed).
    AtEndOfFirstTrack,
    /// At `start` on a track, or on a new track of its kind added on top.
    At {
        track_id: Option<TrackId>,
        start: Timestamp,
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
        EditRequest::AddMediaAtEnd { media_id } => add_media(
            session,
            adopts_first_video_format,
            *media_id,
            MediaPlacement::AtEndOfFirstTrack,
        ),
        EditRequest::AddMediaAt {
            media_id,
            track_id,
            start_ticks,
        } => add_media(
            session,
            adopts_first_video_format,
            *media_id,
            MediaPlacement::At {
                track_id: *track_id,
                start: Timestamp::from_ticks(*start_ticks),
            },
        ),
        EditRequest::MoveClip {
            clip_id,
            track_id,
            start_ticks,
        } => arrangement::move_clip(
            session,
            *clip_id,
            *track_id,
            Timestamp::from_ticks(*start_ticks),
        ),
        EditRequest::TrimClip {
            clip_id,
            edge,
            time_ticks,
        } => arrangement::trim_clip(session, *clip_id, *edge, Timestamp::from_ticks(*time_ticks)),
        EditRequest::SplitClips {
            clip_ids,
            time_ticks,
        } => arrangement::split_clips(session, clip_ids, Timestamp::from_ticks(*time_ticks)),
        EditRequest::RemoveClips { clip_ids } => arrangement::remove_clips(session, clip_ids),
        EditRequest::SetTrackMuted { track_id, is_muted } => {
            arrangement::set_track_muted(session, *track_id, *is_muted)
        }
        EditRequest::SetTrackHidden {
            track_id,
            is_hidden,
        } => arrangement::set_track_hidden(session, *track_id, *is_hidden),
        EditRequest::RemoveTrack { track_id } => arrangement::remove_track(session, *track_id),
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

/// Places a media file on the timeline as one undo step; the first video of an
/// "Automatic" project also sets the project format.
fn add_media(
    session: &mut EditSession,
    adopts_first_video_format: &mut bool,
    media_id: MediaAssetId,
    placement: MediaPlacement,
) -> Result<EditOutcome, EditError> {
    let description = media_description(session.project(), media_id)?;
    let should_adopt_format = *adopts_first_video_format && description.kind == MediaKind::Video;
    let undo_label = if should_adopt_format {
        ADD_FIRST_VIDEO_UNDO_LABEL
    } else {
        ADD_CLIP_UNDO_LABEL
    };
    let outcome = session.transaction(undo_label, |transaction| {
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
        let (track_id, start) = match placement {
            MediaPlacement::AtEndOfFirstTrack => {
                let track_id = first_track_of_kind_or_new(transaction, track_kind)?;
                (track_id, track_end(transaction.project(), track_id))
            }
            MediaPlacement::At {
                track_id: Some(track_id),
                start,
            } => (track_id, start),
            MediaPlacement::At {
                track_id: None,
                start,
            } => (new_track_on_top(transaction, track_kind)?, start),
        };
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

/// A new track of `kind` above all the others (a drop on the "new track" zone).
fn new_track_on_top(
    transaction: &mut Transaction<'_>,
    kind: TrackKind,
) -> Result<TrackId, EditError> {
    transaction
        .execute(AddTrack::new(kind, TrackPosition::Top))?
        .first_track_id()
        .ok_or_else(missing_created_object)
}

/// The kind of the track holding `clip_id` (a moved clip's new track has the same kind).
fn clip_track_kind(project: &Project, clip_id: ClipId) -> Result<TrackKind, EditError> {
    project
        .timeline()
        .find_clip(clip_id)
        .map(|(track, _)| track.kind)
        .ok_or_else(|| EditError::ClipNotFound {
            clip_id,
            existing_clip_ids: project
                .timeline()
                .all_clips()
                .map(|(_, clip)| clip.id)
                .collect(),
        })
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
    use kadr_core::clip::ClipContent;
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
        assert_eq!(session.next_undo_label(), Some(ADD_FIRST_VIDEO_UNDO_LABEL));
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

    fn apply(session: &mut EditSession, request: EditRequest) -> Result<EditOutcome, EditError> {
        apply_edit_request(session, &mut false, &request)
    }

    fn seconds(whole_seconds: u64) -> u64 {
        Timestamp::from_whole_seconds(whole_seconds).ticks()
    }

    /// Two videos end to end on one track (0–3 s and 3–7 s) and a sound on another.
    fn arranged_session() -> (EditSession, Vec<ClipId>) {
        let (mut session, ids) = session_with(vec![
            video("a.mp4", 1920, 1080, 3),
            video("b.mp4", 1920, 1080, 4),
            sound("music.mp3", 9),
        ]);
        let clip_ids = ids
            .iter()
            .flat_map(|media_id| {
                apply(
                    &mut session,
                    EditRequest::AddMediaAtEnd {
                        media_id: *media_id,
                    },
                )
                .unwrap()
                .created_clip_ids
            })
            .collect();
        (session, clip_ids)
    }

    fn clip_start_and_end(session: &EditSession, clip_id: ClipId) -> (Timestamp, Timestamp) {
        let (_, clip) = session.project().timeline().find_clip(clip_id).unwrap();
        (clip.timeline_start, clip.timeline_end())
    }

    #[test]
    fn a_drop_goes_where_it_is_dropped_or_on_a_new_track_on_top() {
        let (mut session, clip_ids) = arranged_session();
        let video_track = session
            .project()
            .timeline()
            .find_clip(clip_ids[0])
            .unwrap()
            .0
            .id;
        let media_id = session.project().media_assets()[0].id;

        let dropped = apply(
            &mut session,
            EditRequest::AddMediaAt {
                media_id,
                track_id: Some(video_track),
                start_ticks: seconds(10),
            },
        )
        .unwrap();
        assert_eq!(
            clip_start_and_end(&session, dropped.created_clip_ids[0]).0,
            Timestamp::from_whole_seconds(10)
        );

        let on_new_track = apply(
            &mut session,
            EditRequest::AddMediaAt {
                media_id,
                track_id: None,
                start_ticks: seconds(1),
            },
        )
        .unwrap();
        let tracks = session.project().timeline().tracks();
        assert_eq!(tracks.len(), 3);
        let top_track = tracks.last().unwrap();
        assert_eq!(top_track.kind, TrackKind::Video);
        assert_eq!(top_track.clips()[0].id, on_new_track.created_clip_ids[0]);

        // One undo removes both the clip and the track it created.
        session.undo().unwrap();
        assert_eq!(session.project().timeline().tracks().len(), 2);
    }

    #[test]
    fn a_drop_over_another_clip_is_refused_and_changes_nothing() {
        let (mut session, clip_ids) = arranged_session();
        let video_track = session
            .project()
            .timeline()
            .find_clip(clip_ids[0])
            .unwrap()
            .0
            .id;
        let before = session.project().clone();
        let media_id = session.project().media_assets()[0].id;
        let refused = apply(
            &mut session,
            EditRequest::AddMediaAt {
                media_id,
                track_id: Some(video_track),
                start_ticks: seconds(2),
            },
        );
        assert!(refused.is_err());
        assert_eq!(session.project(), &before);
    }

    #[test]
    fn clips_move_along_their_track_and_to_a_new_track() {
        let (mut session, clip_ids) = arranged_session();
        apply(
            &mut session,
            EditRequest::MoveClip {
                clip_id: clip_ids[1],
                track_id: None,
                start_ticks: seconds(20),
            },
        )
        .unwrap();
        assert_eq!(session.next_undo_label(), Some("moveClip"));
        let (track, clip) = session.project().timeline().find_clip(clip_ids[1]).unwrap();
        assert_eq!(clip.timeline_start, Timestamp::from_whole_seconds(20));
        assert_eq!(
            track.id,
            session.project().timeline().tracks().last().unwrap().id
        );

        // Moving back over the first clip is refused.
        let first_track = session
            .project()
            .timeline()
            .find_clip(clip_ids[0])
            .unwrap()
            .0
            .id;
        let refused = apply(
            &mut session,
            EditRequest::MoveClip {
                clip_id: clip_ids[1],
                track_id: Some(first_track),
                start_ticks: seconds(1),
            },
        );
        assert!(refused.is_err());
    }

    #[test]
    fn trimming_moves_one_edge_and_stays_within_the_file() {
        let (mut session, clip_ids) = arranged_session();
        apply(
            &mut session,
            EditRequest::TrimClip {
                clip_id: clip_ids[1],
                edge: TrimEdge::End,
                time_ticks: seconds(5),
            },
        )
        .unwrap();
        assert_eq!(
            clip_start_and_end(&session, clip_ids[1]),
            (
                Timestamp::from_whole_seconds(3),
                Timestamp::from_whole_seconds(5)
            )
        );
        apply(
            &mut session,
            EditRequest::TrimClip {
                clip_id: clip_ids[1],
                edge: TrimEdge::Start,
                time_ticks: seconds(4),
            },
        )
        .unwrap();
        let (_, clip) = session.project().timeline().find_clip(clip_ids[1]).unwrap();
        assert_eq!(clip.timeline_start, Timestamp::from_whole_seconds(4));
        let ClipContent::Media(segment) = &clip.content else {
            panic!("a video clip shows a media segment");
        };
        assert_eq!(segment.source_start, Timestamp::from_whole_seconds(1));
        assert_eq!(session.next_undo_label(), Some("trimClip"));

        // The end cannot go past the end of the 4 s file.
        let too_far = apply(
            &mut session,
            EditRequest::TrimClip {
                clip_id: clip_ids[1],
                edge: TrimEdge::End,
                time_ticks: seconds(30),
            },
        );
        assert!(too_far.is_err());
    }

    #[test]
    fn splitting_cuts_the_chosen_clips_or_every_clip_under_the_time() {
        let (mut session, clip_ids) = arranged_session();

        // Nothing given: every clip under 2 s (the first video and the sound).
        let outcome = apply(
            &mut session,
            EditRequest::SplitClips {
                clip_ids: vec![],
                time_ticks: seconds(2),
            },
        )
        .unwrap();
        assert_eq!(outcome.created_clip_ids.len(), 2);
        assert_eq!(session.next_undo_label(), Some("splitClip"));
        assert_eq!(session.project().timeline().all_clips().count(), 5);

        // Only the chosen clip, and a time on its edge cuts nothing.
        let outcome = apply(
            &mut session,
            EditRequest::SplitClips {
                clip_ids: vec![clip_ids[1]],
                time_ticks: seconds(5),
            },
        )
        .unwrap();
        assert_eq!(outcome.created_clip_ids.len(), 1);
        let on_an_edge = apply(
            &mut session,
            EditRequest::SplitClips {
                clip_ids: vec![clip_ids[1]],
                time_ticks: seconds(3),
            },
        )
        .unwrap();
        assert!(on_an_edge.created_clip_ids.is_empty());
        assert_eq!(session.project().timeline().all_clips().count(), 6);

        // One undo puts the first cut's two clips back together.
        session.undo().unwrap();
        session.undo().unwrap();
        assert_eq!(session.project().timeline().all_clips().count(), 3);
    }

    #[test]
    fn deleting_clips_is_one_undo_step_and_leaves_the_gap() {
        let (mut session, clip_ids) = arranged_session();
        apply(
            &mut session,
            EditRequest::RemoveClips {
                clip_ids: vec![clip_ids[0], clip_ids[2]],
            },
        )
        .unwrap();
        assert_eq!(session.project().timeline().all_clips().count(), 1);
        assert_eq!(
            clip_start_and_end(&session, clip_ids[1]).0,
            Timestamp::from_whole_seconds(3)
        );
        assert_eq!(session.next_undo_label(), Some("removeClips"));
        session.undo().unwrap();
        assert_eq!(session.project().timeline().all_clips().count(), 3);
    }

    #[test]
    fn tracks_can_be_muted_hidden_and_removed() {
        let (mut session, clip_ids) = arranged_session();
        let video_track = session
            .project()
            .timeline()
            .find_clip(clip_ids[0])
            .unwrap()
            .0
            .id;
        apply(
            &mut session,
            EditRequest::SetTrackMuted {
                track_id: video_track,
                is_muted: true,
            },
        )
        .unwrap();
        assert_eq!(session.next_undo_label(), Some("muteTrack"));
        apply(
            &mut session,
            EditRequest::SetTrackHidden {
                track_id: video_track,
                is_hidden: true,
            },
        )
        .unwrap();
        assert_eq!(session.next_undo_label(), Some("hideTrack"));
        let track = session.project().timeline().track(video_track).unwrap();
        assert!(track.is_muted && track.is_hidden);

        // The interface offers removal on empty tracks only; the engine takes the
        // clips along, in one step that undo brings back.
        apply(
            &mut session,
            EditRequest::RemoveTrack {
                track_id: video_track,
            },
        )
        .unwrap();
        assert_eq!(session.next_undo_label(), Some("removeTrack"));
        assert_eq!(session.project().timeline().tracks().len(), 1);
        session.undo().unwrap();
        assert_eq!(session.project().timeline().all_clips().count(), 3);
    }

    #[test]
    fn every_request_reads_from_the_front_end_json() {
        let requests = [
            r#"{ "type": "addMediaAt", "mediaId": 1, "trackId": null, "startTicks": 0 }"#,
            r#"{ "type": "moveClip", "clipId": 2, "trackId": 3, "startTicks": 10 }"#,
            r#"{ "type": "trimClip", "clipId": 2, "edge": "start", "timeTicks": 10 }"#,
            r#"{ "type": "splitClips", "clipIds": [], "timeTicks": 10 }"#,
            r#"{ "type": "removeClips", "clipIds": [2, 4] }"#,
            r#"{ "type": "setTrackMuted", "trackId": 3, "isMuted": true }"#,
            r#"{ "type": "setTrackHidden", "trackId": 3, "isHidden": false }"#,
            r#"{ "type": "removeTrack", "trackId": 3 }"#,
        ];
        for json in requests {
            assert!(
                serde_json::from_str::<EditRequest>(json).is_ok(),
                "could not read {json}"
            );
        }
        let trim: EditRequest = serde_json::from_str(requests[2]).unwrap();
        assert!(matches!(
            trim,
            EditRequest::TrimClip {
                edge: TrimEdge::Start,
                ..
            }
        ));
    }
}
