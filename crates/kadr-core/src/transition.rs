//! Transitions: how a video or an image enters and leaves the picture.
//!
//! A clip has an optional **entry** transition, played over its first moments,
//! and an optional **exit** transition, played over its last moments. Both stay
//! inside the clip's own time range, so the timeline length never changes and
//! clips never overlap.
//!
//! When another clip on the same track touches the edge, it keeps showing
//! underneath during the transition (see `docs/rendering.md`): an entry fade over
//! the end of the previous clip is a dissolve, an entry slide pushes it out of
//! the picture. With nothing touching, the clip appears over (or disappears to)
//! whatever is below it: lower tracks or the background.
//!
//! The same timing rules serve the text animations of
//! [`text_animation`](crate::text_animation): [`fitted_edge_durations`] and
//! [`edge_phase_at`].
//!
//! Decision record: `docs/decisions/0050-transitions-at-clip-edges.md`.

use serde::{Deserialize, Serialize};

use crate::time::TimeSpan;

/// The longest transition or text animation allowed.
pub const MAXIMUM_EDGE_ANIMATION_DURATION: TimeSpan = TimeSpan::from_whole_seconds(60);

/// How a video or an image enters or leaves the picture, and for how long.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Transition {
    /// What the picture does.
    pub style: TransitionStyle,
    /// How long it takes, from the clip's edge.
    pub duration: TimeSpan,
}

/// What a picture does during a transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TransitionStyle {
    /// The picture fades in or out. Over a clip touching it on the same track,
    /// this is a dissolve.
    Fade,
    /// The picture slides in from one side of the canvas, or out to the other.
    /// A clip touching it on the same track is pushed along.
    Slide {
        /// Which way the pictures move.
        direction: SlideDirection,
    },
    /// The picture grows or shrinks while fading.
    Zoom {
        /// Whether pictures grow (in) or shrink (out).
        direction: ZoomDirection,
    },
}

impl TransitionStyle {
    /// A short name for messages, for example `"slide"`.
    pub fn display_name(&self) -> &'static str {
        match self {
            TransitionStyle::Fade => "fade",
            TransitionStyle::Slide { .. } => "slide",
            TransitionStyle::Zoom { .. } => "zoom",
        }
    }
}

/// The direction pictures move in during a slide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlideDirection {
    /// Towards the left: an entering picture comes from the right edge.
    Left,
    /// Towards the right: an entering picture comes from the left edge.
    Right,
    /// Upwards: an entering picture comes from the bottom edge.
    Up,
    /// Downwards: an entering picture comes from the top edge.
    Down,
}

impl SlideDirection {
    /// The direction as a unit step on the canvas (x to the right, y downwards).
    pub fn unit_step(self) -> (f32, f32) {
        match self {
            SlideDirection::Left => (-1.0, 0.0),
            SlideDirection::Right => (1.0, 0.0),
            SlideDirection::Up => (0.0, -1.0),
            SlideDirection::Down => (0.0, 1.0),
        }
    }
}

/// Whether pictures grow or shrink during a zoom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoomDirection {
    /// Pictures grow, as if the camera moved forwards.
    In,
    /// Pictures shrink, as if the camera moved backwards.
    Out,
}

/// Where a clip is in its entry or exit at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EdgePhase {
    /// During the entry: `progress` goes from 0 at the clip's start to 1 when the
    /// entry is over.
    Entering {
        /// From 0 to 1.
        progress: f32,
    },
    /// During the exit: `progress` goes from 0 when the exit starts to 1 at the
    /// clip's end.
    Leaving {
        /// From 0 to 1.
        progress: f32,
    },
    /// Neither entering nor leaving.
    Settled,
}

/// The durations an entry and an exit really last on a clip of `clip_duration`.
///
/// They are kept as they are, unless together they are longer than the clip:
/// then both are shortened in proportion so that they meet without overlapping.
/// That way trimming or splitting a clip never has to be refused because of its
/// transitions. A duration that would round down to nothing is dropped.
pub fn fitted_edge_durations(
    entry_duration: Option<TimeSpan>,
    exit_duration: Option<TimeSpan>,
    clip_duration: TimeSpan,
) -> (Option<TimeSpan>, Option<TimeSpan>) {
    let total_ticks =
        entry_duration.map_or(0, TimeSpan::ticks) + exit_duration.map_or(0, TimeSpan::ticks);
    let fit = |duration: Option<TimeSpan>| {
        let duration = duration?;
        let fitted = if total_ticks > clip_duration.ticks() {
            duration.scaled_by_ratio(clip_duration.ticks(), total_ticks)
        } else {
            duration
        };
        (!fitted.is_zero()).then_some(fitted)
    };
    (fit(entry_duration), fit(exit_duration))
}

