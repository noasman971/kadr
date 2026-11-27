//! Subtitle tools: import an `.srt` file, list, edit and restyle its cues, and
//! export them back. Decision record 0056.

use std::path::{Path, PathBuf};

use kadr_core::color::Color;
use kadr_core::commands::{
    AddSubtitleCues, AddTrack, CreatedObjectsExt, MergeSubtitleCues, RemoveClip,
    SetSubtitleCueText, SetSubtitleCueTiming, SetSubtitleStyle, SplitClip, TrackPosition,
};
use kadr_core::history::Transaction;
use kadr_core::ids::{ClipId, TrackId};
use kadr_core::project::Project;
use kadr_core::subtitle::{
    SubtitleBackdrop, SubtitlePlacement, SubtitleStyle, TimedSubtitleCue, plain_text_of_cue,
};
use kadr_core::time::TimeSpan;
use kadr_core::timeline::{Track, TrackKind};
use kadr_engine::subtitle_files::{
    SUBTITLE_FILE_EXTENSION, add_subtitle_track_with_cues, export_subtitle_track,
    is_subtitle_file_path, read_subtitle_file, subtitle_track_name_for,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{ToolEffect, ToolEntry, declare_tool};
use crate::output::{ToolError, ToolOutput};
use crate::time_input::{
    half_frame, seconds_of, snap_to_nearby_clip_edge, span_from_seconds, timestamp_from_seconds,
};
use crate::toolbox::ToolCall;
use crate::tools::common::{parse_color, with_extension};
use crate::tools::timeline_tools::missing_created_object;

/// The subtitle tools.
pub(crate) fn subtitle_tools() -> Vec<ToolEntry> {
    vec![
        declare_tool(
            "import_subtitles",
            "Import subtitles (.srt)",
            IMPORT_SUBTITLES_DESCRIPTION,
            ToolEffect::ChangesProject,
            import_subtitles,
        ),
        declare_tool(
            "list_subtitles",
            "List subtitle cues",
            LIST_SUBTITLES_DESCRIPTION,
            ToolEffect::ReadsOnly,
            list_subtitles,
        ),
        declare_tool(
            "edit_subtitles",
            "Edit subtitle cues",
            EDIT_SUBTITLES_DESCRIPTION,
            ToolEffect::ChangesProject,
            edit_subtitles,
        ),
        declare_tool(
            "set_subtitle_style",
            "Change the subtitle style",
            SET_SUBTITLE_STYLE_DESCRIPTION,
            ToolEffect::ChangesProject,
            set_subtitle_style,
        ),
        declare_tool(
            "export_subtitles",
            "Export subtitles (.srt)",
            EXPORT_SUBTITLES_DESCRIPTION,
            ToolEffect::WritesFiles,
            export_subtitles,
        ),
    ]
}

/// How many cues answers show by default.
const DEFAULT_LISTED_CUE_COUNT: usize = 200;
/// The most cues one answer may show.
const MAXIMUM_LISTED_CUE_COUNT: usize = 1_000;
/// How many cues the answer of an import shows.
const IMPORT_PREVIEW_CUE_COUNT: usize = 5;

// ----- Shared: style, views, tracks -----

/// Style values to change; omitted ones keep their value.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SubtitleStyleParameters {
    /// A font installed on the computer. Default "Inter" (bundled with Kadr).
    font_family: Option<String>,
    /// Letter height in canvas pixels, 1 to 2000. Default: the canvas's shorter side ÷ 18 (60 for 1080p).
    font_size_in_pixels: Option<f32>,
    /// Every word bold. Default true on vertical canvases, false otherwise. Single words: <b>…</b> in the text.
    bold: Option<bool>,
    /// "#RRGGBB" or "#RRGGBBAA". Default white.
    text_color: Option<String>,
    /// What is behind the letters: "outline" (default, a line around every letter), "box" (a box behind each line) or "none".
    backdrop: Option<BackdropParameter>,
    /// Colour of the outline or box, "#RRGGBB" or "#RRGGBBAA". Default black for an outline, 70 % opaque black ("#000000B3") for a box.
    backdrop_color: Option<String>,
    /// How far the outline reaches beyond the letters, in canvas pixels (0.1 to 100). Default: 8 % of the font size.
    outline_width_in_pixels: Option<f32>,
    /// "bottom" (default) or "top" of the canvas.
    position: Option<PositionParameter>,
    /// Distance from that edge of the canvas to the nearest line, in canvas pixels. Default: 25 % of the height on vertical canvases (clear of TikTok's, Reels' and Shorts' buttons), 8 % otherwise.
    margin_in_pixels: Option<f32>,
    /// The widest a line may be, in percent of the canvas width (20 to 100, default 80); longer lines wrap between words.
    maximum_line_width_percent: Option<f32>,
}

