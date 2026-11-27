//! Removing the silences of a clip on a real encoded file: the cuts land where
//! the file is silent, the clip closes up into pieces end to end, the later
//! clips of its track follow, and the exported video has no long silence left.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use kadr_core::commands::{AddClip, AddTrack, CreatedObjectsExt, ImportMedia, TrackPosition};
use kadr_core::history::EditSession;
use kadr_core::ids::ClipId;
use kadr_core::media_asset::PictureSize;
use kadr_core::project::{Project, ProjectSettings};
use kadr_core::time::{FrameRate, TimeSpan, Timestamp};
use kadr_core::timeline::TrackKind;
use kadr_engine::silence_analysis::{SilenceDetectionSettings, find_silent_parts};
use kadr_engine::silence_removal::{
    SilenceRemovalError, SilenceRemovalSettings, plan_silence_removal,
    plan_silence_removal_with_progress,
};
use kadr_engine::{ExportCancellation, ExportRequest, RendererPreference, export_project};
use kadr_media::test_patterns::{BeepPattern, TestVideoDescription, write_test_video};
use kadr_media::{EncodingSpeed, FfmpegBackend, MediaBackend, VideoEncoderPreference};

/// Encoding and decoding sound shifts its edges by a few milliseconds.
const EDGE_TOLERANCE_IN_SECONDS: f64 = 0.03;

/// A 6 s video sounding from 1 to 2 s, 3 to 4 s and 5 to 6 s: silent from 0 to
/// 1 s, 2 to 3 s and 4 to 5 s.
fn video_with_three_silences(directory: &Path) -> std::path::PathBuf {
    let path = directory.join("speech.mp4");
    let duration = TimeSpan::from_whole_seconds(6);
    write_test_video(
        &FfmpegBackend::new().unwrap(),
        &TestVideoDescription {
            output_path: path.clone(),
            width_in_pixels: 160,
            height_in_pixels: 120,
            frame_rate: FrameRate::FPS_25,
            duration,
            audio_sample_rate_in_hertz: 48_000,
            sound: BeepPattern {
                frequency_in_hertz: 440.0,
                amplitude: 0.5,
                beep_starts: [1, 3, 5].map(Timestamp::from_whole_seconds).to_vec(),
                beep_length: TimeSpan::from_whole_seconds(1),
            },
        },
    )
    .unwrap();
    path
}

/// A project at 25 fps with the whole video at 0 s, and a title at 7 s on the
/// same track. Returns the session, the video clip and the title.
fn project_with_video_and_title(video: &Path) -> (EditSession, ClipId, ClipId) {
    let settings = ProjectSettings {
        canvas_size: PictureSize {
            width_in_pixels: 160,
            height_in_pixels: 120,
        },
        frame_rate: FrameRate::FPS_25,
        ..ProjectSettings::landscape_1080p()
    };
    let mut session = EditSession::new(Project::new(settings)).unwrap();
    let backend = FfmpegBackend::new().unwrap();
    let media_id = session
        .execute(ImportMedia::new(backend.probe(video).unwrap()))
        .unwrap()
        .first_media_asset_id()
        .unwrap();
    let track_id = session
        .execute(AddTrack::new(TrackKind::Video, TrackPosition::Top))
        .unwrap()
        .first_track_id()
        .unwrap();
    let video_clip_id = session
        .execute(AddClip::media(
            track_id,
            media_id,
            Timestamp::ZERO,
            Timestamp::from_whole_seconds(6),
            Timestamp::ZERO,
        ))
        .unwrap()
        .first_clip_id()
        .unwrap();
    let title_id = session
        .execute(AddClip::text(
            track_id,
            "The end",
            TimeSpan::from_whole_seconds(1),
            Timestamp::from_whole_seconds(7),
        ))
        .unwrap()
        .first_clip_id()
        .unwrap();
    (session, video_clip_id, title_id)
}

fn assert_close(found: Timestamp, expected_in_seconds: f64, context: &str) {
    assert!(
        (found.as_seconds() - expected_in_seconds).abs() <= EDGE_TOLERANCE_IN_SECONDS + 0.04,
        "{context}: {found} instead of about {expected_in_seconds} s"
    );
}

