//! Shared by the integration tests: a Kadr MCP server connected to an MCP
//! client in the same process (as Claude Desktop would talk to it), allowed to
//! use one temporary folder, and small generated test videos.

// Each test file uses a different part of this module.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use kadr_core::time::{FrameRate, TimeSpan};
use kadr_engine::FfmpegBackend;
use kadr_mcp::KadrMcpServer;
use kadr_media::test_patterns::{BeepPattern, TestVideoDescription, write_test_video};
use kadr_tools::{AllowedFolders, KadrToolbox};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock};
use rmcp::service::{RoleClient, RunningService};
use serde_json::Value;
use tempfile::TempDir;

/// Room for the largest message (a base64 picture) in the in-memory pipe.
const PIPE_BUFFER_IN_BYTES: usize = 4 * 1024 * 1024;

/// A server and a connected client.
pub struct TestServer {
    pub client: RunningService<RoleClient, ()>,
    /// The folder the server is allowed to use.
    pub allowed_folder: TempDir,
    /// A folder the server must not touch.
    pub outside_folder: TempDir,
}

impl TestServer {
    /// Starts a server allowed to use a new temporary folder, and connects to it.
    pub async fn start() -> Self {
        let allowed_folder = tempfile::tempdir().unwrap();
        let outside_folder = tempfile::tempdir().unwrap();
        let toolbox = KadrToolbox::new(
            AllowedFolders::new([allowed_folder.path().to_path_buf()]),
            Arc::new(FfmpegBackend::new().unwrap()),
        );
        let (client_end, server_end) = tokio::io::duplex(PIPE_BUFFER_IN_BYTES);
        tokio::spawn(async move {
            let running = KadrMcpServer::new(toolbox).serve(server_end).await.unwrap();
            let _ = running.waiting().await;
        });
        let client = ().serve(client_end).await.unwrap();
        Self {
            client,
            allowed_folder,
            outside_folder,
        }
    }

    /// A path inside the allowed folder, as text.
    pub fn allowed_path(&self, file_name: &str) -> String {
        self.allowed_folder
            .path()
            .join(file_name)
            .display()
            .to_string()
    }

    /// Calls a tool with JSON arguments (an object).
    pub async fn call(&self, tool_name: &str, arguments: Value) -> CallToolResult {
        let Value::Object(arguments) = arguments else {
            panic!("arguments must be a JSON object");
        };
        self.client
            .call_tool(CallToolRequestParams::new(tool_name.to_string()).with_arguments(arguments))
            .await
            .unwrap_or_else(|error| panic!("{tool_name}: protocol error {error}"))
    }

    /// Calls a tool that must succeed, and returns its text.
    pub async fn call_ok(&self, tool_name: &str, arguments: Value) -> String {
        let result = self.call(tool_name, arguments).await;
        let text = text_of(&result);
        assert_ne!(result.is_error, Some(true), "{tool_name} failed: {text}");
        text
    }

    /// Calls a tool that must fail, and returns its error text.
    pub async fn call_error(&self, tool_name: &str, arguments: Value) -> String {
        let result = self.call(tool_name, arguments).await;
        let text = text_of(&result);
        assert_eq!(
            result.is_error,
            Some(true),
            "{tool_name} should have failed: {text}"
        );
        text
    }

    /// Creates a project in the allowed folder and opens it.
    pub async fn create_project(&self, extra_arguments: Value) -> PathBuf {
        let mut arguments = serde_json::json!({ "file_path": self.allowed_path("edit.kadr") });
        if let (Value::Object(arguments), Value::Object(extra)) = (&mut arguments, extra_arguments)
        {
            arguments.extend(extra);
        }
        self.call_ok("create_project", arguments).await;
        self.allowed_folder.path().join("edit.kadr")
    }

    /// Imports a file and returns its media id.
    pub async fn import(&self, file_path: &Path) -> u64 {
        let text = self
            .call_ok(
                "import_media",
                serde_json::json!({ "file_paths": [file_path.display().to_string()] }),
            )
            .await;
        json_details(&text)[0]["media_id"].as_u64().unwrap()
    }
}

/// All the text blocks of a result, joined.
pub fn text_of(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(ContentBlock::as_text)
        .map(|text| text.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The pictures of a result, decoded.
pub fn images_of(result: &CallToolResult) -> Vec<image::DynamicImage> {
    result
        .content
        .iter()
        .filter_map(ContentBlock::as_image)
        .map(|picture| {
            assert_eq!(picture.mime_type, "image/jpeg");
            let bytes = BASE64.decode(&picture.data).unwrap();
            image::load_from_memory(&bytes).unwrap()
        })
        .collect()
}

/// The JSON details that follow the first sentence of a tool's text.
pub fn json_details(text: &str) -> Value {
    let json_text = text
        .split_once("\n\n")
        .map(|(_, json_text)| json_text)
        .unwrap_or_else(|| panic!("no details in: {text}"));
    serde_json::from_str(json_text).unwrap_or_else(|error| panic!("{error} in: {text}"))
}

/// Writes a video of `duration_in_seconds` at `width` × `height`, 30 fps, with a
/// short beep at every whole second (silence in between).
pub fn write_beeping_video(
    folder: &Path,
    file_name: &str,
    width: u32,
    height: u32,
    duration_in_seconds: u64,
) -> PathBuf {
    let output_path = folder.join(file_name);
    let duration = TimeSpan::from_whole_seconds(duration_in_seconds);
    write_test_video(
        &FfmpegBackend::new().unwrap(),
        &TestVideoDescription {
            output_path: output_path.clone(),
            width_in_pixels: width,
            height_in_pixels: height,
            frame_rate: FrameRate::FPS_30,
            duration,
            audio_sample_rate_in_hertz: 48_000,
            sound: BeepPattern::every_second(
                880.0,
                0.5,
                duration,
                TimeSpan::from_milliseconds(200),
            ),
        },
    )
    .unwrap();
    output_path
}
