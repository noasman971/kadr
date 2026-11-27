//! Subtitles in the app (decision record 0056): an imported `.srt` file becomes a
//! subtitle track; cues are edited in the inspector, added at the playhead and
//! merged with the next one; the track's style changes every cue at once.
//!
//! Like the other settings, cue and style requests name only what changes; the
//! rest is read from the project when the request is applied.

use kadr_core::color::Color;
use kadr_core::commands::{
    AddSubtitleCues, AddTrack, CreatedObjectsExt, MergeSubtitleCues, SetSubtitleCueText,
    SetSubtitleCueTiming, SetSubtitleStyle, TrackPosition,
};
use kadr_core::error::EditError;
use kadr_core::history::{EditSession, Transaction};
use kadr_core::ids::{ClipId, TrackId};
use kadr_core::project::Project;
use kadr_core::subtitle::{SubtitleBackdrop, SubtitlePlacement, SubtitleStyle, TimedSubtitleCue};
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_core::timeline::TrackKind;
use kadr_engine::subtitle_files::add_subtitle_track_with_cues;
use serde::{Deserialize, Serialize};

use super::{EditNotice, EditOutcome, missing_created_object};

/// The name of the track cues added by hand go on when there is none (the front
/// end translates it).
pub const SUBTITLES_TRACK_NAME: &str = "Subtitles";
/// How long a cue added at the playhead lasts, at most.
pub const NEW_CUE_DURATION: TimeSpan = TimeSpan::from_whole_seconds(2);
/// The shortest cue added at the playhead (less room than this is refused).
const SHORTEST_NEW_CUE: TimeSpan = TimeSpan::from_milliseconds(200);
/// The colour of a new box behind subtitles: black, 70 % opaque.
const NEW_BOX_COLOR: Color = Color {
    red: 0,
    green: 0,
    blue: 0,
    alpha: 179,
};
/// A new outline's width, as a fraction of the font size.
const NEW_OUTLINE_WIDTH_FRACTION: f32 = 0.08;

/// The words or times of a cue to change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CueChanges {
    /// New words (line breaks, `<i>`, `<b>`).
    pub text: Option<String>,
    /// New start on the timeline.
    pub start_ticks: Option<u64>,
    /// New end on the timeline.
    pub end_ticks: Option<u64>,
}

/// What is drawn behind subtitle letters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackdropChoice {
    /// Nothing.
    None,
    /// A line around every letter.
    Outline,
    /// A box behind each line.
    Box,
}

/// Bottom or top of the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlacementChoice {
    /// Near the bottom.
    Bottom,
    /// Near the top.
    Top,
}

/// The settings of a subtitle style to change.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubtitleStyleChanges {
    /// Font family.
    pub font_family: Option<String>,
    /// Size in canvas pixels.
    pub font_size: Option<f32>,
    /// Every word bold.
    pub is_bold: Option<bool>,
    /// `#RRGGBB` or `#RRGGBBAA`.
    pub text_color: Option<String>,
    /// Outline, box or nothing.
    pub backdrop: Option<BackdropChoice>,
    /// Colour of the outline or box.
    pub backdrop_color: Option<String>,
    /// Width of the outline, in canvas pixels.
    pub outline_width: Option<f32>,
    /// Bottom or top.
    pub placement: Option<PlacementChoice>,
    /// From that edge to the nearest line, in canvas pixels.
    pub margin: Option<f32>,
    /// The widest line, as a fraction of the canvas width (0.2 to 1).
    pub maximum_line_width: Option<f32>,
}

impl SubtitleStyleChanges {
    /// `style` with these changes.
    ///
    /// # Errors
    ///
    /// A colour that cannot be read.
    pub fn applied_to(&self, style: &SubtitleStyle) -> Result<SubtitleStyle, EditError> {
        let mut changed = style.clone();
        if let Some(font_family) = &self.font_family {
            changed.font_family = font_family.trim().to_string();
        }
        if let Some(font_size) = self.font_size {
            changed.font_size_in_pixels = font_size;
        }
        if let Some(is_bold) = self.is_bold {
            changed.is_bold = is_bold;
        }
        if let Some(text_color) = &self.text_color {
            changed.text_color = color_from(text_color)?;
        }
        let backdrop_color = self.backdrop_color.as_deref().map(color_from).transpose()?;
        changed.backdrop = self.changed_backdrop(style, backdrop_color);
        if let Some(placement) = self.placement {
            changed.placement = match placement {
                PlacementChoice::Bottom => SubtitlePlacement::Bottom,
                PlacementChoice::Top => SubtitlePlacement::Top,
            };
        }
        if let Some(margin) = self.margin {
            changed.margin_in_pixels = margin;
        }
        if let Some(maximum_line_width) = self.maximum_line_width {
            changed.maximum_line_width_fraction = maximum_line_width;
        }
        Ok(changed)
    }

