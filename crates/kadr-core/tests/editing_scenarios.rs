//! End-to-end editing scenarios, using only the public API of `kadr-core`.
//!
//! These tests play the role of the desktop app and of Claude: they build a real
//! project with commands, undo and redo everything, save and reload it.

// The helper functions below build test projects: if a step fails, panicking
// with the error is exactly how the test should fail. Clippy only exempts
// functions marked `#[test]`, so the exemption is extended to the whole file.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use kadr_core::animation::{Interpolation, Keyframe};
use kadr_core::clip::{AnimatableProperty, PlaybackSpeed};
use kadr_core::commands::{
    AddClip, AddEffect, AddTrack, ClipEdge, CreatedObjectsExt, EditCommand, ImportMedia, MoveClip,
    RemoveClip, ReplaceEffect, SetClipPropertyValue, SetClipSpeed, SetClipTransition, SetKeyframe,
    SetTextAnimation, SplitClip, TrackPosition, TrimClip,
};
use kadr_core::effect::{
    BlurShape, BlurStyle, ColorAdjustment, ColorLut, EffectKind, PixelRegion, RegionSample,
    RegionSampleOrigin, TrackedBlur, TrackingConfidence,
};
use kadr_core::history::EditSession;
use kadr_core::ids::{ClipId, TrackId};
use kadr_core::media_asset::{AudioFormat, MediaAssetDescription, MediaKind, PictureSize};
use kadr_core::project::{Project, ProjectSettings};
use kadr_core::project_file::{load_project_from_json, save_project_to_json};
use kadr_core::text_animation::{TextAnimation, TextAnimationStyle};
use kadr_core::time::{FrameRate, TimeSpan, Timestamp};
use kadr_core::timeline::TrackKind;
use kadr_core::transition::{SlideDirection, Transition, TransitionStyle, ZoomDirection};
use kadr_core::validation::validate_project;

fn seconds(value: u64) -> Timestamp {
    Timestamp::from_whole_seconds(value)
}

fn interview_video() -> MediaAssetDescription {
    MediaAssetDescription {
        file_path: PathBuf::from("footage/interview.mp4"),
        display_name: "interview.mp4".to_string(),
        kind: MediaKind::Video,
        duration: Some(TimeSpan::from_whole_seconds(30)),
        picture_size: Some(PictureSize {
            width_in_pixels: 1920,
            height_in_pixels: 1080,
        }),
        frame_rate: Some(FrameRate::FPS_30),
        audio_format: Some(AudioFormat {
            sample_rate_in_hertz: 48_000,
            channel_count: 2,
        }),
    }
}

fn background_music() -> MediaAssetDescription {
    MediaAssetDescription {
        file_path: PathBuf::from("music/calm.mp3"),
        display_name: "calm.mp3".to_string(),
        kind: MediaKind::Audio,
        duration: Some(TimeSpan::from_whole_seconds(120)),
        picture_size: None,
        frame_rate: None,
        audio_format: Some(AudioFormat {
            sample_rate_in_hertz: 44_100,
            channel_count: 2,
        }),
    }
}

/// Builds a short vertical video the way a user (or Claude) would:
/// two cuts of an interview, a speed-up, a blurred face, a fading title and music,
/// then (with phase 8) transitions, a text animation and a colour LUT.
fn edit_a_vertical_interview(session: &mut EditSession) {
    edit_a_vertical_interview_before_phase_8c(session);
    // The second part of the interview gets a teal-and-orange look, at 80 %. The
    // path is written with forward slashes so the snapshot is the same on every
    // system; the rules do not look at the disk.
    let last_interview_clip_id = session.project().timeline().tracks()[0].clips()[1].id;
    session
        .execute(AddEffect::new(
            last_interview_clip_id,
            EffectKind::ColorLut(ColorLut {
                file_path: PathBuf::from("looks/teal_orange.cube"),
                strength_fraction: 0.8,
            }),
        ))
        .unwrap();
}

