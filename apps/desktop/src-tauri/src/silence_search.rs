//! Looking for the silences of a clip in the background, for the silence dialog.
//!
//! Only the latest search matters: starting another one (the user changed a
//! setting) or stopping (the dialog closed) makes the running one stop after
//! its current 30 s of sound. Nothing changes until the dialog sends the cuts
//! back as an edit ([`crate::editing::EditRequest::RemoveSilences`]).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use kadr_core::clip::ClipContent;
use kadr_core::ids::ClipId;
use kadr_core::project::Project;
use kadr_core::time::TimeSpan;
use kadr_engine::silence_analysis::SilenceDetectionSettings;
use kadr_engine::silence_removal::{
    SilenceRemovalError, SilenceRemovalSettings, plan_silence_removal_with_progress,
};
use kadr_media::MediaBackend;
use serde::{Deserialize, Serialize};

use crate::command_error::{CommandError, CommandResult, ErrorCode};
use crate::editing::TickRange;

/// The loudest threshold and the quietest accepted, in dBFS.
const THRESHOLD_RANGE_IN_DECIBELS: (f32, f32) = (-100.0, -1.0);
/// The shortest and longest "shortest silence", in seconds.
const MINIMUM_SILENCE_RANGE_IN_SECONDS: (f64, f64) = (0.05, 60.0);
/// The shortest and longest pause kept, in seconds.
const KEPT_PAUSE_RANGE_IN_SECONDS: (f64, f64) = (0.0, 2.0);

/// The searches started, so that a newer one stops an older one.
#[derive(Debug, Default)]
pub struct SilenceSearches {
    latest_search_number: Arc<AtomicU64>,
}

impl SilenceSearches {
    /// A ticket for a new search; every earlier search stops.
    pub fn start_new(&self) -> SearchTicket {
        let number = self.latest_search_number.fetch_add(1, Ordering::SeqCst) + 1;
        SearchTicket {
            number,
            latest_search_number: Arc::clone(&self.latest_search_number),
        }
    }

    /// Stops every search.
    pub fn stop_all(&self) {
        self.latest_search_number.fetch_add(1, Ordering::SeqCst);
    }
}

/// One search, wanted until a newer one starts or the searches are stopped.
#[derive(Debug, Clone)]
pub struct SearchTicket {
    number: u64,
    latest_search_number: Arc<AtomicU64>,
}

impl SearchTicket {
    /// Whether the search should go on.
    pub fn is_still_wanted(&self) -> bool {
        self.latest_search_number.load(Ordering::SeqCst) == self.number
    }
}

/// The dialog's settings.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SilenceSearchSettings {
    /// Quieter than this many dBFS is silence (for example −40).
    pub threshold_in_decibels: f32,
    /// Only silences at least this long are shortened.
    pub minimum_silence_in_seconds: f64,
    /// How long each silence lasts once shortened.
    pub kept_pause_in_seconds: f64,
}

impl SilenceSearchSettings {
    /// The engine's settings, or why these are out of range.
    fn to_engine_settings(self) -> CommandResult<SilenceRemovalSettings> {
        let out_of_range = |name: &str, (lowest, highest): (f64, f64)| {
            CommandError::new(
                ErrorCode::EditRefused,
                format!("{name} must be from {lowest} to {highest}"),
            )
        };
        let (lowest, highest) = THRESHOLD_RANGE_IN_DECIBELS;
        if !(lowest..=highest).contains(&self.threshold_in_decibels) {
            return Err(out_of_range(
                "the threshold",
                (f64::from(lowest), f64::from(highest)),
            ));
        }
        if !(MINIMUM_SILENCE_RANGE_IN_SECONDS.0..=MINIMUM_SILENCE_RANGE_IN_SECONDS.1)
            .contains(&self.minimum_silence_in_seconds)
        {
            return Err(out_of_range(
                "the shortest silence",
                MINIMUM_SILENCE_RANGE_IN_SECONDS,
            ));
        }
        if !(KEPT_PAUSE_RANGE_IN_SECONDS.0..=KEPT_PAUSE_RANGE_IN_SECONDS.1)
            .contains(&self.kept_pause_in_seconds)
        {
            return Err(out_of_range("the pause kept", KEPT_PAUSE_RANGE_IN_SECONDS));
        }
        Ok(SilenceRemovalSettings {
            detection: SilenceDetectionSettings {
                threshold_in_decibels: self.threshold_in_decibels,
                minimum_silence_in_seconds: self.minimum_silence_in_seconds,
            },
            kept_pause: TimeSpan::from_seconds(self.kept_pause_in_seconds)
                .map_err(CommandError::internal)?,
        })
    }
}

