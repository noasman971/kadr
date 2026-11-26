//! Sound tools: find the silences of a file or a clip, and cut them out.

use kadr_core::clip::ClipContent;
use kadr_core::commands::CreatedObject;
use kadr_core::ids::{ClipId, MediaAssetId};
use kadr_core::media_asset::MediaAsset;
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_engine::silence_analysis::{
    DEFAULT_MINIMUM_SILENCE_IN_SECONDS, DEFAULT_SILENCE_THRESHOLD_IN_DECIBELS,
    SilenceDetectionSettings, SilentPart, find_silent_parts,
};
use kadr_engine::silence_removal::{
    DEFAULT_KEPT_PAUSE, SilenceRemovalSettings, plan_silence_removal,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{ToolEffect, ToolEntry, declare_tool};
use crate::output::{ToolError, ToolOutput};
use crate::time_input::{seconds_in, seconds_of};
use crate::toolbox::KadrToolbox;

/// The sound tools.
pub(crate) fn sound_tools() -> Vec<ToolEntry> {
    vec![
        declare_tool(
            "detect_silences",
            "Find the silences",
            DETECT_SILENCES_DESCRIPTION,
            ToolEffect::ReadsOnly,
            detect_silences,
        ),
        declare_tool(
            "remove_silences",
            "Remove the silences of a clip",
            REMOVE_SILENCES_DESCRIPTION,
            ToolEffect::ChangesProject,
            remove_silences,
        ),
    ]
}

const DETECT_SILENCES_DESCRIPTION: &str = "\
Finds the silent parts of an imported file's sound (media_id, times into the file) or of a \
clip (clip_id, times both into the file and on the timeline), by measuring the loudness every \
10 ms: quieter than threshold_in_decibels (default −40 dBFS) for at least \
minimum_silence_in_seconds (default 0.5) is a silence. Nothing is changed.

To cut the silences of a clip, use remove_silences with the same settings: it shortens them all \
in one undoable step. If too much or too little is found, raise or lower the threshold \
(−50 keeps quieter sounds, −30 also removes background noise).

Example: {\"clip_id\": 4} or {\"media_id\": 1, \"threshold_in_decibels\": -45, \
\"minimum_silence_in_seconds\": 0.8}";

/// Parameters of `detect_silences`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DetectSilencesParameters {
    /// Look at this imported file's whole sound.
    media_id: Option<u64>,
    /// Look at the part of the file this clip plays.
    clip_id: Option<u64>,
    /// Loudness below which sound is silence, in dBFS (0 is the loudest), from −100 to −1.
    /// Default −40.
    threshold_in_decibels: Option<f32>,
    /// Shortest silence reported, in seconds, from 0.05 to 60. Default 0.5.
    minimum_silence_in_seconds: Option<f64>,
}

const THRESHOLD_RANGE_IN_DECIBELS: (f32, f32) = (-100.0, -1.0);
const MINIMUM_SILENCE_RANGE_IN_SECONDS: (f64, f64) = (0.05, 60.0);

#[derive(Debug, Serialize)]
struct SilenceView {
    source_start_in_seconds: f64,
    source_end_in_seconds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeline_start_in_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeline_end_in_seconds: Option<f64>,
    duration_in_seconds: f64,
}

#[derive(Debug, Serialize)]
struct SilencesView {
    analysed_seconds: f64,
    silent_seconds: f64,
    silences: Vec<SilenceView>,
}

fn detect_silences(
    toolbox: &mut KadrToolbox,
    parameters: DetectSilencesParameters,
) -> Result<ToolOutput, ToolError> {
    let settings = settings_from(
        parameters.threshold_in_decibels,
        parameters.minimum_silence_in_seconds,
    )?;
    let project = toolbox.require_project()?;
    let (media_asset, source_start, source_end, clip) =
        match (parameters.clip_id, parameters.media_id) {
            (Some(_), Some(_)) => return Err(ToolError::new("give clip_id or media_id, not both")),
            (None, None) => return Err(ToolError::new("give clip_id or media_id")),
            (None, Some(raw_media_id)) => {
                let media_asset =
                    project.require_media_asset(MediaAssetId::from_raw(raw_media_id))?;
                let file_end =
                    Timestamp::ZERO + media_asset.description.duration.unwrap_or(TimeSpan::ZERO);
                (media_asset, Timestamp::ZERO, file_end, None)
            }
            (Some(raw_clip_id), None) => {
                let clip_id = ClipId::from_raw(raw_clip_id);
                let (_, clip) = project.require_clip(clip_id)?;
                let ClipContent::Media(segment) = &clip.content else {
                    return Err(ToolError::new(format!(
                        "clip {clip_id} is a text: it has no sound"
                    )));
                };
                let media_asset = project.require_media_asset(segment.media_asset_id)?;
                (
                    media_asset,
                    segment.source_start,
                    segment.source_end,
                    Some(clip),
                )
            }
        };
    ensure_has_readable_sound(toolbox, media_asset)?;
    let silent_parts = find_silent_parts(
        toolbox.backend(),
        &media_asset.description.file_path,
        source_start,
        source_end,
        settings,
    )?;
    let views: Vec<SilenceView> = silent_parts
        .iter()
        .map(|part| silence_view(part, clip))
        .collect();
    let silent_span = silent_parts
        .iter()
        .filter_map(|part| part.end.span_since(part.start))
        .fold(TimeSpan::ZERO, |total, span| total + span);
    let analysed_span = source_end
        .span_since(source_start)
        .unwrap_or(TimeSpan::ZERO);
    let view = SilencesView {
        analysed_seconds: seconds_in(analysed_span),
        silent_seconds: seconds_in(silent_span),
        silences: views,
    };
    let message = format!(
        "Found {} silence(s), {} s in all, in {} s of sound (quieter than {} dBFS for at least \
         {} s).",
        view.silences.len(),
        view.silent_seconds,
        view.analysed_seconds,
        settings.threshold_in_decibels,
        settings.minimum_silence_in_seconds
    );
    Ok(ToolOutput::message_with_details(message, &view))
}

// ----- remove_silences -----

const REMOVE_SILENCES_DESCRIPTION: &str = "\
Shortens every silence of a video or sound clip to a short pause, in one undoable step: the \
silences are found as detect_silences does (threshold_in_decibels, default −40 dBFS; \
minimum_silence_in_seconds, default 0.5), each one keeps kept_pause_in_seconds (default 0.2, \
half next to the sound on each side, so speech does not sound rushed), the rest is cut out. \
The clip becomes several clips end to end (the first keeps the clip_id), and the later clips \
of the same track move earlier by the time removed; other tracks do not move, so check titles \
or music placed over the clip afterwards. One undo brings everything back. Look at the result \
with get_contact_sheet or get_frame near the cuts. If speech is cut, lower the threshold (−50); \
if background noise stays, raise it (−30). \
Example: {\"clip_id\": 4} or {\"clip_id\": 4, \"threshold_in_decibels\": -45, \
\"kept_pause_in_seconds\": 0.3}";

/// Parameters of `remove_silences`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RemoveSilencesParameters {
    /// The video or sound clip, from get_timeline. Example: 4.
    clip_id: u64,
    /// Loudness below which sound is silence, in dBFS (0 is the loudest), from −100 to −1.
    /// Default −40.
    threshold_in_decibels: Option<f32>,
    /// Shortest silence shortened, in seconds, from 0.05 to 60. Default 0.5.
    minimum_silence_in_seconds: Option<f64>,
    /// How long each silence lasts once shortened, in seconds, from 0 to 2. Default 0.2.
    kept_pause_in_seconds: Option<f64>,
}

