//! Which clips a video track shows at one moment, and how they move.
//!
//! Usually a track shows the one clip under the playhead. During a transition it
//! may show two: the transitioning clip on top, and the clip **touching** that
//! edge on the same track underneath, continuing past its own cut (see
//! `docs/decisions/0030-transitions-at-clip-edges.md`).
//!
//! The motion of each picture comes from `kadr-render`
//! ([`transition_motion`], [`text_animation_frame`]); this module only decides
//! which clips take part and at which moment of each one.

use kadr_core::clip::{Clip, ClipContent};
use kadr_core::media_asset::PictureSize;
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_core::timeline::Track;
use kadr_core::transition::EdgePhase;
use kadr_render::{
    LayerMotion, TextReveal, TransitionRole, text_animation_frame, transition_motion,
};

/// One clip picture to draw for a track.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PlannedClipLayer<'project> {
    /// The clip whose picture is drawn.
    pub(crate) clip: &'project Clip,
    /// The moment from the clip's start used for its keyframes, kept inside the
    /// clip: a clip shown past its edge looks as it did at that edge.
    pub(crate) time_in_clip: TimeSpan,
    /// How the layer moves on top of its keyframes (transitions, text animations).
    pub(crate) motion: LayerMotion,
    /// How much of a text clip is visible.
    pub(crate) text_reveal: TextReveal,
}

/// The pictures `track` shows at `timeline_time`, from bottom to top.
pub(crate) fn plan_track_layers(
    track: &Track,
    timeline_time: Timestamp,
    canvas_size: PictureSize,
) -> Vec<PlannedClipLayer<'_>> {
    let Some(clip) = clip_under_playhead(track, timeline_time) else {
        return Vec::new();
    };
    let main_layer = plan_clip_layer(clip, timeline_time, canvas_size);
    let time_in_clip = main_layer.time_in_clip;
    let Some((style, phase)) = clip.transition_at(time_in_clip) else {
        return vec![main_layer];
    };
    let touching_clip = match phase {
        EdgePhase::Entering { .. } => clip_ending_at(track, clip.timeline_start),
        EdgePhase::Leaving { .. } => clip_starting_at(track, clip.timeline_end()),
        EdgePhase::Settled => None,
    };
    let Some(touching_clip) = touching_clip else {
        return vec![main_layer];
    };
    let mut touching_layer = plan_clip_layer(touching_clip, timeline_time, canvas_size);
    let pushed_or_covered =
        transition_motion(style, phase, TransitionRole::TouchingClip, canvas_size);
    touching_layer.motion = touching_layer.motion.combined_with(pushed_or_covered);
    vec![touching_layer, main_layer]
}

/// A clip's own picture at `timeline_time`, which may be outside the clip (for
/// a clip shown under its neighbour's transition), with its own transition or
/// text animation.
fn plan_clip_layer(
    clip: &Clip,
    timeline_time: Timestamp,
    canvas_size: PictureSize,
) -> PlannedClipLayer<'_> {
    let time_in_clip = timeline_time
        .span_since(clip.timeline_start)
        .unwrap_or(TimeSpan::ZERO)
        .min(clip.timeline_duration());
    let (motion, text_reveal) = match &clip.content {
        ClipContent::Text(text_content) => text_content.animation_at(time_in_clip).map_or(
            (LayerMotion::NONE, TextReveal::Everything),
            |(style, phase)| {
                text_animation_frame(style, phase, text_content.style.font_size_in_pixels)
            },
        ),
        ClipContent::Media(_) => {
            let motion =
                clip.transition_at(time_in_clip)
                    .map_or(LayerMotion::NONE, |(style, phase)| {
                        transition_motion(
                            style,
                            phase,
                            TransitionRole::TransitioningClip,
                            canvas_size,
                        )
                    });
            (motion, TextReveal::Everything)
        }
    };
    PlannedClipLayer {
        clip,
        time_in_clip,
        motion,
        text_reveal,
    }
}

/// The clip of `track` containing `timeline_time`, if any.
pub(crate) fn clip_under_playhead(track: &Track, timeline_time: Timestamp) -> Option<&Clip> {
    let clips = track.clips();
    // Clips are sorted by start and never overlap: the candidate is the last
    // clip starting at or before the playhead.
    let clips_started = clips.partition_point(|clip| clip.timeline_start <= timeline_time);
    let candidate = clips.get(clips_started.checked_sub(1)?)?;
    candidate
        .contains_timeline_time(timeline_time)
        .then_some(candidate)
}

/// The clip of `track` ending exactly at `time`, if any.
fn clip_ending_at(track: &Track, time: Timestamp) -> Option<&Clip> {
    let clips = track.clips();
    let clips_started_before = clips.partition_point(|clip| clip.timeline_start < time);
    let candidate = clips.get(clips_started_before.checked_sub(1)?)?;
    (candidate.timeline_end() == time).then_some(candidate)
}

