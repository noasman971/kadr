//! Timeline tools: read the timeline, add and remove tracks, place, move, trim,
//! split and remove clips.

use kadr_core::commands::{
    AddClip, AddTrack, ClipEdge, CreatedObjectsExt, MoveClip, RemoveClip, RemoveTrack, SplitClip,
    TrackPosition, TrimClip,
};
use kadr_core::error::EditError;
use kadr_core::history::Transaction;
use kadr_core::ids::{ClipId, MediaAssetId, TrackId};
use kadr_core::media_asset::MediaKind;
use kadr_core::project::Project;
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_core::timeline::{Track, TrackKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{NoParameters, ToolEffect, ToolEntry, declare_tool};
use crate::output::{ToolError, ToolOutput};
use crate::time_input::{
    half_frame, positive_span_from_seconds, seconds_in, seconds_of, snap_to_nearby_clip_edge,
    timestamp_from_seconds,
};
use crate::toolbox::ToolCall;
use crate::views::{ClipView, SettingsView, TrackView, track_kind_name};

/// The timeline tools.
pub(crate) fn timeline_tools() -> Vec<ToolEntry> {
    vec![
        declare_tool(
            "get_timeline",
            "Show the timeline",
            GET_TIMELINE_DESCRIPTION,
            ToolEffect::ReadsOnly,
            get_timeline,
        ),
        declare_tool(
            "get_clip",
            "Show one clip",
            GET_CLIP_DESCRIPTION,
            ToolEffect::ReadsOnly,
            get_clip,
        ),
        declare_tool(
            "add_track",
            "Add a track",
            ADD_TRACK_DESCRIPTION,
            ToolEffect::ChangesProject,
            add_track,
        ),
        declare_tool(
            "remove_track",
            "Remove a track",
            REMOVE_TRACK_DESCRIPTION,
            ToolEffect::ChangesProject,
            remove_track,
        ),
        declare_tool(
            "add_clip",
            "Place media on the timeline",
            ADD_CLIP_DESCRIPTION,
            ToolEffect::ChangesProject,
            add_clip,
        ),
        declare_tool(
            "remove_clip",
            "Remove a clip",
            REMOVE_CLIP_DESCRIPTION,
            ToolEffect::ChangesProject,
            remove_clip,
        ),
        declare_tool(
            "move_clip",
            "Move a clip",
            MOVE_CLIP_DESCRIPTION,
            ToolEffect::ChangesProject,
            move_clip,
        ),
        declare_tool(
            "trim_clip",
            "Trim a clip",
            TRIM_CLIP_DESCRIPTION,
            ToolEffect::ChangesProject,
            trim_clip,
        ),
        declare_tool(
            "split_clip",
            "Split a clip in two",
            SPLIT_CLIP_DESCRIPTION,
            ToolEffect::ChangesProject,
            split_clip,
        ),
    ]
}

/// Describes one clip as JSON details, after a change.
pub(crate) fn clip_details(project: &Project, clip_id: ClipId) -> Result<ClipView, ToolError> {
    let (track, clip) = project.require_clip(clip_id)?;
    Ok(ClipView::of(project, track, clip))
}

/// The error for an object a command should have reported creating.
pub(crate) fn missing_created_object(what: &str) -> EditError {
    EditError::InternalInconsistency {
        details: format!("the {what} created was not reported"),
    }
}

/// The distance under which clip edges attract each other: half a frame.
fn snapping_tolerance(project: &Project) -> TimeSpan {
    half_frame(project.settings().frame_rate)
}

// ----- get_timeline -----

const GET_TIMELINE_DESCRIPTION: &str = "\
Shows the whole timeline: canvas size, total duration, and every track with its clips (clip_id, \
kind, what it shows, start and end on the timeline in seconds, source range and speed for \
media). Tracks are listed from the bottom (position_from_bottom 0) to the top; on screen, a \
higher video track is drawn over the lower ones. Video tracks hold videos, images and texts \
(a video clip also plays its sound); audio tracks hold sound. Use get_clip for every detail of \
one clip.";

#[derive(Debug, Serialize)]
struct TimelineView {
    settings: SettingsView,
    duration_in_seconds: f64,
    tracks: Vec<TrackView>,
}

fn get_timeline(toolbox: &mut ToolCall<'_>, _: NoParameters) -> Result<ToolOutput, ToolError> {
    let project = toolbox.require_project()?;
    let view = TimelineView {
        settings: SettingsView::of(project.settings()),
        duration_in_seconds: seconds_in(project.timeline().duration()),
        tracks: project
            .timeline()
            .tracks()
            .iter()
            .enumerate()
            .map(|(position, track)| TrackView::of(project, track, position))
            .collect(),
    };
    let message = if view.tracks.is_empty() {
        "The timeline is empty: add clips with add_clip (or add_text).".to_string()
    } else {
        format!("The timeline lasts {} s:", view.duration_in_seconds)
    };
    Ok(ToolOutput::message_with_details(message, &view))
}

// ----- get_clip -----

const GET_CLIP_DESCRIPTION: &str = "\
Shows everything about one clip: its place on the timeline and in its source file (seconds), \
speed, volume, opacity, position on the canvas (center_x/center_y in pixels from the top-left \
corner of the canvas), scale (1 = fits inside the canvas), rotation, crop, text and style for \
texts, animations (keyframes, for example of fades) and effects with their effect_id. \
Example: {\"clip_id\": 4}";

/// Parameters of tools acting on one clip.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ClipParameters {
    /// The clip, from get_timeline. Example: 4.
    clip_id: u64,
}