const KEPT_PAUSE_RANGE_IN_SECONDS: (f64, f64) = (0.0, 2.0);

/// What `remove_silences` did.
#[derive(Debug, Serialize)]
struct RemovedSilencesView {
    silences_shortened: usize,
    removed_seconds: f64,
    /// The clips the clip became, in order (the first keeps its clip_id).
    clip_ids: Vec<u64>,
    clip_start_in_seconds: f64,
    clip_end_in_seconds: f64,
    later_clips_moved: usize,
    /// The parts cut, in timeline times before the change.
    cuts: Vec<CutView>,
}

#[derive(Debug, Serialize)]
struct CutView {
    timeline_start_in_seconds: f64,
    timeline_end_in_seconds: f64,
}

fn remove_silences(
    toolbox: &mut KadrToolbox,
    parameters: RemoveSilencesParameters,
) -> Result<ToolOutput, ToolError> {
    let settings = SilenceRemovalSettings {
        detection: settings_from(
            parameters.threshold_in_decibels,
            parameters.minimum_silence_in_seconds,
        )?,
        kept_pause: kept_pause_from(parameters.kept_pause_in_seconds)?,
    };
    let clip_id = ClipId::from_raw(parameters.clip_id);
    let project = toolbox.require_project()?;
    let (track, clip) = project.require_clip(clip_id)?;
    if let ClipContent::Media(segment) = &clip.content {
        ensure_has_readable_sound(
            toolbox,
            project.require_media_asset(segment.media_asset_id)?,
        )?;
    }
    let plan = plan_silence_removal(project, toolbox.backend(), clip_id, settings)?;
    if plan.whole_clip_is_silent {
        return Ok(ToolOutput::text(format!(
            "Clip {clip_id} is silent from start to end: nothing was cut. Remove it with \
             remove_clip if it is not needed, or lower threshold_in_decibels if it has quiet sound."
        )));
    }
    let Some(command) = plan.removal_command() else {
        return Ok(ToolOutput::text(format!(
            "Clip {clip_id} has no silence of at least {} s quieter than {} dBFS to shorten: \
             nothing changed.",
            settings.detection.minimum_silence_in_seconds, settings.detection.threshold_in_decibels
        )));
    };
    let (track_id, clip_start) = (track.id, clip.timeline_start);
    let later_clips_moved = track
        .clips()
        .iter()
        .filter(|other| other.timeline_start >= clip.timeline_end())
        .count();
    let cuts: Vec<CutView> = plan
        .cuts
        .iter()
        .map(|cut| CutView {
            timeline_start_in_seconds: seconds_of(cut.start()),
            timeline_end_in_seconds: seconds_of(cut.end()),
        })
        .collect();
    let removed = plan.removed_duration();
    let label = format!("Remove the silences of clip {clip_id}");
    let created =
        toolbox.change_project(&label, |transaction| Ok(transaction.execute(command)?))?;
    let mut clip_ids = vec![clip_id];
    clip_ids.extend(created.iter().filter_map(|object| match object {
        CreatedObject::Clip(piece_id) => Some(*piece_id),
        _ => None,
    }));
    let pieces_end = toolbox
        .require_project()?
        .timeline()
        .track(track_id)
        .and_then(|track| track.clip(*clip_ids.last().unwrap_or(&clip_id)))
        .map_or(clip_start, |last_piece| last_piece.timeline_end());
    let view = RemovedSilencesView {
        silences_shortened: cuts.len(),
        removed_seconds: seconds_in(removed),
        clip_ids: clip_ids.iter().map(|id| id.raw()).collect(),
        clip_start_in_seconds: seconds_of(clip_start),
        clip_end_in_seconds: seconds_of(pieces_end),
        later_clips_moved,
        cuts,
    };
    let message = format!(
        "Shortened {} silence(s) of clip {clip_id} to {} s pauses: {} s removed. It is now {} \
         clip(s) end to end, from {} s to {} s; {} later clip(s) of track {track_id} moved {} s \
         earlier, other tracks did not move. One undo brings everything back.",
        view.silences_shortened,
        seconds_in(settings.kept_pause),
        view.removed_seconds,
        view.clip_ids.len(),
        view.clip_start_in_seconds,
        view.clip_end_in_seconds,
        view.later_clips_moved,
        view.removed_seconds
    );
    Ok(ToolOutput::message_with_details(message, &view))
}