/// The clip of `track` starting exactly at `time`, if any.
fn clip_starting_at(track: &Track, time: Timestamp) -> Option<&Clip> {
    let clips = track.clips();
    let index = clips.partition_point(|clip| clip.timeline_start < time);
    clips.get(index).filter(|clip| clip.timeline_start == time)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kadr_core::commands::{
        AddClip, AddTrack, ClipEdge, CreatedObjectsExt, ImportMedia, SetClipTransition,
        TrackPosition,
    };
    use kadr_core::history::EditSession;
    use kadr_core::ids::ClipId;
    use kadr_core::media_asset::{MediaAssetDescription, MediaKind};
    use kadr_core::project::{Project, ProjectSettings};
    use kadr_core::time::FrameRate;
    use kadr_core::timeline::TrackKind;
    use kadr_core::transition::{SlideDirection, Transition, TransitionStyle};

    const CANVAS: PictureSize = PictureSize {
        width_in_pixels: 1920,
        height_in_pixels: 1080,
    };

    fn milliseconds(value: u64) -> Timestamp {
        Timestamp::from_milliseconds(value)
    }

    /// Two clips of a 10-second video touching at 2 s, and a third one after a gap.
    fn three_clips() -> (EditSession, [ClipId; 3]) {
        let mut session =
            EditSession::new(Project::new(ProjectSettings::landscape_1080p())).unwrap();
        let video_id = session
            .execute(ImportMedia::new(MediaAssetDescription {
                file_path: "video.mp4".into(),
                display_name: "video.mp4".to_string(),
                kind: MediaKind::Video,
                duration: Some(TimeSpan::from_whole_seconds(10)),
                picture_size: Some(CANVAS),
                frame_rate: Some(FrameRate::FPS_30),
                audio_format: None,
            }))
            .unwrap()
            .first_media_asset_id()
            .unwrap();
        let track_id = session
            .execute(AddTrack::new(TrackKind::Video, TrackPosition::Top))
            .unwrap()
            .first_track_id()
            .unwrap();
        let mut add = |source_start: u64, timeline_start: u64| {
            session
                .execute(AddClip::media(
                    track_id,
                    video_id,
                    milliseconds(source_start),
                    milliseconds(source_start + 2_000),
                    milliseconds(timeline_start),
                ))
                .unwrap()
                .first_clip_id()
                .unwrap()
        };
        let clip_ids = [add(0, 0), add(5_000, 2_000), add(8_000, 6_000)];
        (session, clip_ids)
    }

    fn set_transition(
        session: &mut EditSession,
        clip_id: ClipId,
        edge: ClipEdge,
        style: TransitionStyle,
    ) {
        session
            .execute(SetClipTransition::new(
                clip_id,
                edge,
                Some(Transition {
                    style,
                    duration: TimeSpan::from_milliseconds(500),
                }),
            ))
            .unwrap();
    }

    fn planned_clip_ids(session: &EditSession, at_millisecond: u64) -> Vec<ClipId> {
        let track = &session.project().timeline().tracks()[0];
        plan_track_layers(track, milliseconds(at_millisecond), CANVAS)
            .iter()
            .map(|layer| layer.clip.id)
            .collect()
    }

    #[test]
    fn without_transitions_a_track_shows_the_clip_under_the_playhead() {
        let (session, [first, second, _]) = three_clips();
        assert_eq!(planned_clip_ids(&session, 1_900), vec![first]);
        assert_eq!(planned_clip_ids(&session, 2_000), vec![second]);
        assert!(planned_clip_ids(&session, 4_500).is_empty());
    }

    #[test]
    fn an_entry_transition_shows_the_touching_clip_underneath() {
        let (mut session, [first, second, third]) = three_clips();
        let slide = TransitionStyle::Slide {
            direction: SlideDirection::Left,
        };
        set_transition(&mut session, second, ClipEdge::Start, slide);
        // The third clip has nothing touching its start: it slides in alone.
        set_transition(&mut session, third, ClipEdge::Start, slide);

        assert_eq!(planned_clip_ids(&session, 2_250), vec![first, second]);
        assert_eq!(planned_clip_ids(&session, 2_500), vec![second]);
        assert_eq!(planned_clip_ids(&session, 6_250), vec![third]);

        let track = &session.project().timeline().tracks()[0];
        let layers = plan_track_layers(track, milliseconds(2_250), CANVAS);
        // Halfway through the slide: the first clip is pushed half a canvas to the
        // left, shown as it was at its end; the second comes from the right.
        assert_eq!(layers[0].time_in_clip, TimeSpan::from_whole_seconds(2));
        assert!((layers[0].motion.offset_x_in_pixels + 960.0).abs() < 1e-3);
        assert!((layers[1].motion.offset_x_in_pixels - 960.0).abs() < 1e-3);
    }

    #[test]
    fn an_exit_transition_shows_the_next_clip_underneath() {
        let (mut session, [first, second, _]) = three_clips();
        set_transition(&mut session, first, ClipEdge::End, TransitionStyle::Fade);

        assert_eq!(planned_clip_ids(&session, 1_400), vec![first]);
        assert_eq!(planned_clip_ids(&session, 1_750), vec![second, first]);
        let track = &session.project().timeline().tracks()[0];
        let layers = plan_track_layers(track, milliseconds(1_750), CANVAS);
        // The next clip is shown as it is at its start, fully opaque; the first
        // one is half faded.
        assert_eq!(layers[0].time_in_clip, TimeSpan::ZERO);
        assert_eq!(layers[0].motion, LayerMotion::NONE);
        assert!((layers[1].motion.opacity_multiplier - 0.5).abs() < 1e-3);
    }
}