/// What is drawn behind subtitle letters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum BackdropParameter {
    /// A line around every letter.
    Outline,
    /// A box behind each line.
    Box,
    /// Nothing.
    None,
}

/// Where subtitles sit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum PositionParameter {
    /// Near the bottom.
    Bottom,
    /// Near the top.
    Top,
}

/// Fraction of the font size used for a new outline's width.
const NEW_OUTLINE_WIDTH_FRACTION_OF_FONT_SIZE: f32 = 0.08;
/// The colour of a new box: black, 70 % opaque.
const NEW_BOX_COLOR: Color = Color {
    red: 0,
    green: 0,
    blue: 0,
    alpha: 179,
};

impl SubtitleStyleParameters {
    /// `style` with these changes applied.
    fn applied_to(&self, style: &SubtitleStyle) -> Result<SubtitleStyle, ToolError> {
        let mut changed = style.clone();
        if let Some(font_family) = &self.font_family {
            changed.font_family = font_family.trim().to_string();
        }
        if let Some(font_size_in_pixels) = self.font_size_in_pixels {
            changed.font_size_in_pixels = font_size_in_pixels;
        }
        if let Some(bold) = self.bold {
            changed.is_bold = bold;
        }
        if let Some(text_color) = &self.text_color {
            changed.text_color = parse_color("text_color", text_color)?;
        }
        let backdrop_color = self
            .backdrop_color
            .as_deref()
            .map(|color| parse_color("backdrop_color", color))
            .transpose()?;
        changed.backdrop = self.changed_backdrop(&changed, backdrop_color)?;
        if let Some(position) = self.position {
            changed.placement = match position {
                PositionParameter::Bottom => SubtitlePlacement::Bottom,
                PositionParameter::Top => SubtitlePlacement::Top,
            };
        }
        if let Some(margin_in_pixels) = self.margin_in_pixels {
            changed.margin_in_pixels = margin_in_pixels;
        }
        if let Some(percent) = self.maximum_line_width_percent {
            changed.maximum_line_width_fraction = percent / 100.0;
        }
        Ok(changed)
    }

    fn changed_backdrop(
        &self,
        style: &SubtitleStyle,
        backdrop_color: Option<Color>,
    ) -> Result<SubtitleBackdrop, ToolError> {
        let current_color = match style.backdrop {
            SubtitleBackdrop::Outline { color, .. } | SubtitleBackdrop::Box { color } => {
                Some(color)
            }
            SubtitleBackdrop::None => None,
        };
        let kind = match self.backdrop {
            Some(kind) => kind,
            None => match style.backdrop {
                SubtitleBackdrop::Outline { .. } => BackdropParameter::Outline,
                SubtitleBackdrop::Box { .. } => BackdropParameter::Box,
                SubtitleBackdrop::None => BackdropParameter::None,
            },
        };
        if self.outline_width_in_pixels.is_some() && kind != BackdropParameter::Outline {
            return Err(ToolError::new(
                "outline_width_in_pixels only applies to an outline: also give \"backdrop\": \"outline\"",
            ));
        }
        let keeps_kind = self.backdrop.is_none()
            || matches!(
                (self.backdrop, style.backdrop),
                (
                    Some(BackdropParameter::Outline),
                    SubtitleBackdrop::Outline { .. }
                ) | (Some(BackdropParameter::Box), SubtitleBackdrop::Box { .. })
            );
        Ok(match kind {
            BackdropParameter::None => {
                if backdrop_color.is_some() {
                    return Err(ToolError::new(
                        "backdrop_color needs a backdrop: give \"backdrop\": \"outline\" or \"box\"",
                    ));
                }
                SubtitleBackdrop::None
            }
            BackdropParameter::Outline => {
                let current_width = match style.backdrop {
                    SubtitleBackdrop::Outline {
                        width_in_pixels, ..
                    } => Some(width_in_pixels),
                    _ => None,
                };
                SubtitleBackdrop::Outline {
                    color: backdrop_color
                        .or(current_color.filter(|_| keeps_kind))
                        .unwrap_or(Color::BLACK),
                    width_in_pixels: self.outline_width_in_pixels.or(current_width).unwrap_or(
                        (style.font_size_in_pixels * NEW_OUTLINE_WIDTH_FRACTION_OF_FONT_SIZE)
                            .max(0.5),
                    ),
                }
            }
            BackdropParameter::Box => SubtitleBackdrop::Box {
                color: backdrop_color
                    .or(current_color.filter(|_| keeps_kind))
                    .unwrap_or(NEW_BOX_COLOR),
            },
        })
    }
}

/// A subtitle style, for Claude.
#[derive(Debug, Serialize)]
struct SubtitleStyleView {
    font_family: String,
    font_size_in_pixels: f32,
    bold: bool,
    text_color: String,
    backdrop: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    backdrop_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    outline_width_in_pixels: Option<f32>,
    position: &'static str,
    margin_in_pixels: f32,
    maximum_line_width_percent: f32,
}

