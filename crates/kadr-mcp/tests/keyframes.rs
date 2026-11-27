//! Animating a title with keyframes and curves through `set_keyframes`, the way
//! the person does it in the app's inspector: canvas coordinates, eased and
//! Bézier curves, one undo step, the project file following, clear refusals.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use kadr_core::animation::Interpolation;
use kadr_core::project_file::load_project_from_json;
use kadr_core::time::TimeSpan;
use serde_json::{Value, json};
use support::{TestServer, json_details};

/// A vertical project with a 3 s title; returns the title's clip id.
async fn project_with_a_title(server: &TestServer) -> (std::path::PathBuf, u64) {
    let project_file = server
        .create_project(json!({ "canvas": "vertical_1080x1920" }))
        .await;
    let title = json_details(
        &server
            .call_ok(
                "add_text",
                json!({ "text": "Hello", "duration_in_seconds": 3 }),
            )
            .await,
    );
    (project_file, title["clip_id"].as_u64().unwrap())
}

fn animation_of<'view>(clip: &'view Value, property: &str) -> &'view Value {
    clip["animations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|animation| animation["property"] == property)
        .unwrap_or_else(|| panic!("no {property} animation in {clip}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_title_slides_and_fades_in_on_eased_keyframes() {
    let server = TestServer::start().await;
    let (project_file, title) = project_with_a_title(&server).await;

    let slid = json_details(
        &server
            .call_ok(
                "set_keyframes",
                json!({ "clip_id": title, "property": "center_x_in_pixels", "keyframes": [
                    { "time_in_clip_in_seconds": 0.8, "value": 540 },
                    { "time_in_clip_in_seconds": 0, "value": -300, "curve": "ease_out" },
                ] }),
            )
            .await,
    );
    // Shown in canvas pixels from the top-left corner, sorted by time.
    let keyframes = &animation_of(&slid, "center_x_in_pixels")["keyframes"];
    assert_eq!(keyframes[0]["value"], -300.0);
    assert_eq!(keyframes[0]["curve"], "ease_out");
    assert_eq!(keyframes[1]["time_in_clip_in_seconds"], 0.8);
    assert_eq!(keyframes[1]["value"], 540.0);
    assert_eq!(slid["transform"]["center_x_in_pixels"], Value::Null);

    let faded = json_details(
        &server
            .call_ok(
                "set_keyframes",
                json!({ "clip_id": title, "property": "opacity", "keyframes": [
                    { "time_in_clip_in_seconds": 0, "value": 0, "curve": "bezier",
                      "bezier_handles": [0.2, 0, 0, 1] },
                    { "time_in_clip_in_seconds": 0.5, "value": 1 },
                ] }),
            )
            .await,
    );
    let keyframes = &animation_of(&faded, "opacity")["keyframes"];
    assert_eq!(keyframes[0]["curve"], "bezier");
    let handles: Vec<f64> = keyframes[0]["bezier_handles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|handle| handle.as_f64().unwrap())
        .collect();
    assert!(
        (handles[0] - 0.2).abs() < 1e-6 && handles[3] == 1.0,
        "{handles:?}"
    );

    // The project file follows: stored from the canvas centre, with the curves.
    let saved = load_project_from_json(&std::fs::read_to_string(&project_file).unwrap()).unwrap();
    let (_, title_clip) = saved
        .timeline()
        .all_clips()
        .find(|(_, clip)| clip.id.raw() == title)
        .unwrap();
    let position_x = &title_clip.transform.position_x_in_pixels;
    assert_eq!(position_x.keyframes()[0].value, -840.0);
    assert_eq!(
        position_x.keyframes()[0].interpolation_to_next,
        Interpolation::EaseOut
    );
    assert_eq!(
        position_x.value_at(TimeSpan::from_milliseconds(400)),
        -210.0
    );
    assert!(matches!(
        title_clip.opacity.keyframes()[0].interpolation_to_next,
        Interpolation::CubicBezier(_)
    ));

    // Each call is one undo step.
    server.call_ok("undo", json!({})).await;
    let clip = json_details(
        &server
            .call_ok("get_clip", json!({ "clip_id": title }))
            .await,
    );
    assert_eq!(clip["opacity"], 1.0);
    assert_eq!(clip["animations"].as_array().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn keyframes_are_moved_by_sending_the_list_again_and_removed_with_an_empty_one() {
    let server = TestServer::start().await;
    let (_, title) = project_with_a_title(&server).await;
    let set_scale =
        |keyframes: Value| json!({ "clip_id": title, "property": "scale", "keyframes": keyframes });
    server
        .call_ok(
            "set_keyframes",
            set_scale(json!([
                { "time_in_clip_in_seconds": 0, "value": 0.5, "curve": "ease_in_out" },
                { "time_in_clip_in_seconds": 1, "value": 1.2 },
            ])),
        )
        .await;
    let moved = json_details(
        &server
            .call_ok(
                "set_keyframes",
                set_scale(json!([
                    { "time_in_clip_in_seconds": 0.5, "value": 0.5, "curve": "hold" },
                    { "time_in_clip_in_seconds": 2, "value": 1.2 },
                ])),
            )
            .await,
    );
    let keyframes = animation_of(&moved, "scale")["keyframes"]
        .as_array()
        .unwrap();
    assert_eq!(keyframes.len(), 2);
    assert_eq!(keyframes[0]["time_in_clip_in_seconds"], 0.5);
    assert_eq!(keyframes[0]["curve"], "hold");

    // An empty list: the scale keeps its last keyframe's value.
    let constant = json_details(&server.call_ok("set_keyframes", set_scale(json!([]))).await);
    let scale = constant["transform"]["scale"].as_f64().unwrap();
    assert!((scale - 1.2).abs() < 1e-6, "{scale}");
    assert!(constant.get("animations").is_none(), "{constant}");
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_keyframes_are_refused_with_clear_messages() {
    let server = TestServer::start().await;
    let (_, title) = project_with_a_title(&server).await;
    let refusal = |property: &str, keyframes: Value| json!({ "clip_id": title, "property": property, "keyframes": keyframes });

    let error = server
        .call_error(
            "set_keyframes",
            refusal(
                "opacity",
                json!([{ "time_in_clip_in_seconds": 4, "value": 1 }]),
            ),
        )
        .await;
    assert!(error.contains("after the end of clip"), "{error}");

    let error = server
        .call_error(
            "set_keyframes",
            refusal(
                "opacity",
                json!([
                    { "time_in_clip_in_seconds": 1, "value": 0 },
                    { "time_in_clip_in_seconds": 1.01, "value": 1 },
                ]),
            ),
        )
        .await;
    assert!(error.contains("same frame"), "{error}");

    let error = server
        .call_error(
            "set_keyframes",
            refusal(
                "opacity",
                json!([{ "time_in_clip_in_seconds": 0, "value": 0, "curve": "bezier",
                         "bezier_handles": [0.2, -0.5, 0.8, 1.5] }]),
            ),
        )
        .await;
    assert!(error.contains("from 0 to 1"), "{error}");

    let error = server
        .call_error(
            "set_keyframes",
            refusal(
                "opacity",
                json!([{ "time_in_clip_in_seconds": 0, "value": 2 }]),
            ),
        )
        .await;
    assert!(
        error.contains("opacity must be from 0 to 1, not 2"),
        "{error}"
    );

    let error = server
        .call_error(
            "set_keyframes",
            refusal(
                "volume",
                json!([{ "time_in_clip_in_seconds": 0, "value": 1 }]),
            ),
        )
        .await;
    assert!(error.contains("has no sound"), "{error}");

    // Nothing changed.
    let clip = json_details(
        &server
            .call_ok("get_clip", json!({ "clip_id": title }))
            .await,
    );
    assert!(clip.get("animations").is_none(), "{clip}");
}
