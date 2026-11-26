//! Exporting through MCP: the tool answers at once with a task id, the status
//! tool follows it to the end, and the file is a real video.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::time::{Duration, Instant};

use kadr_engine::{FfmpegBackend, MediaBackend};
use serde_json::{Value, json};
use support::{TestServer, json_details, write_beeping_video};

/// Longest wait for a test export.
const EXPORT_TIMEOUT: Duration = Duration::from_secs(600);

/// Polls get_export_status until the task is no longer running.
async fn wait_for_export(server: &TestServer, task_id: &str) -> Value {
    let started = Instant::now();
    loop {
        let status = json_details(
            &server
                .call_ok("get_export_status", json!({ "task_id": task_id }))
                .await,
        );
        if status["state"] != "running" {
            return status;
        }
        assert!(
            started.elapsed() < EXPORT_TIMEOUT,
            "the export did not finish: {status}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn task_id_in(text: &str) -> String {
    let start = text.find("export-").unwrap();
    text[start..]
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '-'))
        .next()
        .unwrap()
        .to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_export_runs_in_the_background_and_produces_the_video() {
    let server = TestServer::start().await;
    let video = write_beeping_video(server.allowed_folder.path(), "source.mp4", 320, 240, 2);
    server
        .create_project(json!({ "width_in_pixels": 320, "height_in_pixels": 180 }))
        .await;
    let media_id = server.import(&video).await;
    server
        .call_ok("add_clip", json!({ "media_id": media_id }))
        .await;
    server
        .call_ok(
            "add_text",
            json!({ "text": "Bye", "start_time_in_seconds": 1, "duration_in_seconds": 1 }),
        )
        .await;

    // Same shape, twice the size: positions and texts are scaled.
    let output = server.allowed_path("final");
    let started = server
        .call_ok(
            "export",
            json!({ "output_path": output, "preset": "custom", "width_in_pixels": 640,
                    "height_in_pixels": 360, "quality": "fast" }),
        )
        .await;
    let task_id = task_id_in(&started);
    let status = wait_for_export(&server, &task_id).await;
    assert_eq!(status["state"], "finished", "{status}");
    assert_eq!(status["video_duration_in_seconds"], 2.0);

    let exported = server.allowed_folder.path().join("final.mp4");
    let description = FfmpegBackend::new().unwrap().probe(&exported).unwrap();
    let picture_size = description.picture_size.unwrap();
    assert_eq!(
        (picture_size.width_in_pixels, picture_size.height_in_pixels),
        (640, 360)
    );
    assert!(description.has_audio());

    // The same name again is refused, without touching the file.
    let size_before = std::fs::metadata(&exported).unwrap().len();
    let error = server
        .call_error("export", json!({ "output_path": output }))
        .await;
    assert!(error.contains("already exists"), "{error}");
    assert_eq!(std::fs::metadata(&exported).unwrap().len(), size_before);

    // The list shows the finished task.
    let all = json_details(&server.call_ok("get_export_status", json!({})).await);
    assert_eq!(all.as_array().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn exports_of_another_shape_are_refused_with_the_way_forward() {
    let server = TestServer::start().await;
    server.create_project(json!({})).await;
    server.call_ok("add_text", json!({ "text": "Title" })).await;
    let error = server
        .call_error(
            "export",
            json!({ "output_path": server.allowed_path("vertical.mp4"), "preset": "tiktok_reels_shorts" }),
        )
        .await;
    assert!(error.contains("set_project_settings"), "{error}");

    let error = server
        .call_error("get_export_status", json!({ "task_id": "export-9" }))
        .await;
    assert!(error.contains("no export was started"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_export_leaves_no_file() {
    let server = TestServer::start().await;
    let video = write_beeping_video(server.allowed_folder.path(), "long.mp4", 320, 240, 10);
    server.create_project(json!({})).await;
    let media_id = server.import(&video).await;
    server
        .call_ok("add_clip", json!({ "media_id": media_id }))
        .await;

    let started = server
        .call_ok(
            "export",
            json!({ "output_path": server.allowed_path("cancelled.mp4") }),
        )
        .await;
    let task_id = task_id_in(&started);
    server
        .call_ok("cancel_export", json!({ "task_id": task_id }))
        .await;
    let status = wait_for_export(&server, &task_id).await;
    // A very fast machine may finish before the cancellation is seen.
    if status["state"] == "cancelled" {
        assert!(!server.allowed_folder.path().join("cancelled.mp4").exists());
    } else {
        assert_eq!(status["state"], "finished", "{status}");
    }
}