impl SubtitleStyleView {
    fn of(style: &SubtitleStyle) -> Self {
        let (backdrop, backdrop_color, outline_width_in_pixels) = match style.backdrop {
            SubtitleBackdrop::None => ("none", None, None),
            SubtitleBackdrop::Outline {
                color,
                width_in_pixels,
            } => ("outline", Some(color.to_hex()), Some(width_in_pixels)),
            SubtitleBackdrop::Box { color } => ("box", Some(color.to_hex()), None),
        };
        Self {
            font_family: style.font_family.clone(),
            font_size_in_pixels: style.font_size_in_pixels,
            bold: style.is_bold,
            text_color: style.text_color.to_hex(),
            backdrop,
            backdrop_color,
            outline_width_in_pixels,
            position: style.placement.display_name(),
            margin_in_pixels: style.margin_in_pixels,
            maximum_line_width_percent: (style.maximum_line_width_fraction * 1000.0).round() / 10.0,
        }
    }
}

/// One cue, for Claude.
#[derive(Debug, Serialize)]
struct CueView {
    clip_id: u64,
    start_time_in_seconds: f64,
    end_time_in_seconds: f64,
    text: String,
}

impl CueView {
    fn of(clip_id: ClipId, cue: &TimedSubtitleCue) -> Self {
        Self {
            clip_id: clip_id.raw(),
            start_time_in_seconds: seconds_of(cue.start),
            end_time_in_seconds: seconds_of(cue.end),
            text: cue.text.clone(),
        }
    }
}

/// A subtitle track and some of its cues.
#[derive(Debug, Serialize)]
struct SubtitleTrackView {
    track_id: u64,
    name: String,
    is_hidden: bool,
    cue_count: usize,
    style: SubtitleStyleView,
    cues: Vec<CueView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cues_not_listed: Option<usize>,
}

impl SubtitleTrackView {
    fn of(project: &Project, track: &Track, cues: Vec<CueView>, cues_not_listed: usize) -> Self {
        let style = track
            .subtitle_style
            .clone()
            .unwrap_or_else(|| SubtitleStyle::for_canvas(project.settings().canvas_size));
        Self {
            track_id: track.id.raw(),
            name: track.name.clone(),
            is_hidden: track.is_hidden,
            cue_count: track.clips().len(),
            style: SubtitleStyleView::of(&style),
            cues,
            cues_not_listed: (cues_not_listed > 0).then_some(cues_not_listed),
        }
    }
}

/// The cues of a track with their identifiers, in time order.
fn timed_cues_with_ids(track: &Track) -> Vec<(ClipId, TimedSubtitleCue)> {
    track
        .clips()
        .iter()
        .filter_map(|clip| match &clip.content {
            kadr_core::clip::ClipContent::Subtitle(cue) => Some((
                clip.id,
                TimedSubtitleCue {
                    start: clip.timeline_start,
                    end: clip.timeline_end(),
                    text: cue.text.clone(),
                },
            )),
            _ => None,
        })
        .collect()
}

