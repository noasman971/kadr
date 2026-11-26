//! Errors raised while reading or writing media files.

use std::path::PathBuf;

/// Why a media operation failed. Messages name the file and say what to try.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MediaError {
    /// The file could not be opened (missing, locked, or not a media file).
    #[error("could not open \"{}\": {reason}", file_path.display())]
    CannotOpenFile {
        /// The file.
        file_path: PathBuf,
        /// What the decoder said.
        reason: String,
    },

    /// The file opened but contains neither pictures nor sound Kadr can use.
    #[error("\"{}\" contains no usable video or audio", file_path.display())]
    NoUsableStream {
        /// The file.
        file_path: PathBuf,
    },

    /// The file has no picture, but pictures were requested.
    #[error("\"{}\" has no video stream", file_path.display())]
    NoVideoStream {
        /// The file.
        file_path: PathBuf,
    },

    /// The file has no sound, but sound was requested.
    #[error("\"{}\" has no audio stream", file_path.display())]
    NoAudioStream {
        /// The file.
        file_path: PathBuf,
    },

    /// Reading the pictures or the sound of the file failed.
    #[error("could not decode \"{}\": {reason}", file_path.display())]
    DecodingFailed {
        /// The file.
        file_path: PathBuf,
        /// What went wrong.
        reason: String,
    },

    /// No video encoder could be opened.
    #[error("no H.264 encoder could be started (tried: {}); check that FFmpeg was built with libx264", tried_encoder_names.join(", "))]
    NoUsableVideoEncoder {
        /// The encoders tried, in order.
        tried_encoder_names: Vec<String>,
    },

    /// Writing the output file failed.
    #[error("could not write \"{}\": {reason}", file_path.display())]
    EncodingFailed {
        /// The output file.
        file_path: PathBuf,
        /// What went wrong.
        reason: String,
    },

    /// The caller asked for something impossible (for example an empty range).
    #[error("invalid media request: {reason}")]
    InvalidRequest {
        /// What is wrong with the request.
        reason: String,
    },
}
