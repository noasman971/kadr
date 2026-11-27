//! A request through a real pipe (Windows) or socket (elsewhere): the server
//! answers with its handler, a missing app is told apart from a broken one.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use kadr_app_link::server::serve_at;
use kadr_app_link::{
    LinkAddress, LinkRequest, LinkResponse, PROTOCOL_VERSION, client::send_to_app_at,
    tool_output_from,
};
use kadr_tools::ToolOutput;

fn unique_address(name: &str) -> LinkAddress {
    let unique = format!("kadr-app-test-{name}-{}", std::process::id());
    if cfg!(windows) {
        LinkAddress::NamedPipe(format!(r"\\.\pipe\{unique}"))
    } else {
        LinkAddress::UnixSocket(std::env::temp_dir().join(format!("{unique}.sock")))
    }
}

fn request(tool_name: &str, protocol_version: u32) -> LinkRequest {
    LinkRequest {
        protocol_version,
        client: "round-trip test".to_string(),
        tool_name: tool_name.to_string(),
        arguments: serde_json::json!({ "clip_id": 4 }),
        allowed_folders: vec![PathBuf::from("C:/Videos")],
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tool_call_goes_to_the_app_and_back() {
    let address = unique_address("call");
    let server_address = address.clone();
    tokio::spawn(serve_at(
        server_address,
        Arc::new(|request: LinkRequest| {
            let output = ToolOutput::text(format!(
                "ran {} with {}",
                request.tool_name, request.arguments
            ))
            .with_jpeg_image(vec![0xFF, 0xD8, 9]);
            LinkResponse::from_tool_output(&output, Some(PathBuf::from("C:/Videos/edit.kadr")))
        }),
    ));
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Several calls in a row, each on its own connection.
    for _ in 0..3 {
        let calling_address = address.clone();
        let answer = tokio::task::spawn_blocking(move || {
            send_to_app_at(&calling_address, &request("split_clip", PROTOCOL_VERSION))
        })
        .await
        .unwrap()
        .unwrap()
        .expect("an app is listening");
        let LinkResponse::ToolOutput {
            content,
            is_error,
            project_file,
        } = answer
        else {
            panic!("expected a tool output: {answer:?}");
        };
        let output = tool_output_from(&content, is_error);
        assert_eq!(output.all_text(), "ran split_clip with {\"clip_id\":4}");
        assert_eq!(output.content.len(), 2);
        assert_eq!(project_file, Some(PathBuf::from("C:/Videos/edit.kadr")));
    }

    // Another protocol version is refused, saying which.
    let calling_address = address.clone();
    let refused = tokio::task::spawn_blocking(move || {
        send_to_app_at(&calling_address, &request("undo", PROTOCOL_VERSION + 1))
    })
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    assert!(
        matches!(refused, LinkResponse::Refused { .. }),
        "{refused:?}"
    );
}

#[test]
fn no_app_listening_is_not_an_error() {
    let answer = send_to_app_at(
        &unique_address("nobody"),
        &request("undo", PROTOCOL_VERSION),
    );
    assert!(answer.unwrap().is_none());
}
