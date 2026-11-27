//! Proxies in the engine (decision record 0055): the cache makes them from a 4K
//! video written by FFmpeg for the test, the preview shows them instead of the
//! original with the same frames and the blurs in the same places, and the
//! export never reads them.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kadr_core::color::Color;
use kadr_core::commands::{
    AddClip, AddEffect, AddTrack, CreatedObjectsExt, ImportMedia, TrackPosition,
};
use kadr_core::effect::{
    BlurShape, BlurStyle, EffectKind, PixelRegion, RegionSample, RegionSampleOrigin, TrackedBlur,
    TrackingConfidence,
};
use kadr_core::history::EditSession;
use kadr_core::media_asset::PictureSize;
use kadr_core::project::{Project, ProjectSettings};
use kadr_core::time::{FrameRate, TimeSpan, Timestamp};
use kadr_core::timeline::TrackKind;
use kadr_engine::preview::PreviewRenderer;
use kadr_engine::proxies::{ProxyCache, ProxyMakingOutcome, ReadyProxies, ReadyProxy};
use kadr_engine::{
    EncodingSpeed, ExportCancellation, ExportRequest, RendererPreference, VideoEncoderPreference,
    export_project,
};
use kadr_media::test_patterns::{
    BeepPattern, TestVideoDescription, read_frame_index_barcode, write_test_video,
};
use kadr_media::{FfmpegBackend, MediaBackend, VideoFrame, YuvPicture};

const FRAME_COUNT: u64 = 24;
const PREVIEW_SIZE: PictureSize = PictureSize {
    width_in_pixels: 960,
    height_in_pixels: 540,
};

fn size(width_in_pixels: u32, height_in_pixels: u32) -> PictureSize {
    PictureSize {
        width_in_pixels,
        height_in_pixels,
    }
}

/// A barcode video (each frame shows its number) with a beep, at `picture_size`.
fn barcode_video(path: &Path, picture_size: PictureSize, frame_count: u64) -> PathBuf {
    let duration = TimeSpan::from_ticks(FrameRate::FPS_30.timestamp_of_frame(frame_count).ticks());
    write_test_video(
        &FfmpegBackend::new().unwrap(),
        &TestVideoDescription {
            output_path: path.to_path_buf(),
            width_in_pixels: picture_size.width_in_pixels,
            height_in_pixels: picture_size.height_in_pixels,
            frame_rate: FrameRate::FPS_30,
            duration,
            audio_sample_rate_in_hertz: 48_000,
            sound: BeepPattern::every_second(
                1_000.0,
                0.5,
                duration,
                TimeSpan::from_milliseconds(100),
            ),
        },
    )
    .unwrap();
    path.to_path_buf()
}

/// A 1920 × 1080 project showing `video` from the start, with `effects` on its clip.
fn project_showing(video: &Path, effects: Vec<EffectKind>) -> Project {
    let backend = FfmpegBackend::new().unwrap();
    let mut session = EditSession::new(Project::new(ProjectSettings::landscape_1080p())).unwrap();
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
    let clip_id = session
        .execute(AddClip::media(
            track_id,
            media_id,
            Timestamp::ZERO,
            FrameRate::FPS_30.timestamp_of_frame(FRAME_COUNT),
            Timestamp::ZERO,
        ))
        .unwrap()
        .first_clip_id()
        .unwrap();
    for effect in effects {
        session.execute(AddEffect::new(clip_id, effect)).unwrap();
    }
    session.into_project()
}

fn make_proxy(cache: &ProxyCache, video: &Path) -> ReadyProxy {
    let backend = FfmpegBackend::new().unwrap();
    let description = backend.probe(video).unwrap();
    match cache
        .make_proxy(&backend, &description, |_| ControlFlow::Continue(()))
        .unwrap()
    {
        ProxyMakingOutcome::Ready(proxy) => proxy,
        ProxyMakingOutcome::Cancelled => panic!("nothing cancelled the proxy"),
    }
}

fn preview_renderer(proxies: Option<&Arc<ReadyProxies>>) -> PreviewRenderer {
    let mut renderer = PreviewRenderer::new(RendererPreference::GpuIfAvailable).unwrap();
    if let Some(proxies) = proxies {
        renderer.use_proxies(Arc::clone(proxies));
    }
    renderer
}

fn preview_at(renderer: &mut PreviewRenderer, project: &Project, frame_index: u64) -> YuvPicture {
    let time = FrameRate::FPS_30.timestamp_of_frame(frame_index) + TimeSpan::from_milliseconds(5);
    renderer
        .render_frame(project, &FfmpegBackend::new().unwrap(), time, PREVIEW_SIZE)
        .unwrap()
}

fn exported_bytes(project: &Project, output_path: &Path) -> Vec<u8> {
    export_project(
        project,
        &FfmpegBackend::new().unwrap(),
        &ExportRequest {
            output_path: output_path.to_path_buf(),
            video_encoder_preference: VideoEncoderPreference::SoftwareOnly,
            encoding_speed: EncodingSpeed::Fast,
            renderer_preference: RendererPreference::GpuIfAvailable,
        },
        &mut |_| {},
        &ExportCancellation::new(),
    )
    .unwrap();
    fs::read(output_path).unwrap()
}