/// What the dialog shows: the cuts, and how much shorter the clip gets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SilencePlanView {
    /// The parts to cut, on the timeline as the clip is now, in order.
    pub cuts: Vec<TickRange>,
    /// How much shorter the clip gets, in ticks.
    pub removed_ticks: u64,
    /// The clip is silent from start to end (nothing is cut then).
    pub whole_clip_is_silent: bool,
}

/// Finds what removing the silences of `clip_id` would cut. `on_progress`
/// receives the fraction analysed (0 to 1).
///
/// # Errors
///
/// When the settings are out of range, the clip has no readable sound, or a
/// newer search replaced this one ([`ErrorCode::Superseded`]).
pub fn search_silences(
    project: &Project,
    backend: &dyn MediaBackend,
    clip_id: ClipId,
    settings: SilenceSearchSettings,
    ticket: &SearchTicket,
    mut on_progress: impl FnMut(f64),
) -> CommandResult<SilencePlanView> {
    let engine_settings = settings.to_engine_settings()?;
    let plan = plan_silence_removal_with_progress(
        project,
        backend,
        clip_id,
        engine_settings,
        |fraction| {
            on_progress(fraction);
            ticket.is_still_wanted()
        },
    )
    .map_err(|error| command_error_for(project, clip_id, error))?;
    Ok(SilencePlanView {
        removed_ticks: plan.removed_duration().ticks(),
        cuts: plan.cuts.iter().copied().map(TickRange::from).collect(),
        whole_clip_is_silent: plan.whole_clip_is_silent,
    })
}

/// The interface's error for a failed search.
fn command_error_for(
    project: &Project,
    clip_id: ClipId,
    error: SilenceRemovalError,
) -> CommandError {
    match error {
        SilenceRemovalError::Edit(edit_error) => edit_error.into(),
        SilenceRemovalError::NoSound { .. } => CommandError::new(ErrorCode::EditRefused, error),
        SilenceRemovalError::Stopped => CommandError::new(ErrorCode::Superseded, error),
        SilenceRemovalError::Media(_) => {
            let file_path = project
                .timeline()
                .find_clip(clip_id)
                .and_then(|(_, clip)| match &clip.content {
                    ClipContent::Media(segment) => project.media_asset(segment.media_asset_id),
                    ClipContent::Text(_) => None,
                })
                .map(|media_asset| media_asset.description.file_path.clone());
            let command_error = CommandError::new(ErrorCode::MediaUnreadable, error);
            match file_path {
                Some(file_path) => command_error.about_file(&file_path),
                None => command_error,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_newer_search_or_a_stop_ends_the_earlier_ones() {
        let searches = SilenceSearches::default();
        let first = searches.start_new();
        assert!(first.is_still_wanted());
        let second = searches.start_new();
        assert!(!first.is_still_wanted());
        assert!(second.is_still_wanted());
        searches.stop_all();
        assert!(!second.is_still_wanted());
    }

    #[test]
    fn settings_out_of_range_are_refused() {
        let settings = SilenceSearchSettings {
            threshold_in_decibels: -40.0,
            minimum_silence_in_seconds: 0.5,
            kept_pause_in_seconds: 0.2,
        };
        assert!(settings.to_engine_settings().is_ok());
        let too_loud = SilenceSearchSettings {
            threshold_in_decibels: 3.0,
            ..settings
        };
        assert_eq!(
            too_loud.to_engine_settings().unwrap_err().code,
            ErrorCode::EditRefused
        );
        let pause_too_long = SilenceSearchSettings {
            kept_pause_in_seconds: 5.0,
            ..settings
        };
        assert!(pause_too_long.to_engine_settings().is_err());
    }
}
