//! Calls going to the open app, then to the project file when the app closes
//! (decision record 0030). The app is played by a fake listening on a pipe of
//! its own, so a real Kadr open on this computer is never reached.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;
use std::time::Duration;

use kadr_app_link::server::serve_at;
use kadr_app_link::{LinkAddress, LinkRequest, LinkResponse};
use kadr_engine::FfmpegBackend;
use kadr_mcp::KadrMcpServer;
use kadr_tools::{AllowedFolders, KadrToolbox, ToolOutput};
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use serde_json::json;
use support::text_of;

fn unique_address() -> LinkAddress {
    let unique = format!("kadr-app-test-relay-{}", std::process::id());
    if cfg!(windows) {
        LinkAddress::NamedPipe(format!(r"\\.\pipe\{unique}"))
    } else {
        LinkAddress::UnixSocket(std::env::temp_dir().join(format!("{unique}.sock")))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn calls_go_to_the_open_app_then_to_the_file_when_it_closes() {
    let allowed_folder = tempfile::tempdir().unwrap();
    let project_file = allowed_folder.path().join("edit.kadr");

    // A project file the "app" has open.
    let mut file_toolbox = KadrToolbox::new(
        AllowedFolders::new([allowed_folder.path().to_path_buf()]),
        Arc::new(FfmpegBackend::new().unwrap()),
    );
    let created = file_toolbox.call_tool("create_project", json!({ "file_path": project_file }));
    assert!(!created.is_error, "{}", created.all_text());

    // The fake app answers every call itself and reports that project.
    let address = unique_address();
    let reported_file = project_file.clone();
    let app = tokio::spawn(serve_at(
        address.clone(),
        Arc::new(move |request: LinkRequest| {
            let answer = ToolOutput::text(format!(
                "the app ran {} with {} allowed folder(s)",
                request.tool_name,
                request.allowed_folders.len()
            ));
            LinkResponse::from_tool_output(&answer, Some(reported_file.clone()))
        }),
    ));
    tokio::time::sleep(Duration::from_millis(200)).await;

    let toolbox = KadrToolbox::new(
        AllowedFolders::new([allowed_folder.path().to_path_buf()]),
        Arc::new(FfmpegBackend::new().unwrap()),
    );
    let (client_end, server_end) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        let running = KadrMcpServer::with_app_link(toolbox, address)
            .serve(server_end)
            .await
            .unwrap();
        let _ = running.waiting().await;
    });
    let client = ().serve(client_end).await.unwrap();
    let call = |name: &'static str| {
        let client = &client;
        async move {
            text_of(
                &client
                    .call_tool(CallToolRequestParams::new(name))
                    .await
                    .unwrap(),
            )
        }
    };

    // The app is open: the call runs there, and the first answer says so.
    let first = call("get_timeline").await;
    assert!(first.contains("the Kadr app is open"), "{first}");
    assert!(
        first.contains("the app ran get_timeline with 1 allowed folder"),
        "{first}"
    );
    let second = call("get_timeline").await;
    assert!(!second.contains("Note:"), "{second}");

    // The app closes: calls run on the file, and its project is reopened there.
    app.abort();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let after = call("get_project_info").await;
    assert!(after.contains("the Kadr app was closed"), "{after}");
    assert!(after.contains("was reopened from its file"), "{after}");
    assert!(after.contains("edit.kadr"), "{after}");
}