fn get_clip(
    toolbox: &mut ToolCall<'_>,
    parameters: ClipParameters,
) -> Result<ToolOutput, ToolError> {
    let project = toolbox.require_project()?;
    let view = clip_details(project, ClipId::from_raw(parameters.clip_id))?;
    Ok(ToolOutput::message_with_details(
        format!("Clip {}:", parameters.clip_id),
        &view,
    ))
}

// ----- add_track -----

const ADD_TRACK_DESCRIPTION: &str = "\
Adds an empty track. Video tracks hold videos, images and texts; a higher video track is drawn \
over the lower ones (put titles on a track above the video). Audio tracks hold music and \
voice-overs. add_clip and add_text create tracks when needed, so this is only required for a \
specific layout. Example: {\"kind\": \"audio\", \"name\": \"Music\"}";

/// The kind of a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TrackKindParameter {
    /// Pictures: videos, images, texts.
    Video,
    /// Sound only.
    Audio,
}

impl From<TrackKindParameter> for TrackKind {
    fn from(kind: TrackKindParameter) -> Self {
        match kind {
            TrackKindParameter::Video => TrackKind::Video,
            TrackKindParameter::Audio => TrackKind::Audio,
        }
    }
}

/// Where a new track goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum TrackPlacement {
    /// Above every track (drawn on top).
    #[default]
    Top,
    /// Below every track (drawn underneath).
    Bottom,
}

/// Parameters of `add_track`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddTrackParameters {
    /// "video" or "audio".
    kind: TrackKindParameter,
    /// A name shown in the editor, for example "Titles". Default "Video 2", "Audio 1"…
    name: Option<String>,
    /// "top" (default) or "bottom".
    #[serde(default)]
    position: TrackPlacement,
}

fn add_track(
    toolbox: &mut ToolCall<'_>,
    parameters: AddTrackParameters,
) -> Result<ToolOutput, ToolError> {
    let kind = TrackKind::from(parameters.kind);
    let position = match parameters.position {
        TrackPlacement::Top => TrackPosition::Top,
        TrackPlacement::Bottom => TrackPosition::Bottom,
    };
    let label = format!("Add a {} track", track_kind_name(kind));
    let track_id = toolbox.change_project(&label, |transaction| {
        let mut command = AddTrack::new(kind, position);
        if let Some(name) = parameters.name {
            command = command.named(name);
        }
        let created = transaction.execute(command)?;
        Ok(created
            .first_track_id()
            .ok_or_else(|| missing_created_object("track"))?)
    })?;
    let project = toolbox.require_project()?;
    let track = project.require_track(track_id)?;
    let position_from_bottom = project.timeline().track_index(track_id).unwrap_or(0);
    Ok(ToolOutput::message_with_details(
        format!("Added track {track_id}."),
        &TrackView::of(project, track, position_from_bottom),
    ))
}

// ----- remove_track -----

const REMOVE_TRACK_DESCRIPTION: &str = "\
Removes a track and every clip on it (the media stay imported; files on disk are never \
touched). One undo brings it all back. Example: {\"track_id\": 2}";

/// Parameters of `remove_track`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RemoveTrackParameters {
    /// The track, from get_timeline. Example: 2.
    track_id: u64,
}

