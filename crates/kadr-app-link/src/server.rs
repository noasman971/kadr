//! The app's side: listens on the pipe and answers each request with a
//! handler run on a blocking thread (tools decode video and render).

use std::io;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use crate::endpoint::{LinkAddress, link_address};
use crate::protocol::{LinkRequest, LinkResponse, PROTOCOL_VERSION};

/// What answers a request.
pub type LinkHandler = Arc<dyn Fn(LinkRequest) -> LinkResponse + Send + Sync>;

/// The longest request accepted, in bytes: a tool call's arguments are small.
const MAXIMUM_REQUEST_LENGTH: u64 = 1024 * 1024;

/// Serves the current user's address until an error stops it (for example,
/// another app already listening there). Runs on a tokio runtime.
///
/// # Errors
///
/// When the pipe cannot be created.
pub async fn serve(handler: LinkHandler) -> io::Result<()> {
    serve_at(link_address(), handler).await
}

/// Same as [`serve`], at `address`.
///
/// # Errors
///
/// When the pipe cannot be created.
pub async fn serve_at(address: LinkAddress, handler: LinkHandler) -> io::Result<()> {
    match address {
        LinkAddress::NamedPipe(name) => serve_named_pipe(&name, handler).await,
        LinkAddress::UnixSocket(path) => serve_unix_socket(&path, handler).await,
    }
}

#[cfg(windows)]
async fn serve_named_pipe(name: &str, handler: LinkHandler) -> io::Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let mut listening = ServerOptions::new()
        .first_pipe_instance(true)
        .reject_remote_clients(true)
        .create(name)?;
    loop {
        listening.connect().await?;
        let connected = listening;
        listening = ServerOptions::new()
            .reject_remote_clients(true)
            .create(name)?;
        tokio::spawn(answer_one_request(connected, Arc::clone(&handler)));
    }
}

#[cfg(not(windows))]
async fn serve_named_pipe(_name: &str, _handler: LinkHandler) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "named pipes only exist on Windows",
    ))
}

#[cfg(unix)]
async fn serve_unix_socket(path: &std::path::Path, handler: LinkHandler) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    // A socket file left by an app that crashed: nobody answers there.
    if path.exists() && std::os::unix::net::UnixStream::connect(path).is_err() {
        std::fs::remove_file(path)?;
    }
    let listener = tokio::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(answer_one_request(stream, Arc::clone(&handler)));
    }
}

#[cfg(not(unix))]
async fn serve_unix_socket(_path: &std::path::Path, _handler: LinkHandler) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Unix sockets are not used on Windows",
    ))
}

/// Reads one request line, answers it, and closes the connection.
async fn answer_one_request<Stream>(stream: Stream, handler: LinkHandler)
where
    Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (reading, mut writing) = tokio::io::split(stream);
    let mut line = String::new();
    let read = BufReader::new(tokio::io::AsyncReadExt::take(
        reading,
        MAXIMUM_REQUEST_LENGTH,
    ))
    .read_line(&mut line)
    .await;
    if read.is_err() || line.trim().is_empty() {
        return;
    }
    let response = match serde_json::from_str::<LinkRequest>(line.trim_end()) {
        Ok(request) if request.protocol_version == PROTOCOL_VERSION => {
            tokio::task::spawn_blocking(move || handler(request))
                .await
                .unwrap_or_else(|join_error| refused(format!("the Kadr app failed: {join_error}")))
        }
        Ok(request) => refused(format!(
            "the Kadr app speaks protocol {PROTOCOL_VERSION}, kadr-mcp {}: update both",
            request.protocol_version
        )),
        Err(error) => refused(format!("the request was unreadable: {error}")),
    };
    let Ok(mut answer) = serde_json::to_string(&response) else {
        return;
    };
    answer.push('\n');
    // The client may have gone: nothing to do then.
    let _ = writing.write_all(answer.as_bytes()).await;
    let _ = writing.flush().await;
    let _ = writing.shutdown().await;
}

fn refused(reason: String) -> LinkResponse {
    LinkResponse::Refused {
        reason,
        protocol_version: PROTOCOL_VERSION,
    }
}
