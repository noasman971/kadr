//! Media assets: the video, audio and image files imported into a project.
//!
//! An asset only *describes* a file (where it is, how long it lasts, its picture
//! size, its frame rate, its audio format). Reading the file is the job of
//! `kadr-media`, which fills these descriptions in when a file is imported.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ids::MediaAssetId;
use crate::time::{FrameRate, TimeSpan};

/// What kind of file an asset is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    /// Moving pictures, usually with sound.
    Video,
    /// Sound only (music, voice-over, sound effect).
    Audio,
    /// A still picture, which can be shown for any duration.
    Image,
}

/// Size of the picture of a video or image file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PictureSize {
    /// Width in pixels.
    pub width_in_pixels: u32,
    /// Height in pixels.
    pub height_in_pixels: u32,
}

/// Format of the sound of a video or audio file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AudioFormat {
    /// Samples per second, for example 48 000.
    pub sample_rate_in_hertz: u32,
    /// Number of channels: 1 for mono, 2 for stereo.
    pub channel_count: u16,
}

/// Everything known about a media file, without its identifier.
///
/// This is what gets imported; the project then gives it a [`MediaAssetId`]
/// and stores it as a [`MediaAsset`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaAssetDescription {
    /// Where the file is on disk.
    pub file_path: PathBuf,
    /// Name shown in the media library, usually the file name.
    pub display_name: String,
    /// Video, audio or image.
    pub kind: MediaKind,
    /// How long the file lasts. `None` for still images.
    pub duration: Option<TimeSpan>,
    /// Picture size. `None` for audio files.
    pub picture_size: Option<PictureSize>,
    /// Frame rate. `None` for audio files and images.
    pub frame_rate: Option<FrameRate>,
    /// Sound format. `None` for images and silent videos.
    pub audio_format: Option<AudioFormat>,
}

impl MediaAssetDescription {
    /// Whether the file contains pictures (a video or an image).
    pub fn has_picture(&self) -> bool {
        self.picture_size.is_some()
    }

    /// Whether the file contains sound.
    pub fn has_audio(&self) -> bool {
        self.audio_format.is_some()
    }
}

/// A media file imported into the project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaAsset {
    /// Identifier inside the project.
    pub id: MediaAssetId,
    /// What is known about the file.
    #[serde(flatten)]
    pub description: MediaAssetDescription,
}

#[cfg(test)]
pub(crate) mod test_samples {
    //! Ready-made asset descriptions for the tests of every module.

    use super::*;

    /// A 10-second 1080p 30 fps video with stereo sound.
    pub(crate) fn ten_second_video() -> MediaAssetDescription {
        MediaAssetDescription {
            file_path: PathBuf::from("videos/interview.mp4"),
            display_name: "interview.mp4".to_string(),
            kind: MediaKind::Video,
            duration: Some(TimeSpan::from_whole_seconds(10)),
            picture_size: Some(PictureSize {
                width_in_pixels: 1920,
                height_in_pixels: 1080,
            }),
            frame_rate: Some(FrameRate::FPS_30),
            audio_format: Some(AudioFormat {
                sample_rate_in_hertz: 48_000,
                channel_count: 2,
            }),
        }
    }

    /// A 60-second stereo music file.
    pub(crate) fn one_minute_music() -> MediaAssetDescription {
        MediaAssetDescription {
            file_path: PathBuf::from("music/background.mp3"),
            display_name: "background.mp3".to_string(),
            kind: MediaKind::Audio,
            duration: Some(TimeSpan::from_whole_seconds(60)),
            picture_size: None,
            frame_rate: None,
            audio_format: Some(AudioFormat {
                sample_rate_in_hertz: 44_100,
                channel_count: 2,
            }),
        }
    }

    /// A still picture.
    pub(crate) fn logo_image() -> MediaAssetDescription {
        MediaAssetDescription {
            file_path: PathBuf::from("images/logo.png"),
            display_name: "logo.png".to_string(),
            kind: MediaKind::Image,
            duration: None,
            picture_size: Some(PictureSize {
                width_in_pixels: 512,
                height_in_pixels: 512,
            }),
            frame_rate: None,
            audio_format: None,
        }
    }
}
