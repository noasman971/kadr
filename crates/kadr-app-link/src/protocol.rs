//! The messages on the link: one request, one answer, each a single line of JSON.

use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use kadr_tools::{ToolContent, ToolOutput};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Version of the messages below. Both sides refuse another version, and
/// `kadr-mcp` then works on the project file itself.
pub const PROTOCOL_VERSION: u32 = 1;

/// What `kadr-mcp` asks the app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkRequest {
    /// [`PROTOCOL_VERSION`] of the sender.
    pub protocol_version: u32,
    /// Who asks, for the app's logs (for example `"kadr-mcp 0.1.0"`).
    pub client: String,
    /// The tool to run.
    pub tool_name: String,
    /// Its arguments, as Claude gave them.
    pub arguments: Value,
    /// The folders the user allowed `kadr-mcp` to use.
    pub allowed_folders: Vec<PathBuf>,
}

/// What the app answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LinkResponse {
    /// The tool ran (successfully or not: see `is_error`).
    ToolOutput {
        /// Text and pictures, in order.
        content: Vec<LinkContent>,
        /// Whether the call failed; the content says why.
        is_error: bool,
        /// The project open in the app after the call, if any.
        project_file: Option<PathBuf>,
    },
    /// The app could not take the request (another protocol version).
    Refused {
        /// Why, in English.
        reason: String,
        /// The app's [`PROTOCOL_VERSION`].
        protocol_version: u32,
    },
}

/// A piece of a tool's answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LinkContent {
    /// Text for Claude.
    Text {
        /// The text.
        text: String,
    },
    /// A picture.
    Image {
        /// The encoded file, in base64.
        data_base64: String,
        /// Its media type, for example `"image/jpeg"`.
        mime_type: String,
    },
}

impl LinkResponse {
    /// The answer carrying `output`.
    pub fn from_tool_output(output: &ToolOutput, project_file: Option<PathBuf>) -> Self {
        Self::ToolOutput {
            content: output
                .content
                .iter()
                .map(|piece| match piece {
                    ToolContent::Text(text) => LinkContent::Text { text: text.clone() },
                    ToolContent::Image { data, mime_type } => LinkContent::Image {
                        data_base64: BASE64.encode(data),
                        mime_type: (*mime_type).to_string(),
                    },
                })
                .collect(),
            is_error: output.is_error,
            project_file,
        }
    }
}

/// Turns the pieces of an answer back into a tool output. Pictures are always
/// JPEG files today; a picture that is not valid base64 becomes a sentence.
pub fn tool_output_from(content: &[LinkContent], is_error: bool) -> ToolOutput {
    let content = content
        .iter()
        .map(|piece| match piece {
            LinkContent::Text { text } => ToolContent::Text(text.clone()),
            LinkContent::Image { data_base64, .. } => match BASE64.decode(data_base64) {
                Ok(data) => ToolContent::Image {
                    data,
                    mime_type: "image/jpeg",
                },
                Err(error) => ToolContent::Text(format!(
                    "(a picture from the Kadr app was unreadable: {error})"
                )),
            },
        })
        .collect();
    ToolOutput { content, is_error }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_go_both_ways_with_their_pictures() {
        let output = ToolOutput::text("A frame.").with_jpeg_image(vec![0xFF, 0xD8, 1, 2, 3]);
        let response = LinkResponse::from_tool_output(&output, Some(PathBuf::from("C:/a.kadr")));
        let line = serde_json::to_string(&response).unwrap();
        assert!(!line.contains('\n'));
        let LinkResponse::ToolOutput {
            content, is_error, ..
        } = serde_json::from_str(&line).unwrap()
        else {
            panic!("expected a tool output");
        };
        assert_eq!(tool_output_from(&content, is_error), output);
    }

    #[test]
    fn requests_read_back() {
        let request = LinkRequest {
            protocol_version: PROTOCOL_VERSION,
            client: "kadr-mcp test".to_string(),
            tool_name: "split_clip".to_string(),
            arguments: serde_json::json!({ "clip_id": 4, "time_in_seconds": 2.5 }),
            allowed_folders: vec![PathBuf::from("C:/Videos")],
        };
        let line = serde_json::to_string(&request).unwrap();
        assert_eq!(serde_json::from_str::<LinkRequest>(&line).unwrap(), request);
    }
}
