//! Adding titles (docs/ux.md, "Add a title"): at the playhead, on a *Titles*
//! track above the videos, 3 seconds long, in one of a few ready-made styles.

use kadr_core::clip::{AnimatableProperty, TextAlignment, TextStyle};
use kadr_core::commands::{
    AddClip, AddTrack, CreatedObjectsExt, SetClipPropertyValue, SetClipText, TrackPosition,
};
use kadr_core::error::EditError;
use kadr_core::history::{EditSession, Transaction};
use kadr_core::ids::TrackId;
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_core::timeline::TrackKind;
use serde::Deserialize;

use super::{EditOutcome, missing_created_object};

/// The name of the tracks titles go on (the front end translates it).
pub const TITLES_TRACK_NAME: &str = "Titles";
/// How long a new title lasts.
pub const TITLE_DURATION: TimeSpan = TimeSpan::from_whole_seconds(3);

/// A ready-made title style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TitleStyle {
    /// A title in the middle of the picture.
    Title,
    /// A line of text at the bottom, like a subtitle.
    Subtitle,
    /// One big word in the middle.
    BigWord,
}

impl TitleStyle {
    /// Font size as a fraction of the canvas height.
    fn font_size_fraction(self) -> f32 {
        match self {
            Self::Title => 0.09,
            Self::Subtitle => 0.05,
            Self::BigWord => 0.2,
        }
    }

    /// Vertical position as a fraction of the canvas height, from the centre (down is positive).
    fn vertical_position_fraction(self) -> f32 {
        match self {
            Self::Title | Self::BigWord => 0.0,
            Self::Subtitle => 0.38,
        }
    }
}

/// Adds a title at `start` with `text`, selected afterwards (its id is returned).
/// It goes on the first *Titles* track free at that time, or on a new one on top.
///
/// # Errors
///
/// The engine's refusal (an empty text, for example).
pub fn add_title(
    session: &mut EditSession,
    start: Timestamp,
    text: &str,
    style: TitleStyle,
) -> Result<EditOutcome, EditError> {
    let canvas_height = session.project().settings().canvas_size.height_in_pixels as f32;
    session.transaction("addTitle", |transaction| {
        let track_id = free_titles_track_at(transaction, start)?;
        let clip_id = transaction
            .execute(AddClip::text(track_id, text, TITLE_DURATION, start))?
            .first_clip_id()
            .ok_or_else(missing_created_object)?;
        let text_style = TextStyle {
            font_size_in_pixels: (canvas_height * style.font_size_fraction()).round(),
            alignment: TextAlignment::Center,
            ..TextStyle::default()
        };
        transaction.execute(SetClipText::new(clip_id, text).with_style(text_style))?;
        let position_y = (canvas_height * style.vertical_position_fraction()).round();
        if position_y != 0.0 {
            transaction.execute(SetClipPropertyValue::new(
                clip_id,
                AnimatableProperty::PositionY,
                position_y,
            ))?;
        }
        Ok(EditOutcome {
            created_clip_ids: vec![clip_id],
            notices: Vec::new(),
        })
    })
}

/// The first *Titles* track with room for a title at `start`, or a new one on top.
fn free_titles_track_at(
    transaction: &mut Transaction<'_>,
    start: Timestamp,
) -> Result<TrackId, EditError> {
    let end = start + TITLE_DURATION;
    let free_track = transaction
        .project()
        .timeline()
        .tracks()
        .iter()
        .filter(|track| track.kind == TrackKind::Video && track.name == TITLES_TRACK_NAME)
        .find(|track| {
            track
                .clips()
                .iter()
                .all(|clip| clip.timeline_end() <= start || end <= clip.timeline_start)
        })
        .map(|track| track.id);
    match free_track {
        Some(track_id) => Ok(track_id),
        None => transaction
            .execute(AddTrack::new(TrackKind::Video, TrackPosition::Top).named(TITLES_TRACK_NAME))?
            .first_track_id()
            .ok_or_else(missing_created_object),
    }
}
