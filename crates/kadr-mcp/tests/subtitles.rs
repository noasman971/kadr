//! Subtitles through the MCP server: Claude imports an `.srt` file, lists and
//! fixes cues in one undo step, restyles them, sees them on a frame, exports
//! the corrected file, and undoes; broken files and wrong cues are refused
//! before anything changes.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::fs;

use serde_json::{Value, json};
use support::{TestServer, images_of, json_details, text_of};

const INTERVIEW_SRT: &str = "\u{FEFF}1\r\n00:00:00,500 --> 00:00:01,500\r\nHello evryone\r\n\r\n\
2\r\n00:00:01,500 --> 00:00:03,000\r\n<i>Welcome</i> to Kadr,\r\nthe video editor\r\n\r\n\
5\r\n00:00:03.200 --> 00:00:04.000\r\nThanks\r\n";

/// A black vertical 360 × 640 project at 30 fps.
async fn vertical_project(server: &TestServer) {
    server
        .create_project(
            json!({ "width_in_pixels": 360, "height_in_pixels": 640, "frames_per_second": 30 }),
        )
        .await;
}

fn cues_of(listed: &Value) -> Vec<Value> {
    listed[0]["cues"].as_array().unwrap().clone()
}

/// How many bright pixels the timeline's picture has at `time`, below and above
/// the middle of the canvas.
async fn bright_pixels_at(server: &TestServer, time: f64) -> (usize, usize) {
    let result = server
        .call("get_frame", json!({ "time_in_seconds": time }))
        .await;
    assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
    let picture = images_of(&result)[0].to_rgb8();
    let middle = picture.height() / 2;
    let (mut below, mut above) = (0, 0);
    for (_, row, pixel) in picture.enumerate_pixels() {
        if pixel.0.iter().all(|&channel| channel > 200) {
            if row > middle {
                below += 1;
            } else {
                above += 1;
            }
        }
    }
    (below, above)
}

