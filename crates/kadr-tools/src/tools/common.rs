//! Parameter types and checks shared by several tools.

use std::path::{Path, PathBuf};

use kadr_core::color::Color;
use kadr_core::media_asset::PictureSize;
use kadr_core::project::ProjectSettings;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::output::ToolError;
use crate::time_input::frame_rate_from_frames_per_second;

/// Standard picture sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
pub(crate) enum CanvasPreset {
    /// 1080 × 1920 vertical: TikTok, Instagram Reels, YouTube Shorts.
    #[serde(rename = "vertical_1080x1920")]
    Vertical1080x1920,
    /// 1920 × 1080 landscape: YouTube 1080p, most screens.
    #[serde(rename = "landscape_1920x1080")]
    Landscape1920x1080,
    /// 3840 × 2160 landscape: YouTube 4K.
    #[serde(rename = "landscape_3840x2160")]
    Landscape3840x2160,
    /// 1080 × 1080 square.
    #[serde(rename = "square_1080x1080")]
    Square1080x1080,
}

impl CanvasPreset {
    /// The picture size of the preset.
    pub(crate) fn size(self) -> PictureSize {
        let (width_in_pixels, height_in_pixels) = match self {
            Self::Vertical1080x1920 => (1080, 1920),
            Self::Landscape1920x1080 => (1920, 1080),
            Self::Landscape3840x2160 => (3840, 2160),
            Self::Square1080x1080 => (1080, 1080),
        };
        PictureSize {
            width_in_pixels,
            height_in_pixels,
        }
    }
}

/// Requested changes to the project settings; `None` keeps the current value.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SettingsChanges<'request> {
    pub(crate) canvas: Option<CanvasPreset>,
    pub(crate) width_in_pixels: Option<u32>,
    pub(crate) height_in_pixels: Option<u32>,
    pub(crate) frames_per_second: Option<f64>,
    pub(crate) background_color: Option<&'request str>,
}

impl SettingsChanges<'_> {
    /// `current` with the requested changes. Validity (even sizes, ranges) is
    /// checked by the project rules when the settings are applied.
    pub(crate) fn applied_to(
        &self,
        current: ProjectSettings,
    ) -> Result<ProjectSettings, ToolError> {
        let mut settings = current;
        if let Some(canvas) = self.canvas {
            settings.canvas_size = canvas.size();
        }
        match (self.width_in_pixels, self.height_in_pixels) {
            (Some(width_in_pixels), Some(height_in_pixels)) => {
                settings.canvas_size = PictureSize {
                    width_in_pixels,
                    height_in_pixels,
                };
            }
            (None, None) => {}
            _ => {
                return Err(ToolError::new(
                    "give both width_in_pixels and height_in_pixels for a custom size, or a \
                     canvas preset",
                ));
            }
        }
        if let Some(frames_per_second) = self.frames_per_second {
            settings.frame_rate = frame_rate_from_frames_per_second(frames_per_second)?;
        }
        if let Some(color_text) = self.background_color {
            settings.background_color = parse_color("background_color", color_text)?;
        }
        Ok(settings)
    }
}

/// Reads a colour written `#RRGGBB` or `#RRGGBBAA`.
pub(crate) fn parse_color(parameter_name: &str, color_text: &str) -> Result<Color, ToolError> {
    Color::from_hex(color_text.trim())
        .map_err(|error| ToolError::from(error).in_context(parameter_name))
}

/// The path with the extension `extension` added when it has none.
///
/// # Errors
///
/// Fails when the path has another extension.
pub(crate) fn with_extension(path: &Path, extension: &str) -> Result<PathBuf, ToolError> {
    match path.extension().and_then(|existing| existing.to_str()) {
        None => Ok(path.with_extension(extension)),
        Some(existing) if existing.eq_ignore_ascii_case(extension) => Ok(path.to_path_buf()),
        Some(existing) => Err(ToolError::new(format!(
            "\"{}\" ends in .{existing}, but this file must be a .{extension} file",
            path.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preset_and_a_frame_rate_change_the_settings() {
        let changes = SettingsChanges {
            canvas: Some(CanvasPreset::Vertical1080x1920),
            frames_per_second: Some(60.0),
            background_color: Some("#FFFFFF"),
            ..SettingsChanges::default()
        };
        let settings = changes
            .applied_to(ProjectSettings::landscape_1080p())
            .unwrap();
        assert_eq!(settings.canvas_size, CanvasPreset::Vertical1080x1920.size());
        assert_eq!(settings.frame_rate, kadr_core::time::FrameRate::FPS_60);
        assert_eq!(settings.background_color, Color::WHITE);
    }

    #[test]
    fn a_custom_size_needs_both_sides() {
        let changes = SettingsChanges {
            width_in_pixels: Some(1280),
            ..SettingsChanges::default()
        };
        assert!(
            changes
                .applied_to(ProjectSettings::landscape_1080p())
                .is_err()
        );
    }

    #[test]
    fn the_extension_is_added_or_checked() {
        assert_eq!(
            with_extension(Path::new("/videos/edit"), "kadr").unwrap(),
            PathBuf::from("/videos/edit.kadr")
        );
        assert!(with_extension(Path::new("/videos/edit.KADR"), "kadr").is_ok());
        assert!(with_extension(Path::new("/videos/edit.mp4"), "kadr").is_err());
    }
}