/// The kept pause, checked.
fn kept_pause_from(kept_pause_in_seconds: Option<f64>) -> Result<TimeSpan, ToolError> {
    let Some(seconds) = kept_pause_in_seconds else {
        return Ok(DEFAULT_KEPT_PAUSE);
    };
    let (shortest, longest) = KEPT_PAUSE_RANGE_IN_SECONDS;
    if !(shortest..=longest).contains(&seconds) {
        return Err(ToolError::new(format!(
            "kept_pause_in_seconds must be from {shortest} to {longest} (for example 0.2)"
        )));
    }
    Ok(TimeSpan::from_seconds(seconds)?)
}

/// The detection settings, checked.
fn settings_from(
    threshold_in_decibels: Option<f32>,
    minimum_silence_in_seconds: Option<f64>,
) -> Result<SilenceDetectionSettings, ToolError> {
    let threshold = threshold_in_decibels.unwrap_or(DEFAULT_SILENCE_THRESHOLD_IN_DECIBELS);
    let (lowest_threshold, highest_threshold) = THRESHOLD_RANGE_IN_DECIBELS;
    if !(lowest_threshold..=highest_threshold).contains(&threshold) {
        return Err(ToolError::new(format!(
            "threshold_in_decibels must be from {lowest_threshold} to {highest_threshold} (for \
             example -40)"
        )));
    }
    let minimum_silence = minimum_silence_in_seconds.unwrap_or(DEFAULT_MINIMUM_SILENCE_IN_SECONDS);
    let (shortest, longest) = MINIMUM_SILENCE_RANGE_IN_SECONDS;
    if !(shortest..=longest).contains(&minimum_silence) {
        return Err(ToolError::new(format!(
            "minimum_silence_in_seconds must be from {shortest} to {longest} (for example 0.5)"
        )));
    }
    Ok(SilenceDetectionSettings {
        threshold_in_decibels: threshold,
        minimum_silence_in_seconds: minimum_silence,
    })
}

