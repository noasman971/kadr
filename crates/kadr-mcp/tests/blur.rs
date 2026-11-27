//! Blurring through MCP, as Claude does it: a rectangle on one frame, the
//! analysis in the background, the ranges to check, a correction, and a
//! cancelled analysis.
//!
//! The video is a synthetic scene (`kadr_vision::synthetic_scenes`) encoded to
//! H.264: a textured object moving over a textured background, hidden by an
//! obstacle for half a second, so the true position is known in every frame.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use image::{Rgba, RgbaImage};
use kadr_core::time::FrameRate;
use kadr_engine::FfmpegBackend;
use kadr_media::{
    EncodingSpeed, ExportSinkSettings, MediaBackend, VideoEncoderPreference, VideoFrame,
};
use kadr_vision::Region;
use kadr_vision::synthetic_scenes::{SceneFrame, SceneObject};
use serde_json::{Value, json};
use support::{TestServer, json_details};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;
const FRAMES_PER_SECOND: u64 = 25;
const FRAME_RATE: FrameRate = FrameRate::FPS_25;
/// The obstacle covers the object on these frames (1.2 s to 1.68 s).
const HIDDEN_FRAMES: std::ops::Range<u64> = 30..42;

/// Longest wait for an analysis.
const ANALYSIS_TIMEOUT: Duration = Duration::from_secs(300);

/// Where the object truly is in frame `index`.
fn true_region(index: u64) -> Region {
    let time = index as f32;
    Region::from_top_left(60.0 + 2.4 * time, 80.0 + 0.6 * time, 44.0, 52.0)
}

/// Writes `frame_count` frames of the scene as an H.264 video.
fn write_scene_video(folder: &Path, name: &str, frame_count: u64) -> PathBuf {
    let path = folder.join(name);
    let backend = FfmpegBackend::new().unwrap();
    let mut sink = backend
        .create_export_sink(&ExportSinkSettings {
            output_path: path.clone(),
            width_in_pixels: WIDTH,
            height_in_pixels: HEIGHT,
            frame_rate: FRAME_RATE,
            audio_sample_rate_in_hertz: 48_000,
            video_encoder_preference: VideoEncoderPreference::SoftwareOnly,
            encoding_speed: EncodingSpeed::Fast,
        })
        .unwrap();
    let obstacle = true_region(HIDDEN_FRAMES.start.midpoint(HIDDEN_FRAMES.end)).scaled_by(1.8);
    for index in 0..frame_count {
        // The object loops back and forth so long videos stay in the picture.
        let loop_index = index % 75;
        let grey = SceneFrame {
            width: WIDTH as usize,
            height: HEIGHT as usize,
            background_seed: 7,
            background_offset_in_pixels: 0.0,
            object: Some(SceneObject {
                region: true_region(loop_index),
                texture_seed: 8,
            }),
            obstacle: HIDDEN_FRAMES.contains(&index).then_some(obstacle),
        }
        .render();
        let picture = RgbaImage::from_fn(WIDTH, HEIGHT, |x, y| {
            let value = grey.value(x as usize, y as usize).round().clamp(0.0, 255.0) as u8;
            Rgba([value, value, value, 255])
        });
        sink.write_video_frame(&VideoFrame::Rgba(picture)).unwrap();
    }
    sink.finish().unwrap();
    path
}

/// The rectangle of `blur_region` for the object at frame `index`.
fn rectangle_at(index: u64) -> Value {
    let region = true_region(index);
    json!({
        "left_in_pixels": region.left(),
        "top_in_pixels": region.top(),
        "width_in_pixels": region.width,
        "height_in_pixels": region.height,
    })
}

fn seconds_of_frame(index: u64) -> f64 {
    index as f64 / FRAMES_PER_SECOND as f64
}

fn blur_task_id_in(text: &str) -> String {
    let start = text.find("blur-").unwrap();
    text[start..]
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '-'))
        .next()
        .unwrap()
        .to_string()
}