/// Where a clip lasting `clip_duration` is in its entry or exit,
/// `time_in_clip` after its start. Times past either edge are clamped to it, so
/// a clip shown beyond its end (under a transition) stays as it was at the end.
///
/// The durations are fitted first (see [`fitted_edge_durations`]).
pub fn edge_phase_at(
    time_in_clip: TimeSpan,
    clip_duration: TimeSpan,
    entry_duration: Option<TimeSpan>,
    exit_duration: Option<TimeSpan>,
) -> EdgePhase {
    let (entry_duration, exit_duration) =
        fitted_edge_durations(entry_duration, exit_duration, clip_duration);
    let time_in_clip = time_in_clip.min(clip_duration);
    if let Some(entry_duration) = entry_duration
        && time_in_clip < entry_duration
    {
        return EdgePhase::Entering {
            progress: time_in_clip.fraction_of(entry_duration) as f32,
        };
    }
    if let Some(exit_duration) = exit_duration {
        let exit_start = clip_duration
            .checked_sub(exit_duration)
            .unwrap_or(TimeSpan::ZERO);
        if let Some(time_in_exit) = time_in_clip.checked_sub(exit_start) {
            return EdgePhase::Leaving {
                progress: time_in_exit.fraction_of(exit_duration) as f32,
            };
        }
    }
    EdgePhase::Settled
}

/// The entry or exit style playing `time_in_clip` after the start of a clip
/// lasting `clip_duration`, with where it is; `None` while the clip is settled.
///
/// `entry` and `exit` give each edge's style and stored duration. Used for both
/// transitions and text animations.
pub fn active_edge_style<Style: Copy>(
    entry: Option<(Style, TimeSpan)>,
    exit: Option<(Style, TimeSpan)>,
    time_in_clip: TimeSpan,
    clip_duration: TimeSpan,
) -> Option<(Style, EdgePhase)> {
    let phase = edge_phase_at(
        time_in_clip,
        clip_duration,
        entry.map(|(_, duration)| duration),
        exit.map(|(_, duration)| duration),
    );
    match phase {
        EdgePhase::Entering { .. } => entry.map(|(style, _)| (style, phase)),
        EdgePhase::Leaving { .. } => exit.map(|(style, _)| (style, phase)),
        EdgePhase::Settled => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn milliseconds(value: u64) -> TimeSpan {
        TimeSpan::from_milliseconds(value)
    }

    fn assert_progress(phase: EdgePhase, expected: EdgePhase) {
        let close = |found: f32, wanted: f32| (found - wanted).abs() < 1e-6;
        let matches = match (phase, expected) {
            (EdgePhase::Entering { progress: found }, EdgePhase::Entering { progress: wanted })
            | (EdgePhase::Leaving { progress: found }, EdgePhase::Leaving { progress: wanted }) => {
                close(found, wanted)
            }
            (EdgePhase::Settled, EdgePhase::Settled) => true,
            _ => false,
        };
        assert!(matches, "{phase:?} instead of {expected:?}");
    }

    #[test]
    fn durations_that_fit_are_kept() {
        assert_eq!(
            fitted_edge_durations(
                Some(milliseconds(500)),
                Some(milliseconds(1_000)),
                milliseconds(4_000)
            ),
            (Some(milliseconds(500)), Some(milliseconds(1_000)))
        );
        assert_eq!(
            fitted_edge_durations(None, Some(milliseconds(4_000)), milliseconds(4_000)),
            (None, Some(milliseconds(4_000)))
        );
    }

    #[test]
    fn durations_longer_than_the_clip_are_shortened_in_proportion() {
        // 3 s + 1 s on a 2 s clip: three quarters and one quarter of 2 s.
        assert_eq!(
            fitted_edge_durations(
                Some(milliseconds(3_000)),
                Some(milliseconds(1_000)),
                milliseconds(2_000)
            ),
            (Some(milliseconds(1_500)), Some(milliseconds(500)))
        );
        assert_eq!(
            fitted_edge_durations(Some(milliseconds(5_000)), None, milliseconds(2_000)),
            (Some(milliseconds(2_000)), None)
        );
    }

    #[test]
    fn the_phase_follows_the_entry_then_the_exit() {
        let (entry, exit) = (Some(milliseconds(1_000)), Some(milliseconds(500)));
        let clip_duration = milliseconds(4_000);
        let phase_at = |time| edge_phase_at(milliseconds(time), clip_duration, entry, exit);
        assert_progress(phase_at(0), EdgePhase::Entering { progress: 0.0 });
        assert_progress(phase_at(250), EdgePhase::Entering { progress: 0.25 });
        assert_progress(phase_at(1_000), EdgePhase::Settled);
        assert_progress(phase_at(3_500), EdgePhase::Leaving { progress: 0.0 });
        assert_progress(phase_at(3_750), EdgePhase::Leaving { progress: 0.5 });
        // Past the end, the clip stays as it was at its end.
        assert_progress(phase_at(9_000), EdgePhase::Leaving { progress: 1.0 });
        assert_progress(
            edge_phase_at(milliseconds(9_000), clip_duration, entry, None),
            EdgePhase::Settled,
        );
    }

    #[test]
    fn slide_directions_point_the_right_way_on_a_canvas_with_y_down() {
        assert_eq!(SlideDirection::Left.unit_step(), (-1.0, 0.0));
        assert_eq!(SlideDirection::Up.unit_step(), (0.0, -1.0));
    }
}