/// Refuses files without sound, or outside the allowed folders.
fn ensure_has_readable_sound(
    toolbox: &KadrToolbox,
    media_asset: &MediaAsset,
) -> Result<(), ToolError> {
    if !media_asset.description.has_audio() {
        return Err(ToolError::new(format!(
            "media {} (\"{}\") has no sound",
            media_asset.id, media_asset.description.display_name
        )));
    }
    toolbox
        .allowed_folders()
        .check_existing_file(&media_asset.description.file_path)
        .map(|_| ())
        .map_err(|error| error.in_context(format!("media {}", media_asset.id)))
}

/// A silence in file time, and on the timeline when it belongs to a clip.
fn silence_view(part: &SilentPart, clip: Option<&kadr_core::clip::Clip>) -> SilenceView {
    let timeline_time = |source_time: Timestamp| -> Option<f64> {
        let clip = clip?;
        let ClipContent::Media(segment) = &clip.content else {
            return None;
        };
        let source_offset = source_time.span_since(segment.source_start)?;
        Some(seconds_of(
            clip.timeline_start + segment.speed.timeline_span_for_source_span(source_offset),
        ))
    };
    SilenceView {
        source_start_in_seconds: seconds_of(part.start),
        source_end_in_seconds: seconds_of(part.end),
        timeline_start_in_seconds: timeline_time(part.start),
        timeline_end_in_seconds: timeline_time(part.end),
        duration_in_seconds: seconds_in(part.end.span_since(part.start).unwrap_or(TimeSpan::ZERO)),
    }
}