/// The same edit as it was possible before phase 8c (format 2).
fn edit_a_vertical_interview_before_phase_8c(session: &mut EditSession) {
    edit_a_vertical_interview_before_phase_8(session);
    let timeline = session.project().timeline();
    let interview_clip_ids: Vec<ClipId> = timeline.tracks()[0]
        .clips()
        .iter()
        .map(|clip| clip.id)
        .collect();
    let title_clip_id = timeline.tracks()[1].clips()[0].id;
    let transition = |style, duration_in_milliseconds| {
        Some(Transition {
            style,
            duration: TimeSpan::from_milliseconds(duration_in_milliseconds),
        })
    };

    // Fade in from black, push from the first cut to the second, zoom out at the end.
    session
        .execute(SetClipTransition::new(
            interview_clip_ids[0],
            ClipEdge::Start,
            transition(TransitionStyle::Fade, 500),
        ))
        .unwrap();
    session
        .execute(SetClipTransition::new(
            interview_clip_ids[1],
            ClipEdge::Start,
            transition(
                TransitionStyle::Slide {
                    direction: SlideDirection::Left,
                },
                400,
            ),
        ))
        .unwrap();
    session
        .execute(SetClipTransition::new(
            interview_clip_ids[1],
            ClipEdge::End,
            transition(
                TransitionStyle::Zoom {
                    direction: ZoomDirection::Out,
                },
                1_000,
            ),
        ))
        .unwrap();
    // The title is erased letter by letter when it leaves.
    session
        .execute(SetTextAnimation::new(
            title_clip_id,
            ClipEdge::End,
            Some(TextAnimation {
                style: TextAnimationStyle::TypeOn,
                duration: TimeSpan::from_milliseconds(800),
            }),
        ))
        .unwrap();
}

/// The same edit as it was possible before phase 8 (format 1).
fn edit_a_vertical_interview_before_phase_8(session: &mut EditSession) {
    let video_id = session
        .execute(ImportMedia::new(interview_video()))
        .unwrap()
        .first_media_asset_id()
        .unwrap();
    let music_id = session
        .execute(ImportMedia::new(background_music()))
        .unwrap()
        .first_media_asset_id()
        .unwrap();
    let video_track_id = add_track(session, TrackKind::Video);
    let title_track_id = add_track(session, TrackKind::Video);
    let music_track_id = add_track(session, TrackKind::Audio);

    // The interview from 2 s to 20 s, then cut out the part between 8 s and 11 s.
    let interview_clip_id = session
        .execute(AddClip::media(
            video_track_id,
            video_id,
            seconds(2),
            seconds(20),
            seconds(0),
        ))
        .unwrap()
        .first_clip_id()
        .unwrap();
    let middle_clip_id = split(session, interview_clip_id, seconds(6));
    let last_clip_id = split(session, middle_clip_id, seconds(9));
    session.execute(RemoveClip::new(middle_clip_id)).unwrap();
    session
        .execute(MoveClip::along_track(last_clip_id, seconds(6)))
        .unwrap();
    session
        .execute(SetClipSpeed::new(
            last_clip_id,
            PlaybackSpeed::from_multiplier(1.5).unwrap(),
        ))
        .unwrap();

    // Blur the interviewee's face on the first part, then store tracked regions.
    let blur_id = session
        .execute(AddEffect::new(
            interview_clip_id,
            EffectKind::TrackedBlur(TrackedBlur::with_default_style()),
        ))
        .unwrap()
        .first_effect_id()
        .unwrap();
    let tracked_blur = TrackedBlur {
        style: BlurStyle::Pixelate {
            block_size_in_pixels: 24,
        },
        shape: BlurShape::Ellipse,
        margin_in_percent: 25.0,
        edge_softness_in_pixels: 6.0,
        region_samples: vec![
            region_sample(
                2,
                800.0,
                RegionSampleOrigin::UserSelection,
                TrackingConfidence::Confident,
            ),
            region_sample(
                3,
                812.5,
                RegionSampleOrigin::Tracker,
                TrackingConfidence::Confident,
            ),
            region_sample(
                4,
                830.0,
                RegionSampleOrigin::Tracker,
                TrackingConfidence::Uncertain,
            ),
        ],
    };
    session
        .execute(ReplaceEffect::new(
            interview_clip_id,
            blur_id,
            EffectKind::TrackedBlur(tracked_blur),
        ))
        .unwrap();
    session
        .execute(AddEffect::new(
            last_clip_id,
            EffectKind::ColorAdjustment(ColorAdjustment {
                saturation_multiplier: 1.2,
                ..ColorAdjustment::NEUTRAL
            }),
        ))
        .unwrap();

    // A title fading in over one second, done as a single undo step.
    session
        .transaction("Add a fading title", |transaction| {
            let title_clip_id = transaction
                .execute(AddClip::text(
                    title_track_id,
                    "Meet Alex",
                    TimeSpan::from_whole_seconds(3),
                    seconds(0),
                ))?
                .first_clip_id()
                .expect("the title clip was created");
            for (at_second, opacity) in [(0, 0.0), (1, 1.0)] {
                transaction.execute(SetKeyframe::new(
                    title_clip_id,
                    AnimatableProperty::Opacity,
                    Keyframe {
                        time_offset: TimeSpan::from_whole_seconds(at_second),
                        value: opacity,
                        interpolation_to_next: Interpolation::EaseInOut,
                    },
                ))?;
            }
            Ok(())
        })
        .unwrap();

    // Quiet background music under the whole edit.
    let music_clip_id = session
        .execute(AddClip::media(
            music_track_id,
            music_id,
            seconds(0),
            seconds(12),
            seconds(0),
        ))
        .unwrap()
        .first_clip_id()
        .unwrap();
    session
        .execute(SetClipPropertyValue::new(
            music_clip_id,
            AnimatableProperty::Volume,
            0.3,
        ))
        .unwrap();
}

