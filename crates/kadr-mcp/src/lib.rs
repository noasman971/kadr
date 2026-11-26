//! # kadr-mcp — the local MCP server
//!
//! This crate exposes the tools of `kadr-tools` through the Model Context
//! Protocol, over **stdio**. The user adds Kadr to Claude Desktop or Claude Code,
//! describes the edit in their own conversation, and Claude calls these tools on
//! their computer.
//!
//! ```text
//! Claude Desktop ──stdin/stdout (JSON-RPC)──► kadr-mcp ──► KadrToolbox (kadr-tools)
//! ```
//!
//! - [`KadrMcpServer`]: the MCP server handler (built on the official Rust SDK,
//!   `rmcp`, see decision record 0010). It lists the tools with their schemas
//!   and runs each call on a blocking thread, one call at a time.
//! - The `kadr-mcp` binary (`src/main.rs`) reads the allowed folders from its
//!   arguments and serves on stdin / stdout.
//!
//! ## Rules for this crate
//!
//! - **No Claude API call and no API key**: Kadr never talks to Anthropic. A
//!   regular Claude subscription is enough.
//! - Over stdio, **nothing but the protocol is written to stdout**. Logs go to
//!   stderr (Claude Desktop keeps them in its MCP log files).
//! - File access is limited to the folders the user allowed (`--allowed-folder`).
//!
//! Phase 5 works directly on a project file; phase 7 connects to the running
//! desktop app. See `docs/mcp-server.md`.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use kadr_tools::{KadrToolbox, ToolContent, ToolDefinition, ToolEffect, ToolOutput};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::Value;

/// The name the server gives itself to Claude.
pub const SERVER_NAME: &str = "kadr";

/// What Claude reads about Kadr when it connects: how to use the tools together.
pub const SERVER_INSTRUCTIONS: &str = "\
Kadr is a free video editor running on the user's computer; these tools edit the user's videos \
there. Nothing is uploaded anywhere.

Typical work: get_project_info (shows the allowed folders) → open_project, or create_project \
(ask where to save it) → find_media_files → import_media → add_clip, split_clip, trim_clip, \
remove_clip, add_text, add_transition, set_transform… → look at the result with get_frame or \
get_contact_sheet → export → get_export_status until finished.

To blur a face (or a plate, a screen): get_frame with the clip_id where the face is clearly \
visible → blur_region with the rectangle around it, in the clip's source pixels → \
get_blur_status until finished → look at each range to check with get_frame, and correct with \
blur_region and the blur's effect_id where the face shows.

Videos attached to this conversation are NOT available to Kadr: ask the user for the file name \
or folder and find it with find_media_files. Times are in seconds. Sizes and positions are in \
pixels, origin at the top-left corner. Every tool call is one undo step (undo / redo). Kadr only \
uses files in the folders the user allowed, never replaces a file unless you pass \"overwrite\": \
true after asking the user, and never deletes files.";

/// How often finished background work is added to the project between calls.
pub const BACKGROUND_WORK_INTERVAL: Duration = Duration::from_secs(1);

/// The MCP server: Kadr's tools, served to Claude.
#[derive(Clone)]
pub struct KadrMcpServer {
    /// One call at a time: the tools edit one project with one undo history.
    toolbox: Arc<Mutex<KadrToolbox>>,
    /// The tool list, built once.
    tools: Arc<Vec<Tool>>,
}

impl std::fmt::Debug for KadrMcpServer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KadrMcpServer")
            .field("tool_count", &self.tools.len())
            .finish_non_exhaustive()
    }
}

impl KadrMcpServer {
    /// A server running the tools of `toolbox`.
    pub fn new(toolbox: KadrToolbox) -> Self {
        let tools = KadrToolbox::tool_definitions()
            .into_iter()
            .map(mcp_tool_from_definition)
            .collect();
        Self {
            toolbox: Arc::new(Mutex::new(toolbox)),
            tools: Arc::new(tools),
        }
    }

    /// Adds the background work that finished (tracking analyses) to the
    /// project every [`BACKGROUND_WORK_INTERVAL`], so that the project file is
    /// up to date even if Claude makes no further call. Runs until the
    /// returned task is aborted.
    pub fn apply_finished_background_work_regularly(&self) -> tokio::task::JoinHandle<()> {
        let toolbox = Arc::clone(&self.toolbox);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(BACKGROUND_WORK_INTERVAL);
            loop {
                interval.tick().await;
                let toolbox = Arc::clone(&toolbox);
                let applied = tokio::task::spawn_blocking(move || {
                    toolbox
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .apply_finished_background_work();
                })
                .await;
                if let Err(join_error) = applied {
                    tracing::error!("adding finished background work crashed: {join_error}");
                }
            }
        })
    }

    /// Runs one tool call on a blocking thread (tools decode video, render and
    /// read files) and turns the answer into MCP content.
    async fn run_tool(&self, tool_name: String, arguments: Value) -> CallToolResult {
        let toolbox = Arc::clone(&self.toolbox);
        let name_for_log = tool_name.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let mut toolbox = toolbox.lock().unwrap_or_else(PoisonError::into_inner);
            toolbox.call_tool(&tool_name, arguments)
        })
        .await;
        match outcome {
            Ok(output) => {
                if output.is_error {
                    tracing::info!(tool = %name_for_log, "tool call refused: {}", output.all_text());
                } else {
                    tracing::info!(tool = %name_for_log, "tool call done");
                }
                mcp_result_from_output(output)
            }
            Err(join_error) => {
                tracing::error!(tool = %name_for_log, "tool call crashed: {join_error}");
                CallToolResult::error(vec![ContentBlock::text(format!(
                    "Kadr hit an internal error while running {name_for_log} ({join_error}). The \
                     project file is unchanged unless the tool reported a change. Please report \
                     this bug at https://github.com/noasman971/kadr/issues"
                ))])
            }
        }
    }
}

impl ServerHandler for KadrMcpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new(SERVER_NAME, env!("CARGO_PKG_VERSION"))
                    .with_title("Kadr video editor"),
            )
            .with_instructions(SERVER_INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools.as_ref().clone()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let arguments = request.arguments.map_or(Value::Null, Value::Object);
        Ok(self
            .run_tool(request.name.into_owned(), arguments)
            .await
            .into())
    }
}

/// The MCP description of a Kadr tool, with hints for clients that show or
/// confirm tool calls.
fn mcp_tool_from_definition(definition: ToolDefinition) -> Tool {
    let annotations = ToolAnnotations::with_title(definition.title)
        .read_only(definition.effect == ToolEffect::ReadsOnly)
        // Project changes can be undone, and files are never replaced without
        // an explicit "overwrite": nothing is destructive by default.
        .destructive(false)
        // Kadr only works on this computer.
        .open_world(false);
    Tool::new(
        definition.name,
        definition.description,
        Arc::new(definition.input_schema),
    )
    .with_title(definition.title)
    .with_annotations(annotations)
}

/// The MCP result of a tool answer: text blocks, and images as base64.
fn mcp_result_from_output(output: ToolOutput) -> CallToolResult {
    let content: Vec<ContentBlock> = output
        .content
        .into_iter()
        .map(|piece| match piece {
            ToolContent::Text(text) => ContentBlock::text(text),
            ToolContent::Image { data, mime_type } => {
                ContentBlock::image(BASE64.encode(data), mime_type)
            }
        })
        .collect();
    if output.is_error {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    }
}
