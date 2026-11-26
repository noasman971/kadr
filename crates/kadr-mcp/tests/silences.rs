//! Finding silences through MCP, and cutting one out as the tool description
//! tells Claude to.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use serde_json::{Value, json};
use support::{TestServer, json_details, write_beeping_video};

/// Encoding and decoding sound moves edges by a few milliseconds.
const EDGE_TOLERANCE_IN_SECONDS: f64 = 0.03;

fn assert_close(value: &Value, expected_in_seconds: f64) {
    let found = value.as_f64().unwrap();
    assert!(
        (found - expected_in_seconds).abs() <= EDGE_TOLERANCE_IN_SECONDS,
        "{found} instead of {expected_in_seconds}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn silences_are_found_in_a_file_and_on_the_timeline() {
    let server = TestServer::start().await;
    // Beeps of 0.2 s at 0, 1 and 2 s; silence in between.
    let video = write_beeping_video(server.allowed_folder.path(), "talk.mp4", 320, 240, 3);
    server.create_project(json!({})).await;
    let media_id = server.import(&video).await;

    let found = json_details(
        &server
            .call_ok("detect_silences", json!({ "media_id": media_id }))
            .await,
    );
    let silences = found["silences"].as_array().unwrap();
    assert_eq!(silences.len(), 3, "{found}");
    assert_close(&silences[0]["source_start_in_seconds"], 0.2);
    assert_close(&silences[0]["source_end_in_seconds"], 1.0);
    assert!(silences[0]["timeline_start_in_seconds"].is_null());

    // A clip playing 0.4 s to 3 s of the file, placed at 10 s on the timeline.
    let clip = json_details(
        &server
            .call_ok(
                "add_clip",
                json!({ "media_id": media_id, "source_start_in_seconds": 0.4,
                        "source_end_in_seconds": 3, "start_time_in_seconds": 10 }),
            )
            .await,
    )["clip_id"]
        .as_u64()
        .unwrap();
    let found = json_details(
        &server
            .call_ok("detect_silences", json!({ "clip_id": clip }))
            .await,
    );
    let silences = found["silences"].as_array().unwrap();
    assert_eq!(silences.len(), 3, "{found}");
    assert_close(&silences[0]["timeline_start_in_seconds"], 10.0);
    assert_close(&silences[1]["timeline_start_in_seconds"], 10.8);
    assert_close(&silences[1]["timeline_end_in_seconds"], 11.6);

    // Cut the middle silence: split at both ends, remove the middle with close_gap.
    let start = silences[1]["timeline_start_in_seconds"].as_f64().unwrap();
    let end = silences[1]["timeline_end_in_seconds"].as_f64().unwrap();
    let split = json_details(
        &server
            .call_ok(
                "split_clip",
                json!({ "clip_id": clip, "time_in_seconds": start }),
            )
            .await,
    );
    let middle = split[1]["clip_id"].as_u64().unwrap();
    server
        .call_ok(
            "split_clip",
            json!({ "clip_id": middle, "time_in_seconds": end }),
        )
        .await;
    server
        .call_ok(
            "remove_clip",
            json!({ "clip_id": middle, "close_gap": true }),
        )
        .await;
    let timeline = json_details(&server.call_ok("get_timeline", json!({})).await);
    assert_close(&timeline["duration_in_seconds"], 12.6 - (end - start));

    let error = server
        .call_error(
            "detect_silences",
            json!({ "media_id": media_id, "threshold_in_decibels": 5 }),
        )
        .await;
    assert!(error.contains("from -100 to -1"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_silences_of_a_clip_are_removed_in_one_undoable_step() {
    let server = TestServer::start().await;
    // Beeps of 0.2 s at 0, 1 and 2 s: silent from 0.2 to 1 s, 1.2 to 2 s, 2.2 to 3 s.
    let video = write_beeping_video(server.allowed_folder.path(), "talk.mp4", 320, 240, 3);
    server.create_project(json!({})).await;
    let media_id = server.import(&video).await;
    let clip = json_details(
        &server
            .call_ok("add_clip", json!({ "media_id": media_id }))
            .await,
    )["clip_id"]
        .as_u64()
        .unwrap();

    let removed = json_details(
        &server
            .call_ok("remove_silences", json!({ "clip_id": clip }))
            .await,
    );

    // Each silence keeps 0.1 s next to each sound; the last one, at the end of
    // the clip, only next to the sound before it.
    assert_eq!(removed["silences_shortened"], 3, "{removed}");
    let removed_seconds = removed["removed_seconds"].as_f64().unwrap();
    assert!((removed_seconds - 1.9).abs() < 0.12, "{removed}");
    assert_eq!(removed["clip_ids"][0], clip);
    assert_eq!(removed["clip_ids"].as_array().unwrap().len(), 3);
    let timeline = json_details(&server.call_ok("get_timeline", json!({})).await);
    let duration = timeline["duration_in_seconds"].as_f64().unwrap();
    assert!(
        (duration - (3.0 - removed_seconds)).abs() < 0.001,
        "{timeline}"
    );

    server.call_ok("undo", json!({})).await;
    let timeline = json_details(&server.call_ok("get_timeline", json!({})).await);
    assert_close(&timeline["duration_in_seconds"], 3.0);

    let title = json_details(&server.call_ok("add_text", json!({ "text": "Hi" })).await)["clip_id"]
        .as_u64()
        .unwrap();
    let error = server
        .call_error("remove_silences", json!({ "clip_id": title }))
        .await;
    assert!(error.contains("no sound"), "{error}");
    let error = server
        .call_error(
            "remove_silences",
            json!({ "clip_id": clip, "kept_pause_in_seconds": 3 }),
        )
        .await;
    assert!(error.contains("from 0 to 2"), "{error}");
}