/// The subtitle track `track_id`, or the only one when `None`.
fn chosen_subtitle_track(project: &Project, track_id: Option<u64>) -> Result<&Track, ToolError> {
    let subtitle_tracks: Vec<&Track> = project.timeline().subtitle_tracks().collect();
    let list = || {
        subtitle_tracks
            .iter()
            .map(|track| format!("{} (\"{}\")", track.id, track.name))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match track_id {
        Some(raw_track_id) => {
            let track = project.require_track(TrackId::from_raw(raw_track_id))?;
            if track.kind != TrackKind::Subtitle {
                return Err(ToolError::new(format!(
                    "track {raw_track_id} is a {} track, not a subtitle track; subtitle tracks: {}",
                    track.kind.display_name(),
                    if subtitle_tracks.is_empty() {
                        "none (import_subtitles adds one)".to_string()
                    } else {
                        list()
                    }
                )));
            }
            Ok(track)
        }
        None => match subtitle_tracks.as_slice() {
            [only_track] => Ok(only_track),
            [] => Err(ToolError::new(
                "the project has no subtitle track: import an .srt file with import_subtitles, or add cues with edit_subtitles (action \"add\")",
            )),
            _ => Err(ToolError::new(format!(
                "the project has several subtitle tracks: give track_id, one of {}",
                list()
            ))),
        },
    }
}

// ----- import_subtitles -----

const IMPORT_SUBTITLES_DESCRIPTION: &str = "\
Imports an .srt subtitle file into a new subtitle track on top of the timeline: one cue per \
subtitle, all drawn in one style that suits the canvas (white letters with a black outline, \
centred, above the bottom margin; on a vertical canvas, clear of TikTok's buttons). Subtitles \
follow the timeline: a cue at 12 s shows at 12 s of the edit. With start_time_in_seconds, every \
cue moves that much later (for subtitles made for a clip that starts later). The file is read \
first: a broken line is reported with its number and nothing changes. Line breaks, <i> and <b> \
are kept; colours and positions written in the file are not (the style sets them). Change the \
style now or later with set_subtitle_style, check a frame with get_frame at a cue's time, and \
fix cues with edit_subtitles. One undo removes the whole import.

Example: {\"file_path\": \"C:/Users/me/Videos/interview.srt\", \"style\": {\"backdrop\": \"box\"}}";

/// Parameters of `import_subtitles`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ImportSubtitlesParameters {
    /// The full path of the .srt file, inside an allowed folder.
    file_path: PathBuf,
    /// Moves every cue this many seconds later on the timeline. Default 0: the file's times are timeline times.
    start_time_in_seconds: Option<f64>,
    /// The new track's name. Default: the file's name without .srt.
    track_name: Option<String>,
    /// Changes to the default style (see set_subtitle_style).
    style: Option<SubtitleStyleParameters>,
}

/// The answer of `import_subtitles`.
#[derive(Debug, Serialize)]
struct ImportedSubtitlesView {
    track_id: u64,
    cue_count: usize,
    first_cue_start_in_seconds: f64,
    last_cue_end_in_seconds: f64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
    style: SubtitleStyleView,
    first_cues: Vec<CueView>,
}

fn import_subtitles(
    toolbox: &mut ToolCall<'_>,
    parameters: ImportSubtitlesParameters,
) -> Result<ToolOutput, ToolError> {
    let file_path = checked_subtitle_file(toolbox, &parameters.file_path)?;
    let project = toolbox.require_project()?;
    let offset = span_from_seconds(
        "start_time_in_seconds",
        parameters.start_time_in_seconds.unwrap_or(0.0),
    )?;
    let canvas_size = project.settings().canvas_size;
    let style = match &parameters.style {
        Some(changes) => changes.applied_to(&SubtitleStyle::for_canvas(canvas_size))?,
        None => SubtitleStyle::for_canvas(canvas_size),
    };
    // Read before changing anything: a broken file is reported now.
    let parsed = read_subtitle_file(&file_path)?;
    let track_name = parameters
        .track_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map_or_else(|| subtitle_track_name_for(&file_path), str::to_string);
    let file_name = file_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let added = toolbox.change_project(
        &format!("Import the subtitles of \"{file_name}\""),
        |transaction| {
            Ok(add_subtitle_track_with_cues(
                transaction,
                &track_name,
                Some(style.clone()),
                &parsed.cues,
                offset,
            )?)
        },
    )?;
    let project = toolbox.require_project()?;
    let track = project.require_track(added.track_id)?;
    let cues = timed_cues_with_ids(track);
    let view = ImportedSubtitlesView {
        track_id: added.track_id.raw(),
        cue_count: cues.len(),
        first_cue_start_in_seconds: cues.first().map_or(0.0, |(_, cue)| seconds_of(cue.start)),
        last_cue_end_in_seconds: cues.last().map_or(0.0, |(_, cue)| seconds_of(cue.end)),
        warnings: parsed.warnings,
        style: SubtitleStyleView::of(&style),
        first_cues: cues
            .iter()
            .take(IMPORT_PREVIEW_CUE_COUNT)
            .map(|(clip_id, cue)| CueView::of(*clip_id, cue))
            .collect(),
    };
    let look_at = cues.first().map_or(0.0, |(_, cue)| {
        ((seconds_of(cue.start) + seconds_of(cue.end)) / 2.0 * 1000.0).round() / 1000.0
    });
    Ok(ToolOutput::message_with_details(
        format!(
            "Imported {} subtitle cue(s) from \"{file_name}\" into subtitle track {} (one undo step). \
             Check the look with get_frame at {look_at} s; list the cues with list_subtitles.",
            view.cue_count, view.track_id
        ),
        &view,
    ))
}

/// The canonical path of an `.srt` file inside the allowed folders.
fn checked_subtitle_file(toolbox: &ToolCall<'_>, file_path: &Path) -> Result<PathBuf, ToolError> {
    let file_path = toolbox.allowed_folders().check_existing_file(file_path)?;
    if !is_subtitle_file_path(&file_path) {
        return Err(ToolError::new(format!(
            "\"{}\" is not an .srt file: Kadr imports subtitles in the SubRip (.srt) format",
            file_path.display()
        )));
    }
    Ok(file_path)
}

// ----- list_subtitles -----

const LIST_SUBTITLES_DESCRIPTION: &str = "\
Lists subtitle cues with their clip_id, start and end on the timeline (seconds) and text (line \
breaks as \\n, <i>/<b> for italic and bold), and the style of their track. Without track_id, \
every subtitle track. Narrow the list with start_time_in_seconds / end_time_in_seconds (cues \
showing in that window) or text_contains (case-insensitive). At most maximum_cues (default \
200) are listed per track; the answer says how many more there are.

Example: {\"start_time_in_seconds\": 60, \"end_time_in_seconds\": 90}";

/// Parameters of `list_subtitles`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListSubtitlesParameters {
    /// One subtitle track. Default: all of them.
    track_id: Option<u64>,
    /// Only cues still showing at or after this time, in seconds.
    start_time_in_seconds: Option<f64>,
    /// Only cues starting before this time, in seconds.
    end_time_in_seconds: Option<f64>,
    /// Only cues whose words contain this (case-insensitive).
    text_contains: Option<String>,
    /// The most cues listed per track, 1 to 1000. Default 200.
    maximum_cues: Option<usize>,
}

fn list_subtitles(
    toolbox: &mut ToolCall<'_>,
    parameters: ListSubtitlesParameters,
) -> Result<ToolOutput, ToolError> {
    let project = toolbox.require_project()?;
    let tracks: Vec<&Track> = match parameters.track_id {
        Some(_) => vec![chosen_subtitle_track(project, parameters.track_id)?],
        None => project.timeline().subtitle_tracks().collect(),
    };
    if tracks.is_empty() {
        return Ok(ToolOutput::text(
            "The project has no subtitle track. Import an .srt file with import_subtitles.",
        ));
    }
    let window_start = parameters
        .start_time_in_seconds
        .map(|seconds| timestamp_from_seconds("start_time_in_seconds", seconds))
        .transpose()?;
    let window_end = parameters
        .end_time_in_seconds
        .map(|seconds| timestamp_from_seconds("end_time_in_seconds", seconds))
        .transpose()?;
    let maximum_cues = parameters.maximum_cues.unwrap_or(DEFAULT_LISTED_CUE_COUNT);
    if !(1..=MAXIMUM_LISTED_CUE_COUNT).contains(&maximum_cues) {
        return Err(ToolError::new(format!(
            "maximum_cues must be between 1 and {MAXIMUM_LISTED_CUE_COUNT}, but is {maximum_cues}"
        )));
    }
    let wanted_words = parameters
        .text_contains
        .as_deref()
        .map(str::to_lowercase)
        .filter(|words| !words.is_empty());
    let views: Vec<SubtitleTrackView> = tracks
        .into_iter()
        .map(|track| {
            let matching: Vec<CueView> = timed_cues_with_ids(track)
                .into_iter()
                .filter(|(_, cue)| window_start.is_none_or(|start| cue.end > start))
                .filter(|(_, cue)| window_end.is_none_or(|end| cue.start < end))
                .filter(|(_, cue)| {
                    wanted_words.as_ref().is_none_or(|words| {
                        plain_text_of_cue(&cue.text).to_lowercase().contains(words)
                    })
                })
                .map(|(clip_id, cue)| CueView::of(clip_id, &cue))
                .collect();
            let not_listed = matching.len().saturating_sub(maximum_cues);
            let listed = matching.into_iter().take(maximum_cues).collect();
            SubtitleTrackView::of(project, track, listed, not_listed)
        })
        .collect();
    let listed_count: usize = views.iter().map(|view| view.cues.len()).sum();
    Ok(ToolOutput::message_with_details(
        format!(
            "{listed_count} cue(s) listed from {} subtitle track(s):",
            views.len()
        ),
        &views,
    ))
}

// ----- edit_subtitles -----

const EDIT_SUBTITLES_DESCRIPTION: &str = "\
Changes subtitle cues: fixes words and timings, adds, removes, splits and merges cues. Give \
every change in one call: they are applied in order as ONE undo step, and if one is impossible \
nothing changes. Times are timeline seconds; cues of a track cannot overlap, and an edge within \
half a frame of a neighbouring cue's edge snaps onto it. Texts may hold \\n (line break), <i> and \
<b>. Actions:
- {\"action\": \"edit\", \"clip_id\": 12, \"text\": \"…\", \"start_time_in_seconds\": 4.2, \
\"end_time_in_seconds\": 6.0}: any of text / start / end (moving only the start keeps the end);
- {\"action\": \"add\", \"start_time_in_seconds\": 8, \"end_time_in_seconds\": 9.5, \"text\": \
\"…\"}: on track_id, or the only subtitle track (created if there is none);
- {\"action\": \"remove\", \"clip_id\": 13};
- {\"action\": \"split\", \"clip_id\": 14, \"time_in_seconds\": 21.3}: two cues; the words are \
shared between words, or give first_text and second_text;
- {\"action\": \"merge\", \"clip_id\": 15, \"next_clip_id\": 16}: one cue from the start of the \
first to the end of the next, words joined by a space.
The answer gives the clip_id of every new cue. Find cues with list_subtitles.

Example: {\"changes\": [{\"action\": \"edit\", \"clip_id\": 12, \"text\": \"It's ready.\"}, \
{\"action\": \"remove\", \"clip_id\": 13}]}";

/// Parameters of `edit_subtitles`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EditSubtitlesParameters {
    /// The changes, applied in this order as one undo step.
    changes: Vec<SubtitleChange>,
}

/// One change to the subtitles.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum SubtitleChange {
    /// New words and/or times for a cue.
    Edit {
        /// The cue, from list_subtitles.
        clip_id: u64,
        /// The new words.
        text: Option<String>,
        /// The new start, in timeline seconds.
        start_time_in_seconds: Option<f64>,
        /// The new end, in timeline seconds.
        end_time_in_seconds: Option<f64>,
    },
    /// A new cue.
    Add {
        /// The subtitle track. Default: the only one (created if there is none).
        track_id: Option<u64>,
        /// When it appears, in timeline seconds.
        start_time_in_seconds: f64,
        /// When it disappears, in timeline seconds.
        end_time_in_seconds: f64,
        /// Its words.
        text: String,
    },
    /// Removes a cue.
    Remove {
        /// The cue.
        clip_id: u64,
    },
    /// Cuts a cue in two at a time.
    Split {
        /// The cue.
        clip_id: u64,
        /// Where to cut, in timeline seconds, strictly inside the cue.
        time_in_seconds: f64,
        /// The first part's words. Default: the words before the cut, chosen by length.
        first_text: Option<String>,
        /// The second part's words. Default: the words after the cut.
        second_text: Option<String>,
    },
    /// Merges a cue with the next cue of its track.
    Merge {
        /// The first cue.
        clip_id: u64,
        /// The cue right after it.
        next_clip_id: u64,
    },
}

/// What one change did, for the answer.
#[derive(Debug, Serialize)]
struct ChangeDoneView {
    action: &'static str,
    clip_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_clip_id: Option<u64>,
}

fn edit_subtitles(
    toolbox: &mut ToolCall<'_>,
    parameters: EditSubtitlesParameters,
) -> Result<ToolOutput, ToolError> {
    if parameters.changes.is_empty() {
        return Err(ToolError::new(
            "give at least one change in \"changes\" (see the examples in the description)",
        ));
    }
    let project = toolbox.require_project()?;
    let tolerance = half_frame(project.settings().frame_rate);
    let change_count = parameters.changes.len();
    let label = if change_count == 1 {
        "Edit a subtitle cue".to_string()
    } else {
        format!("Edit {change_count} subtitle cues")
    };
    let done = toolbox.change_project(&label, |transaction| {
        let mut done = Vec::with_capacity(change_count);
        for (index, change) in parameters.changes.iter().enumerate() {
            let result = apply_subtitle_change(transaction, change, tolerance)
                .map_err(|error| error.in_context(format!("change {}", index + 1)))?;
            done.push(result);
        }
        Ok(done)
    })?;
    let new_clip_ids: Vec<String> = done
        .iter()
        .filter_map(|change| change.new_clip_id)
        .map(|clip_id| clip_id.to_string())
        .collect();
    let mut message =
        format!("Applied {change_count} change(s) to the subtitles, as one undo step.");
    if !new_clip_ids.is_empty() {
        message.push_str(&format!(" New cue(s): {}.", new_clip_ids.join(", ")));
    }
    Ok(ToolOutput::message_with_details(message, &done))
}

fn apply_subtitle_change(
    transaction: &mut Transaction<'_>,
    change: &SubtitleChange,
    tolerance: TimeSpan,
) -> Result<ChangeDoneView, ToolError> {
    match change {
        SubtitleChange::Edit {
            clip_id,
            text,
            start_time_in_seconds,
            end_time_in_seconds,
        } => {
            let cue_id = ClipId::from_raw(*clip_id);
            let (track, clip) = require_cue(transaction.project(), cue_id)?;
            let snap = |time| snap_to_nearby_clip_edge(track, time, Some(cue_id), tolerance);
            let start = start_time_in_seconds
                .map(|seconds| timestamp_from_seconds("start_time_in_seconds", seconds))
                .transpose()?
                .map(snap);
            let end = end_time_in_seconds
                .map(|seconds| timestamp_from_seconds("end_time_in_seconds", seconds))
                .transpose()?
                .map(snap);
            let (current_start, current_end) = (clip.timeline_start, clip.timeline_end());
            if text.is_none() && start.is_none() && end.is_none() {
                return Err(ToolError::new(format!(
                    "the edit of cue {clip_id} changes nothing: give text, start_time_in_seconds or end_time_in_seconds"
                )));
            }
            if let Some(text) = text {
                transaction.execute(SetSubtitleCueText::new(cue_id, text.clone()))?;
            }
            if start.is_some() || end.is_some() {
                transaction.execute(SetSubtitleCueTiming::new(
                    cue_id,
                    start.unwrap_or(current_start),
                    end.unwrap_or(current_end),
                ))?;
            }
            Ok(ChangeDoneView {
                action: "edit",
                clip_id: *clip_id,
                new_clip_id: None,
            })
        }
        SubtitleChange::Add {
            track_id,
            start_time_in_seconds,
            end_time_in_seconds,
            text,
        } => {
            let start = timestamp_from_seconds("start_time_in_seconds", *start_time_in_seconds)?;
            let end = timestamp_from_seconds("end_time_in_seconds", *end_time_in_seconds)?;
            let track_id = subtitle_track_for_new_cue(transaction, *track_id)?;
            let track = transaction.project().require_track(track_id)?;
            let (start, end) = (
                snap_to_nearby_clip_edge(track, start, None, tolerance),
                snap_to_nearby_clip_edge(track, end, None, tolerance),
            );
            let created = transaction.execute(AddSubtitleCues::new(
                track_id,
                vec![TimedSubtitleCue {
                    start,
                    end,
                    text: text.clone(),
                }],
            ))?;
            let new_clip_id = created
                .first_clip_id()
                .ok_or_else(|| missing_created_object("cue"))?;
            Ok(ChangeDoneView {
                action: "add",
                clip_id: new_clip_id.raw(),
                new_clip_id: Some(new_clip_id.raw()),
            })
        }
        SubtitleChange::Remove { clip_id } => {
            let cue_id = ClipId::from_raw(*clip_id);
            require_cue(transaction.project(), cue_id)?;
            transaction.execute(RemoveClip::new(cue_id))?;
            Ok(ChangeDoneView {
                action: "remove",
                clip_id: *clip_id,
                new_clip_id: None,
            })
        }
        SubtitleChange::Split {
            clip_id,
            time_in_seconds,
            first_text,
            second_text,
        } => {
            let cue_id = ClipId::from_raw(*clip_id);
            require_cue(transaction.project(), cue_id)?;
            let time = timestamp_from_seconds("time_in_seconds", *time_in_seconds)?;
            let created = transaction.execute(SplitClip::new(cue_id, time))?;
            let second_id = created
                .first_clip_id()
                .ok_or_else(|| missing_created_object("cue"))?;
            if let Some(first_text) = first_text {
                transaction.execute(SetSubtitleCueText::new(cue_id, first_text.clone()))?;
            }
            if let Some(second_text) = second_text {
                transaction.execute(SetSubtitleCueText::new(second_id, second_text.clone()))?;
            }
            Ok(ChangeDoneView {
                action: "split",
                clip_id: *clip_id,
                new_clip_id: Some(second_id.raw()),
            })
        }
        SubtitleChange::Merge {
            clip_id,
            next_clip_id,
        } => {
            transaction.execute(MergeSubtitleCues::new(
                ClipId::from_raw(*clip_id),
                ClipId::from_raw(*next_clip_id),
            ))?;
            Ok(ChangeDoneView {
                action: "merge",
                clip_id: *clip_id,
                new_clip_id: None,
            })
        }
    }
}

/// The cue `clip_id` and its track, or why it is not a cue.
fn require_cue(
    project: &Project,
    clip_id: ClipId,
) -> Result<(&Track, &kadr_core::clip::Clip), ToolError> {
    let (track, clip) = project.require_clip(clip_id)?;
    if track.kind != TrackKind::Subtitle {
        return Err(ToolError::new(format!(
            "clip {clip_id} is not a subtitle cue (it is on {} track {}); find cues with list_subtitles",
            track.kind.display_name(),
            track.id
        )));
    }
    Ok((track, clip))
}

/// The track a new cue goes on: `track_id`, the only subtitle track, or a new one.
fn subtitle_track_for_new_cue(
    transaction: &mut Transaction<'_>,
    track_id: Option<u64>,
) -> Result<TrackId, ToolError> {
    let project = transaction.project();
    if track_id.is_none() && project.timeline().subtitle_tracks().next().is_none() {
        let created =
            transaction.execute(AddTrack::new(TrackKind::Subtitle, TrackPosition::Top))?;
        return Ok(created
            .first_track_id()
            .ok_or_else(|| missing_created_object("track"))?);
    }
    Ok(chosen_subtitle_track(project, track_id)?.id)
}

// ----- set_subtitle_style -----

const SET_SUBTITLE_STYLE_DESCRIPTION: &str = "\
Changes how every cue of a subtitle track looks: font, size, bold, colour, outline or box behind \
the letters, bottom or top, margin from that edge, widest line. Omitted values stay. Sizes are \
canvas pixels. On a vertical canvas, keep the margin at about a quarter of the height so TikTok, \
Reels and Shorts do not cover the subtitles with their buttons. Check with get_frame at a cue's \
time.

Example: {\"style\": {\"backdrop\": \"box\", \"font_size_in_pixels\": 72, \"margin_in_pixels\": 520}}";

/// Parameters of `set_subtitle_style`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetSubtitleStyleParameters {
    /// The subtitle track. Default: the only one.
    track_id: Option<u64>,
    /// The values to change.
    style: SubtitleStyleParameters,
}

fn set_subtitle_style(
    toolbox: &mut ToolCall<'_>,
    parameters: SetSubtitleStyleParameters,
) -> Result<ToolOutput, ToolError> {
    let project = toolbox.require_project()?;
    let track = chosen_subtitle_track(project, parameters.track_id)?;
    let track_id = track.id;
    let current = track
        .subtitle_style
        .clone()
        .unwrap_or_else(|| SubtitleStyle::for_canvas(project.settings().canvas_size));
    let style = parameters.style.applied_to(&current)?;
    toolbox.change_project(
        &format!("Change the subtitle style of track {track_id}"),
        |transaction| {
            transaction.execute(SetSubtitleStyle::new(track_id, style.clone()))?;
            Ok(())
        },
    )?;
    Ok(ToolOutput::message_with_details(
        format!(
            "Changed the style of subtitle track {track_id}. Check it with get_frame at a cue's time."
        ),
        &SubtitleStyleView::of(&style),
    ))
}

// ----- export_subtitles -----

const EXPORT_SUBTITLES_DESCRIPTION: &str = "\
Writes the cues of a subtitle track to an .srt file (UTF-8, numbered from 1, times to the \
millisecond), for YouTube or any player. The folder must be allowed; an existing file is only \
replaced with \"overwrite\": true. The subtitles burned into the video by export are separate: \
this file is for platforms that show subtitles themselves.

Example: {\"file_path\": \"C:/Users/me/Videos/interview.corrected.srt\"}";

/// Parameters of `export_subtitles`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExportSubtitlesParameters {
    /// The subtitle track. Default: the only one.
    track_id: Option<u64>,
    /// The full path of the .srt file to write (".srt" is added if missing).
    file_path: PathBuf,
    /// Replace the file if it exists. Default false: ask the user first.
    #[serde(default)]
    overwrite: bool,
}

fn export_subtitles(
    toolbox: &mut ToolCall<'_>,
    parameters: ExportSubtitlesParameters,
) -> Result<ToolOutput, ToolError> {
    let requested_path = with_extension(&parameters.file_path, SUBTITLE_FILE_EXTENSION)?;
    let file_path = toolbox
        .allowed_folders()
        .check_file_to_write(&requested_path, parameters.overwrite)?;
    let project = toolbox.require_project()?;
    let track = chosen_subtitle_track(project, parameters.track_id)?;
    if track.clips().is_empty() {
        return Err(ToolError::new(format!(
            "subtitle track {} has no cue to export",
            track.id
        )));
    }
    let cue_count = export_subtitle_track(track, &file_path)?;
    Ok(ToolOutput::text(format!(
        "Wrote {cue_count} cue(s) of subtitle track {} to \"{}\".",
        track.id,
        file_path.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_changes_switch_backdrops_with_sensible_colours() {
        let vertical = SubtitleStyle::for_canvas(kadr_core::media_asset::PictureSize {
            width_in_pixels: 1080,
            height_in_pixels: 1920,
        });
        let boxed = SubtitleStyleParameters {
            backdrop: Some(BackdropParameter::Box),
            ..SubtitleStyleParameters::default()
        }
        .applied_to(&vertical)
        .unwrap();
        assert_eq!(
            boxed.backdrop,
            SubtitleBackdrop::Box {
                color: NEW_BOX_COLOR
            }
        );
        let yellow_outline = SubtitleStyleParameters {
            backdrop: Some(BackdropParameter::Outline),
            backdrop_color: Some("#FFD400".to_string()),
            outline_width_in_pixels: Some(3.0),
            maximum_line_width_percent: Some(90.0),
            ..SubtitleStyleParameters::default()
        }
        .applied_to(&boxed)
        .unwrap();
        assert_eq!(
            yellow_outline.backdrop,
            SubtitleBackdrop::Outline {
                color: Color::from_rgb(255, 212, 0),
                width_in_pixels: 3.0
            }
        );
        assert!((yellow_outline.maximum_line_width_fraction - 0.9).abs() < 1e-6);
        let refused = SubtitleStyleParameters {
            outline_width_in_pixels: Some(3.0),
            ..SubtitleStyleParameters::default()
        }
        .applied_to(&boxed);
        assert!(refused.is_err());
    }
}
