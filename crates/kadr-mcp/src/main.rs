//! `kadr-mcp`: the Kadr MCP server, launched by Claude Desktop or Claude Code.
//!
//! ```text
//! kadr-mcp --allowed-folder "C:\Users\sam\Videos" [--allowed-folder <another folder>]
//! ```
//!
//! It speaks the Model Context Protocol on stdin / stdout. **Nothing else is
//! ever written to stdout**: logs go to stderr, where Claude Desktop keeps them
//! (`mcp-server-kadr.log`). Set `KADR_LOG=debug` for more detail.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use kadr_engine::FfmpegBackend;
use kadr_mcp::KadrMcpServer;
use kadr_tools::{AllowedFolders, KadrToolbox};
use rmcp::ServiceExt;
use tracing_subscriber::EnvFilter;

/// Kadr's MCP server: lets Claude edit videos on this computer, with a regular
/// Claude subscription (no API key). Claude Desktop or Claude Code starts it;
/// you do not run it yourself.
#[derive(Debug, Parser)]
#[command(name = "kadr-mcp", version)]
struct ServerArguments {
    /// A folder Kadr may read and write in (videos, projects, exports), with
    /// everything inside it. Repeat the option for several folders. Without any,
    /// Kadr cannot use files.
    #[arg(long = "allowed-folder", value_name = "FOLDER")]
    allowed_folders: Vec<PathBuf>,
}

/// The environment variable choosing how detailed the logs are.
const LOG_LEVEL_VARIABLE: &str = "KADR_LOG";

fn main() -> ExitCode {
    let arguments = ServerArguments::parse();
    start_logging_to_stderr();
    match serve_until_disconnected(arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            tracing::error!("{message}");
            eprintln!("kadr-mcp: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Logs go to stderr only: stdout belongs to the protocol.
fn start_logging_to_stderr() {
    let filter =
        EnvFilter::try_from_env(LOG_LEVEL_VARIABLE).unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(filter)
        .init();
}

fn serve_until_disconnected(arguments: ServerArguments) -> Result<(), String> {
    let allowed_folders = AllowedFolders::new(arguments.allowed_folders);
    for folder in allowed_folders.folders() {
        tracing::info!("allowed folder: {}", folder.display());
    }
    for rejected in allowed_folders.rejected_folders() {
        tracing::warn!("allowed folder left out: {rejected}");
    }
    if allowed_folders.folders().is_empty() {
        tracing::warn!("no allowed folder: add --allowed-folder <FOLDER> to the server arguments");
    }
    let backend =
        FfmpegBackend::new().map_err(|error| format!("FFmpeg could not be started: {error}"))?;
    let server = KadrMcpServer::new(KadrToolbox::new(allowed_folders, Arc::new(backend)));

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("the server could not start: {error}"))?;
    runtime.block_on(async move {
        tracing::info!(
            "Kadr MCP server {} ready on stdio",
            env!("CARGO_PKG_VERSION")
        );
        let running = server
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|error| format!("the MCP connection could not be opened: {error}"))?;
        let quit_reason = running
            .waiting()
            .await
            .map_err(|error| format!("the MCP connection ended abnormally: {error}"))?;
        tracing::info!("MCP connection closed: {quit_reason:?}");
        Ok(())
    })
}
