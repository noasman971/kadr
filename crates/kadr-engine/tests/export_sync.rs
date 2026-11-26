//! Phase 2 acceptance test: a project with three cuts, a speed change and an
//! audio track exports correctly, with sound in sync with the pictures.
//!
//! The source video shows its own frame number as a barcode and beeps at every
//! whole second. After export, every output frame is checked to come from the
//! exact expected source frame, and every beep to be where the edit puts it.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use kadr_core::clip::{AnimatableProperty, PlaybackSpeed};
use kadr_core::commands::{
    AddClip, AddTrack, CreatedObjectsExt, ImportMedia, SetClipPropertyValue, SetClipSpeed,
    TrackPosition,
};
use kadr_core::history::EditSession;
use kadr_core::media_asset::PictureSize;
use kadr_core::project::{Project, ProjectSettings};
use kadr_core::time::{FrameRate, TimeSpan, Timestamp};
use kadr_core::timeline::TrackKind;
use kadr_engine::quick_assembly::assemble_project_from_files;
use kadr_engine::{
    ExportCancellation, ExportError, ExportRequest, RendererPreference, export_project,
};
use kadr_media::test_patterns::{
    BeepPattern, TestVideoDescription, find_beep_starts, read_frame_index_barcode, tone_strength,
    write_test_video,
};
use kadr_media::{
    AudioSegmentRequest, EncodingSpeed, FfmpegBackend, MediaBackend, VideoEncoderPreference,
};

const SAMPLE_RATE: u32 = 48_000;
const BEEP_FREQUENCY: f32 = 1_000.0;
const MUSIC_FREQUENCY: f32 = 300.0;
/// Under one frame (33 ms at 30 fps); covers AAC delay and the pitch-keeping stretch.
const SYNC_TOLERANCE: TimeSpan = TimeSpan::from_milliseconds(20);

fn seconds(value: u64) -> Timestamp {
    Timestamp::from_whole_seconds(value)
}

fn write_source(
    directory: &Path,
    file_name: &str,
    duration_in_seconds: u64,
    sound: BeepPattern,
) -> PathBuf {
    let output_path = directory.join(file_name);
    write_test_video(
        &FfmpegBackend::new().unwrap(),
        &TestVideoDescription {
            output_path: output_path.clone(),
            width_in_pixels: 320,
            height_in_pixels: 240,
            frame_rate: FrameRate::FPS_30,
            duration: TimeSpan::from_whole_seconds(duration_in_seconds),
            audio_sample_rate_in_hertz: SAMPLE_RATE,
            sound,
        },
    )
    .unwrap();
    output_path
}

fn small_canvas_settings() -> ProjectSettings {
    ProjectSettings {
        canvas_size: PictureSize {
            width_in_pixels: 320,
            height_in_pixels: 240,
        },
        ..ProjectSettings::landscape_1080p()
    }
}

/// The edit under test:
/// - video track: source 1–3 s at 0 s; source 5–8 s at 2 s played at 1.5× (2 s
///   long); source 9–10 s at 4 s. Total: 5 s.
/// - audio track: a continuous 300 Hz "music" tone under the whole edit, at half volume.
fn build_three_cut_project(
    backend: &dyn MediaBackend,
    interview_path: &Path,
    music_path: &Path,
) -> Project {
    let mut session = EditSession::new(Project::new(small_canvas_settings())).unwrap();
    let interview_id = session
        .execute(ImportMedia::new(backend.probe(interview_path).unwrap()))
        .unwrap()
        .first_media_asset_id()
        .unwrap();
    let music_id = session
        .execute(ImportMedia::new(backend.probe(music_path).unwrap()))
        .unwrap()
        .first_media_asset_id()
        .unwrap();
    let video_track = session
        .execute(AddTrack::new(TrackKind::Video, TrackPosition::Top))
        .unwrap()
        .first_track_id()
        .unwrap();
    let audio_track = session
        .execute(AddTrack::new(TrackKind::Audio, TrackPosition::Top))
        .unwrap()
        .first_track_id()
        .unwrap();

    session
        .execute(AddClip::media(
            video_track,
            interview_id,
            seconds(1),
            seconds(3),
            seconds(0),
        ))
        .unwrap();
    let faster_clip = session
        .execute(AddClip::media(
            video_track,
            interview_id,
            seconds(5),
            seconds(8),
            seconds(2),
        ))
        .unwrap()
        .first_clip_id()
        .unwrap();
    session
        .execute(SetClipSpeed::new(
            faster_clip,
            PlaybackSpeed::from_fraction(3, 2).unwrap(),
        ))
        .unwrap();
    session
        .execute(AddClip::media(
            video_track,
            interview_id,
            seconds(9),
            seconds(10),
            seconds(4),
        ))
        .unwrap();

    let music_clip = session
        .execute(AddClip::media(
            audio_track,
            music_id,
            seconds(0),
            seconds(5),
            seconds(0),
        ))
        .unwrap()
        .first_clip_id()
        .unwrap();
    session
        .execute(SetClipPropertyValue::new(
            music_clip,
            AnimatableProperty::Volume,
            0.5,
        ))
        .unwrap();
    session.into_project()
}