fn add_track(session: &mut EditSession, kind: TrackKind) -> TrackId {
    session
        .execute(AddTrack::new(kind, TrackPosition::Top))
        .unwrap()
        .first_track_id()
        .unwrap()
}

fn split(session: &mut EditSession, clip_id: ClipId, at: Timestamp) -> ClipId {
    session
        .execute(SplitClip::new(clip_id, at))
        .unwrap()
        .first_clip_id()
        .unwrap()
}

fn region_sample(
    at_second: u64,
    left_in_pixels: f32,
    origin: RegionSampleOrigin,
    confidence: TrackingConfidence,
) -> RegionSample {
    RegionSample {
        source_time: seconds(at_second),
        region: PixelRegion {
            left_in_pixels,
            top_in_pixels: 200.0,
            width_in_pixels: 180.0,
            height_in_pixels: 220.0,
        },
        confidence,
        origin,
    }
}

#[test]
fn a_full_edit_can_be_undone_and_redone_step_by_step() {
    let empty_project = Project::new(ProjectSettings::vertical_1080x1920());
    let mut session = EditSession::new(empty_project.clone()).unwrap();
    edit_a_vertical_interview(&mut session);
    let finished_project = session.project().clone();

    let mut undone_step_count = 0;
    while session.can_undo() {
        session.undo().unwrap();
        validate_project(session.project()).unwrap();
        undone_step_count += 1;
    }
    assert_eq!(session.project(), &empty_project);
    // 2 imports, 3 tracks, 1 clip, 2 splits, 1 removal, 1 move, 1 speed change,
    // 3 effect changes, 1 grouped title (with its 2 keyframes), 1 music clip, 1 volume,
    // 3 transitions, 1 text animation, 1 colour LUT.
    assert_eq!(undone_step_count, 22);

    while session.can_redo() {
        session.redo().unwrap();
    }
    assert_eq!(session.project(), &finished_project);
}

#[test]
fn the_edited_timeline_has_the_expected_shape() {
    let mut session =
        EditSession::new(Project::new(ProjectSettings::vertical_1080x1920())).unwrap();
    edit_a_vertical_interview(&mut session);
    let timeline = session.project().timeline();

    let video_track = &timeline.tracks()[0];
    let clip_ranges: Vec<(Timestamp, Timestamp)> = video_track
        .clips()
        .iter()
        .map(|clip| (clip.timeline_start, clip.timeline_end()))
        .collect();
    // The interview (source 2–20 s) ran 0–18 s on the timeline. Cutting at 6 s and
    // 9 s and removing the middle leaves 0–6 s and 9–18 s. The last part (9 s of
    // source) moved to 6 s and played at 1.5× lasts 6 s: it runs 6–12 s.
    assert_eq!(
        clip_ranges,
        vec![(seconds(0), seconds(6)), (seconds(6), seconds(12))]
    );
    assert_eq!(timeline.duration(), TimeSpan::from_whole_seconds(12));

    let title_clip = &timeline.tracks()[1].clips()[0];
    assert_eq!(
        title_clip
            .opacity
            .value_at(TimeSpan::from_milliseconds(500)),
        0.5
    );
}