    /// The backdrop: another kind starts with that kind's usual colour; a colour
    /// or width given changes the current one.
    fn changed_backdrop(
        &self,
        style: &SubtitleStyle,
        backdrop_color: Option<Color>,
    ) -> SubtitleBackdrop {
        let current = style.backdrop;
        let kind = self.backdrop.unwrap_or(match current {
            SubtitleBackdrop::None => BackdropChoice::None,
            SubtitleBackdrop::Outline { .. } => BackdropChoice::Outline,
            SubtitleBackdrop::Box { .. } => BackdropChoice::Box,
        });
        match (kind, current) {
            (BackdropChoice::None, _) => SubtitleBackdrop::None,
            (
                BackdropChoice::Outline,
                SubtitleBackdrop::Outline {
                    color,
                    width_in_pixels,
                },
            ) => SubtitleBackdrop::Outline {
                color: backdrop_color.unwrap_or(color),
                width_in_pixels: self.outline_width.unwrap_or(width_in_pixels),
            },
            (BackdropChoice::Outline, _) => SubtitleBackdrop::Outline {
                color: backdrop_color.unwrap_or(Color::BLACK),
                width_in_pixels: self.outline_width.unwrap_or(
                    (style.font_size_in_pixels * NEW_OUTLINE_WIDTH_FRACTION * 10.0).round() / 10.0,
                ),
            },
            (BackdropChoice::Box, SubtitleBackdrop::Box { color }) => SubtitleBackdrop::Box {
                color: backdrop_color.unwrap_or(color),
            },
            (BackdropChoice::Box, _) => SubtitleBackdrop::Box {
                color: backdrop_color.unwrap_or(NEW_BOX_COLOR),
            },
        }
    }
}

fn color_from(text: &str) -> Result<Color, EditError> {
    Color::from_hex(text).map_err(|error| EditError::InternalInconsistency {
        details: format!("the interface sent a colour that cannot be read: {error}"),
    })
}

/// The cue `clip_id` with its start and end, or why it is not a cue.
fn cue_times(project: &Project, clip_id: ClipId) -> Result<(Timestamp, Timestamp), EditError> {
    let (track, clip) = project.require_clip(clip_id)?;
    if track.kind != TrackKind::Subtitle {
        return Err(EditError::NotASubtitleCue { clip_id });
    }
    Ok((clip.timeline_start, clip.timeline_end()))
}

/// Changes a cue's words and times as one undo step: "editSubtitle", or
/// "moveSubtitle" when only its times change.
///
/// # Errors
///
/// The engine's refusal (no words, an end before the start, an overlap).
pub fn set_subtitle_cue(
    session: &mut EditSession,
    clip_id: ClipId,
    changes: &CueChanges,
) -> Result<EditOutcome, EditError> {
    let (current_start, current_end) = cue_times(session.project(), clip_id)?;
    let label = if changes.text.is_some() {
        "editSubtitle"
    } else {
        "moveSubtitle"
    };
    session.transaction(label, |transaction| {
        if let Some(text) = &changes.text {
            transaction.execute(SetSubtitleCueText::new(clip_id, text.clone()))?;
        }
        if changes.start_ticks.is_some() || changes.end_ticks.is_some() {
            let start = changes
                .start_ticks
                .map_or(current_start, Timestamp::from_ticks);
            let end = changes.end_ticks.map_or(current_end, Timestamp::from_ticks);
            transaction.execute(SetSubtitleCueTiming::new(clip_id, start, end))?;
        }
        Ok(EditOutcome::default())
    })
}

/// Adds a cue with `text` at `start` ("addSubtitle"): on `track_id`, or the top
/// subtitle track, or a new *Subtitles* track. It lasts two seconds, less if the
/// next cue starts sooner.
///
/// # Errors
///
/// Another cue already shows at `start`, or there is less than 0.2 s before the
/// next one.
pub fn add_subtitle_cue(
    session: &mut EditSession,
    start: Timestamp,
    track_id: Option<TrackId>,
    text: &str,
) -> Result<EditOutcome, EditError> {
    session.transaction("addSubtitle", |transaction| {
        let track_id = subtitle_track_for_new_cue(transaction, track_id)?;
        let track = transaction.project().require_track(track_id)?;
        let next_cue_start = track
            .clips()
            .iter()
            .map(|clip| clip.timeline_start)
            .filter(|cue_start| *cue_start > start)
            .min();
        let mut end = start + NEW_CUE_DURATION;
        if let Some(next_cue_start) = next_cue_start {
            end = end.min(next_cue_start);
        }
        let too_short = end
            .span_since(start)
            .is_none_or(|duration| duration < SHORTEST_NEW_CUE);
        if too_short {
            return Err(EditError::CueEndsBeforeItStarts { start, end });
        }
        let created = transaction.execute(AddSubtitleCues::new(
            track_id,
            vec![TimedSubtitleCue {
                start,
                end,
                text: text.to_string(),
            }],
        ))?;
        let clip_id = created.first_clip_id().ok_or_else(missing_created_object)?;
        Ok(EditOutcome {
            created_clip_ids: vec![clip_id],
            notices: Vec::new(),
        })
    })
}

