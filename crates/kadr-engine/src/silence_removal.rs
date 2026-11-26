//! Removing the silences of a clip.
//!
//! 1. The part of the file the clip plays is analysed by
//!    [`crate::silence_analysis`]: quieter than a threshold for long enough is a
//!    silence.
//! 2. Each silence is **shortened to a short pause** rather than removed
//!    entirely, so speech does not sound rushed: half of
//!    [`SilenceRemovalSettings::kept_pause`] stays after the sound before it,
//!    half before the sound after it. A silence at the clip's start or end keeps
//!    only that half, next to the sound.
//! 3. The cuts are moved onto **frame starts** (a cut starts a little later and
//!    ends a little earlier), so every piece left lasts whole frames and no
//!    sound is lost to rounding.
//! 4. [`SilenceRemovalPlan::removal_command`] turns the cuts into one
//!    [`RemoveClipParts`]: one undo step; the pieces end to end, the later clips
//!    of the track moved earlier. Clips on other tracks do not move.
//!
//! Finding the cuts and making them are separate, so an interface can show what
//! will go before anything changes. Why it works this way:
//! `docs/decisions/0051-silence-removal.md`.

use kadr_core::clip::{Clip, ClipContent, MediaSegment};
use kadr_core::commands::RemoveClipParts;
use kadr_core::error::EditError;
use kadr_core::ids::ClipId;
use kadr_core::project::Project;
use kadr_core::time::{FrameRate, TimeRange, TimeSpan, Timestamp};
use kadr_media::{MediaBackend, MediaError};

use crate::silence_analysis::{
    SilenceDetectionSettings, SilentPart, find_silent_parts_with_progress,
};

/// How much of each silence is kept by default: a fifth of a second, a short
/// natural pause.
pub const DEFAULT_KEPT_PAUSE: TimeSpan = TimeSpan::from_milliseconds(200);

/// What counts as a silence, and how much of it stays.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SilenceRemovalSettings {
    /// What counts as a silence (threshold, shortest silence).
    pub detection: SilenceDetectionSettings,
    /// How long each silence lasts once shortened (half on each side of the cut).
    pub kept_pause: TimeSpan,
}

impl Default for SilenceRemovalSettings {
    fn default() -> Self {
        Self {
            detection: SilenceDetectionSettings::default(),
            kept_pause: DEFAULT_KEPT_PAUSE,
        }
    }
}

/// What removing the silences of a clip would cut, found before changing anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SilenceRemovalPlan {
    /// The clip analysed.
    pub clip_id: ClipId,
    /// The parts to cut out, on the timeline as the clip is now: in order,
    /// apart, on frame starts.
    pub cuts: Vec<TimeRange>,
    /// The whole clip is silent: there is nothing to keep, so nothing is cut.
    pub whole_clip_is_silent: bool,
}

impl SilenceRemovalPlan {
    /// How much shorter the clip gets.
    pub fn removed_duration(&self) -> TimeSpan {
        self.cuts
            .iter()
            .map(|cut| cut.duration())
            .fold(TimeSpan::ZERO, |total, duration| total + duration)
    }

    /// The command making the cuts, or `None` when there is nothing to cut.
    pub fn removal_command(&self) -> Option<RemoveClipParts> {
        (!self.cuts.is_empty()).then(|| RemoveClipParts::new(self.clip_id, self.cuts.clone()))
    }
}

