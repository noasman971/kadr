//! Colour LUTs: Claude finds `.cube` files, applies one to a clip (read before
//! the project changes), changes its strength, sees it on the timeline, and
//! removes it; wrong files and files outside the allowed folders are refused,
//! including LUT paths a project file names.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use support::{TestServer, images_of, json_details, text_of, write_beeping_video};

/// A 2 × 2 × 2 LUT giving every colour pure green.
const ALL_GREEN_CUBE: &str = "TITLE \"All green\"\nLUT_3D_SIZE 2\n\
    0 1 0\n0 1 0\n0 1 0\n0 1 0\n0 1 0\n0 1 0\n0 1 0\n0 1 0\n";

fn write_cube(folder: &Path, name: &str, text: &str) -> PathBuf {
    let path = folder.join(name);
    fs::write(&path, text).unwrap();
    path
}

fn text_path(path: &Path) -> String {
    path.display().to_string()
}

/// A 320 × 240 project with one 2-second clip; returns its clip id.
async fn project_with_one_clip(server: &TestServer) -> u64 {
    let video = write_beeping_video(server.allowed_folder.path(), "clip.mp4", 320, 240, 2);
    server
        .create_project(
            json!({ "width_in_pixels": 320, "height_in_pixels": 240, "frames_per_second": 30 }),
        )
        .await;
    let media_id = server.import(&video).await;
    json_details(
        &server
            .call_ok("add_clip", json!({ "media_id": media_id }))
            .await,
    )["clip_id"]
        .as_u64()
        .unwrap()
}

/// The middle pixel of the timeline's picture at 0.5 s.
async fn middle_of_timeline(server: &TestServer) -> [u8; 3] {
    let result = server
        .call("get_frame", json!({ "time_in_seconds": 0.5 }))
        .await;
    assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
    images_of(&result)[0].to_rgb8().get_pixel(160, 120).0
}