/// Which source time the edit shows at `timeline_time`.
fn expected_source_time(timeline_time: Timestamp) -> Timestamp {
    let elapsed = |clip_start: u64| timeline_time.span_since(seconds(clip_start)).unwrap();
    if timeline_time < seconds(2) {
        seconds(1) + elapsed(0)
    } else if timeline_time < seconds(4) {
        seconds(5)
            + PlaybackSpeed::from_fraction(3, 2)
                .unwrap()
                .source_span_for_timeline_span(elapsed(2))
    } else {
        seconds(9) + elapsed(4)
    }
}

fn software_export_request(output_path: PathBuf) -> ExportRequest {
    ExportRequest {
        output_path,
        video_encoder_preference: VideoEncoderPreference::SoftwareOnly,
        encoding_speed: EncodingSpeed::Fast,
        renderer_preference: RendererPreference::GpuIfAvailable,
    }
}

#[test]
fn three_cuts_a_speed_change_and_music_export_in_sync() {
    let directory = tempfile::tempdir().unwrap();
    let backend = FfmpegBackend::new().unwrap();
    let interview_path = write_source(
        directory.path(),
        "interview.mp4",
        10,
        BeepPattern::every_second(
            BEEP_FREQUENCY,
            0.5,
            TimeSpan::from_whole_seconds(10),
            TimeSpan::from_milliseconds(100),
        ),
    );
    let music_path = write_source(
        directory.path(),
        "music.mp4",
        6,
        BeepPattern::continuous(MUSIC_FREQUENCY, 0.3, TimeSpan::from_whole_seconds(6)),
    );
    let project = build_three_cut_project(&backend, &interview_path, &music_path);
    let output_path = directory.path().join("export.mp4");

    let mut last_progress = None;
    let report = export_project(
        &project,
        &backend,
        &software_export_request(output_path.clone()),
        &mut |progress| last_progress = Some(progress),
        &ExportCancellation::new(),
    )
    .unwrap();

    // The report.
    assert_eq!(report.frame_count, 150);
    assert_eq!(report.video_encoder_name, "libx264");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert_eq!(last_progress.unwrap().fraction(), 1.0);

    // Every picture comes from the exact expected source frame.
    let mut exported_video = backend.open_video_source(&output_path).unwrap();
    for frame_index in 0..150 {
        let timeline_time = FrameRate::FPS_30.timestamp_of_frame(frame_index);
        let expected_source_frame =
            FrameRate::FPS_30.frame_index_at(expected_source_time(timeline_time));
        let exported_frame = exported_video.frame_at(timeline_time).unwrap();
        assert_eq!(
            u64::from(read_frame_index_barcode(&exported_frame)),
            expected_source_frame,
            "output frame {frame_index} at {timeline_time}"
        );
    }

    // Every beep is where the edit puts it.
    let exported_sound = backend
        .decode_audio_segment(&AudioSegmentRequest {
            file_path: output_path,
            source_start: Timestamp::ZERO,
            source_end: seconds(5),
            speed: PlaybackSpeed::NORMAL,
            output_sample_rate_in_hertz: SAMPLE_RATE,
            output_samples_per_channel: 5 * SAMPLE_RATE as usize,
        })
        .unwrap();
    let expected_beeps_in_milliseconds = [0, 1_000, 2_000, 2_667, 3_333, 4_000];
    let found_beeps = find_beep_starts(&exported_sound, BEEP_FREQUENCY);
    assert_eq!(
        found_beeps.len(),
        expected_beeps_in_milliseconds.len(),
        "{found_beeps:?}"
    );
    for (found, expected_in_milliseconds) in found_beeps.iter().zip(expected_beeps_in_milliseconds)
    {
        let expected = Timestamp::from_milliseconds(expected_in_milliseconds);
        assert!(
            found.ticks().abs_diff(expected.ticks()) <= SYNC_TOLERANCE.ticks(),
            "beep found at {found}, expected at {expected}"
        );
    }

    // The music plays under the whole edit, at half volume (0.3 × 0.5 = 0.15, strength ≈ 0.0225).
    let mono_sound = exported_sound.to_mono();
    for check_second in [0.5_f32, 2.5, 4.5] {
        let first_sample = (check_second * SAMPLE_RATE as f32) as usize;
        let strength = tone_strength(
            &mono_sound[first_sample..first_sample + 4_800],
            SAMPLE_RATE,
            MUSIC_FREQUENCY,
        );
        assert!(
            (0.015..0.03).contains(&strength),
            "music strength {strength} at {check_second} s"
        );
    }
}

