//! Tools that let Claude see the video: single frames and contact sheets come
//! back as JPEG pictures of at most 1024 pixels, with the original size stated.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use serde_json::json;
use support::{TestServer, images_of, json_details, text_of, write_beeping_video};

#[tokio::test(flavor = "multi_thread")]
async fn a_large_frame_is_scaled_to_1024_pixels_with_its_original_size_stated() {
    let server = TestServer::start().await;
    let video = write_beeping_video(server.allowed_folder.path(), "wide.mp4", 1280, 720, 1);
    server.create_project(json!({})).await;
    let media_id = server.import(&video).await;

    let result = server
        .call(
            "get_frame",
            json!({ "media_id": media_id, "time_in_seconds": 0.5 }),
        )
        .await;
    let text = text_of(&result);
    assert_ne!(result.is_error, Some(true), "{text}");
    let pictures = images_of(&result);
    assert_eq!(pictures.len(), 1);
    assert_eq!((pictures[0].width(), pictures[0].height()), (1024, 576));
    assert!(text.contains("1280×720"), "{text}");
    assert!(
        text.contains("1.2500"),
        "the conversion factor is given: {text}"
    );
    assert!(text.contains("top-left"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn frames_of_a_clip_and_of_the_timeline_show_what_they_should() {
    let server = TestServer::start().await;
    let video = write_beeping_video(server.allowed_folder.path(), "clip.mp4", 320, 240, 2);
    server
        .create_project(
            json!({ "width_in_pixels": 640, "height_in_pixels": 360, "frames_per_second": 30 }),
        )
        .await;
    let media_id = server.import(&video).await;
    let clip = json_details(
        &server
            .call_ok(
                "add_clip",
                json!({ "media_id": media_id, "start_time_in_seconds": 1 }),
            )
            .await,
    )["clip_id"]
        .as_u64()
        .unwrap();

    // The clip's source picture keeps the source size; its time is on the timeline.
    let result = server
        .call(
            "get_frame",
            json!({ "clip_id": clip, "time_in_seconds": 1.5 }),
        )
        .await;
    let text = text_of(&result);
    assert_eq!(
        (
            images_of(&result)[0].width(),
            images_of(&result)[0].height()
        ),
        (320, 240),
        "{text}"
    );
    assert!(text.contains("0.5 s into media"), "{text}");

    // The timeline picture is the canvas; before the clip it is the black background.
    let before = server
        .call("get_frame", json!({ "time_in_seconds": 0.2 }))
        .await;
    let before_picture = images_of(&before)[0].to_rgb8();
    assert_eq!(before_picture.dimensions(), (640, 360));
    assert!(
        before_picture
            .pixels()
            .all(|pixel| pixel.0.iter().all(|channel| *channel < 16))
    );

    // During the clip, the 4:3 picture is centred with bars on the sides.
    let during = images_of(
        &server
            .call("get_frame", json!({ "time_in_seconds": 1.5 }))
            .await,
    )[0]
    .to_rgb8();
    let left_bar = during.get_pixel(20, 180).0;
    assert!(left_bar.iter().all(|channel| *channel < 16), "{left_bar:?}");

    let error = server
        .call_error("get_frame", json!({ "time_in_seconds": 30 }))
        .await;
    assert!(error.contains("the timeline lasts 3 s"), "{error}");
    let error = server
        .call_error(
            "get_frame",
            json!({ "clip_id": clip, "time_in_seconds": 0.5 }),
        )
        .await;
    assert!(error.contains("is not inside clip"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_contact_sheet_shows_labelled_thumbnails_across_the_video() {
    let server = TestServer::start().await;
    let video = write_beeping_video(server.allowed_folder.path(), "sheet.mp4", 640, 360, 4);
    server.create_project(json!({})).await;
    let media_id = server.import(&video).await;

    let result = server
        .call(
            "get_contact_sheet",
            json!({ "media_id": media_id, "count": 8 }),
        )
        .await;
    let text = text_of(&result);
    assert_ne!(result.is_error, Some(true), "{text}");
    let details = json_details(&text);
    let times = details["thumbnail_times_in_seconds"].as_array().unwrap();
    assert_eq!(times.len(), 8);
    assert_eq!(times[0], 0.25);
    assert_eq!(details["original_width_in_pixels"], 640);
    let sheet = &images_of(&result)[0];
    assert!(sheet.width() <= 1024 && sheet.height() <= 1024);
    assert!(details["columns"].as_u64().unwrap() * details["rows"].as_u64().unwrap() >= 8);

    // Across the finished timeline too.
    server
        .call_ok("add_clip", json!({ "media_id": media_id }))
        .await;
    let timeline_sheet = server
        .call("get_contact_sheet", json!({ "count": 4 }))
        .await;
    assert_eq!(images_of(&timeline_sheet).len(), 1);

    let error = server
        .call_error(
            "get_contact_sheet",
            json!({ "media_id": media_id, "count": 100 }),
        )
        .await;
    assert!(error.contains("from 2 to 48"), "{error}");
}
