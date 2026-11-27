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
use kadr_core::ids::{ClipId, EffectId, MediaAssetId, TrackId};
use kadr_core::media_asset::{MediaAssetDescription, MediaKind, PictureSize};
use kadr_core::project::{Project, ProjectSettings};
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_core::timeline::TrackKind;
use kadr_engine::export_presets::refit_clips_to_canvas;
use serde::{Deserialize, Serialize};

mod arrangement;
mod blur_settings;
mod clip_settings;
mod keyframes;
mod silences;
mod subtitles;
mod titles;

pub use arrangement::TrimEdge;
pub use blur_settings::{
    BlurChanges, BlurSettings, BlurShapeChoice, BlurStyleChoice, tracked_blur_of,
};
pub use clip_settings::{
    ClipValue, ColorChanges, CropChanges, LutChanges, TextAlignmentChoice, TextChanges,
};
pub use keyframes::KeyframeCurve;
pub use silences::TickRange;
pub use subtitles::{
    BackdropChoice, CueChanges, NEW_CUE_DURATION, PlacementChoice, SUBTITLES_TRACK_NAME,
    SubtitleStyleChanges, import_subtitles,
};
pub use titles::{TITLE_DURATION, TITLES_TRACK_NAME, TitleStyle};

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
    /// Sets one value of a clip (inspector number fields and sliders). When the
    /// value is animated, sets its keyframe at `time_ticks` (the playhead),
    /// adding one there if needed.
    SetClipValue {
        /// The clip.
        clip_id: ClipId,
        /// Which value.
        value: ClipValue,
        /// The new value, in the value's own unit (multiplier, pixels, degrees).
        amount: f32,
        /// The playhead, on the timeline; used only when the value is animated.
        time_ticks: Option<u64>,
    },
    /// Adds a keyframe at `time_ticks` (the playhead) holding the value the
    /// property has there. The first one turns the animation on.
    AddKeyframe {
        /// The clip.
        clip_id: ClipId,
        /// Which value.
        value: ClipValue,
        /// Where, on the timeline.
        time_ticks: u64,
    },
    /// Moves keyframes in time: those of `values` at `from_offset_ticks`, or every
    /// keyframe there when `values` is empty.
    MoveKeyframe {
        /// The clip.
        clip_id: ClipId,
        /// Which values (empty: all of those with a keyframe there).
        values: Vec<ClipValue>,
        /// Where the keyframes are, from the clip's start.
        from_offset_ticks: u64,
        /// Where they go, from the clip's start.
        to_offset_ticks: u64,
    },
    /// Changes the curve from a keyframe to the next one.
    SetKeyframeCurve {
        /// The clip.
        clip_id: ClipId,
        /// Which value.
        value: ClipValue,
        /// The keyframe, from the clip's start.
        offset_ticks: u64,
        /// The new curve.
        curve: KeyframeCurve,
    },
    /// Removes keyframes: those of `values` at `offset_ticks`, or every keyframe
    /// there when `values` is empty. A value losing its last keyframe keeps its
    /// value.
    RemoveKeyframe {
        /// The clip.
        clip_id: ClipId,
        /// Which values (empty: all of those with a keyframe there).
        values: Vec<ClipValue>,
        /// The keyframes, from the clip's start.
        offset_ticks: u64,
    },
    /// Crops a clip's picture: the sides given change, the others keep the value
    /// they have in the project when the request is applied.
    SetClipCrop {
        /// The clip.
        clip_id: ClipId,
        /// The sides to change.
        changes: CropChanges,
    },
    /// Puts a clip's picture settings back to their defaults.
    ResetClipPicture {
        /// The clip.
        clip_id: ClipId,
    },
    /// Changes a clip's speed (its length changes with it).
    SetClipSpeed {
        /// The clip.
        clip_id: ClipId,
        /// Multiplier: 0.5 slower, 2 faster.
        speed: f64,
    },
    /// Changes a clip's colour adjustment (added on the first change): the
    /// settings given change, the others keep the value they have in the project
    /// when the request is applied.
    SetClipColor {
        /// The clip.
        clip_id: ClipId,
        /// The settings to change.
        changes: ColorChanges,
    },
    /// Removes a clip's colour adjustment and its colour LUT.
    ResetClipColor {
        /// The clip.
        clip_id: ClipId,
    },
    /// Changes a clip's colour LUT: the settings given change, the others keep the
    /// value they have in the project when the request is applied. The front end
    /// only sends the strength: a file is chosen with `choose_clip_lut_file`.
    SetClipLut {
        /// The clip.
        clip_id: ClipId,
        /// The settings to change.
        changes: LutChanges,
    },
    /// Removes a clip's colour LUT.
    RemoveClipLut {
        /// The clip.
        clip_id: ClipId,
    },
    /// Changes a title's words or style: the settings given change, the others
    /// keep the value they have in the project when the request is applied.
    SetClipText {
        /// The title.
        clip_id: ClipId,
        /// The settings to change.
        changes: TextChanges,
    },
    /// Adds a title at `start_ticks` on a *Titles* track.
    AddTitle {
        /// Where it starts.
        start_ticks: u64,
        /// Its words.
        text: String,
        /// One of the ready-made styles.
        style: TitleStyle,
    },
    /// Changes a subtitle cue's words or times: the ones given change, the
    /// others keep the value they have in the project when the request is applied.
    SetSubtitleCue {
        /// The cue.
        clip_id: ClipId,
        /// What changes.
        changes: CueChanges,
    },
    /// Adds a cue at `start_ticks` (two seconds long, less if the next cue
    /// starts sooner) on `track_id`, the top subtitle track, or a new one.
    AddSubtitleCue {
        /// Where it starts.
        start_ticks: u64,
        /// The subtitle track, or `None` for the top one.
        track_id: Option<TrackId>,
        /// Its words.
        text: String,
    },
    /// Merges a cue with the next cue of its track.
    MergeSubtitleWithNext {
        /// The first cue.
        clip_id: ClipId,
    },
    /// Changes some settings of a subtitle track's style: only the ones named.
    SetSubtitleStyle {
        /// The subtitle track.
        track_id: TrackId,
        /// The settings that change.
        changes: SubtitleStyleChanges,
    },
    /// Cuts parts out of a clip and closes the gaps (the silences the silence
    /// dialog found), moving the later clips of its track earlier.
    RemoveSilences {
        /// The clip.
        clip_id: ClipId,
        /// The parts to cut, on the timeline as the clip is now.
        cuts: Vec<TickRange>,
    },
    /// Changes the canvas size (an export preset of another shape, for example).
    /// The format is no longer automatic afterwards.
    SetCanvasSize {
        /// New width in pixels (made even, within the engine's limits).
        width: u32,
        /// New height in pixels (made even, within the engine's limits).
        height: u32,
    },
    /// Changes some of a tracked blur's settings (style, strength, mask colour,
    /// shape, margin): only the ones named.
    SetBlurSettings {
        /// The clip.
        clip_id: ClipId,
        /// The blur.
        effect_id: EffectId,
        /// The settings that change.
        changes: BlurChanges,
    },
    /// Turns a tracked blur off (kept, not drawn) or back on.
    SetBlurEnabled {
        /// The clip.
        clip_id: ClipId,
        /// The blur.
        effect_id: EffectId,
        /// `false` to turn it off.
        is_enabled: bool,
    },
    /// Removes a tracked blur.
    RemoveBlur {
        /// The clip.
        clip_id: ClipId,
        /// The blur.
        effect_id: EffectId,
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
    /// An `.srt` file was imported.
    SubtitlesImported {
        /// How many cues it gave.
        cue_count: usize,
        /// What was changed to read it (in English, from the engine).
        warnings: Vec<String>,
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
        EditRequest::SetClipValue {
            clip_id,
            value,
            amount,
            time_ticks,
        } => clip_settings::set_clip_value(
            session,
            *clip_id,
            *value,
            *amount,
            time_ticks.map(Timestamp::from_ticks),
        ),
        EditRequest::AddKeyframe {
            clip_id,
            value,
            time_ticks,
        } => keyframes::add_keyframe(
            session,
            *clip_id,
            *value,
            Timestamp::from_ticks(*time_ticks),
        ),
        EditRequest::MoveKeyframe {
            clip_id,
            values,
            from_offset_ticks,
            to_offset_ticks,
        } => keyframes::move_keyframes(
            session,
            *clip_id,
            values,
            TimeSpan::from_ticks(*from_offset_ticks),
            TimeSpan::from_ticks(*to_offset_ticks),
        ),
        EditRequest::SetKeyframeCurve {
            clip_id,
            value,
            offset_ticks,
            curve,
        } => keyframes::set_keyframe_curve(
            session,
            *clip_id,
            *value,
            TimeSpan::from_ticks(*offset_ticks),
            *curve,
        ),
        EditRequest::RemoveKeyframe {
            clip_id,
            values,
            offset_ticks,
        } => keyframes::remove_keyframes(
            session,
            *clip_id,
            values,
            TimeSpan::from_ticks(*offset_ticks),
        ),
        EditRequest::SetClipCrop { clip_id, changes } => {
            clip_settings::set_clip_crop(session, *clip_id, *changes)
        }
        EditRequest::ResetClipPicture { clip_id } => {
            clip_settings::reset_clip_picture(session, *clip_id)
        }
        EditRequest::SetClipSpeed { clip_id, speed } => {
            clip_settings::set_clip_speed(session, *clip_id, *speed)
        }
        EditRequest::SetClipColor { clip_id, changes } => {
            clip_settings::set_clip_color(session, *clip_id, *changes)
        }
        EditRequest::ResetClipColor { clip_id } => {
            clip_settings::reset_clip_color(session, *clip_id)
        }
        EditRequest::SetClipLut { clip_id, changes } => {
            clip_settings::set_clip_lut(session, *clip_id, changes)
        }
        EditRequest::RemoveClipLut { clip_id } => clip_settings::remove_clip_lut(session, *clip_id),
        EditRequest::SetClipText { clip_id, changes } => {
            clip_settings::set_clip_text(session, *clip_id, changes)
        }
        EditRequest::AddTitle {
            start_ticks,
            text,
            style,
        } => titles::add_title(session, Timestamp::from_ticks(*start_ticks), text, *style),
        EditRequest::RemoveSilences { clip_id, cuts } => {
            silences::remove_silences(session, *clip_id, cuts)
        }
        EditRequest::SetSubtitleCue { clip_id, changes } => {
            subtitles::set_subtitle_cue(session, *clip_id, changes)
        }
        EditRequest::AddSubtitleCue {
            start_ticks,
            track_id,
            text,
        } => subtitles::add_subtitle_cue(
            session,
            Timestamp::from_ticks(*start_ticks),
            *track_id,
            text,
        ),
        EditRequest::MergeSubtitleWithNext { clip_id } => {
            subtitles::merge_subtitle_with_next(session, *clip_id)
        }
        EditRequest::SetSubtitleStyle { track_id, changes } => {
            subtitles::set_subtitle_style(session, *track_id, changes)
        }
        EditRequest::SetCanvasSize { width, height } => {
            set_canvas_size(session, adopts_first_video_format, *width, *height)
        }
        EditRequest::SetBlurSettings {
            clip_id,
            effect_id,
            changes,
        } => blur_settings::set_blur_settings(session, *clip_id, *effect_id, changes),
        EditRequest::SetBlurEnabled {
            clip_id,
            effect_id,
            is_enabled,
        } => blur_settings::set_blur_enabled(session, *clip_id, *effect_id, *is_enabled),
        EditRequest::RemoveBlur { clip_id, effect_id } => {
            blur_settings::remove_blur(session, *clip_id, *effect_id)
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

/// Changes the canvas size as one undo step ("changeFormat"), refitting titles.
fn set_canvas_size(
    session: &mut EditSession,
    adopts_first_video_format: &mut bool,
    width: u32,
    height: u32,
) -> Result<EditOutcome, EditError> {
    let mut settings = *session.project().settings();
    let old_size = settings.canvas_size;
    settings.canvas_size = PictureSize {
        width_in_pixels: usable_canvas_side(width),
        height_in_pixels: usable_canvas_side(height),
    };
    session.transaction("changeFormat", |transaction| {
        transaction.execute(ChangeProjectSettings::new(settings))?;
        // Titles keep their place on the picture; videos are fitted anyway.
        refit_clips_to_canvas(transaction, old_size, settings.canvas_size)?;
        Ok(EditOutcome::default())
    })?;
    *adopts_first_video_format = false;
    Ok(EditOutcome::default())
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
    use kadr_core::clip::{AnimatableProperty, ClipContent};
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
    fn removing_silences_closes_the_clip_up_in_one_step_and_keeps_its_id() {
        let (mut session, clip_ids) = arranged_session();
        let request: EditRequest = serde_json::from_value(serde_json::json!({
            "type": "removeSilences",
            "clipId": clip_ids[0].raw(),
            "cuts": [
                { "startTicks": seconds(1), "endTicks": seconds(2) },
            ],
        }))
        .unwrap();

        let outcome = apply(&mut session, request).unwrap();

        assert!(outcome.created_clip_ids.is_empty(), "the selection stays");
        assert_eq!(session.next_undo_label(), Some("removeSilences"));
        assert_eq!(
            clip_start_and_end(&session, clip_ids[0]),
            (Timestamp::ZERO, Timestamp::from_whole_seconds(1))
        );
        assert_eq!(
            clip_start_and_end(&session, clip_ids[1]).0,
            Timestamp::from_whole_seconds(2),
            "the next video of the track moved 1 s earlier"
        );
        assert_eq!(
            clip_start_and_end(&session, clip_ids[2]).0,
            Timestamp::ZERO,
            "the sound on its own track did not move"
        );

        let empty_cut = EditRequest::RemoveSilences {
            clip_id: clip_ids[1],
            cuts: vec![TickRange {
                start_ticks: seconds(3),
                end_ticks: seconds(3),
            }],
        };
        assert!(apply(&mut session, empty_cut).is_err());
        session.undo().unwrap();
        assert_eq!(
            clip_start_and_end(&session, clip_ids[1]).0,
            Timestamp::from_whole_seconds(3)
        );
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
    fn the_canvas_size_changes_in_one_undo_step_and_stops_being_automatic() {
        let (mut session, _) = arranged_session();
        let mut adopts = true;
        apply_edit_request(
            &mut session,
            &mut adopts,
            &EditRequest::SetCanvasSize {
                width: 1080,
                height: 1921,
            },
        )
        .unwrap();
        let canvas_size = session.project().settings().canvas_size;
        assert_eq!(
            (canvas_size.width_in_pixels, canvas_size.height_in_pixels),
            (1080, 1920)
        );
        assert!(!adopts);
        assert_eq!(session.next_undo_label(), Some("changeFormat"));
        session.undo().unwrap();
        assert_eq!(
            session.project().settings(),
            &ProjectSettings::landscape_1080p()
        );
    }

    fn clip_of(session: &EditSession, clip_id: ClipId) -> kadr_core::clip::Clip {
        session
            .project()
            .timeline()
            .find_clip(clip_id)
            .unwrap()
            .1
            .clone()
    }

    #[test]
    fn inspector_values_change_one_step_at_a_time_and_reset() {
        let (mut session, clip_ids) = arranged_session();
        let clip_id = clip_ids[0];
        for (value, amount) in [
            (ClipValue::Opacity, 0.5),
            (ClipValue::Scale, 1.5),
            (ClipValue::PositionX, -120.0),
        ] {
            apply(
                &mut session,
                EditRequest::SetClipValue {
                    clip_id,
                    value,
                    amount,
                    time_ticks: None,
                },
            )
            .unwrap();
        }
        assert_eq!(session.next_undo_label(), Some("changePosition"));
        let clip = clip_of(&session, clip_id);
        assert!(
            (clip
                .animated_property(AnimatableProperty::Opacity)
                .constant_value
                - 0.5)
                .abs()
                < 0.001
        );
        apply(
            &mut session,
            EditRequest::SetClipCrop {
                clip_id,
                changes: CropChanges {
                    left: Some(0.1),
                    right: Some(0.1),
                    ..CropChanges::default()
                },
            },
        )
        .unwrap();
        apply(&mut session, EditRequest::ResetClipPicture { clip_id }).unwrap();
        assert_eq!(session.next_undo_label(), Some("resetPicture"));
        let clip = clip_of(&session, clip_id);
        assert!(
            (clip
                .animated_property(AnimatableProperty::Scale)
                .constant_value
                - 1.0)
                .abs()
                < 0.001
        );
        assert_eq!(
            clip.animated_property(AnimatableProperty::PositionX)
                .constant_value,
            0.0
        );
    }

    #[test]
    fn a_faster_clip_gets_shorter_and_a_longer_one_may_not_overlap() {
        let (mut session, clip_ids) = arranged_session();
        apply(
            &mut session,
            EditRequest::SetClipSpeed {
                clip_id: clip_ids[1],
                speed: 2.0,
            },
        )
        .unwrap();
        assert_eq!(
            clip_of(&session, clip_ids[1]).timeline_duration(),
            TimeSpan::from_whole_seconds(2)
        );
        assert_eq!(session.next_undo_label(), Some("changeSpeed"));
        // The first clip (0–3 s) is followed by the second at 3 s: slower would overlap.
        let refused = apply(
            &mut session,
            EditRequest::SetClipSpeed {
                clip_id: clip_ids[0],
                speed: 0.5,
            },
        );
        assert!(refused.is_err());
    }

    #[test]
    fn the_colour_adjustment_is_added_once_replaced_then_removed() {
        let (mut session, clip_ids) = arranged_session();
        let clip_id = clip_ids[0];
        let exposure = |exposure| EditRequest::SetClipColor {
            clip_id,
            changes: ColorChanges {
                exposure: Some(exposure),
                ..ColorChanges::default()
            },
        };
        apply(&mut session, exposure(0.5)).unwrap();
        // The settings not named start neutral.
        let mut expected = kadr_core::effect::ColorAdjustment {
            exposure_in_stops: 0.5,
            ..kadr_core::effect::ColorAdjustment::NEUTRAL
        };
        assert_eq!(color_adjustment_of(&session, clip_id), expected);
        apply(&mut session, exposure(-0.5)).unwrap();
        assert_eq!(clip_of(&session, clip_id).effects.len(), 1);
        expected.exposure_in_stops = -0.5;
        assert_eq!(color_adjustment_of(&session, clip_id), expected);
        apply(&mut session, EditRequest::ResetClipColor { clip_id }).unwrap();
        assert!(clip_of(&session, clip_id).effects.is_empty());
        assert_eq!(session.next_undo_label(), Some("resetColor"));
    }

    fn color_lut_of(session: &EditSession, clip_id: ClipId) -> Option<kadr_core::effect::ColorLut> {
        clip_of(session, clip_id)
            .effects
            .iter()
            .find_map(|effect| match &effect.kind {
                kadr_core::effect::EffectKind::ColorLut(color_lut) => Some(color_lut.clone()),
                _ => None,
            })
    }

    fn choose_lut(clip_id: ClipId, file_path: &str) -> EditRequest {
        EditRequest::SetClipLut {
            clip_id,
            changes: LutChanges {
                file_path: Some(file_path.into()),
                strength: None,
            },
        }
    }

    fn lut_strength(clip_id: ClipId, strength: f32) -> EditRequest {
        EditRequest::SetClipLut {
            clip_id,
            changes: LutChanges {
                strength: Some(strength),
                ..LutChanges::default()
            },
        }
    }

    #[test]
    fn the_lut_is_chosen_its_strength_changed_then_removed() {
        let (mut session, clip_ids) = arranged_session();
        let clip_id = clip_ids[0];

        apply(&mut session, choose_lut(clip_id, "C:/looks/film.cube")).unwrap();
        assert_eq!(session.next_undo_label(), Some("chooseLut"));
        let lut = color_lut_of(&session, clip_id).unwrap();
        assert_eq!(
            lut.file_path,
            std::path::PathBuf::from("C:/looks/film.cube")
        );
        assert_eq!(lut.strength_fraction, 1.0);

        apply(&mut session, lut_strength(clip_id, 0.5)).unwrap();
        assert_eq!(session.next_undo_label(), Some("changeLutStrength"));
        apply(&mut session, choose_lut(clip_id, "C:/looks/teal.cube")).unwrap();
        let lut = color_lut_of(&session, clip_id).unwrap();
        assert_eq!(
            (lut.file_path, lut.strength_fraction),
            ("C:/looks/teal.cube".into(), 0.5),
            "another file keeps the strength"
        );
        assert_eq!(clip_of(&session, clip_id).effects.len(), 1);
        assert!(apply(&mut session, lut_strength(clip_id, 1.5)).is_err());

        apply(&mut session, EditRequest::RemoveClipLut { clip_id }).unwrap();
        assert_eq!(session.next_undo_label(), Some("removeLut"));
        assert_eq!(color_lut_of(&session, clip_id), None);
        // A strength for a LUT removed meanwhile (by Claude) changes nothing.
        apply(&mut session, lut_strength(clip_id, 0.3)).unwrap();
        assert_eq!(color_lut_of(&session, clip_id), None);
        assert_eq!(session.next_undo_label(), Some("removeLut"));
    }

    #[test]
    fn resetting_the_colour_also_removes_the_lut() {
        let (mut session, clip_ids) = arranged_session();
        let clip_id = clip_ids[0];
        apply(&mut session, choose_lut(clip_id, "C:/looks/film.cube")).unwrap();
        apply(
            &mut session,
            EditRequest::SetClipColor {
                clip_id,
                changes: ColorChanges {
                    saturation: Some(0.5),
                    ..ColorChanges::default()
                },
            },
        )
        .unwrap();
        apply(&mut session, EditRequest::ResetClipColor { clip_id }).unwrap();
        assert!(clip_of(&session, clip_id).effects.is_empty());
        session.undo().unwrap();
        assert_eq!(clip_of(&session, clip_id).effects.len(), 2, "one undo step");
    }

    #[test]
    fn quick_lut_changes_each_keep_the_other() {
        let (mut session, clip_ids) = arranged_session();
        let clip_id = clip_ids[0];
        apply(&mut session, choose_lut(clip_id, "C:/looks/film.cube")).unwrap();
        // A strength, then another file chosen before the first answer arrived.
        apply(&mut session, lut_strength(clip_id, 0.4)).unwrap();
        apply(&mut session, choose_lut(clip_id, "C:/looks/teal.cube")).unwrap();
        let lut = color_lut_of(&session, clip_id).unwrap();
        assert_eq!(
            (lut.file_path, lut.strength_fraction),
            ("C:/looks/teal.cube".into(), 0.4)
        );
    }

    #[test]
    fn titles_go_on_titles_tracks_and_their_text_can_change() {
        let (mut session, _) = arranged_session();
        let first = apply(
            &mut session,
            EditRequest::AddTitle {
                start_ticks: seconds(1),
                text: "Lisbonne".to_string(),
                style: TitleStyle::Subtitle,
            },
        )
        .unwrap()
        .created_clip_ids[0];
        assert_eq!(session.next_undo_label(), Some("addTitle"));
        let tracks = session.project().timeline().tracks();
        assert_eq!(tracks.last().unwrap().name, TITLES_TRACK_NAME);
        let title = clip_of(&session, first);
        assert_eq!(title.timeline_duration(), TITLE_DURATION);
        assert!(
            title
                .animated_property(AnimatableProperty::PositionY)
                .constant_value
                > 0.0
        );

        // A second title over the first goes on a new Titles track.
        apply(
            &mut session,
            EditRequest::AddTitle {
                start_ticks: seconds(2),
                text: "Jour 1".to_string(),
                style: TitleStyle::BigWord,
            },
        )
        .unwrap();
        let titles_tracks = session
            .project()
            .timeline()
            .tracks()
            .iter()
            .filter(|track| track.name == TITLES_TRACK_NAME)
            .count();
        assert_eq!(titles_tracks, 2);

        apply(
            &mut session,
            EditRequest::SetClipText {
                clip_id: first,
                changes: TextChanges {
                    text: Some("Lisboa".to_string()),
                    font_family: Some("Inter".to_string()),
                    font_size: Some(72.0),
                    color: Some("not a colour".to_string()),
                    alignment: Some(TextAlignmentChoice::Left),
                },
            },
        )
        .unwrap();
        let text = text_of(&session, first);
        assert_eq!(text.text, "Lisboa");
        assert!((text.style.font_size_in_pixels - 72.0).abs() < 0.001);
        // An unreadable colour keeps the current one.
        assert_eq!(
            text.style.color,
            kadr_core::clip::TextStyle::default().color
        );
    }

    fn text_of(session: &EditSession, clip_id: ClipId) -> kadr_core::clip::TextContent {
        match clip_of(session, clip_id).content {
            ClipContent::Text(text) => text,
            ClipContent::Media(_) | ClipContent::Subtitle(_) => panic!("a title is a text clip"),
        }
    }

    fn color_adjustment_of(
        session: &EditSession,
        clip_id: ClipId,
    ) -> kadr_core::effect::ColorAdjustment {
        clip_of(session, clip_id)
            .effects
            .iter()
            .find_map(|effect| match effect.kind {
                kadr_core::effect::EffectKind::ColorAdjustment(adjustment) => Some(adjustment),
                _ => None,
            })
            .expect("the clip has a colour adjustment")
    }

    // The inspector sends a change before the answer to the previous one has
    // arrived: each request names only what it changes, so neither puts back
    // what the other changed.
    #[test]
    fn quick_changes_to_a_title_each_keep_the_other() {
        let session_with_title = || {
            let (mut session, _) = arranged_session();
            let title = apply(
                &mut session,
                EditRequest::AddTitle {
                    start_ticks: 0,
                    text: "Titre".to_string(),
                    style: TitleStyle::Title,
                },
            )
            .unwrap()
            .created_clip_ids[0];
            (session, title)
        };
        let new_words = |clip_id| EditRequest::SetClipText {
            clip_id,
            changes: TextChanges {
                text: Some("Bonjour Kadr".to_string()),
                ..TextChanges::default()
            },
        };
        let align_left = |clip_id| EditRequest::SetClipText {
            clip_id,
            changes: TextChanges {
                alignment: Some(TextAlignmentChoice::Left),
                ..TextChanges::default()
            },
        };

        // In either order, both changes stay.
        for words_first in [true, false] {
            let (mut session, title) = session_with_title();
            let style_before = text_of(&session, title).style;
            let (first, second) = if words_first {
                (new_words(title), align_left(title))
            } else {
                (align_left(title), new_words(title))
            };
            apply(&mut session, first).unwrap();
            apply(&mut session, second).unwrap();
            let text = text_of(&session, title);
            assert_eq!(text.text, "Bonjour Kadr");
            assert_eq!(text.style.alignment, kadr_core::clip::TextAlignment::Left);
            assert_eq!(text.style.font_family, style_before.font_family);
            assert_eq!(text.style.color, style_before.color);
        }

        // One undo step per change.
        let (mut session, title) = session_with_title();
        let style_before = text_of(&session, title).style;
        apply(&mut session, new_words(title)).unwrap();
        apply(&mut session, align_left(title)).unwrap();
        session.undo().unwrap();
        let text = text_of(&session, title);
        assert_eq!(text.text, "Bonjour Kadr");
        assert_eq!(text.style, style_before);
        session.undo().unwrap();
        assert_eq!(text_of(&session, title).text, "Titre");
    }

    #[test]
    fn quick_blur_changes_each_keep_the_other() {
        use kadr_core::commands::{AddEffect, CreatedObjectsExt};
        use kadr_core::effect::{BlurShape, EffectKind, TrackedBlur};

        let (mut session, clip_ids) = arranged_session();
        let clip_id = clip_ids[0];
        let effect_id = session
            .execute(AddEffect::new(
                clip_id,
                EffectKind::TrackedBlur(TrackedBlur::with_default_style()),
            ))
            .unwrap()
            .first_effect_id()
            .unwrap();
        let blur_of = |session: &EditSession| {
            BlurSettings::of(tracked_blur_of(session.project(), clip_id, effect_id).unwrap())
        };
        let before = blur_of(&session);
        // Sent before the answer to the first one arrived: neither names the other.
        for changes in [
            BlurChanges {
                margin_in_percent: Some(45.0),
                ..BlurChanges::default()
            },
            BlurChanges {
                shape: Some(BlurShapeChoice::Rectangle),
                ..BlurChanges::default()
            },
        ] {
            apply(
                &mut session,
                EditRequest::SetBlurSettings {
                    clip_id,
                    effect_id,
                    changes,
                },
            )
            .unwrap();
        }
        let after = blur_of(&session);
        assert_eq!(after.margin_in_percent, 45.0);
        assert_eq!(after.shape, BlurShapeChoice::Rectangle);
        assert_eq!(
            (after.style, after.strength),
            (before.style, before.strength)
        );
        assert_eq!(
            tracked_blur_of(session.project(), clip_id, effect_id)
                .unwrap()
                .shape,
            BlurShape::Rectangle
        );
        // One undo step per change.
        session.undo().unwrap();
        assert_eq!(blur_of(&session).margin_in_percent, 45.0);
        assert_eq!(blur_of(&session).shape, before.shape);
    }

    #[test]
    fn quick_colour_changes_each_keep_the_other() {
        let (mut session, clip_ids) = arranged_session();
        let clip_id = clip_ids[0];
        for changes in [
            ColorChanges {
                exposure: Some(0.5),
                ..ColorChanges::default()
            },
            ColorChanges {
                contrast: Some(1.2),
                ..ColorChanges::default()
            },
        ] {
            apply(&mut session, EditRequest::SetClipColor { clip_id, changes }).unwrap();
        }
        let both = kadr_core::effect::ColorAdjustment {
            exposure_in_stops: 0.5,
            contrast_multiplier: 1.2,
            ..kadr_core::effect::ColorAdjustment::NEUTRAL
        };
        assert_eq!(color_adjustment_of(&session, clip_id), both);
        session.undo().unwrap();
        assert_eq!(
            color_adjustment_of(&session, clip_id),
            kadr_core::effect::ColorAdjustment {
                contrast_multiplier: 1.0,
                ..both
            }
        );
    }

    #[test]
    fn quick_crop_changes_each_keep_the_other_sides() {
        let (mut session, clip_ids) = arranged_session();
        let clip_id = clip_ids[0];
        for changes in [
            CropChanges {
                left: Some(0.1),
                ..CropChanges::default()
            },
            CropChanges {
                bottom: Some(0.2),
                ..CropChanges::default()
            },
        ] {
            apply(&mut session, EditRequest::SetClipCrop { clip_id, changes }).unwrap();
        }
        let crop = clip_of(&session, clip_id).transform.crop;
        assert_eq!(
            crop,
            kadr_core::clip::CropInsets {
                left_fraction: 0.1,
                bottom_fraction: 0.2,
                ..kadr_core::clip::CropInsets::default()
            }
        );
        assert_eq!(session.next_undo_label(), Some("changeCrop"));
    }

    // ----- Keyframes (docs/ux.md, "Animate a clip") -----

    /// A 3 s title at 1 s, and its clip id.
    fn session_with_a_title() -> (EditSession, ClipId) {
        let (mut session, _) = arranged_session();
        let title = apply(
            &mut session,
            EditRequest::AddTitle {
                start_ticks: seconds(1),
                text: "Hello".to_string(),
                style: TitleStyle::Title,
            },
        )
        .unwrap()
        .created_clip_ids[0];
        (session, title)
    }

    fn keyframes_of(
        session: &EditSession,
        clip_id: ClipId,
        property: AnimatableProperty,
    ) -> Vec<kadr_core::animation::Keyframe> {
        clip_of(session, clip_id)
            .animated_property(property)
            .keyframes()
            .to_vec()
    }

    fn add_keyframe(session: &mut EditSession, clip_id: ClipId, value: ClipValue, at: u64) {
        apply(
            session,
            EditRequest::AddKeyframe {
                clip_id,
                value,
                time_ticks: at,
            },
        )
        .unwrap();
    }

    fn set_value_at(
        session: &mut EditSession,
        clip_id: ClipId,
        value: ClipValue,
        amount: f32,
        at: u64,
    ) {
        apply(
            session,
            EditRequest::SetClipValue {
                clip_id,
                value,
                amount,
                time_ticks: Some(at),
            },
        )
        .unwrap();
    }

    #[test]
    fn a_title_is_animated_by_mouse_and_each_gesture_is_one_undo_step() {
        use kadr_core::animation::Interpolation;

        let (mut session, title) = session_with_a_title();
        let start_x = clip_of(&session, title)
            .animated_property(AnimatableProperty::PositionX)
            .constant_value;

        // The first keyframe, at the playhead on the title's first frame, holds
        // the current value: nothing moves yet.
        add_keyframe(&mut session, title, ClipValue::PositionX, seconds(1));
        assert_eq!(session.next_undo_label(), Some("addKeyframe"));
        let keyframes = keyframes_of(&session, title, AnimatableProperty::PositionX);
        assert_eq!(keyframes.len(), 1);
        assert_eq!(keyframes[0].time_offset, TimeSpan::ZERO);
        assert_eq!(keyframes[0].value, start_x);
        // Adding it again changes nothing and makes no undo step.
        let step_count = session.undo_history_labels().len();
        add_keyframe(&mut session, title, ClipValue::PositionX, seconds(1));
        assert_eq!(session.undo_history_labels().len(), step_count);

        // At the start, the title comes from the left; a second later it is in place.
        set_value_at(
            &mut session,
            title,
            ClipValue::PositionX,
            -600.0,
            seconds(1),
        );
        assert_eq!(session.next_undo_label(), Some("changePosition"));
        set_value_at(&mut session, title, ClipValue::PositionX, 0.0, seconds(2));
        let keyframes = keyframes_of(&session, title, AnimatableProperty::PositionX);
        assert_eq!(
            keyframes.iter().map(|it| it.value).collect::<Vec<_>>(),
            [-600.0, 0.0]
        );

        // Ease out, then a hand-drawn curve.
        apply(
            &mut session,
            EditRequest::SetKeyframeCurve {
                clip_id: title,
                value: ClipValue::PositionX,
                offset_ticks: 0,
                curve: KeyframeCurve::EaseOut,
            },
        )
        .unwrap();
        assert_eq!(session.next_undo_label(), Some("changeKeyframeCurve"));
        let position_x = clip_of(&session, title).transform.position_x_in_pixels;
        assert_eq!(
            position_x.value_at(TimeSpan::from_milliseconds(500)),
            -150.0
        );
        let bezier = KeyframeCurve::Bezier {
            x1: 0.2,
            y1: 0.0,
            x2: 0.0,
            y2: 1.0,
        };
        apply(
            &mut session,
            EditRequest::SetKeyframeCurve {
                clip_id: title,
                value: ClipValue::PositionX,
                offset_ticks: 0,
                curve: bezier,
            },
        )
        .unwrap();
        let keyframes = keyframes_of(&session, title, AnimatableProperty::PositionX);
        assert!(matches!(
            keyframes[0].interpolation_to_next,
            Interpolation::CubicBezier(_)
        ));
        assert_eq!(
            KeyframeCurve::from(keyframes[0].interpolation_to_next),
            bezier
        );

        // Moving the arrival later keeps its value; undo puts it back.
        apply(
            &mut session,
            EditRequest::MoveKeyframe {
                clip_id: title,
                values: vec![ClipValue::PositionX],
                from_offset_ticks: TimeSpan::from_whole_seconds(1).ticks(),
                to_offset_ticks: TimeSpan::from_milliseconds(1_500).ticks(),
            },
        )
        .unwrap();
        assert_eq!(session.next_undo_label(), Some("moveKeyframe"));
        let keyframes = keyframes_of(&session, title, AnimatableProperty::PositionX);
        assert_eq!(keyframes[1].time_offset, TimeSpan::from_milliseconds(1_500));
        assert_eq!(keyframes[1].value, 0.0);
        session.undo().unwrap();
        let keyframes = keyframes_of(&session, title, AnimatableProperty::PositionX);
        assert_eq!(keyframes[1].time_offset, TimeSpan::from_whole_seconds(1));
    }

    #[test]
    fn a_mark_on_the_timeline_moves_and_removes_every_keyframe_at_its_time() {
        let (mut session, title) = session_with_a_title();
        for value in [ClipValue::Opacity, ClipValue::Scale] {
            add_keyframe(&mut session, title, value, seconds(1));
            add_keyframe(&mut session, title, value, seconds(2));
        }
        add_keyframe(&mut session, title, ClipValue::Rotation, seconds(3));
        apply(
            &mut session,
            EditRequest::MoveKeyframe {
                clip_id: title,
                values: Vec::new(),
                from_offset_ticks: TimeSpan::from_whole_seconds(1).ticks(),
                to_offset_ticks: TimeSpan::from_milliseconds(1_200).ticks(),
            },
        )
        .unwrap();
        for property in [AnimatableProperty::Opacity, AnimatableProperty::Scale] {
            let keyframes = keyframes_of(&session, title, property);
            assert_eq!(keyframes[1].time_offset, TimeSpan::from_milliseconds(1_200));
        }
        assert_eq!(
            keyframes_of(&session, title, AnimatableProperty::Rotation)[0].time_offset,
            TimeSpan::from_whole_seconds(2)
        );

        // Changing the opacity there, then removing the mark: each value keeps
        // the value it had, the opacity keeps its keyframe at the start.
        set_value_at(&mut session, title, ClipValue::Opacity, 0.0, seconds(1));
        apply(
            &mut session,
            EditRequest::RemoveKeyframe {
                clip_id: title,
                values: Vec::new(),
                offset_ticks: TimeSpan::from_milliseconds(1_200).ticks(),
            },
        )
        .unwrap();
        assert_eq!(session.next_undo_label(), Some("removeKeyframe"));
        assert_eq!(
            keyframes_of(&session, title, AnimatableProperty::Scale).len(),
            1
        );
        assert_eq!(
            keyframes_of(&session, title, AnimatableProperty::Opacity).len(),
            1
        );
        session.undo().unwrap();
        assert_eq!(
            keyframes_of(&session, title, AnimatableProperty::Scale).len(),
            2
        );
    }

    #[test]
    fn removing_the_last_keyframe_keeps_the_value_on_screen() {
        let (mut session, title) = session_with_a_title();
        add_keyframe(&mut session, title, ClipValue::Scale, seconds(1));
        set_value_at(&mut session, title, ClipValue::Scale, 1.4, seconds(1));
        apply(
            &mut session,
            EditRequest::RemoveKeyframe {
                clip_id: title,
                values: vec![ClipValue::Scale],
                offset_ticks: 0,
            },
        )
        .unwrap();
        let scale = clip_of(&session, title).transform.scale_factor;
        assert!(!scale.is_animated());
        assert!((scale.constant_value - 1.4).abs() < 1e-6);
    }

    #[test]
    fn keyframe_gestures_outside_the_clip_or_onto_another_are_refused() {
        let (mut session, title) = session_with_a_title();
        let history_length = session.undo_history_labels().len();
        let refused = [
            // The playhead before the title, then on its end (already the next moment).
            EditRequest::AddKeyframe {
                clip_id: title,
                value: ClipValue::Opacity,
                time_ticks: seconds(0),
            },
            EditRequest::AddKeyframe {
                clip_id: title,
                value: ClipValue::Opacity,
                time_ticks: seconds(4),
            },
        ];
        for request in refused {
            assert!(apply(&mut session, request).is_err());
        }
        add_keyframe(&mut session, title, ClipValue::Opacity, seconds(1));
        add_keyframe(&mut session, title, ClipValue::Opacity, seconds(2));
        let onto_another = apply(
            &mut session,
            EditRequest::MoveKeyframe {
                clip_id: title,
                values: vec![ClipValue::Opacity],
                from_offset_ticks: 0,
                to_offset_ticks: TimeSpan::from_whole_seconds(1).ticks(),
            },
        );
        assert!(matches!(
            onto_another,
            Err(EditError::KeyframeAlreadyThere { .. })
        ));
        let past_the_end = apply(
            &mut session,
            EditRequest::MoveKeyframe {
                clip_id: title,
                values: vec![ClipValue::Opacity],
                from_offset_ticks: 0,
                to_offset_ticks: TimeSpan::from_whole_seconds(4).ticks(),
            },
        );
        assert!(matches!(
            past_the_end,
            Err(EditError::KeyframeOutsideClip { .. })
        ));
        // An animated value with no playhead given is refused, not made constant.
        let without_time = apply(
            &mut session,
            EditRequest::SetClipValue {
                clip_id: title,
                value: ClipValue::Opacity,
                amount: 0.3,
                time_ticks: None,
            },
        );
        assert!(matches!(
            without_time,
            Err(EditError::PropertyIsAnimated { .. })
        ));
        assert_eq!(session.undo_history_labels().len(), history_length + 2);
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
            r#"{ "type": "setCanvasSize", "width": 1080, "height": 1920 }"#,
            r#"{ "type": "setClipValue", "clipId": 2, "value": "positionX", "amount": -40.5 }"#,
            r#"{ "type": "setClipCrop", "clipId": 2, "changes": { "left": 0.1, "bottom": 0.2 } }"#,
            r#"{ "type": "resetClipPicture", "clipId": 2 }"#,
            r#"{ "type": "setClipSpeed", "clipId": 2, "speed": 1.5 }"#,
            r#"{ "type": "setClipColor", "clipId": 2, "changes": { "exposure": 0.3, "contrast": 1, "saturation": 1.2, "temperature": -0.1 } }"#,
            r#"{ "type": "setClipColor", "clipId": 2, "changes": { "saturation": 1.2 } }"#,
            r#"{ "type": "resetClipColor", "clipId": 2 }"#,
            r#"{ "type": "setClipLut", "clipId": 2, "changes": { "strength": 0.5 } }"#,
            r#"{ "type": "removeClipLut", "clipId": 2 }"#,
            r##"{ "type": "setClipText", "clipId": 2, "changes": { "text": "Hi", "fontFamily": "Inter", "fontSize": 64, "color": "#ffffffff", "alignment": "center" } }"##,
            r#"{ "type": "setClipText", "clipId": 2, "changes": { "alignment": "right" } }"#,
            r#"{ "type": "addTitle", "startTicks": 0, "text": "Title", "style": "bigWord" }"#,
            r#"{ "type": "setSubtitleCue", "clipId": 2, "changes": { "text": "<i>Hi</i>" } }"#,
            r#"{ "type": "setSubtitleCue", "clipId": 2, "changes": { "startTicks": 10, "endTicks": 20 } }"#,
            r#"{ "type": "addSubtitleCue", "startTicks": 0, "trackId": null, "text": "New" }"#,
            r#"{ "type": "mergeSubtitleWithNext", "clipId": 2 }"#,
            r##"{ "type": "setSubtitleStyle", "trackId": 3, "changes": { "backdrop": "box", "backdropColor": "#000000B3", "placement": "top", "margin": 40, "maximumLineWidth": 0.9, "isBold": false } }"##,
            r#"{ "type": "setClipValue", "clipId": 2, "value": "opacity", "amount": 0.5, "timeTicks": 10 }"#,
            r#"{ "type": "addKeyframe", "clipId": 2, "value": "scale", "timeTicks": 10 }"#,
            r#"{ "type": "moveKeyframe", "clipId": 2, "values": [], "fromOffsetTicks": 0, "toOffsetTicks": 10 }"#,
            r#"{ "type": "setKeyframeCurve", "clipId": 2, "value": "positionX", "offsetTicks": 0, "curve": { "type": "easeOut" } }"#,
            r#"{ "type": "setKeyframeCurve", "clipId": 2, "value": "positionX", "offsetTicks": 0, "curve": { "type": "bezier", "x1": 0.2, "y1": 0, "x2": 0, "y2": 1 } }"#,
            r#"{ "type": "removeKeyframe", "clipId": 2, "values": ["opacity"], "offsetTicks": 0 }"#,
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
        let alignment_only: EditRequest = serde_json::from_str(
            r#"{ "type": "setClipText", "clipId": 2, "changes": { "alignment": "right" } }"#,
        )
        .unwrap();
        assert_eq!(
            alignment_only,
            EditRequest::SetClipText {
                clip_id: ClipId::from_raw(2),
                changes: TextChanges {
                    alignment: Some(TextAlignmentChoice::Right),
                    ..TextChanges::default()
                },
            }
        );
    }

    #[test]
    fn a_misspelt_setting_is_refused_rather_than_ignored() {
        for json in [
            r#"{ "type": "setClipText", "clipId": 2, "changes": { "alignement": "left" } }"#,
            r#"{ "type": "setClipColor", "clipId": 2, "changes": { "exposition": 1 } }"#,
            r#"{ "type": "setClipCrop", "clipId": 2, "changes": { "leftFraction": 0.1 } }"#,
            r#"{ "type": "setClipLut", "clipId": 2, "changes": { "intensity": 0.5 } }"#,
            // A file is only chosen through choose_clip_lut_file, which reads it first.
            r#"{ "type": "setClipLut", "clipId": 2, "changes": { "filePath": "C:/x.cube" } }"#,
            r#"{ "type": "setSubtitleCue", "clipId": 2, "changes": { "words": "Hi" } }"#,
            r#"{ "type": "setSubtitleStyle", "trackId": 3, "changes": { "backdrop": "shadow" } }"#,
        ] {
            assert!(
                serde_json::from_str::<EditRequest>(json).is_err(),
                "{json} was accepted"
            );
        }
    }
}