fn remove_track(
    toolbox: &mut ToolCall<'_>,
    parameters: RemoveTrackParameters,
) -> Result<ToolOutput, ToolError> {
    let track_id = TrackId::from_raw(parameters.track_id);
    let clip_count = toolbox
        .require_project()?
        .require_track(track_id)?
        .clips()
        .len();
    let label = format!("Remove track {track_id} and its {clip_count} clip(s)");
    toolbox.change_project(&label, |transaction| {
        transaction.execute(RemoveTrack::new(track_id))?;
        Ok(())
    })?;
    Ok(ToolOutput::text(format!(
        "Removed track {track_id} and its {clip_count} clip(s). undo would bring them back."
    )))
}

// ----- add_clip -----

const ADD_CLIP_DESCRIPTION: &str = "\
Places an imported media file (see import_media / list_media) on the timeline and returns the \
new clip_id. By default the whole file is used and the clip goes at the end of the first track \
of the right kind (videos and images on a video track, sounds on an audio track), creating the \
track if there is none. Give source_start_in_seconds / source_end_in_seconds to use only part \
of the file (times measured in the file), start_time_in_seconds to choose where it starts on \
the timeline, and track_id to choose the track. Clips on one track cannot overlap; a start \
within half a frame of another clip's end snaps onto it.

Examples: {\"media_id\": 1}; {\"media_id\": 2, \"source_start_in_seconds\": 12.5, \
\"source_end_in_seconds\": 20, \"start_time_in_seconds\": 0}; a still image for 3 s: \
{\"media_id\": 3, \"duration_in_seconds\": 3}";

/// Parameters of `add_clip`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddClipParameters {
    /// The imported media to place. Example: 1.
    media_id: u64,
    /// The track to place it on. Default: the first track of the right kind (created if needed).
    track_id: Option<u64>,
    /// Where the clip starts on the timeline, in seconds. Default: the end of the track.
    start_time_in_seconds: Option<f64>,
    /// Where to start reading the file, in seconds from the file's start. Default 0.
    source_start_in_seconds: Option<f64>,
    /// Where to stop reading the file (excluded), in seconds from the file's start. Default:
    /// the end of the file. Not for still images.
    source_end_in_seconds: Option<f64>,
    /// For still images only: how long the image stays on screen, in seconds. Default 5.
    duration_in_seconds: Option<f64>,
}

/// How long a still image stays on screen when no duration is given.
const DEFAULT_IMAGE_DURATION_IN_SECONDS: f64 = 5.0;

fn add_clip(
    toolbox: &mut ToolCall<'_>,
    parameters: AddClipParameters,
) -> Result<ToolOutput, ToolError> {
    let project = toolbox.require_project()?;
    let media_id = MediaAssetId::from_raw(parameters.media_id);
    let media_asset = project.require_media_asset(media_id)?;
    let (source_start, source_end) = source_range_for(&media_asset.description, &parameters)?;
    let length = source_end
        .span_since(source_start)
        .filter(|span| !span.is_zero())
        .ok_or_else(|| {
            ToolError::new("source_end_in_seconds must be after source_start_in_seconds")
        })?;
    let track_kind = if media_asset.description.kind == MediaKind::Audio {
        TrackKind::Audio
    } else {
        TrackKind::Video
    };
    let requested_start = parameters
        .start_time_in_seconds
        .map(|seconds| timestamp_from_seconds("start_time_in_seconds", seconds))
        .transpose()?;
    let placement = choose_placement(
        project,
        track_kind,
        parameters.track_id,
        requested_start,
        length,
    )?;
    let label = format!(
        "Add {} at {}",
        media_asset.description.display_name, placement.start
    );
    let clip_id = toolbox.change_project(&label, |transaction| {
        let track_id = match placement.track {
            Some(track_id) => track_id,
            None => create_track(transaction, track_kind)?,
        };
        let created = transaction.execute(AddClip::media(
            track_id,
            media_id,
            source_start,
            source_end,
            placement.start,
        ))?;
        Ok(created
            .first_clip_id()
            .ok_or_else(|| missing_created_object("clip"))?)
    })?;
    let view = clip_details(toolbox.require_project()?, clip_id)?;
    Ok(ToolOutput::message_with_details(
        format!(
            "Added clip {clip_id} on track {} from {} s to {} s.",
            view.track_id,
            view.summary.timeline_start_in_seconds,
            view.summary.timeline_end_in_seconds
        ),
        &view,
    ))
}