/// Why the silences of a clip could not be found.
#[derive(Debug, thiserror::Error)]
pub enum SilenceRemovalError {
    /// The clip or its media file does not exist.
    #[error(transparent)]
    Edit(#[from] EditError),
    /// The clip plays no sound: a text, a picture, or a video without sound.
    #[error("clip {clip_id} has no sound, so it has no silence to remove")]
    NoSound {
        /// The clip asked for.
        clip_id: ClipId,
    },
    /// The sound could not be read.
    #[error(transparent)]
    Media(#[from] MediaError),
    /// The analysis was stopped before the end.
    #[error("the search for silences was stopped")]
    Stopped,
}

/// Finds what removing the silences of `clip_id` would cut. Changes nothing.
///
/// # Errors
///
/// Fails when the clip does not exist, plays no sound, or its file cannot be read.
pub fn plan_silence_removal(
    project: &Project,
    backend: &dyn MediaBackend,
    clip_id: ClipId,
    settings: SilenceRemovalSettings,
) -> Result<SilenceRemovalPlan, SilenceRemovalError> {
    plan_silence_removal_with_progress(project, backend, clip_id, settings, |_| true)
}

/// Like [`plan_silence_removal`], for long clips: `keep_going` receives the
/// fraction analysed (0 to 1) every 30 s of sound, and returns `false` to stop.
///
/// # Errors
///
/// As [`plan_silence_removal`], plus [`SilenceRemovalError::Stopped`].
pub fn plan_silence_removal_with_progress(
    project: &Project,
    backend: &dyn MediaBackend,
    clip_id: ClipId,
    settings: SilenceRemovalSettings,
    keep_going: impl FnMut(f64) -> bool,
) -> Result<SilenceRemovalPlan, SilenceRemovalError> {
    let (_, clip) = project.require_clip(clip_id)?;
    let ClipContent::Media(segment) = &clip.content else {
        return Err(SilenceRemovalError::NoSound { clip_id });
    };
    let media_asset = project.require_media_asset(segment.media_asset_id)?;
    if !media_asset.description.has_audio() {
        return Err(SilenceRemovalError::NoSound { clip_id });
    }
    let silent_parts = find_silent_parts_with_progress(
        backend,
        &media_asset.description.file_path,
        segment.source_start,
        segment.source_end,
        settings.detection,
        keep_going,
    )?
    .ok_or(SilenceRemovalError::Stopped)?;
    let whole_clip_is_silent = silent_parts
        .iter()
        .any(|part| part.start <= segment.source_start && part.end >= segment.source_end);
    let cuts = if whole_clip_is_silent {
        Vec::new()
    } else {
        cuts_for_silences(
            clip,
            segment,
            &silent_parts,
            settings.kept_pause,
            project.settings().frame_rate,
        )
    };
    Ok(SilenceRemovalPlan {
        clip_id,
        cuts,
        whole_clip_is_silent,
    })
}

/// The timeline parts to cut so that each silence (file times) is shortened to
/// `kept_pause`. Silences too short for that are left alone.
fn cuts_for_silences(
    clip: &Clip,
    segment: &MediaSegment,
    silent_parts: &[SilentPart],
    kept_pause: TimeSpan,
    frame_rate: FrameRate,
) -> Vec<TimeRange> {
    let half_pause = TimeSpan::from_ticks(kept_pause.ticks() / 2);
    let (clip_start, clip_end) = (clip.timeline_start, clip.timeline_end());
    let timeline_time_of = |source_time: Timestamp| {
        let source_offset = source_time
            .span_since(segment.source_start)
            .unwrap_or(TimeSpan::ZERO);
        clip_start + segment.speed.timeline_span_for_source_span(source_offset)
    };
    silent_parts
        .iter()
        .filter_map(|part| {
            let cut_start = if part.start <= segment.source_start {
                clip_start
            } else {
                frame_rate.frame_start_at_or_after(timeline_time_of(part.start) + half_pause)
            };
            let cut_end = if part.end >= segment.source_end {
                clip_end
            } else {
                let pause_start = timeline_time_of(part.end).checked_sub(half_pause)?;
                frame_rate.frame_start_at_or_before(pause_start)
            };
            TimeRange::new(cut_start.max(clip_start), cut_end.min(clip_end))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use kadr_core::clip::PlaybackSpeed;
    use kadr_core::ids::MediaAssetId;

    use super::*;

    const FPS_25: FrameRate = FrameRate::FPS_25; // one frame = 40 ms

    fn milliseconds(value: u64) -> Timestamp {
        Timestamp::from_milliseconds(value)
    }

    /// A clip playing 0–10 s of a file at timeline 20 s, at `speed`.
    fn clip_and_segment(speed: PlaybackSpeed) -> (Clip, MediaSegment) {
        let segment = MediaSegment {
            media_asset_id: MediaAssetId::from_raw(1),
            source_start: Timestamp::ZERO,
            source_end: Timestamp::from_whole_seconds(10),
            speed,
        };
        let clip = Clip::new(
            ClipId::from_raw(2),
            ClipContent::Media(segment.clone()),
            Timestamp::from_whole_seconds(20),
        );
        (clip, segment)
    }

    fn silence(start_in_milliseconds: u64, end_in_milliseconds: u64) -> SilentPart {
        SilentPart {
            start: milliseconds(start_in_milliseconds),
            end: milliseconds(end_in_milliseconds),
        }
    }

    fn cut_ends_in_milliseconds(cuts: &[TimeRange]) -> Vec<(u64, u64)> {
        let in_milliseconds =
            |time: Timestamp| time.ticks() / (TimeSpan::from_milliseconds(1).ticks());
        cuts.iter()
            .map(|cut| (in_milliseconds(cut.start()), in_milliseconds(cut.end())))
            .collect()
    }

    #[test]
    fn each_silence_keeps_half_the_pause_on_each_side_on_frame_starts() {
        let (clip, segment) = clip_and_segment(PlaybackSpeed::NORMAL);
        let cuts = cuts_for_silences(
            &clip,
            &segment,
            &[silence(2_010, 3_500)],
            DEFAULT_KEPT_PAUSE,
            FPS_25,
        );
        // 22.010 + 0.100 = 22.110 → next frame start 22.120; 23.500 − 0.100 = 23.400.
        assert_eq!(cut_ends_in_milliseconds(&cuts), vec![(22_120, 23_400)]);
    }

    #[test]
    fn silences_at_the_clip_edges_keep_half_a_pause_next_to_the_sound() {
        let (clip, segment) = clip_and_segment(PlaybackSpeed::NORMAL);
        let cuts = cuts_for_silences(
            &clip,
            &segment,
            &[silence(0, 1_000), silence(9_000, 10_000)],
            DEFAULT_KEPT_PAUSE,
            FPS_25,
        );
        assert_eq!(
            cut_ends_in_milliseconds(&cuts),
            vec![(20_000, 20_880), (29_120, 30_000)]
        );
    }

    #[test]
    fn silences_shorter_than_the_kept_pause_stay() {
        let (clip, segment) = clip_and_segment(PlaybackSpeed::NORMAL);
        let cuts = cuts_for_silences(
            &clip,
            &segment,
            &[silence(2_000, 2_150)],
            DEFAULT_KEPT_PAUSE,
            FPS_25,
        );
        assert!(cuts.is_empty(), "{cuts:?}");
    }

    #[test]
    fn a_faster_clip_has_its_silences_earlier_and_shorter_on_the_timeline() {
        let (clip, segment) = clip_and_segment(PlaybackSpeed::from_fraction(2, 1).unwrap());
        let cuts = cuts_for_silences(
            &clip,
            &segment,
            &[silence(4_000, 6_000)],
            DEFAULT_KEPT_PAUSE,
            FPS_25,
        );
        // File 4–6 s plays at timeline 22–23 s.
        assert_eq!(cut_ends_in_milliseconds(&cuts), vec![(22_120, 22_880)]);
    }
}