/// Polls get_blur_status until the task is no longer running.
async fn wait_for_analysis(server: &TestServer, task_id: &str) -> Value {
    let started = Instant::now();
    loop {
        let status = json_details(
            &server
                .call_ok("get_blur_status", json!({ "task_id": task_id }))
                .await,
        );
        if status["state"] != "running" {
            return status;
        }
        assert!(
            started.elapsed() < ANALYSIS_TIMEOUT,
            "the analysis did not finish: {status}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A project with the scene video as its only clip; returns the clip id.
async fn project_with_scene(server: &TestServer, frame_count: u64) -> u64 {
    let video = write_scene_video(server.allowed_folder.path(), "scene.mp4", frame_count);
    server
        .create_project(
            json!({ "width_in_pixels": WIDTH, "height_in_pixels": HEIGHT,
                                "frames_per_second": FRAMES_PER_SECOND }),
        )
        .await;
    let media_id = server.import(&video).await;
    let added = server
        .call_ok("add_clip", json!({ "media_id": media_id }))
        .await;
    json_details(&added)["clip_id"].as_u64().unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_face_is_blurred_checked_and_corrected() {
    let server = TestServer::start().await;
    let clip_id = project_with_scene(&server, 75).await;

    // Claude looks at the clip's source picture, then gives the rectangle.
    server
        .call_ok(
            "get_frame",
            json!({ "clip_id": clip_id, "time_in_seconds": seconds_of_frame(5) }),
        )
        .await;
    let mut arguments = rectangle_at(5);
    arguments["clip_id"] = json!(clip_id);
    arguments["time_in_seconds"] = json!(seconds_of_frame(5));
    let started = server.call_ok("blur_region", arguments).await;
    assert!(started.contains("get_blur_status"), "{started}");
    let task_id = blur_task_id_in(&started);
    assert_eq!(task_id, "blur-1");

    let status = wait_for_analysis(&server, &task_id).await;
    assert_eq!(status["state"], "finished", "{status}");
    let blur = &status["tracked_blur"];
    let effect_id = blur["effect_id"].as_u64().unwrap();
    assert_eq!(blur["style"], "gaussian_blur");
    assert_eq!(blur["shape"], "ellipse");
    assert!(
        status["frame_counts"]["lost"].as_u64().unwrap() > 0,
        "{status}"
    );

    // The hidden stretch is the range to check, in timeline seconds.
    let ranges = blur["ranges_to_check"].as_array().unwrap();
    assert_eq!(ranges.len(), 1, "{status}");
    let range = &ranges[0];
    assert_eq!(range["confidence"], "lost");
    let start = range["start_time_in_seconds"].as_f64().unwrap();
    let end = range["end_time_in_seconds"].as_f64().unwrap();
    let look_at = range["look_at_time_in_seconds"].as_f64().unwrap();
    assert!(
        start <= seconds_of_frame(31) && end >= seconds_of_frame(40),
        "{range}"
    );
    assert!(start <= look_at && look_at <= end);

    // One undo step labelled for Claude, saved in the file.
    let info = server.call_ok("get_project_info", json!({})).await;
    assert!(
        info.contains(&format!("Claude: Blur a region on clip {clip_id}")),
        "{info}"
    );
    let saved = std::fs::read_to_string(server.allowed_folder.path().join("edit.kadr")).unwrap();
    assert!(saved.contains("tracked_blur"));

    // Claude corrects the first frame after the obstacle.
    let mut correction = rectangle_at(HIDDEN_FRAMES.end);
    correction["clip_id"] = json!(clip_id);
    correction["effect_id"] = json!(effect_id);
    correction["time_in_seconds"] = json!(seconds_of_frame(HIDDEN_FRAMES.end));
    correction["style"] = json!("pixelate");
    let started = server.call_ok("blur_region", correction).await;
    let correction_task = blur_task_id_in(&started);
    let status = wait_for_analysis(&server, &correction_task).await;
    assert_eq!(status["state"], "finished", "{status}");
    let corrected = &status["tracked_blur"];
    assert_eq!(corrected["effect_id"].as_u64().unwrap(), effect_id);
    assert_eq!(corrected["correction_count"], 1);
    assert_eq!(corrected["style"], "pixelate");
    assert_eq!(corrected["strength_in_pixels"], 24.0);
    for range in corrected["ranges_to_check"].as_array().unwrap() {
        let end = range["end_time_in_seconds"].as_f64().unwrap();
        assert!(end < seconds_of_frame(HIDDEN_FRAMES.end), "{range}");
    }

    // The clip view lists the same blur.
    let by_clip = json_details(
        &server
            .call_ok("get_blur_status", json!({ "clip_id": clip_id }))
            .await,
    );
    assert_eq!(by_clip.as_array().unwrap().len(), 1);

    // Undo removes the correction only.
    let undone = server.call_ok("undo", json!({})).await;
    assert!(undone.contains("Correct blur"), "{undone}");
    let by_clip = json_details(
        &server
            .call_ok("get_blur_status", json!({ "clip_id": clip_id }))
            .await,
    );
    assert_eq!(by_clip[0]["correction_count"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn mistakes_are_explained_before_anything_runs() {
    let server = TestServer::start().await;
    let clip_id = project_with_scene(&server, 25).await;

    let outside = server
        .call_error(
            "blur_region",
            json!({ "clip_id": clip_id, "time_in_seconds": 0.2, "left_in_pixels": 400,
                    "top_in_pixels": 10, "width_in_pixels": 50, "height_in_pixels": 50 }),
        )
        .await;
    assert!(outside.contains("320 × 240"), "{outside}");

    let too_small = server
        .call_error(
            "blur_region",
            json!({ "clip_id": clip_id, "time_in_seconds": 0.2, "left_in_pixels": 10,
                    "top_in_pixels": 10, "width_in_pixels": 4, "height_in_pixels": 50 }),
        )
        .await;
    assert!(too_small.contains("at least"), "{too_small}");

    let too_late = server
        .call_error(
            "blur_region",
            json!({ "clip_id": clip_id, "time_in_seconds": 30, "left_in_pixels": 60,
                    "top_in_pixels": 80, "width_in_pixels": 44, "height_in_pixels": 52 }),
        )
        .await;
    assert!(too_late.contains("not on screen"), "{too_late}");

    let mut bad_strength = rectangle_at(2);
    bad_strength["clip_id"] = json!(clip_id);
    bad_strength["time_in_seconds"] = json!(seconds_of_frame(2));
    bad_strength["strength_in_pixels"] = json!(9000);
    let error = server.call_error("blur_region", bad_strength).await;
    assert!(error.contains("blur radius"), "{error}");

    let mut unknown_blur = rectangle_at(2);
    unknown_blur["clip_id"] = json!(clip_id);
    unknown_blur["time_in_seconds"] = json!(seconds_of_frame(2));
    unknown_blur["effect_id"] = json!(999);
    let error = server.call_error("blur_region", unknown_blur).await;
    assert!(error.contains("999"), "{error}");

    let error = server
        .call_error("get_blur_status", json!({ "task_id": "blur-7" }))
        .await;
    assert!(error.contains("blur_region was not called"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_analysis_adds_nothing() {
    let server = TestServer::start().await;
    let clip_id = project_with_scene(&server, 1_500).await;
    let mut arguments = rectangle_at(5);
    arguments["clip_id"] = json!(clip_id);
    arguments["time_in_seconds"] = json!(seconds_of_frame(5));
    let started = server.call_ok("blur_region", arguments).await;
    let task_id = blur_task_id_in(&started);
    let cancelled = server
        .call_ok("cancel_blur", json!({ "task_id": task_id }))
        .await;
    assert!(cancelled.contains("cancelled"), "{cancelled}");
    let status = wait_for_analysis(&server, &task_id).await;
    assert_eq!(status["state"], "cancelled", "{status}");
    let by_clip = json_details(
        &server
            .call_ok("get_blur_status", json!({ "clip_id": clip_id }))
            .await,
    );
    assert!(by_clip.as_array().unwrap().is_empty());
}