fn only_color_lut(effects: &Value) -> &Value {
    let luts: Vec<_> = effects
        .as_array()
        .unwrap()
        .iter()
        .filter(|effect| effect["type"] == "color_lut")
        .collect();
    assert_eq!(luts.len(), 1, "{effects}");
    luts[0]
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lut_is_found_applied_changed_and_removed() {
    let server = TestServer::start().await;
    let clip = project_with_one_clip(&server).await;
    let green = write_cube(
        server.allowed_folder.path(),
        "All Green.CUBE",
        ALL_GREEN_CUBE,
    );

    // Found only when asked for: LUTs are not media.
    let media = server.call_ok("find_media_files", json!({})).await;
    assert!(!media.contains("All Green"), "{media}");
    let luts = server
        .call_ok("find_media_files", json!({ "kind": "color_lut" }))
        .await;
    assert!(luts.contains("colour LUT file(s)"), "{luts}");
    assert_eq!(json_details(&luts)[0]["kind"], "color_lut");

    let applied = server
        .call_ok(
            "add_color_lut",
            json!({ "clip_id": clip, "lut_file_path": text_path(&green) }),
        )
        .await;
    assert!(applied.contains("\"All green\", 2 × 2 × 2"), "{applied}");
    assert!(applied.contains("get_frame without clip_id"), "{applied}");
    let effects = json_details(&applied);
    let lut = only_color_lut(&effects);
    assert_eq!(lut["strength"], 1.0);
    assert!(
        lut["lut_file_path"]
            .as_str()
            .unwrap()
            .ends_with("All Green.CUBE")
    );
    let effect_id = lut["effect_id"].as_u64().unwrap();

    // The timeline shows the look; the clip's own picture does not.
    let [red, green_channel, blue] = middle_of_timeline(&server).await;
    assert!(
        green_channel > 200 && red < 60 && blue < 60,
        "{:?}",
        [red, green_channel, blue]
    );

    // A second LUT is refused; the strength changes and the file stays.
    let error = server
        .call_error(
            "add_color_lut",
            json!({ "clip_id": clip, "lut_file_path": text_path(&green) }),
        )
        .await;
    assert!(
        error.contains(&format!("already has colour LUT {effect_id}")),
        "{error}"
    );
    let changed = json_details(
        &server
            .call_ok(
                "add_color_lut",
                json!({ "clip_id": clip, "effect_id": effect_id, "strength": 0.5 }),
            )
            .await,
    );
    let lut = only_color_lut(&changed);
    assert_eq!(lut["strength"], 0.5);
    assert!(
        lut["lut_file_path"]
            .as_str()
            .unwrap()
            .ends_with("All Green.CUBE")
    );

    // Removed, then back with undo.
    server
        .call_ok(
            "remove_effect",
            json!({ "clip_id": clip, "effect_id": effect_id }),
        )
        .await;
    let details = json_details(&server.call_ok("get_clip", json!({ "clip_id": clip })).await);
    assert!(
        !details.to_string().contains("color_lut"),
        "removed: {details}"
    );
    server.call_ok("undo", json!({})).await;
    let details = json_details(&server.call_ok("get_clip", json!({ "clip_id": clip })).await);
    assert!(details.to_string().contains("color_lut"), "{details}");
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_luts_are_refused_before_the_project_changes() {
    let server = TestServer::start().await;
    let clip = project_with_one_clip(&server).await;
    let allowed = server.allowed_folder.path();
    let outside = write_cube(server.outside_folder.path(), "secret.cube", ALL_GREEN_CUBE);
    let not_cube = write_cube(allowed, "look.txt", ALL_GREEN_CUBE);
    let broken = write_cube(
        allowed,
        "broken.cube",
        "LUT_3D_SIZE 2\n0 0 0\nnot numbers at all 1\n0 x 1\n",
    );
    let green = write_cube(allowed, "green.cube", ALL_GREEN_CUBE);
    let add = |arguments: Value| server.call_error("add_color_lut", arguments);

    let error = add(json!({ "clip_id": clip, "lut_file_path": text_path(&outside) })).await;
    assert!(error.contains("outside the folders"), "{error}");
    let error = add(json!({ "clip_id": clip, "lut_file_path": "green.cube" })).await;
    assert!(error.contains("not a full path"), "{error}");
    let error = add(json!({ "clip_id": clip, "lut_file_path": text_path(&not_cube) })).await;
    assert!(error.contains("is not a .cube file"), "{error}");
    let error = add(json!({ "clip_id": clip, "lut_file_path": text_path(&broken) })).await;
    assert!(error.contains("line 4"), "{error}");
    assert!(!error.contains("0 x 1"), "lines are never quoted: {error}");
    let error = add(json!({ "clip_id": clip })).await;
    assert!(error.contains("give lut_file_path"), "{error}");
    let error =
        add(json!({ "clip_id": clip, "lut_file_path": text_path(&green), "strength": 1.5 })).await;
    assert!(
        error.contains("the strength must be between 0 and 1, but is 1.5"),
        "{error}"
    );
    let error =
        add(json!({ "clip_id": clip, "lut_file_path": text_path(&green), "intensity": 1 })).await;
    assert!(error.contains("intensity"), "{error}");

    // An effect of another kind is not changed into a LUT.
    let adjustment = json_details(
        &server
            .call_ok(
                "add_color_adjustment",
                json!({ "clip_id": clip, "saturation": 0 }),
            )
            .await,
    )[0]["effect_id"]
        .as_u64()
        .unwrap();
    let error = add(json!({ "clip_id": clip, "effect_id": adjustment, "strength": 0.5 })).await;
    assert!(
        error.contains("is a colour adjustment, not a colour LUT"),
        "{error}"
    );

    // A LUT is applied, not imported.
    let error = server
        .call_error("import_media", json!({ "file_paths": [text_path(&green)] }))
        .await;
    assert!(error.contains("add_color_lut"), "{error}");

    // Nothing above changed the clip's effects.
    let details = json_details(&server.call_ok("get_clip", json!({ "clip_id": clip })).await);
    assert!(!details.to_string().contains("color_lut"), "{details}");
}

#[tokio::test(flavor = "multi_thread")]
async fn lut_paths_in_a_project_file_are_checked_too() {
    let server = TestServer::start().await;
    let clip = project_with_one_clip(&server).await;
    let green = write_cube(server.allowed_folder.path(), "green.cube", ALL_GREEN_CUBE);
    let applied = json_details(
        &server
            .call_ok(
                "add_color_lut",
                json!({ "clip_id": clip, "lut_file_path": text_path(&green) }),
            )
            .await,
    );
    let stored_path = only_color_lut(&applied)["lut_file_path"]
        .as_str()
        .unwrap()
        .to_string();

    // A missing LUT is not an error: the clip is shown without its look.
    let moved = server.allowed_folder.path().join("moved.cube");
    fs::rename(&green, &moved).unwrap();
    let [red, green_channel, blue] = middle_of_timeline(&server).await;
    let is_green = green_channel > 200 && red < 60 && blue < 60;
    assert!(
        !is_green && red.abs_diff(green_channel) < 30 && blue.abs_diff(green_channel) < 30,
        "the black and white barcode, not green: {:?}",
        [red, green_channel, blue]
    );
    fs::rename(&moved, &green).unwrap();

    // Another program points the LUT outside the allowed folders.
    let secret = write_cube(server.outside_folder.path(), "secret.cube", ALL_GREEN_CUBE);
    let project_path = server.allowed_folder.path().join("edit.kadr");
    let project_text = fs::read_to_string(&project_path).unwrap();
    let quoted = |path: &str| serde_json::to_string(path).unwrap();
    assert!(project_text.contains(&quoted(&stored_path)));
    fs::write(
        &project_path,
        project_text.replace(&quoted(&stored_path), &quoted(&text_path(&secret))),
    )
    .unwrap();

    let opened = server
        .call_ok(
            "open_project",
            json!({ "file_path": text_path(&project_path) }),
        )
        .await;
    assert!(
        opened.contains("the colour LUT files of effect"),
        "{opened}"
    );
    let error = server
        .call_error("get_frame", json!({ "time_in_seconds": 0.5 }))
        .await;
    assert!(error.contains("colour LUT"), "{error}");
    assert!(error.contains("outside the folders"), "{error}");
    let error = server
        .call_error("get_contact_sheet", json!({ "count": 4 }))
        .await;
    assert!(error.contains("outside the folders"), "{error}");
    let error = server
        .call_error(
            "export",
            json!({ "output_path": server.allowed_path("out.mp4") }),
        )
        .await;
    assert!(error.contains("outside the folders"), "{error}");

    // A clip's own pictures and the face tracking never read LUTs: still allowed.
    server
        .call_ok(
            "get_frame",
            json!({ "clip_id": clip, "time_in_seconds": 0.5 }),
        )
        .await;
    let blur = server
        .call(
            "blur_region",
            json!({ "clip_id": clip, "time_in_seconds": 0.5, "left_in_pixels": 100,
                    "top_in_pixels": 60, "width_in_pixels": 120, "height_in_pixels": 120 }),
        )
        .await;
    let blur_text = text_of(&blur);
    assert!(
        !blur_text.contains("colour LUT") && !blur_text.contains("outside the folders"),
        "{blur_text}"
    );
    if blur.is_error != Some(true) {
        server
            .call_ok("cancel_blur", json!({ "task_id": "blur-1" }))
            .await;
    }
}
