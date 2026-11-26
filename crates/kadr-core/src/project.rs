//! The project: settings, imported media and the timeline.
//!
//! A [`Project`] can be read freely, but it can only be **changed through
//! commands** (see [`crate::commands`]), applied by an
//! [`EditSession`](crate::history::EditSession). This guarantees that every
//! change can be undone and that the project always stays valid.

use serde::{Deserialize, Serialize};

use crate::clip::Clip;
use crate::color::Color;
use crate::error::EditError;
use crate::ids::{ClipId, IdAllocator, MediaAssetId, TrackId};
use crate::media_asset::{MediaAsset, PictureSize};
use crate::time::FrameRate;
use crate::timeline::{Timeline, Track};

/// The output format of the project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectSettings {
    /// Size of the rendered picture.
    pub canvas_size: PictureSize,
    /// Frames per second of the rendered video.
    pub frame_rate: FrameRate,
    /// Samples per second of the rendered sound.
    pub audio_sample_rate_in_hertz: u32,
    /// Colour shown where no clip covers the canvas.
    pub background_color: Color,
}

impl ProjectSettings {
    /// Landscape Full HD, 1920 × 1080, 30 fps (YouTube and most screens).
    pub const fn landscape_1080p() -> Self {
        Self::with_canvas_size(1920, 1080)
    }

    /// Vertical Full HD, 1080 × 1920, 30 fps (TikTok, Reels, Shorts).
    pub const fn vertical_1080x1920() -> Self {
        Self::with_canvas_size(1080, 1920)
    }

    /// Square, 1080 × 1080, 30 fps.
    pub const fn square_1080() -> Self {
        Self::with_canvas_size(1080, 1080)
    }

    /// 30 fps, 48 kHz, black background, with the given canvas size.
    const fn with_canvas_size(width_in_pixels: u32, height_in_pixels: u32) -> Self {
        Self {
            canvas_size: PictureSize {
                width_in_pixels,
                height_in_pixels,
            },
            frame_rate: FrameRate::FPS_30,
            audio_sample_rate_in_hertz: 48_000,
            background_color: Color::BLACK,
        }
    }
}

/// A video project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    settings: ProjectSettings,
    media_assets: Vec<MediaAsset>,
    timeline: Timeline,
    id_allocator: IdAllocator,
}

impl Project {
    /// An empty project: no media, no track.
    pub fn new(settings: ProjectSettings) -> Self {
        Self {
            settings,
            media_assets: Vec::new(),
            timeline: Timeline::default(),
            id_allocator: IdAllocator::default(),
        }
    }

    /// The output format.
    pub fn settings(&self) -> &ProjectSettings {
        &self.settings
    }

    /// The imported media files, in import order.
    pub fn media_assets(&self) -> &[MediaAsset] {
        &self.media_assets
    }

    /// The imported media file with this identifier.
    pub fn media_asset(&self, media_asset_id: MediaAssetId) -> Option<&MediaAsset> {
        self.media_assets
            .iter()
            .find(|asset| asset.id == media_asset_id)
    }

    /// The timeline.
    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    /// The identifier allocator (to know which number comes next).
    pub fn id_allocator(&self) -> IdAllocator {
        self.id_allocator
    }

    /// The clips showing this media file, in increasing identifier order.
    pub fn clips_using_media_asset(&self, media_asset_id: MediaAssetId) -> Vec<ClipId> {
        let mut clip_ids: Vec<ClipId> = self
            .timeline
            .all_clips()
            .filter(|(_, clip)| clip.media_asset_id() == Some(media_asset_id))
            .map(|(_, clip)| clip.id)
            .collect();
        clip_ids.sort();
        clip_ids
    }

    // ----- Lookups that explain what exists when something is missing -----

    /// The track with this identifier, or an error listing the existing tracks.
    pub(crate) fn require_track(&self, track_id: TrackId) -> Result<&Track, EditError> {
        self.timeline
            .track(track_id)
            .ok_or_else(|| EditError::TrackNotFound {
                track_id,
                existing_track_ids: self.timeline.track_ids(),
            })
    }

    /// The clip with this identifier and its track, or an error listing the existing clips.
    pub(crate) fn require_clip(&self, clip_id: ClipId) -> Result<(&Track, &Clip), EditError> {
        self.timeline
            .find_clip(clip_id)
            .ok_or_else(|| EditError::ClipNotFound {
                clip_id,
                existing_clip_ids: self.timeline.clip_ids(),
            })
    }

    /// The media file with this identifier, or an error listing the existing ones.
    pub(crate) fn require_media_asset(
        &self,
        media_asset_id: MediaAssetId,
    ) -> Result<&MediaAsset, EditError> {
        self.media_asset(media_asset_id)
            .ok_or_else(|| EditError::MediaAssetNotFound {
                media_asset_id,
                existing_media_asset_ids: self.media_assets.iter().map(|asset| asset.id).collect(),
            })
    }

    // ----- Modification, reserved to commands -----

    /// The settings, for modification by commands.
    pub(crate) fn settings_mut(&mut self) -> &mut ProjectSettings {
        &mut self.settings
    }

    /// The media list, for modification by commands.
    pub(crate) fn media_assets_mut(&mut self) -> &mut Vec<MediaAsset> {
        &mut self.media_assets
    }

    /// The timeline, for modification by commands.
    pub(crate) fn timeline_mut(&mut self) -> &mut Timeline {
        &mut self.timeline
    }

    /// The identifier allocator, for commands that create objects.
    pub(crate) fn id_allocator_mut(&mut self) -> &mut IdAllocator {
        &mut self.id_allocator
    }

    /// Puts the identifier allocator back to an earlier state, when undoing.
    pub(crate) fn restore_id_allocator(&mut self, earlier_state: IdAllocator) {
        self.id_allocator = earlier_state;
    }
}
