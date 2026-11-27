//! Removing the silences of a clip, as the silence dialog found them
//! ([`crate::silence_search`]).

use kadr_core::commands::RemoveClipParts;
use kadr_core::error::EditError;
use kadr_core::history::EditSession;
use kadr_core::ids::ClipId;
use kadr_core::time::{TimeRange, Timestamp};
use serde::{Deserialize, Serialize};

use super::EditOutcome;

/// Undo label of the step that removes the silences of a clip.
const REMOVE_SILENCES_UNDO_LABEL: &str = "removeSilences";

/// A stretch of the timeline, in ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TickRange {
    /// Where it starts (included).
    pub start_ticks: u64,
    /// Where it ends (excluded).
    pub end_ticks: u64,
}

impl From<TimeRange> for TickRange {
    fn from(range: TimeRange) -> Self {
        Self {
            start_ticks: range.start().ticks(),
            end_ticks: range.end().ticks(),
        }
    }
}

/// Cuts `cuts` (timeline times) out of the clip and closes the gaps: one undo
/// step. The clip keeps its identifier for its first piece, so the inspector
/// stays on it.
///
/// # Errors
///
/// When the clip does not exist, a cut is not inside it, or nothing would be left.
pub fn remove_silences(
    session: &mut EditSession,
    clip_id: ClipId,
    cuts: &[TickRange],
) -> Result<EditOutcome, EditError> {
    let parts = cuts
        .iter()
        .map(|cut| {
            TimeRange::new(
                Timestamp::from_ticks(cut.start_ticks),
                Timestamp::from_ticks(cut.end_ticks),
            )
            .ok_or_else(|| EditError::InternalInconsistency {
                details: format!(
                    "a cut of clip {clip_id} does not end after it starts ({} to {} ticks)",
                    cut.start_ticks, cut.end_ticks
                ),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    session.transaction(REMOVE_SILENCES_UNDO_LABEL, |transaction| {
        transaction.execute(RemoveClipParts::new(clip_id, parts))?;
        Ok(EditOutcome::default())
    })
}