#[tokio::test(flavor = "multi_thread")]
async fn subtitles_are_imported_fixed_restyled_and_exported() {
    let server = TestServer::start().await;
    vertical_project(&server).await;
    let srt_path = server.allowed_folder.path().join("interview.srt");
    fs::write(&srt_path, INTERVIEW_SRT).unwrap();

    // Found only when asked for: subtitles are not media.
    let media = server.call_ok("find_media_files", json!({})).await;
    assert!(!media.contains("interview.srt"), "{media}");
    let found = server
        .call_ok("find_media_files", json!({ "kind": "subtitles" }))
        .await;
    assert!(found.contains("Found 1 subtitle file(s)"), "{found}");
    let error = server
        .call_error(
            "import_media",
            json!({ "file_paths": [srt_path.display().to_string()] }),
        )
        .await;
    assert!(error.contains("import_subtitles"), "{error}");

    let imported = server
        .call_ok(
            "import_subtitles",
            json!({ "file_path": srt_path.display().to_string() }),
        )
        .await;
    assert!(
        imported.contains("Imported 3 subtitle cue(s)"),
        "{imported}"
    );
    let details = json_details(&imported);
    let track_id = details["track_id"].as_u64().unwrap();
    assert_eq!(details["style"]["backdrop"], "outline");
    assert_eq!(
        details["style"]["margin_in_pixels"], 160.0,
        "a quarter of 640"
    );

    // The timeline summarises the track; list_subtitles shows the cues.
    let timeline = json_details(&server.call_ok("get_timeline", json!({})).await);
    let track = &timeline["tracks"][0];
    assert_eq!(track["kind"], "subtitle");
    assert_eq!(track["subtitles"]["cue_count"], 3);
    let listed = json_details(&server.call_ok("list_subtitles", json!({})).await);
    let cues = cues_of(&listed);
    assert_eq!(cues[1]["text"], "<i>Welcome</i> to Kadr,\nthe video editor");
    assert_eq!(cues[2]["start_time_in_seconds"], 3.2);
    let ids: Vec<u64> = cues
        .iter()
        .map(|cue| cue["clip_id"].as_u64().unwrap())
        .collect();

    // Burned into the timeline picture, in the bottom half.
    let (below, above) = bright_pixels_at(&server, 1.0).await;
    assert!(below > 50 && above == 0, "below {below}, above {above}");

    // One call, one undo step: a typo, a timing, a split, a merge and a new cue.
    let edited = server
        .call_ok(
            "edit_subtitles",
            json!({ "changes": [
                { "action": "edit", "clip_id": ids[0], "text": "Hello everyone" },
                { "action": "edit", "clip_id": ids[2], "start_time_in_seconds": 3.0 },
                { "action": "split", "clip_id": ids[1], "time_in_seconds": 2.2 },
                { "action": "add", "start_time_in_seconds": 5.0, "end_time_in_seconds": 6.0, "text": "Bye" }
            ] }),
        )
        .await;
    assert!(edited.contains("Applied 4 change(s)"), "{edited}");
    let listed = json_details(
        &server
            .call_ok("list_subtitles", json!({ "track_id": track_id }))
            .await,
    );
    let texts: Vec<String> = cues_of(&listed)
        .iter()
        .map(|cue| cue["text"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        texts,
        [
            "Hello everyone",
            "<i>Welcome</i> to Kadr,",
            "the video editor",
            "Thanks",
            "Bye"
        ]
    );
    assert_eq!(cues_of(&listed)[3]["start_time_in_seconds"], 3.0);
    let narrowed = json_details(
        &server
            .call_ok("list_subtitles", json!({ "text_contains": "THANKS" }))
            .await,
    );
    assert_eq!(cues_of(&narrowed).len(), 1);

    // A box at the top.
    let styled = server
        .call_ok(
            "set_subtitle_style",
            json!({ "style": { "backdrop": "box", "position": "top", "margin_in_pixels": 40 } }),
        )
        .await;
    assert_eq!(json_details(&styled)["backdrop_color"], "#000000B3");
    let (below, above) = bright_pixels_at(&server, 1.0).await;
    assert!(above > 50 && below == 0, "below {below}, above {above}");

    // The corrected file, then nothing replaced without overwrite.
    let corrected_path = server.allowed_path("interview.corrected.srt");
    let exported = server
        .call_ok("export_subtitles", json!({ "file_path": corrected_path }))
        .await;
    assert!(exported.contains("Wrote 5 cue(s)"), "{exported}");
    let written =
        fs::read_to_string(server.allowed_folder.path().join("interview.corrected.srt")).unwrap();
    assert!(
        written.starts_with("1\r\n00:00:00,500 --> 00:00:01,500\r\nHello everyone\r\n"),
        "{written}"
    );
    assert!(written.contains("4\r\n00:00:03,000 --> 00:00:04,000\r\nThanks\r\n"));
    let error = server
        .call_error("export_subtitles", json!({ "file_path": corrected_path }))
        .await;
    assert!(error.contains("already exists"), "{error}");

    // Undo takes back the style, then the whole edit at once.
    server.call_ok("undo", json!({})).await;
    server.call_ok("undo", json!({})).await;
    let listed = json_details(&server.call_ok("list_subtitles", json!({})).await);
    assert_eq!(cues_of(&listed).len(), 3);
    assert_eq!(cues_of(&listed)[0]["text"], "Hello evryone");
    assert_eq!(listed[0]["style"]["backdrop"], "outline");
}

#[tokio::test(flavor = "multi_thread")]
async fn broken_files_and_wrong_changes_change_nothing() {
    let server = TestServer::start().await;
    vertical_project(&server).await;
    let broken_path = server.allowed_folder.path().join("broken.srt");
    fs::write(&broken_path, "1\n00:00:01,000 -> 00:00:02,000\nText\n").unwrap();
    let error = server
        .call_error(
            "import_subtitles",
            json!({ "file_path": broken_path.display().to_string() }),
        )
        .await;
    assert!(error.contains("line 2"), "{error}");
    let not_srt = server.allowed_folder.path().join("notes.txt");
    fs::write(&not_srt, "hello").unwrap();
    let error = server
        .call_error(
            "import_subtitles",
            json!({ "file_path": not_srt.display().to_string() }),
        )
        .await;
    assert!(error.contains("not an .srt file"), "{error}");
    let error = server
        .call_error("list_subtitles", json!({ "track_id": 99 }))
        .await;
    assert!(error.contains("does not exist"), "{error}");

    // A cue added without a track creates one; an overlapping one is refused
    // with the whole call, so the first change does not stay either.
    server
        .call_ok(
            "edit_subtitles",
            json!({ "changes": [{ "action": "add", "start_time_in_seconds": 1, "end_time_in_seconds": 2, "text": "One" }] }),
        )
        .await;
    let error = server
        .call_error(
            "edit_subtitles",
            json!({ "changes": [
                { "action": "add", "start_time_in_seconds": 3, "end_time_in_seconds": 4, "text": "Two" },
                { "action": "add", "start_time_in_seconds": 1.5, "end_time_in_seconds": 2.5, "text": "Overlap" }
            ] }),
        )
        .await;
    assert!(error.contains("change 2"), "{error}");
    assert!(error.contains("overlap"), "{error}");
    let listed = json_details(&server.call_ok("list_subtitles", json!({})).await);
    assert_eq!(cues_of(&listed).len(), 1);
    let error = server
        .call_error(
            "edit_subtitles",
            json!({ "changes": [{ "action": "rename", "clip_id": 1 }] }),
        )
        .await;
    assert!(error.contains("rename"), "{error}");
}