/// The part of the file the clip shows.
fn source_range_for(
    description: &kadr_core::media_asset::MediaAssetDescription,
    parameters: &AddClipParameters,
) -> Result<(Timestamp, Timestamp), ToolError> {
    let source_start = match parameters.source_start_in_seconds {
        Some(seconds) => timestamp_from_seconds("source_start_in_seconds", seconds)?,
        None => Timestamp::ZERO,
    };
    if description.kind == MediaKind::Image {
        let duration_in_seconds = parameters
            .duration_in_seconds
            .unwrap_or(DEFAULT_IMAGE_DURATION_IN_SECONDS);
        let duration = positive_span_from_seconds("duration_in_seconds", duration_in_seconds)?;
        return Ok((Timestamp::ZERO, Timestamp::ZERO + duration));
    }
    if parameters.duration_in_seconds.is_some() {
        return Err(ToolError::new(
            "duration_in_seconds is only for still images; for videos and sounds use \
             source_start_in_seconds and source_end_in_seconds",
        ));
    }
    let file_end = Timestamp::ZERO + description.duration.unwrap_or(TimeSpan::ZERO);
    let source_end = match parameters.source_end_in_seconds {
        Some(seconds) => timestamp_from_seconds("source_end_in_seconds", seconds)?,
        None => file_end,
    };
    Ok((source_start, source_end))
}

/// Where a new clip goes: an existing track (or a new one) and a start time.
struct Placement {
    /// `None`: create a new track of the right kind.
    track: Option<TrackId>,
    start: Timestamp,
}

fn choose_placement(
    project: &Project,
    track_kind: TrackKind,
    requested_track: Option<u64>,
    requested_start: Option<Timestamp>,
    length: TimeSpan,
) -> Result<Placement, ToolError> {
    let tolerance = snapping_tolerance(project);
    if let Some(raw_track_id) = requested_track {
        let track = project.require_track(TrackId::from_raw(raw_track_id))?;
        let start = match requested_start {
            Some(start) => snap_to_nearby_clip_edge(track, start, None, tolerance),
            None => track.end(),
        };
        return Ok(Placement {
            track: Some(track.id),
            start,
        });
    }
    let tracks_of_kind: Vec<&Track> = project
        .timeline()
        .tracks()
        .iter()
        .filter(|track| track.kind == track_kind)
        .collect();
    let Some(requested_start) = requested_start else {
        return Ok(match tracks_of_kind.first() {
            Some(track) => Placement {
                track: Some(track.id),
                start: track.end(),
            },
            None => Placement {
                track: None,
                start: Timestamp::ZERO,
            },
        });
    };
    for track in tracks_of_kind {
        let start = snap_to_nearby_clip_edge(track, requested_start, None, tolerance);
        if is_free(track, start, start + length, None) {
            return Ok(Placement {
                track: Some(track.id),
                start,
            });
        }
    }
    Ok(Placement {
        track: None,
        start: requested_start,
    })
}

/// Whether nothing (but `ignored_clip`) occupies `start`..`end` on the track.
pub(crate) fn is_free(
    track: &Track,
    start: Timestamp,
    end: Timestamp,
    ignored_clip: Option<ClipId>,
) -> bool {
    track
        .clips()
        .iter()
        .filter(|clip| Some(clip.id) != ignored_clip)
        .all(|clip| clip.timeline_end() <= start || clip.timeline_start >= end)
}

/// Adds a track of `kind` on top, inside a transaction.
pub(crate) fn create_track(
    transaction: &mut Transaction<'_>,
    kind: TrackKind,
) -> Result<TrackId, ToolError> {
    let created = transaction.execute(AddTrack::new(kind, TrackPosition::Top))?;
    Ok(created
        .first_track_id()
        .ok_or_else(|| missing_created_object("track"))?)
}

// ----- remove_clip -----

const REMOVE_CLIP_DESCRIPTION: &str = "\
Removes a clip from the timeline (the media stays imported; files on disk are never touched). \
With \"close_gap\": true, the later clips of the same track move earlier to fill the hole \
(other tracks do not move). One undo brings everything back. \
Example: {\"clip_id\": 7, \"close_gap\": true}";

/// Parameters of `remove_clip`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RemoveClipParameters {
    /// The clip, from get_timeline. Example: 7.
    clip_id: u64,
    /// Move the later clips of the same track earlier by the removed clip's duration. Default false.
    #[serde(default)]
    close_gap: bool,
}