#[test]
fn the_silences_of_a_clip_are_shortened_in_one_undoable_change() {
    let directory = tempfile::tempdir().unwrap();
    let video = video_with_three_silences(directory.path());
    let (mut session, video_clip_id, title_id) = project_with_video_and_title(&video);
    let backend = FfmpegBackend::new().unwrap();

    let plan = plan_silence_removal(
        session.project(),
        &backend,
        video_clip_id,
        SilenceRemovalSettings::default(),
    )
    .unwrap();

    // Each silence keeps 0.1 s next to the sound, cuts on frame starts (40 ms).
    assert_eq!(plan.cuts.len(), 3, "{:?}", plan.cuts);
    assert_eq!(plan.cuts[0].start(), Timestamp::ZERO);
    assert_close(plan.cuts[0].end(), 0.9, "end of the first cut");
    assert_close(plan.cuts[1].start(), 2.1, "start of the second cut");
    assert_close(plan.cuts[1].end(), 2.9, "end of the second cut");
    assert_close(plan.cuts[2].start(), 4.1, "start of the third cut");
    assert_close(plan.cuts[2].end(), 4.9, "end of the third cut");
    let frame_rate = FrameRate::FPS_25;
    for cut in &plan.cuts {
        for edge in [cut.start(), cut.end()] {
            assert_eq!(frame_rate.frame_start_at_or_before(edge), edge, "{cut}");
        }
    }

    let removed = plan.removed_duration();
    session.execute(plan.removal_command().unwrap()).unwrap();

    let (track, _) = session.project().timeline().find_clip(title_id).unwrap();
    let pieces: Vec<_> = track
        .clips()
        .iter()
        .filter(|clip| clip.id != title_id)
        .collect();
    assert_eq!(pieces.len(), 3, "one piece of sound between each cut");
    assert!(
        pieces
            .windows(2)
            .all(|pair| pair[0].timeline_end() == pair[1].timeline_start),
        "the pieces are end to end"
    );
    let title_start = track.clip(title_id).unwrap().timeline_start;
    assert_eq!(
        Some(removed),
        Timestamp::from_whole_seconds(7).span_since(title_start),
        "the title moved earlier by the time removed"
    );

    // The exported sound has no silence of half a second or more left.
    let output = directory.path().join("without-silences.mp4");
    export_project(
        session.project(),
        &backend,
        &ExportRequest {
            output_path: output.clone(),
            video_encoder_preference: VideoEncoderPreference::SoftwareOnly,
            encoding_speed: EncodingSpeed::Fast,
            renderer_preference: RendererPreference::CpuOnly,
        },
        &mut |_| {},
        &ExportCancellation::new(),
    )
    .unwrap();
    let pieces_end = pieces[2].timeline_end();
    let silences_left = find_silent_parts(
        &backend,
        &output,
        Timestamp::ZERO,
        pieces_end,
        SilenceDetectionSettings::default(),
    )
    .unwrap();
    assert!(silences_left.is_empty(), "{silences_left:?}");

    // One undo brings the clip and the title back.
    session.undo().unwrap();
    let (track, clip) = session
        .project()
        .timeline()
        .find_clip(video_clip_id)
        .unwrap();
    assert_eq!(clip.timeline_duration(), TimeSpan::from_whole_seconds(6));
    assert_eq!(track.clips().len(), 2);
}

#[test]
fn a_search_for_silences_can_be_stopped_and_needs_sound() {
    let directory = tempfile::tempdir().unwrap();
    let video = video_with_three_silences(directory.path());
    let (session, video_clip_id, title_id) = project_with_video_and_title(&video);
    let backend = FfmpegBackend::new().unwrap();

    let stopped = plan_silence_removal_with_progress(
        session.project(),
        &backend,
        video_clip_id,
        SilenceRemovalSettings::default(),
        |_| false,
    );
    assert!(matches!(stopped, Err(SilenceRemovalError::Stopped)));

    let on_a_title = plan_silence_removal(
        session.project(),
        &backend,
        title_id,
        SilenceRemovalSettings::default(),
    );
    assert!(matches!(
        on_a_title,
        Err(SilenceRemovalError::NoSound { .. })
    ));
}
