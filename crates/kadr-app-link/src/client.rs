//! `kadr-mcp`'s side: one connection per tool call, blocking (it runs on the
//! tool's own thread).

use std::io::{self, BufRead, BufReader, Write};
use std::time::Duration;

use crate::endpoint::{LinkAddress, link_address};
use crate::protocol::{LinkRequest, LinkResponse};

/// How many times a busy pipe is tried again before giving up.
const BUSY_RETRIES: u32 = 40;
/// Pause between two tries of a busy pipe.
const BUSY_RETRY_PAUSE: Duration = Duration::from_millis(50);
/// The Windows error "all pipe instances are busy".
const ERROR_PIPE_BUSY: i32 = 231;

/// Why a call through the link failed.
#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    /// The pipe broke or could not be opened.
    #[error("the link with the Kadr app failed: {0}")]
    Io(#[from] io::Error),
    /// The app answered something that is not a valid message.
    #[error("the Kadr app answered something unreadable: {0}")]
    Unreadable(#[from] serde_json::Error),
    /// The app closed the connection without answering.
    #[error("the Kadr app closed the link without answering")]
    NoAnswer,
}

/// Sends `request` to the open app and waits for its answer.
///
/// `Ok(None)` when no app is listening (it is closed): the caller works on the
/// project file itself.
///
/// # Errors
///
/// When an app is there but the exchange fails.
pub fn send_to_app(request: &LinkRequest) -> Result<Option<LinkResponse>, LinkError> {
    send_to_app_at(&link_address(), request)
}

/// Same as [`send_to_app`], to the app listening at `address`.
///
/// # Errors
///
/// When an app is there but the exchange fails.
pub fn send_to_app_at(
    address: &LinkAddress,
    request: &LinkRequest,
) -> Result<Option<LinkResponse>, LinkError> {
    let Some(connection) = connect(address)? else {
        return Ok(None);
    };
    exchange(connection, request).map(Some)
}

/// A connection to the app, readable and writable.
trait Connection: io::Read + io::Write {}
impl<Stream: io::Read + io::Write> Connection for Stream {}

fn exchange(
    mut connection: Box<dyn Connection>,
    request: &LinkRequest,
) -> Result<LinkResponse, LinkError> {
    let mut line = serde_json::to_string(request)?;
    line.push('\n');
    connection.write_all(line.as_bytes())?;
    connection.flush()?;
    let mut answer = String::new();
    BufReader::new(connection).read_line(&mut answer)?;
    if answer.trim().is_empty() {
        return Err(LinkError::NoAnswer);
    }
    Ok(serde_json::from_str(answer.trim_end())?)
}

/// Opens the app's pipe; `None` when nothing listens there.
fn connect(address: &LinkAddress) -> Result<Option<Box<dyn Connection>>, LinkError> {
    for _ in 0..BUSY_RETRIES {
        match open(address) {
            Ok(connection) => return Ok(Some(connection)),
            Err(error) if is_absent(&error) => return Ok(None),
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                std::thread::sleep(BUSY_RETRY_PAUSE);
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(LinkError::Io(io::Error::other(
        "the Kadr app's pipe stayed busy",
    )))
}

/// Whether the error means that no app is listening.
fn is_absent(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

fn open(address: &LinkAddress) -> io::Result<Box<dyn Connection>> {
    match address {
        LinkAddress::NamedPipe(name) => Ok(Box::new(
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(name)?,
        )),
        LinkAddress::UnixSocket(path) => open_unix_socket(path),
    }
}

#[cfg(unix)]
fn open_unix_socket(path: &std::path::Path) -> io::Result<Box<dyn Connection>> {
    Ok(Box::new(std::os::unix::net::UnixStream::connect(path)?))
}

#[cfg(not(unix))]
fn open_unix_socket(_path: &std::path::Path) -> io::Result<Box<dyn Connection>> {
    Err(io::Error::from(io::ErrorKind::NotFound))
}