#[test]
fn a_project_survives_saving_and_loading() {
    let mut session =
        EditSession::new(Project::new(ProjectSettings::vertical_1080x1920())).unwrap();
    edit_a_vertical_interview(&mut session);

    let json_text = save_project_to_json(session.project()).unwrap();
    let reloaded_project = load_project_from_json(&json_text).unwrap();

    assert_eq!(&reloaded_project, session.project());
    insta::assert_snapshot!("vertical_interview_project_file", json_text);
}

#[test]
fn a_format_1_file_is_upgraded_to_the_same_project() {
    let mut session =
        EditSession::new(Project::new(ProjectSettings::vertical_1080x1920())).unwrap();
    edit_a_vertical_interview_before_phase_8(&mut session);

    // The snapshot of this edit saved before phase 8 changed the format.
    let format_1_file = include_str!("fixtures/vertical_interview_format_1.kadr");
    assert!(format_1_file.contains("\"format_version\": 1"));
    assert_eq!(
        &load_project_from_json(format_1_file).unwrap(),
        session.project()
    );
}

#[test]
fn a_format_2_file_is_upgraded_to_the_same_project() {
    let mut session =
        EditSession::new(Project::new(ProjectSettings::vertical_1080x1920())).unwrap();
    edit_a_vertical_interview_before_phase_8c(&mut session);

    // The snapshot of this edit saved before phase 8c changed the format.
    let format_2_file = include_str!("fixtures/vertical_interview_format_2.kadr");
    assert!(format_2_file.contains("\"format_version\": 2"));
    assert_eq!(
        &load_project_from_json(format_2_file).unwrap(),
        session.project()
    );
}

/// A tiny deterministic random number generator, so the test is reproducible
/// without an extra dependency.
struct PseudoRandomSequence {
    state: u64,
}

impl PseudoRandomSequence {
    fn next_below(&mut self, upper_bound: u64) -> u64 {
        // Knuth's MMIX linear congruential generator.
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.state >> 33) % upper_bound
    }
}

#[test]
fn hundreds_of_random_edits_keep_the_project_valid_and_fully_undoable() {
    let empty_project = Project::new(ProjectSettings::landscape_1080p());
    let mut session = EditSession::new(empty_project.clone()).unwrap();
    let video_id = session
        .execute(ImportMedia::new(interview_video()))
        .unwrap()
        .first_media_asset_id()
        .unwrap();
    let track_ids = [
        add_track(&mut session, TrackKind::Video),
        add_track(&mut session, TrackKind::Video),
    ];
    let mut random = PseudoRandomSequence { state: 42 };
    let mut refused_change_count = 0;

    for _ in 0..400 {
        let existing_clip_ids = session.project().timeline().clip_ids();
        let any_clip = || {
            existing_clip_ids
                .get(random.state as usize % existing_clip_ids.len().max(1))
                .copied()
                .unwrap_or(ClipId::from_raw(9_999))
        };
        let picked_clip_id = any_clip();
        let some_time = seconds(random.next_below(40));
        let some_track_id = track_ids[random.next_below(2) as usize];
        let command: Box<dyn EditCommand> = match random.next_below(6) {
            0 => {
                let source_start = random.next_below(25);
                Box::new(AddClip::media(
                    some_track_id,
                    video_id,
                    seconds(source_start),
                    seconds(source_start + 1 + random.next_below(5)),
                    some_time,
                ))
            }
            1 => Box::new(SplitClip::new(picked_clip_id, some_time)),
            2 => Box::new(MoveClip::to_track(picked_clip_id, some_track_id, some_time)),
            3 => Box::new(TrimClip::new(picked_clip_id, ClipEdge::End, some_time)),
            4 => Box::new(TrimClip::new(picked_clip_id, ClipEdge::Start, some_time)),
            _ => Box::new(RemoveClip::new(picked_clip_id)),
        };
        let project_before = session.project().clone();
        if session.execute_all("Random edit", vec![command]).is_err() {
            refused_change_count += 1;
            assert_eq!(
                session.project(),
                &project_before,
                "a refused change must change nothing"
            );
        }
        validate_project(session.project()).unwrap();
    }

    assert!(
        refused_change_count > 0,
        "the sequence should hit some refusals"
    );
    assert!(
        refused_change_count < 400,
        "the sequence should apply some changes"
    );
    while session.can_undo() {
        session.undo().unwrap();
    }
    assert_eq!(session.project(), &empty_project);
}
