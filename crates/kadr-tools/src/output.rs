//! What a tool call gives back: text, pictures, or an error in plain words.

use std::fmt;

use serde::Serialize;

/// One piece of a tool's answer.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolContent {
    /// Text read by Claude: a sentence, often followed by JSON details.
    Text(String),
    /// A picture Claude can look at.
    Image {
        /// The encoded picture file.
        data: Vec<u8>,
        /// Its media type, for example `"image/jpeg"`.
        mime_type: &'static str,
    },
}

/// The answer to a tool call.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    /// Text and pictures, in order.
    pub content: Vec<ToolContent>,
    /// Whether the call failed. The content then explains why.
    pub is_error: bool,
}

impl ToolOutput {
    /// A successful answer made of one text.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContent::Text(text.into())],
            is_error: false,
        }
    }

    /// A successful answer: a sentence for Claude, then the details as indented JSON.
    pub fn message_with_details(message: impl Into<String>, details: &impl Serialize) -> Self {
        let mut text = message.into();
        match serde_json::to_string_pretty(details) {
            Ok(json_text) => {
                text.push_str("\n\n");
                text.push_str(&json_text);
            }
            Err(error) => text.push_str(&format!("\n\n(details unavailable: {error})")),
        }
        Self::text(text)
    }

    /// Adds a JPEG picture after the text.
    pub fn with_jpeg_image(mut self, jpeg_data: Vec<u8>) -> Self {
        self.content.push(ToolContent::Image {
            data: jpeg_data,
            mime_type: "image/jpeg",
        });
        self
    }

    /// A failed call, explained in `error`.
    pub fn from_error(error: &ToolError) -> Self {
        Self {
            content: vec![ToolContent::Text(error.to_string())],
            is_error: true,
        }
    }

    /// All the text of the answer, joined by blank lines (handy in tests and logs).
    pub fn all_text(&self) -> String {
        let texts: Vec<&str> = self
            .content
            .iter()
            .filter_map(|content| match content {
                ToolContent::Text(text) => Some(text.as_str()),
                ToolContent::Image { .. } => None,
            })
            .collect();
        texts.join("\n\n")
    }
}

/// Why a tool call failed, in words a person and Claude understand, saying what
/// to do instead when possible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolError {
    message: String,
}

impl ToolError {
    /// An error with this message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The same error with some context in front, for example the parameter name.
    pub fn in_context(self, context: impl fmt::Display) -> Self {
        Self::new(format!("{context}: {}", self.message))
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ToolError {}

/// Turns the errors of the other Kadr crates into tool errors. Their messages
/// are already written for people (they name the objects and list the valid ones).
macro_rules! tool_error_from {
    ($($error_type:ty),* $(,)?) => {
        $(
            impl From<$error_type> for ToolError {
                fn from(error: $error_type) -> Self {
                    Self::new(error.to_string())
                }
            }
        )*
    };
}

tool_error_from!(
    kadr_core::error::EditError,
    kadr_core::validation::ValidationError,
    kadr_core::time::TimeError,
    kadr_core::clip::PlaybackSpeedError,
    kadr_core::color::ColorParseError,
    kadr_engine::MediaError,
    kadr_engine::project_files::ProjectFileIoError,
    kadr_engine::still_frames::StillFrameError,
    kadr_engine::silence_removal::SilenceRemovalError,
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_follow_the_message_as_json() {
        let output =
            ToolOutput::message_with_details("Done.", &serde_json::json!({ "clip_id": 4 }));
        assert!(!output.is_error);
        assert!(output.all_text().starts_with("Done.\n\n{"));
        assert!(output.all_text().contains("\"clip_id\": 4"));
    }

    #[test]
    fn errors_keep_their_message_and_can_take_context() {
        let error = ToolError::new("-1 is not a valid time").in_context("start_time_in_seconds");
        let output = ToolOutput::from_error(&error);
        assert!(output.is_error);
        assert_eq!(
            output.all_text(),
            "start_time_in_seconds: -1 is not a valid time"
        );
    }
}
