//! Converting the seconds Claude gives into exact Kadr time, and back.
//!
//! Claude speaks in seconds (`2.5`); Kadr counts exact integer ticks (see
//! `kadr_core::time`). Values shown to Claude are rounded to the microsecond,
//! which is far below one frame.
//!
//! Because shown values are rounded, a time Claude copies from a clip's end may
//! differ from the real end by a fraction of a microsecond. To keep "place the
//! next clip at the end of the previous one" from failing on such a tiny
//! overlap, positions given for clip edges snap to the nearest existing clip
//! edge on the same track when they are within half a frame of it
//! ([`snap_to_nearby_clip_edge`]), like the magnetism of a mouse-driven editor.

use kadr_core::ids::ClipId;
use kadr_core::time::{FrameRate, TICKS_PER_SECOND, TimeSpan, Timestamp};
use kadr_core::timeline::Track;

use crate::output::ToolError;

/// Number of decimals kept when showing seconds: microseconds.
const SHOWN_DECIMALS_PER_SECOND: f64 = 1_000_000.0;

/// The moment `value_in_seconds`, for the parameter called `parameter_name`.
///
/// # Errors
///
/// Fails when the value is negative, not finite or too large.
pub fn timestamp_from_seconds(
    parameter_name: &str,
    value_in_seconds: f64,
) -> Result<Timestamp, ToolError> {
    Timestamp::from_seconds(value_in_seconds)
        .map_err(|error| ToolError::from(error).in_context(parameter_name))
}

/// The length `value_in_seconds`, for the parameter called `parameter_name`.
///
/// # Errors
///
/// Fails when the value is negative, not finite or too large.
pub fn span_from_seconds(
    parameter_name: &str,
    value_in_seconds: f64,
) -> Result<TimeSpan, ToolError> {
    TimeSpan::from_seconds(value_in_seconds)
        .map_err(|error| ToolError::from(error).in_context(parameter_name))
}

/// Like [`span_from_seconds`], but also refuses zero.
///
/// # Errors
///
/// Fails when the value is zero, negative, not finite or too large.
pub fn positive_span_from_seconds(
    parameter_name: &str,
    value_in_seconds: f64,
) -> Result<TimeSpan, ToolError> {
    let span = span_from_seconds(parameter_name, value_in_seconds)?;
    if span.is_zero() {
        return Err(ToolError::new(format!(
            "{parameter_name} must be more than 0 seconds"
        )));
    }
    Ok(span)
}

/// A moment in seconds, rounded to the microsecond, for showing to Claude.
pub fn seconds_of(timestamp: Timestamp) -> f64 {
    rounded_seconds(timestamp.ticks())
}

/// A length in seconds, rounded to the microsecond, for showing to Claude.
pub fn seconds_in(span: TimeSpan) -> f64 {
    rounded_seconds(span.ticks())
}

fn rounded_seconds(ticks: u64) -> f64 {
    let seconds = ticks as f64 / TICKS_PER_SECOND as f64;
    (seconds * SHOWN_DECIMALS_PER_SECOND).round() / SHOWN_DECIMALS_PER_SECOND
}

/// Half the duration of one frame at `frame_rate`: the snapping distance.
pub fn half_frame(frame_rate: FrameRate) -> TimeSpan {
    let one_frame = frame_rate
        .timestamp_of_frame(1)
        .span_since(Timestamp::ZERO)
        .unwrap_or(TimeSpan::ZERO);
    TimeSpan::from_ticks(one_frame.ticks() / 2)
}

/// `time`, moved onto the nearest start or end of a clip of `track` (other than
/// `moving_clip`) when it is closer than `tolerance`; otherwise unchanged.
pub fn snap_to_nearby_clip_edge(
    track: &Track,
    time: Timestamp,
    moving_clip: Option<ClipId>,
    tolerance: TimeSpan,
) -> Timestamp {
    let mut best: Option<(u64, Timestamp)> = None;
    let other_clips = track
        .clips()
        .iter()
        .filter(|clip| Some(clip.id) != moving_clip);
    for clip in other_clips {
        for edge in [clip.timeline_start, clip.timeline_end()] {
            let distance = time.ticks().abs_diff(edge.ticks());
            if distance <= tolerance.ticks() && best.is_none_or(|(closest, _)| distance < closest) {
                best = Some((distance, edge));
            }
        }
    }
    best.map_or(time, |(_, edge)| edge)
}