/// The track a new cue goes on.
fn subtitle_track_for_new_cue(
    transaction: &mut Transaction<'_>,
    track_id: Option<TrackId>,
) -> Result<TrackId, EditError> {
    if let Some(track_id) = track_id {
        return Ok(track_id);
    }
    let top_subtitle_track = transaction
        .project()
        .timeline()
        .subtitle_tracks()
        .last()
        .map(|track| track.id);
    match top_subtitle_track {
        Some(track_id) => Ok(track_id),
        None => transaction
            .execute(
                AddTrack::new(TrackKind::Subtitle, TrackPosition::Top).named(SUBTITLES_TRACK_NAME),
            )?
            .first_track_id()
            .ok_or_else(missing_created_object),
    }
}

/// Merges a cue with the next cue of its track ("mergeSubtitles").
///
/// # Errors
///
/// The cue is the last of its track, or not a cue.
pub fn merge_subtitle_with_next(
    session: &mut EditSession,
    clip_id: ClipId,
) -> Result<EditOutcome, EditError> {
    let project = session.project();
    cue_times(project, clip_id)?;
    let (track, _) = project.require_clip(clip_id)?;
    let clips = track.clips();
    let next_clip_id = clips
        .iter()
        .position(|clip| clip.id == clip_id)
        .and_then(|index| clips.get(index + 1))
        .map(|clip| clip.id)
        .ok_or(EditError::CuesCannotBeMerged {
            first_clip_id: clip_id,
            second_clip_id: clip_id,
            reason: "it is the last cue of its track".to_string(),
        })?;
    session.transaction("mergeSubtitles", |transaction| {
        transaction.execute(MergeSubtitleCues::new(clip_id, next_clip_id))?;
        Ok(EditOutcome::default())
    })
}

/// Changes some settings of a subtitle track's style ("changeSubtitleStyle").
///
/// # Errors
///
/// Not a subtitle track, or a setting out of range.
pub fn set_subtitle_style(
    session: &mut EditSession,
    track_id: TrackId,
    changes: &SubtitleStyleChanges,
) -> Result<EditOutcome, EditError> {
    let project = session.project();
    let track = project.require_track(track_id)?;
    let current = track
        .subtitle_style
        .clone()
        .unwrap_or_else(|| SubtitleStyle::for_canvas(project.settings().canvas_size));
    let style = changes.applied_to(&current)?;
    session.transaction("changeSubtitleStyle", |transaction| {
        transaction.execute(SetSubtitleStyle::new(track_id, style))?;
        Ok(EditOutcome::default())
    })
}

