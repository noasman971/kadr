//! Sound tools: find the silences of a file or a clip, to cut them.

use kadr_core::clip::ClipContent;
use kadr_core::ids::{ClipId, MediaAssetId};
use kadr_core::media_asset::MediaAsset;
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_engine::silence_analysis::{
    DEFAULT_MINIMUM_SILENCE_IN_SECONDS, DEFAULT_SILENCE_THRESHOLD_IN_DECIBELS,
    SilenceDetectionSettings, SilentPart, find_silent_parts,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{ToolEffect, ToolEntry, declare_tool};
use crate::output::{ToolError, ToolOutput};
use crate::time_input::{seconds_in, seconds_of};
use crate::toolbox::KadrToolbox;

/// The sound tools.
pub(crate) fn sound_tools() -> Vec<ToolEntry> {
    vec![declare_tool(
        "detect_silences",
        "Find the silences",
        DETECT_SILENCES_DESCRIPTION,
        ToolEffect::ReadsOnly,
        detect_silences,
    )]
}

const DETECT_SILENCES_DESCRIPTION: &str = "\
Finds the silent parts of an imported file's sound (media_id, times into the file) or of a \
clip (clip_id, times both into the file and on the timeline), by measuring the loudness every \
10 ms: quieter than threshold_in_decibels (default −40 dBFS) for at least \
minimum_silence_in_seconds (default 0.5) is a silence. Nothing is changed.

To cut the silences of a clip: for each silence, split_clip at its timeline start and end, then \
remove_clip the middle part with close_gap. Work from the LAST silence to the first, so the \
timeline times of the remaining silences stay valid. Keep about 0.1 s of each pause so speech \
does not sound rushed. If too much or too little is found, raise or lower the threshold \
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
    let settings = settings_from(&parameters)?;
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

/// The detection settings, checked.
fn settings_from(
    parameters: &DetectSilencesParameters,
) -> Result<SilenceDetectionSettings, ToolError> {
    let threshold = parameters
        .threshold_in_decibels
        .unwrap_or(DEFAULT_SILENCE_THRESHOLD_IN_DECIBELS);
    let (lowest_threshold, highest_threshold) = THRESHOLD_RANGE_IN_DECIBELS;
    if !(lowest_threshold..=highest_threshold).contains(&threshold) {
        return Err(ToolError::new(format!(
            "threshold_in_decibels must be from {lowest_threshold} to {highest_threshold} (for \
             example -40)"
        )));
    }
    let minimum_silence = parameters
        .minimum_silence_in_seconds
        .unwrap_or(DEFAULT_MINIMUM_SILENCE_IN_SECONDS);
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