/// Converts a frame rate given as a number of frames per second: whole numbers
/// (24, 25, 30, 50, 60…) and the NTSC rates 23.976, 29.97 and 59.94.
///
/// # Errors
///
/// Fails for other fractional rates and values out of range.
pub fn frame_rate_from_frames_per_second(frames_per_second: f64) -> Result<FrameRate, ToolError> {
    const NTSC_RATES: [(f64, u32); 3] = [(23.976, 24_000), (29.97, 30_000), (59.94, 60_000)];
    for (approximate_rate, numerator) in NTSC_RATES {
        if (frames_per_second - approximate_rate).abs() < 0.01 {
            return Ok(FrameRate::new(numerator, 1_001)?);
        }
    }
    let rounded = frames_per_second.round();
    if (frames_per_second - rounded).abs() > 1e-9 || !(1.0..=240.0).contains(&rounded) {
        return Err(ToolError::new(format!(
            "{frames_per_second} frames per second is not supported: use a whole number from 1 \
             to 240 (24, 25, 30, 50, 60…) or 23.976, 29.97, 59.94"
        )));
    }
    Ok(FrameRate::new(rounded as u32, 1)?)
}

#[cfg(test)]
mod tests {
    use kadr_core::commands::{AddClip, AddTrack, CreatedObjectsExt, TrackPosition};
    use kadr_core::history::EditSession;
    use kadr_core::project::{Project, ProjectSettings};
    use kadr_core::timeline::TrackKind;

    use super::*;

    #[test]
    fn negative_times_are_refused_with_the_parameter_name() {
        let error = timestamp_from_seconds("start_time_in_seconds", -2.0).unwrap_err();
        assert!(
            error.to_string().starts_with("start_time_in_seconds: "),
            "{error}"
        );
        assert!(positive_span_from_seconds("duration_in_seconds", 0.0).is_err());
    }

    #[test]
    fn shown_seconds_are_rounded_to_the_microsecond() {
        let one_third = Timestamp::from_ticks(TICKS_PER_SECOND / 3);
        assert_eq!(seconds_of(one_third), 0.333_333);
    }

    #[test]
    fn frame_rates_accept_whole_and_ntsc_values() {
        assert_eq!(
            frame_rate_from_frames_per_second(30.0).unwrap(),
            FrameRate::FPS_30
        );
        assert_eq!(
            frame_rate_from_frames_per_second(29.97).unwrap(),
            FrameRate::FPS_29_97
        );
        assert!(frame_rate_from_frames_per_second(31.5).is_err());
        assert!(frame_rate_from_frames_per_second(0.0).is_err());
    }

    #[test]
    fn a_time_close_to_a_clip_edge_snaps_onto_it() {
        let mut session =
            EditSession::new(Project::new(ProjectSettings::landscape_1080p())).unwrap();
        let track_id = session
            .execute(AddTrack::new(TrackKind::Video, TrackPosition::Top))
            .unwrap()
            .first_track_id()
            .unwrap();
        session
            .execute(AddClip::text(
                track_id,
                "Hi",
                TimeSpan::from_ticks(TICKS_PER_SECOND / 3),
                Timestamp::ZERO,
            ))
            .unwrap();
        let track = session.project().timeline().track(track_id).unwrap();
        let tolerance = half_frame(FrameRate::FPS_30);

        // The clip ends at 0.3333333… s; Claude read "0.333333".
        let copied_end = Timestamp::from_seconds(0.333_333).unwrap();
        let snapped = snap_to_nearby_clip_edge(track, copied_end, None, tolerance);
        assert_eq!(snapped, Timestamp::from_ticks(TICKS_PER_SECOND / 3));

        let far_away = Timestamp::from_seconds(0.5).unwrap();
        assert_eq!(
            snap_to_nearby_clip_edge(track, far_away, None, tolerance),
            far_away
        );
    }
}