/// Adds the cues of an `.srt` file on a new subtitle track named `track_name`
/// ("importSubtitles"). The first cue is selected; the notice gives the count
/// and what was changed to read the file.
///
/// # Errors
///
/// The engine's refusal (none expected: the reader already checked the cues).
pub fn import_subtitles(
    session: &mut EditSession,
    track_name: &str,
    cues: &[TimedSubtitleCue],
    warnings: Vec<String>,
) -> Result<EditOutcome, EditError> {
    let added = session.transaction("importSubtitles", |transaction| {
        add_subtitle_track_with_cues(transaction, track_name, None, cues, TimeSpan::ZERO)
    })?;
    Ok(EditOutcome {
        created_clip_ids: added.cue_ids.first().copied().into_iter().collect(),
        notices: vec![EditNotice::SubtitlesImported {
            cue_count: added.cue_ids.len(),
            warnings,
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kadr_core::clip::ClipContent;
    use kadr_core::project::ProjectSettings;

    fn session_with_cues() -> (EditSession, Vec<ClipId>) {
        let mut session =
            EditSession::new(Project::new(ProjectSettings::vertical_1080x1920())).unwrap();
        let cue = |start: u64, end: u64, text: &str| TimedSubtitleCue {
            start: Timestamp::from_milliseconds(start),
            end: Timestamp::from_milliseconds(end),
            text: text.to_string(),
        };
        let outcome = import_subtitles(
            &mut session,
            "interview",
            &[
                cue(0, 1_000, "One"),
                cue(1_000, 2_500, "Two"),
                cue(4_000, 5_000, "Three"),
            ],
            vec!["a warning".to_string()],
        )
        .unwrap();
        assert_eq!(
            outcome.notices,
            vec![EditNotice::SubtitlesImported {
                cue_count: 3,
                warnings: vec!["a warning".to_string()]
            }]
        );
        let cue_ids = session
            .project()
            .timeline()
            .subtitle_tracks()
            .next()
            .unwrap()
            .clips()
            .iter()
            .map(|clip| clip.id)
            .collect();
        (session, cue_ids)
    }

    fn cue_text(session: &EditSession, clip_id: ClipId) -> String {
        match &session
            .project()
            .timeline()
            .find_clip(clip_id)
            .unwrap()
            .1
            .content
        {
            ClipContent::Subtitle(cue) => cue.text.clone(),
            _ => panic!("not a cue"),
        }
    }

    #[test]
    fn a_cue_changes_only_what_is_named() {
        let (mut session, cue_ids) = session_with_cues();
        set_subtitle_cue(
            &mut session,
            cue_ids[2],
            &CueChanges {
                start_ticks: Some(Timestamp::from_milliseconds(3_000).ticks()),
                ..CueChanges::default()
            },
        )
        .unwrap();
        assert_eq!(session.next_undo_label(), Some("moveSubtitle"));
        let (_, clip) = session.project().timeline().find_clip(cue_ids[2]).unwrap();
        assert_eq!(clip.timeline_start, Timestamp::from_milliseconds(3_000));
        assert_eq!(clip.timeline_end(), Timestamp::from_milliseconds(5_000));
        set_subtitle_cue(
            &mut session,
            cue_ids[2],
            &CueChanges {
                text: Some("Three!".to_string()),
                ..CueChanges::default()
            },
        )
        .unwrap();
        assert_eq!(cue_text(&session, cue_ids[2]), "Three!");
        assert_eq!(session.next_undo_label(), Some("editSubtitle"));
    }

    #[test]
    fn a_cue_added_at_the_playhead_fits_before_the_next_one() {
        let (mut session, _) = session_with_cues();
        let outcome = add_subtitle_cue(
            &mut session,
            Timestamp::from_milliseconds(2_500),
            None,
            "New",
        )
        .unwrap();
        let (_, clip) = session
            .project()
            .timeline()
            .find_clip(outcome.created_clip_ids[0])
            .unwrap();
        assert_eq!(clip.timeline_end(), Timestamp::from_milliseconds(4_000));
        assert!(
            add_subtitle_cue(&mut session, Timestamp::from_milliseconds(3_900), None, "x").is_err()
        );
        // Without a subtitle track, one is made.
        let mut empty = EditSession::new(Project::new(ProjectSettings::landscape_1080p())).unwrap();
        add_subtitle_cue(&mut empty, Timestamp::ZERO, None, "First").unwrap();
        let track = empty.project().timeline().subtitle_tracks().next().unwrap();
        assert_eq!(track.name, SUBTITLES_TRACK_NAME);
    }

    #[test]
    fn a_cue_merges_with_the_next_and_the_style_changes_by_parts() {
        let (mut session, cue_ids) = session_with_cues();
        merge_subtitle_with_next(&mut session, cue_ids[0]).unwrap();
        assert_eq!(cue_text(&session, cue_ids[0]), "One Two");
        assert!(merge_subtitle_with_next(&mut session, cue_ids[2]).is_err());

        let track_id = session
            .project()
            .timeline()
            .subtitle_tracks()
            .next()
            .unwrap()
            .id;
        set_subtitle_style(
            &mut session,
            track_id,
            &SubtitleStyleChanges {
                backdrop: Some(BackdropChoice::Box),
                margin: Some(300.0),
                ..SubtitleStyleChanges::default()
            },
        )
        .unwrap();
        let style = session
            .project()
            .timeline()
            .track(track_id)
            .unwrap()
            .subtitle_style
            .clone()
            .unwrap();
        assert_eq!(
            style.backdrop,
            SubtitleBackdrop::Box {
                color: NEW_BOX_COLOR
            }
        );
        assert_eq!(style.margin_in_pixels, 300.0);
        assert!(style.is_bold, "unchanged");
        let refused = set_subtitle_style(
            &mut session,
            track_id,
            &SubtitleStyleChanges {
                maximum_line_width: Some(0.05),
                ..SubtitleStyleChanges::default()
            },
        );
        assert!(refused.is_err());
    }
}