#[test]
fn a_cancelled_export_leaves_no_file() {
    let directory = tempfile::tempdir().unwrap();
    let backend = FfmpegBackend::new().unwrap();
    let source_path = write_source(
        directory.path(),
        "source.mp4",
        2,
        BeepPattern::continuous(MUSIC_FREQUENCY, 0.3, TimeSpan::from_whole_seconds(2)),
    );
    let project = assemble_project_from_files(
        &backend,
        &[source_path],
        small_canvas_settings(),
        TimeSpan::from_whole_seconds(3),
    )
    .unwrap();
    let output_path = directory.path().join("cancelled.mp4");
    let cancellation = ExportCancellation::new();
    let cancel_from_progress = cancellation.clone();

    let result = export_project(
        &project,
        &backend,
        &software_export_request(output_path.clone()),
        &mut |progress| {
            if progress.encoded_frame_count == 10 {
                cancel_from_progress.cancel();
            }
        },
        &cancellation,
    );

    assert!(matches!(result, Err(ExportError::Cancelled)));
    assert!(!output_path.exists());
    let leftover_files: Vec<_> = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.to_string_lossy().contains("cancelled"))
        .collect();
    assert!(leftover_files.is_empty(), "{leftover_files:?}");
}

#[test]
fn files_are_assembled_end_to_end_and_an_empty_project_is_refused() {
    let directory = tempfile::tempdir().unwrap();
    let backend = FfmpegBackend::new().unwrap();
    let first = write_source(
        directory.path(),
        "first.mp4",
        2,
        BeepPattern::continuous(300.0, 0.3, TimeSpan::from_whole_seconds(2)),
    );
    let second = write_source(
        directory.path(),
        "second.mp4",
        3,
        BeepPattern::continuous(300.0, 0.3, TimeSpan::from_whole_seconds(3)),
    );

    let project = assemble_project_from_files(
        &backend,
        &[first, second],
        small_canvas_settings(),
        TimeSpan::from_whole_seconds(3),
    )
    .unwrap();

    let starts: Vec<Timestamp> = project.timeline().tracks()[0]
        .clips()
        .iter()
        .map(|clip| clip.timeline_start)
        .collect();
    assert_eq!(starts, vec![seconds(0), seconds(2)]);

    let empty_project = Project::new(small_canvas_settings());
    let result = export_project(
        &empty_project,
        &backend,
        &software_export_request(directory.path().join("empty.mp4")),
        &mut |_| {},
        &ExportCancellation::new(),
    );
    assert!(matches!(result, Err(ExportError::EmptyTimeline)));
}
