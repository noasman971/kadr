//! The real `kadr-mcp` program, started as Claude Desktop starts it: a child
//! process speaking MCP on stdin / stdout. If anything else were printed on
//! stdout, the client could not read the protocol and this test would fail.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::TokioChildProcess;
use support::text_of;
use tokio::process::Command;

#[tokio::test(flavor = "multi_thread")]
async fn the_program_speaks_only_mcp_on_stdout() {
    let allowed_folder = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_kadr-mcp"));
    command.arg("--allowed-folder").arg(allowed_folder.path());
    command
        .arg("--allowed-folder")
        .arg(allowed_folder.path().join("typo"));
    let client = ().serve(TokioChildProcess::new(command).unwrap()).await.unwrap();

    let tools = client.list_all_tools().await.unwrap();
    assert!(tools.len() >= 30, "{} tools", tools.len());

    let result = client
        .call_tool(CallToolRequestParams::new("get_project_info"))
        .await
        .unwrap();
    let text = text_of(&result);
    assert!(text.contains("No project is open"), "{text}");
    assert!(
        text.contains("typo"),
        "the unusable folder is reported: {text}"
    );

    client.cancel().await.unwrap();
}