fn remove_clip(
    toolbox: &mut ToolCall<'_>,
    parameters: RemoveClipParameters,
) -> Result<ToolOutput, ToolError> {
    let clip_id = ClipId::from_raw(parameters.clip_id);
    let project = toolbox.require_project()?;
    let (track, clip) = project.require_clip(clip_id)?;
    let removed_length = clip.timeline_duration();
    let removed_end = clip.timeline_end();
    let tolerance = snapping_tolerance(project);
    let later_clips: Vec<(ClipId, Timestamp)> = if parameters.close_gap {
        track
            .clips()
            .iter()
            .filter(|other| other.id != clip_id && other.timeline_start + tolerance >= removed_end)
            .map(|other| (other.id, other.timeline_start))
            .collect()
    } else {
        Vec::new()
    };
    let label = if parameters.close_gap {
        format!("Remove clip {clip_id} and close the gap")
    } else {
        format!("Remove clip {clip_id}")
    };
    toolbox.change_project(&label, |transaction| {
        transaction.execute(RemoveClip::new(clip_id))?;
        for (later_clip_id, later_start) in &later_clips {
            let new_start = later_start
                .checked_sub(removed_length)
                .unwrap_or(Timestamp::ZERO);
            transaction.execute(MoveClip::along_track(*later_clip_id, new_start))?;
        }
        Ok(())
    })?;
    let mut message = format!("Removed clip {clip_id}.");
    if !later_clips.is_empty() {
        message.push_str(&format!(
            " {} later clip(s) moved {} s earlier.",
            later_clips.len(),
            seconds_in(removed_length)
        ));
    }
    Ok(ToolOutput::text(message))
}

// ----- move_clip -----

const MOVE_CLIP_DESCRIPTION: &str = "\
Moves a clip to a new start time on the timeline (in seconds), and optionally to another track \
of the same kind. Its length and content do not change. Clips on one track cannot overlap; an \
edge within half a frame of another clip's edge snaps onto it. \
Example: {\"clip_id\": 5, \"start_time_in_seconds\": 12.0, \"track_id\": 3}";

/// Parameters of `move_clip`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MoveClipParameters {
    /// The clip, from get_timeline. Example: 5.
    clip_id: u64,
    /// The new start on the timeline, in seconds.
    start_time_in_seconds: f64,
    /// The track to move it to. Default: it stays on its track.
    track_id: Option<u64>,
}

fn move_clip(
    toolbox: &mut ToolCall<'_>,
    parameters: MoveClipParameters,
) -> Result<ToolOutput, ToolError> {
    let clip_id = ClipId::from_raw(parameters.clip_id);
    let project = toolbox.require_project()?;
    let (current_track, clip) = project.require_clip(clip_id)?;
    let destination = match parameters.track_id {
        Some(raw_track_id) => project.require_track(TrackId::from_raw(raw_track_id))?,
        None => current_track,
    };
    let requested_start =
        timestamp_from_seconds("start_time_in_seconds", parameters.start_time_in_seconds)?;
    let new_start = snapped_start_for_move(
        project,
        destination,
        clip_id,
        requested_start,
        clip.timeline_duration(),
    );
    let is_changing_track = destination.id != current_track.id;
    let destination_id = destination.id;
    let label = format!("Move clip {clip_id} to {new_start}");
    toolbox.change_project(&label, |transaction| {
        let command = if is_changing_track {
            MoveClip::to_track(clip_id, destination_id, new_start)
        } else {
            MoveClip::along_track(clip_id, new_start)
        };
        transaction.execute(command)?;
        Ok(())
    })?;
    let view = clip_details(toolbox.require_project()?, clip_id)?;
    Ok(ToolOutput::message_with_details(
        format!(
            "Clip {clip_id} now runs from {} s to {} s on track {}.",
            view.summary.timeline_start_in_seconds,
            view.summary.timeline_end_in_seconds,
            view.track_id
        ),
        &view,
    ))
}

/// The start of a moved clip after snapping its start, or else its end, to a
/// nearby clip edge.
fn snapped_start_for_move(
    project: &Project,
    destination: &Track,
    clip_id: ClipId,
    requested_start: Timestamp,
    length: TimeSpan,
) -> Timestamp {
    let tolerance = snapping_tolerance(project);
    let snapped_start =
        snap_to_nearby_clip_edge(destination, requested_start, Some(clip_id), tolerance);
    if snapped_start != requested_start {
        return snapped_start;
    }
    let requested_end = requested_start + length;
    let snapped_end =
        snap_to_nearby_clip_edge(destination, requested_end, Some(clip_id), tolerance);
    snapped_end.checked_sub(length).unwrap_or(requested_start)
}

// ----- trim_clip -----

