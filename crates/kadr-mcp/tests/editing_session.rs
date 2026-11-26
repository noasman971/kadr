//! A whole editing conversation through MCP calls: create a project, find and
//! import a video, cut it, add a title and a fade, and check that the project
//! file follows, that each call is one undo step, and that errors are clear.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use kadr_core::project_file::load_project_from_json;
use serde_json::json;
use support::{TestServer, json_details, write_beeping_video};

fn clip_ids_on_track(timeline: &serde_json::Value, track_index: usize) -> Vec<u64> {
    timeline["tracks"][track_index]["clips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|clip| clip["clip_id"].as_u64().unwrap())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_simple_edit_from_creation_to_undo() {
    let server = TestServer::start().await;
    let video = write_beeping_video(server.allowed_folder.path(), "interview.mp4", 320, 240, 4);
    let project_file = server
        .create_project(json!({ "canvas": "landscape_1920x1080" }))
        .await;
    assert!(project_file.exists(), "the project is saved at once");

    // Claude finds the file the user talks about.
    let found = server
        .call_ok("find_media_files", json!({ "name_contains": "INTERVIEW" }))
        .await;
    let found = json_details(&found);
    assert_eq!(found.as_array().unwrap().len(), 1);
    assert_eq!(found[0]["kind"], "video");

    // Import, then importing again gives the same media.
    let media_id = server.import(&video).await;
    assert_eq!(server.import(&video).await, media_id);

    // Place the whole video: a video track is created.
    let added = server
        .call_ok("add_clip", json!({ "media_id": media_id }))
        .await;
    let added = json_details(&added);
    let first_clip = added["clip_id"].as_u64().unwrap();
    assert_eq!(added["timeline_end_in_seconds"], 4.0);

    // Cut out 1 s to 2 s: two splits and a removal that closes the gap.
    let split = server
        .call_ok(
            "split_clip",
            json!({ "clip_id": first_clip, "time_in_seconds": 1.0 }),
        )
        .await;
    let middle_clip = json_details(&split)[1]["clip_id"].as_u64().unwrap();
    let split = server
        .call_ok(
            "split_clip",
            json!({ "clip_id": middle_clip, "time_in_seconds": 2.0 }),
        )
        .await;
    let last_clip = json_details(&split)[1]["clip_id"].as_u64().unwrap();
    server
        .call_ok(
            "remove_clip",
            json!({ "clip_id": middle_clip, "close_gap": true }),
        )
        .await;

    let timeline = json_details(&server.call_ok("get_timeline", json!({})).await);
    assert_eq!(timeline["duration_in_seconds"], 3.0);
    assert_eq!(clip_ids_on_track(&timeline, 0), vec![first_clip, last_clip]);
    assert_eq!(
        timeline["tracks"][0]["clips"][1]["timeline_start_in_seconds"],
        1.0
    );
    assert_eq!(
        timeline["tracks"][0]["clips"][1]["source_start_in_seconds"],
        2.0
    );

    // A title on a new track above, and a fade out at the end.
    let title = server
        .call_ok(
            "add_text",
            json!({ "text": "Interview", "start_time_in_seconds": 0, "duration_in_seconds": 2,
                    "center_y_in_pixels": 200, "color": "#FFD400" }),
        )
        .await;
    let title = json_details(&title);
    assert_eq!(title["transform"]["center_y_in_pixels"], 200.0);
    assert_eq!(title["text_style"]["color"], "#FFD400FF");
    server
        .call_ok(
            "add_transition",
            json!({ "clip_id": last_clip, "kind": "fade_out", "duration_in_seconds": 0.5 }),
        )
        .await;
    let faded = json_details(
        &server
            .call_ok("get_clip", json!({ "clip_id": last_clip }))
            .await,
    );
    assert!(faded["opacity"].is_null(), "the opacity is animated now");
    let animated: Vec<&str> = faded["animations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|animation| animation["property"].as_str().unwrap())
        .collect();
    assert_eq!(animated, vec!["volume", "opacity"]);

    // The project file follows every change.
    let saved = load_project_from_json(&std::fs::read_to_string(&project_file).unwrap()).unwrap();
    assert_eq!(saved.timeline().clip_ids().len(), 3);

    // One undo reverts one whole call: the removal that also moved a clip.
    let info = json_details(&server.call_ok("get_project_info", json!({})).await);
    let history: Vec<&str> = info["recent_changes_oldest_first"]
        .as_array()
        .unwrap()
        .iter()
        .map(|label| label.as_str().unwrap())
        .collect();
    assert!(
        history.iter().all(|label| label.starts_with("Claude: ")),
        "{history:?}"
    );
    server.call_ok("undo", json!({})).await; // the fade
    server.call_ok("undo", json!({})).await; // the title
    let undone = server.call_ok("undo", json!({})).await; // the removal and the move
    assert!(undone.contains("close the gap"), "{undone}");
    let timeline = json_details(&server.call_ok("get_timeline", json!({})).await);
    assert_eq!(timeline["duration_in_seconds"], 4.0);
    assert_eq!(
        clip_ids_on_track(&timeline, 0),
        vec![first_clip, middle_clip, last_clip]
    );

    server.call_ok("redo", json!({})).await;
    let timeline = json_details(&server.call_ok("get_timeline", json!({})).await);
    assert_eq!(clip_ids_on_track(&timeline, 0), vec![first_clip, last_clip]);
}

#[tokio::test(flavor = "multi_thread")]
async fn clip_properties_change_and_are_shown_in_canvas_coordinates() {
    let server = TestServer::start().await;
    let video = write_beeping_video(server.allowed_folder.path(), "clip.mp4", 320, 240, 2);
    server
        .create_project(json!({ "canvas": "vertical_1080x1920" }))
        .await;
    let media_id = server.import(&video).await;
    let clip = json_details(
        &server
            .call_ok("add_clip", json!({ "media_id": media_id }))
            .await,
    )["clip_id"]
        .as_u64()
        .unwrap();

    // Filling a vertical canvas with a 4:3 picture: scale = (1920/240) / (1080/320).
    let placed = json_details(
        &server
            .call_ok(
                "set_transform",
                json!({ "clip_id": clip, "fit": "fill_canvas" }),
            )
            .await,
    );
    let scale = placed["scale"].as_f64().unwrap();
    assert!(
        (scale - (1920.0 / 240.0) / (1080.0 / 320.0)).abs() < 1e-3,
        "{scale}"
    );
    assert_eq!(placed["center_x_in_pixels"], 540.0);

    let placed = json_details(
        &server
            .call_ok(
                "set_transform",
                json!({ "clip_id": clip, "center_x_in_pixels": 600, "rotation_in_degrees": 90 }),
            )
            .await,
    );
    assert_eq!(placed["center_x_in_pixels"], 600.0);
    assert_eq!(placed["rotation_in_degrees"], 90.0);

    // Half speed doubles the length.
    let slowed = json_details(
        &server
            .call_ok("set_clip_speed", json!({ "clip_id": clip, "speed": 0.5 }))
            .await,
    );
    assert_eq!(slowed["timeline_end_in_seconds"], 4.0);

    // A fade in, then a volume change keeps the fade's shape.
    server
        .call_ok(
            "add_transition",
            json!({ "clip_id": clip, "kind": "fade_in", "duration_in_seconds": 1 }),
        )
        .await;
    let quieter = json_details(
        &server
            .call_ok("set_clip_volume", json!({ "clip_id": clip, "volume": 0.5 }))
            .await,
    );
    let volume_keyframes = &quieter["animations"][0]["keyframes"];
    assert_eq!(volume_keyframes[0]["value"], 0.0);
    assert_eq!(volume_keyframes[1]["value"], 0.5);
    server
        .call_ok("remove_transitions", json!({ "clip_id": clip }))
        .await;

    // Colour adjustment, changed, then removed.
    let effects = json_details(
        &server
            .call_ok(
                "add_color_adjustment",
                json!({ "clip_id": clip, "saturation": 0 }),
            )
            .await,
    );
    let effect_id = effects[0]["effect_id"].as_u64().unwrap();
    let effects = json_details(
        &server
            .call_ok(
                "add_color_adjustment",
                json!({ "clip_id": clip, "effect_id": effect_id, "exposure_in_stops": 1 }),
            )
            .await,
    );
    assert_eq!(effects.as_array().unwrap().len(), 1);
    assert_eq!(effects[0]["saturation"], 0.0);
    assert_eq!(effects[0]["exposure_in_stops"], 1.0);
    server
        .call_ok(
            "remove_effect",
            json!({ "clip_id": clip, "effect_id": effect_id }),
        )
        .await;

    // Settings and a text edit.
    server
        .call_ok(
            "set_project_settings",
            json!({ "frames_per_second": 25, "background_color": "#202020" }),
        )
        .await;
    let title = json_details(&server.call_ok("add_text", json!({ "text": "Hi" })).await)["clip_id"]
        .as_u64()
        .unwrap();
    let edited = json_details(
        &server
            .call_ok(
                "edit_text",
                json!({ "clip_id": title, "text": "Hello", "alignment": "left" }),
            )
            .await,
    );
    assert_eq!(edited["shows"], "Hello");
    assert_eq!(edited["text_style"]["alignment"], "left");
}

#[tokio::test(flavor = "multi_thread")]
async fn tracks_and_moves_follow_the_requested_layout() {
    let server = TestServer::start().await;
    let video = write_beeping_video(server.allowed_folder.path(), "a.mp4", 320, 240, 2);
    server.create_project(json!({})).await;
    let media_id = server.import(&video).await;
    let music_track = json_details(
        &server
            .call_ok("add_track", json!({ "kind": "audio", "name": "Music" }))
            .await,
    );
    assert_eq!(music_track["name"], "Music");

    let first = json_details(
        &server
            .call_ok("add_clip", json!({ "media_id": media_id }))
            .await,
    )["clip_id"]
        .as_u64()
        .unwrap();
    let second = json_details(
        &server
            .call_ok("add_clip", json!({ "media_id": media_id }))
            .await,
    );
    assert_eq!(
        second["timeline_start_in_seconds"], 2.0,
        "appended at the end of the track"
    );
    let second = second["clip_id"].as_u64().unwrap();

    // A start copied with rounding (1.9999996 s) snaps onto the end of the first clip.
    server
        .call_ok(
            "move_clip",
            json!({ "clip_id": second, "start_time_in_seconds": 5.0 }),
        )
        .await;
    let moved = json_details(
        &server
            .call_ok(
                "move_clip",
                json!({ "clip_id": second, "start_time_in_seconds": 1.9999996 }),
            )
            .await,
    );
    assert_eq!(moved["timeline_start_in_seconds"], 2.0);

    let trimmed = json_details(
        &server
            .call_ok(
                "trim_clip",
                json!({ "clip_id": first, "start_time_in_seconds": 0.5 }),
            )
            .await,
    );
    assert_eq!(trimmed["source_start_in_seconds"], 0.5);
    assert_eq!(trimmed["timeline_end_in_seconds"], 2.0);

    let removed = server
        .call_ok(
            "remove_track",
            json!({ "track_id": music_track["track_id"] }),
        )
        .await;
    assert!(removed.contains("0 clip(s)"), "{removed}");
}

#[tokio::test(flavor = "multi_thread")]
async fn mistakes_are_explained_with_the_valid_choices() {
    let server = TestServer::start().await;
    let no_project = server.call_error("get_timeline", json!({})).await;
    assert!(no_project.contains("open_project"), "{no_project}");

    let video = write_beeping_video(server.allowed_folder.path(), "a.mp4", 320, 240, 2);
    server.create_project(json!({})).await;
    let media_id = server.import(&video).await;
    let clip = json_details(
        &server
            .call_ok("add_clip", json!({ "media_id": media_id }))
            .await,
    )["clip_id"]
        .as_u64()
        .unwrap();

    let missing_clip = server
        .call_error(
            "split_clip",
            json!({ "clip_id": 999, "time_in_seconds": 1 }),
        )
        .await;
    assert!(
        missing_clip.contains("clip 999 does not exist"),
        "{missing_clip}"
    );
    assert!(
        missing_clip.contains(&format!("existing clips: {clip}")),
        "{missing_clip}"
    );

    let missing_media = server
        .call_error("add_clip", json!({ "media_id": 42 }))
        .await;
    assert!(
        missing_media.contains(&format!("existing media: {media_id}")),
        "{missing_media}"
    );

    let outside = server
        .call_error(
            "split_clip",
            json!({ "clip_id": clip, "time_in_seconds": 9 }),
        )
        .await;
    assert!(outside.contains("not strictly inside"), "{outside}");

    let typo = server
        .call_error(
            "split_clip",
            json!({ "clipId": clip, "time_in_seconds": 1 }),
        )
        .await;
    assert!(typo.contains("clipId"), "{typo}");

    let unknown_tool = server.call_error("split_clips", json!({})).await;
    assert!(unknown_tool.contains("split_clip"), "{unknown_tool}");

    let negative = server
        .call_error(
            "move_clip",
            json!({ "clip_id": clip, "start_time_in_seconds": -1 }),
        )
        .await;
    assert!(negative.contains("start_time_in_seconds"), "{negative}");

    // A refused call changes nothing and adds no undo step.
    let info = json_details(&server.call_ok("get_project_info", json!({})).await);
    // The import and the add_clip only.
    assert_eq!(
        info["recent_changes_oldest_first"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_project_can_be_reopened_and_saved_under_another_name() {
    let server = TestServer::start().await;
    let project_file = server
        .create_project(json!({ "canvas": "square_1080x1080" }))
        .await;
    server.call_ok("add_text", json!({ "text": "Kept" })).await;

    let copy = server.allowed_path("copy");
    let saved = server
        .call_ok("save_project", json!({ "file_path": copy }))
        .await;
    assert!(saved.contains("copy.kadr"), "{saved}");
    server
        .call_ok(
            "add_text",
            json!({ "text": "Only in the copy", "start_time_in_seconds": 5 }),
        )
        .await;

    let reopened = server
        .call_ok(
            "open_project",
            json!({ "file_path": project_file.display().to_string() }),
        )
        .await;
    let info = support::json_details(&reopened);
    assert_eq!(info["clip_count"], 1);
    assert_eq!(info["settings"]["orientation"], "square");
}