#[test]
fn the_preview_shows_the_same_frames_from_a_proxy_and_the_export_never_reads_it() {
    let folder = tempfile::tempdir().unwrap();
    let video = barcode_video(
        &folder.path().join("barcode_4k.mp4"),
        size(3840, 2160),
        FRAME_COUNT,
    );
    let project = project_showing(&video, Vec::new());
    let export_before = exported_bytes(&project, &folder.path().join("before.mp4"));

    let backend = FfmpegBackend::new().unwrap();
    let cache = ProxyCache::new(folder.path().join("proxies"));
    let mut fractions = Vec::new();
    let outcome = cache
        .make_proxy(&backend, &backend.probe(&video).unwrap(), |progress| {
            fractions.push(progress.fraction());
            ControlFlow::Continue(())
        })
        .unwrap();
    let ProxyMakingOutcome::Ready(proxy) = outcome else {
        panic!("nothing cancelled the proxy");
    };
    assert_eq!(proxy.picture_size, size(1920, 1080));
    assert!(proxy.proxy_path.starts_with(cache.folder()));
    assert!(
        fractions.windows(2).all(|pair| pair[0] <= pair[1]),
        "{fractions:?}"
    );
    assert!(*fractions.last().unwrap() > 0.9, "{fractions:?}");
    assert_eq!(cache.ready_proxy_of(&video), Some(proxy.clone()));

    let proxies = Arc::new(ReadyProxies::new());
    proxies.add(proxy);
    let mut with_proxy = preview_renderer(Some(&proxies));
    let mut without_proxy = preview_renderer(None);
    for frame_index in [0, 1, 2, 12, 23, 5, 22, 0, 13] {
        let shown = preview_at(&mut with_proxy, &project, frame_index);
        assert_eq!(
            u64::from(read_frame_index_barcode(&VideoFrame::Yuv420(shown))),
            frame_index
        );
        let original = preview_at(&mut without_proxy, &project, frame_index);
        assert_eq!(
            u64::from(read_frame_index_barcode(&VideoFrame::Yuv420(original))),
            frame_index
        );
    }

    let export_after = exported_bytes(&project, &folder.path().join("after.mp4"));
    assert!(
        export_before == export_after,
        "the export is the same with a proxy"
    );
}

/// The smallest rectangle holding every bright pixel: left, top, right, bottom.
fn bright_area_of(picture: &YuvPicture) -> (u32, u32, u32, u32) {
    let width = picture.width();
    let mut bounds = (u32::MAX, u32::MAX, 0, 0);
    for (index, &luma) in picture.y_plane().iter().enumerate() {
        if luma > 128 {
            let (x, y) = (index as u32 % width, index as u32 / width);
            bounds = (
                bounds.0.min(x),
                bounds.1.min(y),
                bounds.2.max(x),
                bounds.3.max(y),
            );
        }
    }
    bounds
}

#[test]
fn blurs_are_hidden_in_the_same_place_on_the_proxy() {
    let folder = tempfile::tempdir().unwrap();
    let video = barcode_video(
        &folder.path().join("barcode_4k.mp4"),
        size(3840, 2160),
        FRAME_COUNT,
    );
    // A white box in the original's pixels, on frame 0 (all its bars are black).
    let white_box = EffectKind::TrackedBlur(TrackedBlur {
        style: BlurStyle::SolidFill {
            color: Color::WHITE,
        },
        shape: BlurShape::Rectangle,
        margin_in_percent: 0.0,
        edge_softness_in_pixels: 0.0,
        region_samples: vec![RegionSample {
            source_time: Timestamp::ZERO,
            region: PixelRegion {
                left_in_pixels: 1536.0,
                top_in_pixels: 864.0,
                width_in_pixels: 768.0,
                height_in_pixels: 432.0,
            },
            confidence: TrackingConfidence::Confident,
            origin: RegionSampleOrigin::Tracker,
        }],
    });
    let project = project_showing(&video, vec![white_box]);
    let proxies = Arc::new(ReadyProxies::new());
    proxies.add(make_proxy(
        &ProxyCache::new(folder.path().join("proxies")),
        &video,
    ));

    let from_proxy = bright_area_of(&preview_at(
        &mut preview_renderer(Some(&proxies)),
        &project,
        0,
    ));
    let from_original = bright_area_of(&preview_at(&mut preview_renderer(None), &project, 0));

    // The 4K box shrinks by four onto the 960 × 540 preview.
    let expected = (384, 216, 575, 323);
    for (found, wanted) in [from_proxy, from_original].into_iter().flat_map(|area| {
        [
            (area.0, expected.0),
            (area.1, expected.1),
            (area.2, expected.2),
            (area.3, expected.3),
        ]
    }) {
        assert!(
            found.abs_diff(wanted) <= 2,
            "{from_proxy:?} / {from_original:?}"
        );
    }
}

#[test]
fn a_source_written_again_gets_no_old_proxy_and_a_cancelled_one_leaves_nothing() {
    let folder = tempfile::tempdir().unwrap();
    let video = barcode_video(&folder.path().join("small.mp4"), size(640, 360), 10);
    let cache = ProxyCache::new(folder.path().join("proxies"));
    make_proxy(&cache, &video);
    assert!(cache.ready_proxy_of(&video).is_some());

    barcode_video(&video, size(640, 360), 12);
    assert_eq!(cache.ready_proxy_of(&video), None);

    let backend = FfmpegBackend::new().unwrap();
    let outcome = cache
        .make_proxy(&backend, &backend.probe(&video).unwrap(), |_| {
            ControlFlow::Break(())
        })
        .unwrap();
    assert_eq!(outcome, ProxyMakingOutcome::Cancelled);
    assert_eq!(cache.ready_proxy_of(&video), None);
    let unfinished: Vec<_> = fs::read_dir(cache.folder())
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".making"))
        .collect();
    assert!(unfinished.is_empty(), "{unfinished:?}");
    // Clearing frees what the first proxy took.
    assert!(cache.size_in_bytes() > 0);
    assert!(cache.remove_proxies_except(&Default::default()) > 0);
    assert_eq!(cache.size_in_bytes(), 0);
}