const TRIM_CLIP_DESCRIPTION: &str = "\
Moves the start and/or the end of a clip on the timeline (in seconds), making it shorter or \
longer. Moving the start of a media clip also moves where it starts reading the file, so the \
picture stays in sync: trimming the start from 10 s to 12 s skips the first 2 s of what the clip \
showed. A media clip cannot extend beyond its file. To remove a part in the middle, use \
split_clip twice and remove_clip. \
Example: {\"clip_id\": 5, \"end_time_in_seconds\": 18.5}";

/// Parameters of `trim_clip`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TrimClipParameters {
    /// The clip, from get_timeline. Example: 5.
    clip_id: u64,
    /// New start of the clip on the timeline, in seconds. Omit to keep it.
    start_time_in_seconds: Option<f64>,
    /// New end of the clip on the timeline, in seconds (excluded). Omit to keep it.
    end_time_in_seconds: Option<f64>,
}

fn trim_clip(
    toolbox: &mut ToolCall<'_>,
    parameters: TrimClipParameters,
) -> Result<ToolOutput, ToolError> {
    let clip_id = ClipId::from_raw(parameters.clip_id);
    let project = toolbox.require_project()?;
    let (track, _) = project.require_clip(clip_id)?;
    let tolerance = snapping_tolerance(project);
    let snap =
        |parameter_name: &str, seconds: Option<f64>| -> Result<Option<Timestamp>, ToolError> {
            seconds
                .map(|seconds| {
                    timestamp_from_seconds(parameter_name, seconds)
                        .map(|time| snap_to_nearby_clip_edge(track, time, Some(clip_id), tolerance))
                })
                .transpose()
        };
    let new_start = snap("start_time_in_seconds", parameters.start_time_in_seconds)?;
    let new_end = snap("end_time_in_seconds", parameters.end_time_in_seconds)?;
    if new_start.is_none() && new_end.is_none() {
        return Err(ToolError::new(
            "give start_time_in_seconds, end_time_in_seconds, or both",
        ));
    }
    toolbox.change_project(&format!("Trim clip {clip_id}"), |transaction| {
        if let Some(new_start) = new_start {
            transaction.execute(TrimClip::new(clip_id, ClipEdge::Start, new_start))?;
        }
        if let Some(new_end) = new_end {
            transaction.execute(TrimClip::new(clip_id, ClipEdge::End, new_end))?;
        }
        Ok(())
    })?;
    let view = clip_details(toolbox.require_project()?, clip_id)?;
    Ok(ToolOutput::message_with_details(
        format!(
            "Clip {clip_id} now runs from {} s to {} s.",
            view.summary.timeline_start_in_seconds, view.summary.timeline_end_in_seconds
        ),
        &view,
    ))
}

// ----- split_clip -----

const SPLIT_CLIP_DESCRIPTION: &str = "\
Cuts a clip in two at a time on the timeline (in seconds, strictly inside the clip). The first \
part keeps the clip_id; the second part gets a new clip_id, returned. Effects and animations are \
kept on both parts. To cut out a section (for example from 4.2 s to 6.0 s): split at 4.2, split \
the second part at 6.0, then remove_clip the middle part with close_gap. To cut silences, \
remove_silences does them all at once. \
Example: {\"clip_id\": 4, \"time_in_seconds\": 12.5}";

/// Parameters of `split_clip`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SplitClipParameters {
    /// The clip, from get_timeline. Example: 4.
    clip_id: u64,
    /// Where to cut, in seconds on the timeline.
    time_in_seconds: f64,
}

fn split_clip(
    toolbox: &mut ToolCall<'_>,
    parameters: SplitClipParameters,
) -> Result<ToolOutput, ToolError> {
    let clip_id = ClipId::from_raw(parameters.clip_id);
    let split_time = timestamp_from_seconds("time_in_seconds", parameters.time_in_seconds)?;
    toolbox.require_project()?.require_clip(clip_id)?;
    let label = format!("Split clip {clip_id} at {split_time}");
    let second_part = toolbox.change_project(&label, |transaction| {
        let created = transaction.execute(SplitClip::new(clip_id, split_time))?;
        Ok(created
            .first_clip_id()
            .ok_or_else(|| missing_created_object("clip"))?)
    })?;
    let project = toolbox.require_project()?;
    let parts = [
        clip_details(project, clip_id)?,
        clip_details(project, second_part)?,
    ];
    Ok(ToolOutput::message_with_details(
        format!(
            "Split clip {clip_id} at {} s: the first part is clip {clip_id}, the second part is \
             clip {second_part}.",
            seconds_of(split_time)
        ),
        &parts,
    ))
}
