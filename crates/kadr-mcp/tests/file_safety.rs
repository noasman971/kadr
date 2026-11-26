//! File safety: only the allowed folders, no file replaced without
//! `"overwrite": true`, no file ever deleted, and no silent overwrite of a
//! project changed by another program.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::fs;

use serde_json::json;
use support::{TestServer, json_details, write_beeping_video};

#[tokio::test(flavor = "multi_thread")]
async fn files_outside_the_allowed_folders_are_refused() {
    let server = TestServer::start().await;
    let secret_video = write_beeping_video(server.outside_folder.path(), "secret.mp4", 320, 240, 1);
    server.create_project(json!({})).await;

    for (tool, arguments) in [
        (
            "probe_media",
            json!({ "file_path": secret_video.display().to_string() }),
        ),
        (
            "import_media",
            json!({ "file_paths": [secret_video.display().to_string()] }),
        ),
        (
            "find_media_files",
            json!({ "folder": server.outside_folder.path().display().to_string() }),
        ),
        (
            "open_project",
            json!({ "file_path": server.outside_folder.path().join("x.kadr").display().to_string() }),
        ),
        (
            "create_project",
            json!({ "file_path": server.outside_folder.path().join("x.kadr").display().to_string() }),
        ),
    ] {
        let error = server.call_error(tool, arguments).await;
        assert!(
            error.contains("outside the folders") || error.contains("cannot be opened"),
            "{tool}: {error}"
        );
    }
    let escape = server.allowed_folder.path().join("..").join("escape.kadr");
    let error = server
        .call_error(
            "save_project",
            json!({ "file_path": escape.display().to_string() }),
        )
        .await;
    assert!(error.contains("outside the folders"), "{error}");

    let relative = server
        .call_error("probe_media", json!({ "file_path": "secret.mp4" }))
        .await;
    assert!(relative.contains("not a full path"), "{relative}");
    assert!(secret_video.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn existing_files_are_never_replaced_without_overwrite() {
    let server = TestServer::start().await;
    let precious = server.allowed_folder.path().join("edit.kadr");
    fs::write(&precious, "precious work").unwrap();

    let error = server
        .call_error(
            "create_project",
            json!({ "file_path": precious.display().to_string() }),
        )
        .await;
    assert!(error.contains("\"overwrite\": true"), "{error}");
    assert_eq!(fs::read_to_string(&precious).unwrap(), "precious work");

    let other = server.allowed_folder.path().join("other.kadr");
    server
        .call_ok(
            "create_project",
            json!({ "file_path": other.display().to_string() }),
        )
        .await;
    let error = server
        .call_error(
            "save_project",
            json!({ "file_path": precious.display().to_string() }),
        )
        .await;
    assert!(error.contains("already exists"), "{error}");
    assert_eq!(fs::read_to_string(&precious).unwrap(), "precious work");

    // With the user's agreement, it is replaced.
    server
        .call_ok(
            "create_project",
            json!({ "file_path": precious.display().to_string(), "overwrite": true }),
        )
        .await;
    assert!(
        fs::read_to_string(&precious)
            .unwrap()
            .contains("format_version")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_media_from_the_timeline_never_deletes_files() {
    let server = TestServer::start().await;
    let video = write_beeping_video(server.allowed_folder.path(), "keep.mp4", 320, 240, 1);
    server.create_project(json!({})).await;
    let media_id = server.import(&video).await;
    let added = json_details(
        &server
            .call_ok("add_clip", json!({ "media_id": media_id }))
            .await,
    );
    server
        .call_ok("remove_clip", json!({ "clip_id": added["clip_id"] }))
        .await;
    server
        .call_ok("remove_track", json!({ "track_id": added["track_id"] }))
        .await;
    assert!(video.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_project_changed_by_another_program_is_not_overwritten() {
    let server = TestServer::start().await;
    let project_file = server.create_project(json!({})).await;
    let other_program_version = fs::read_to_string(&project_file)
        .unwrap()
        .replace("1920", "1280");
    fs::write(&project_file, &other_program_version).unwrap();

    let error = server
        .call_error("add_text", json!({ "text": "Lost?" }))
        .await;
    assert!(error.contains("changed by another program"), "{error}");
    assert_eq!(
        fs::read_to_string(&project_file).unwrap(),
        other_program_version
    );

    // Reopening loads the other program's version, and editing goes on.
    let reopened = json_details(
        &server
            .call_ok(
                "open_project",
                json!({ "file_path": project_file.display().to_string() }),
            )
            .await,
    );
    assert_eq!(reopened["settings"]["width_in_pixels"], 1280);
    server.call_ok("add_text", json!({ "text": "Safe" })).await;
}
