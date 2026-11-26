//! The **project view**: what the front end draws, rebuilt after every change.
//!
//! The front end never reads the `.kadr` format: it reads this description,
//! made for drawing. Values the interface would otherwise have to compute (a
//! clip's length after a speed change, whether a file is missing, which media
//! are used) are computed here, once, by the engine's own rules.
//!
//! Times are integer ticks (705 600 000 per second, see `kadr_core::time`).
//! Tracks come in **display order**: video tracks from the top layer down, then
//! audio tracks.

use std::path::Path;

use kadr_core::animation::AnimatedValue;
use kadr_core::clip::{Clip, ClipContent, TextAlignment};
use kadr_core::effect::EffectKind;
use kadr_core::ids::{ClipId, EffectId, MediaAssetId, TrackId};
use kadr_core::media_asset::{MediaAsset, MediaKind};
use kadr_core::project::Project;
use kadr_core::timeline::{Track, TrackKind};
use serde::Serialize;

/// Everything the interface shows about the open project.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    /// Goes up by one after every change, so the interface can drop stale answers.
    pub revision: u64,
    /// The project's name: its file name without `.kadr`.
    pub name: String,
    /// The `.kadr` file.
    pub file_path: String,
    /// Why the last automatic save failed, if it did.
    pub save_failure: Option<String>,
    /// Output format.
    pub settings: SettingsView,
    /// Whether the format will follow the first video placed on the timeline.
    pub adopts_first_video_format: bool,
    /// Length of the timeline, in ticks.
    pub duration_ticks: u64,
    /// Imported files, in import order.
    pub media: Vec<MediaView>,
    /// Tracks in display order.
    pub tracks: Vec<TrackView>,
    /// The label of the step Ctrl+Z would undo.
    pub undo_label: Option<String>,
    /// The label of the step Ctrl+Y would redo.
    pub redo_label: Option<String>,
}

/// The output format.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    /// Canvas width in pixels.
    pub canvas_width: u32,
    /// Canvas height in pixels.
    pub canvas_height: u32,
    /// Frames per second, as a fraction: numerator.
    pub frame_rate_numerator: u32,
    /// Frames per second, as a fraction: denominator.
    pub frame_rate_denominator: u32,
    /// Sound samples per second.
    pub audio_sample_rate: u32,
    /// Background colour, `#RRGGBBAA`.
    pub background_color: String,
}

/// The kind of an imported file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaKindView {
    /// Moving pictures.
    Video,
    /// Sound only.
    Audio,
    /// A still picture.
    Image,
}

/// One imported file.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaView {
    /// Identifier in the project.
    pub id: MediaAssetId,
    /// File name shown to the user.
    pub name: String,
    /// Full path.
    pub file_path: String,
    /// Video, sound or picture.
    pub kind: MediaKindView,
    /// Length, for videos and sounds.
    pub duration_ticks: Option<u64>,
    /// Picture width, for videos and pictures.
    pub width: Option<u32>,
    /// Picture height, for videos and pictures.
    pub height: Option<u32>,
    /// Whether the file has pictures.
    pub has_picture: bool,
    /// Whether the file has sound.
    pub has_sound: bool,
    /// How many clips show this file.
    pub use_count: usize,
    /// The file is no longer where it was imported from.
    pub is_missing: bool,
}

/// One track.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackView {
    /// Identifier in the project.
    pub id: TrackId,
    /// `video` or `audio`.
    pub kind: TrackKindView,
    /// Name, for example "Video 1" (the interface translates default names).
    pub name: String,
    /// Its sound is off.
    pub is_muted: bool,
    /// Its pictures are not drawn.
    pub is_hidden: bool,
    /// Clips by start time.
    pub clips: Vec<ClipView>,
}

/// The kind of a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackKindView {
    /// Pictures (videos, images, texts), with the sound of videos.
    Video,
    /// Sound.
    Audio,
}

/// What a clip shows, for its colour and icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ClipKindView {
    /// A range of a video file.
    Video,
    /// A range of a sound file.
    Audio,
    /// A still picture.
    Image,
    /// A title.
    Text,
}

/// A value that may be animated.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertyView {
    /// The value when it is not animated.
    pub value: f32,
    /// Number of keyframes; 0 when the value is constant.
    pub keyframe_count: usize,
}

/// Picture removed from each edge, as fractions of the picture.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CropView {
    /// From the left edge.
    pub left: f32,
    /// From the top edge.
    pub top: f32,
    /// From the right edge.
    pub right: f32,
    /// From the bottom edge.
    pub bottom: f32,
}

/// The words and style of a title.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextView {
    /// The words, with line breaks.
    pub text: String,
    /// Font family.
    pub font_family: String,
    /// Size in canvas pixels.
    pub font_size: f32,
    /// `#RRGGBBAA`.
    pub color: String,
    /// `left`, `center` or `right`.
    pub alignment: TextAlignment,
}

/// The first colour adjustment of a clip.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorAdjustmentView {
    /// The effect holding it.
    pub effect_id: EffectId,
    /// Whether it is applied.
    pub is_enabled: bool,
    /// In photographic stops.
    pub exposure: f32,
    /// Multiplier, 1 unchanged.
    pub contrast: f32,
    /// Multiplier, 1 unchanged.
    pub saturation: f32,
    /// −1 (cooler) to 1 (warmer).
    pub temperature: f32,
}

/// One clip.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipView {
    /// Identifier in the project.
    pub id: ClipId,
    /// The track it is on.
    pub track_id: TrackId,
    /// What it shows.
    pub kind: ClipKindView,
    /// The file name, or the first line of a title.
    pub name: String,
    /// Where it starts on the timeline.
    pub start_ticks: u64,
    /// How long it lasts on the timeline.
    pub duration_ticks: u64,
    /// The file it shows, for media clips.
    pub media_id: Option<MediaAssetId>,
    /// Start of the range of the file, for media clips.
    pub source_start_ticks: Option<u64>,
    /// End of the range of the file, for media clips.
    pub source_end_ticks: Option<u64>,
    /// Length of the whole file, when known (for trimming limits).
    pub source_duration_ticks: Option<u64>,
    /// Playback speed multiplier (1 = normal).
    pub speed: f64,
    /// Whether it draws a picture.
    pub has_picture: bool,
    /// Whether it plays sound.
    pub has_sound: bool,
    /// Volume multiplier.
    pub volume: PropertyView,
    /// 0 invisible to 1 opaque.
    pub opacity: PropertyView,
    /// Pixels from the canvas centre, rightwards.
    pub position_x: PropertyView,
    /// Pixels from the canvas centre, downwards.
    pub position_y: PropertyView,
    /// Size multiplier (1 = fits the canvas).
    pub scale: PropertyView,
    /// Clockwise degrees.
    pub rotation: PropertyView,
    /// Cropped edges.
    pub crop: CropView,
    /// Words and style, for titles.
    pub text: Option<TextView>,
    /// The colour adjustment, if the clip has one.
    pub color_adjustment: Option<ColorAdjustmentView>,
    /// Number of effects of any kind.
    pub effect_count: usize,
}

/// Everything needed to build a view besides the project itself.
pub struct ViewContext<'context> {
    /// The change counter.
    pub revision: u64,
    /// The `.kadr` file.
    pub file_path: &'context Path,
    /// Why the last save failed, if it did.
    pub save_failure: Option<&'context str>,
    /// Whether the format follows the first video.
    pub adopts_first_video_format: bool,
    /// Undo label.
    pub undo_label: Option<&'context str>,
    /// Redo label.
    pub redo_label: Option<&'context str>,
}

/// Builds the view of `project`.
pub fn build_project_view(project: &Project, context: &ViewContext<'_>) -> ProjectView {
    let settings = project.settings();
    ProjectView {
        revision: context.revision,
        name: project_name_of(context.file_path),
        file_path: context.file_path.display().to_string(),
        save_failure: context.save_failure.map(str::to_string),
        settings: SettingsView {
            canvas_width: settings.canvas_size.width_in_pixels,
            canvas_height: settings.canvas_size.height_in_pixels,
            frame_rate_numerator: settings.frame_rate.numerator(),
            frame_rate_denominator: settings.frame_rate.denominator(),
            audio_sample_rate: settings.audio_sample_rate_in_hertz,
            background_color: settings.background_color.to_hex(),
        },
        adopts_first_video_format: context.adopts_first_video_format,
        duration_ticks: project.timeline().duration().ticks(),
        media: project
            .media_assets()
            .iter()
            .map(|media_asset| media_view_of(project, media_asset))
            .collect(),
        tracks: tracks_in_display_order(project)
            .into_iter()
            .map(|track| track_view_of(project, track))
            .collect(),
        undo_label: context.undo_label.map(str::to_string),
        redo_label: context.redo_label.map(str::to_string),
    }
}

/// The project name shown to the user: the file name without its extension.
pub fn project_name_of(file_path: &Path) -> String {
    file_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Video tracks from the top layer down, then audio tracks in their order.
pub fn tracks_in_display_order(project: &Project) -> Vec<&Track> {
    let tracks = project.timeline().tracks();
    let video_tracks = tracks
        .iter()
        .rev()
        .filter(|track| track.kind == TrackKind::Video);
    let audio_tracks = tracks
        .iter()
        .filter(|track| track.kind == TrackKind::Audio);
    video_tracks.chain(audio_tracks).collect()
}

fn media_view_of(project: &Project, media_asset: &MediaAsset) -> MediaView {
    let description = &media_asset.description;
    MediaView {
        id: media_asset.id,
        name: description.display_name.clone(),
        file_path: description.file_path.display().to_string(),
        kind: match description.kind {
            MediaKind::Video => MediaKindView::Video,
            MediaKind::Audio => MediaKindView::Audio,
            MediaKind::Image => MediaKindView::Image,
        },
        duration_ticks: description.duration.map(|duration| duration.ticks()),
        width: description.picture_size.map(|size| size.width_in_pixels),
        height: description.picture_size.map(|size| size.height_in_pixels),
        has_picture: description.has_picture(),
        has_sound: description.has_audio(),
        use_count: project.clips_using_media_asset(media_asset.id).len(),
        is_missing: !description.file_path.exists(),
    }
}

fn track_view_of(project: &Project, track: &Track) -> TrackView {
    TrackView {
        id: track.id,
        kind: match track.kind {
            TrackKind::Video => TrackKindView::Video,
            TrackKind::Audio => TrackKindView::Audio,
        },
        name: track.name.clone(),
        is_muted: track.is_muted,
        is_hidden: track.is_hidden,
        clips: track
            .clips()
            .iter()
            .map(|clip| clip_view_of(project, track, clip))
            .collect(),
    }
}

fn property_view_of(value: &AnimatedValue) -> PropertyView {
    PropertyView {
        value: value.constant_value,
        keyframe_count: value.keyframes().len(),
    }
}

fn clip_view_of(project: &Project, track: &Track, clip: &Clip) -> ClipView {
    let (kind, name, media_fields, text) = match &clip.content {
        ClipContent::Media(segment) => {
            let media_asset = project.media_asset(segment.media_asset_id);
            let description = media_asset.map(|media_asset| &media_asset.description);
            let kind = match description.map(|description| description.kind) {
                Some(MediaKind::Audio) => ClipKindView::Audio,
                Some(MediaKind::Image) => ClipKindView::Image,
                _ => ClipKindView::Video,
            };
            let name = description
                .map(|description| description.display_name.clone())
                .unwrap_or_default();
            let media_fields = MediaFields {
                media_id: Some(segment.media_asset_id),
                source_start_ticks: Some(segment.source_start.ticks()),
                source_end_ticks: Some(segment.source_end.ticks()),
                source_duration_ticks: description
                    .and_then(|description| description.duration)
                    .map(|duration| duration.ticks()),
                speed: segment.speed.as_multiplier(),
                has_picture: description.is_some_and(|description| description.has_picture()),
                has_sound: description.is_some_and(|description| description.has_audio()),
            };
            (kind, name, media_fields, None)
        }
        ClipContent::Text(text_content) => {
            let first_line = text_content.text.lines().next().unwrap_or_default();
            let text = TextView {
                text: text_content.text.clone(),
                font_family: text_content.style.font_family.clone(),
                font_size: text_content.style.font_size_in_pixels,
                color: text_content.style.color.to_hex(),
                alignment: text_content.style.alignment,
            };
            (
                ClipKindView::Text,
                first_line.to_string(),
                MediaFields::for_text(),
                Some(text),
            )
        }
    };
    let transform = &clip.transform;
    ClipView {
        id: clip.id,
        track_id: track.id,
        kind,
        name,
        start_ticks: clip.timeline_start.ticks(),
        duration_ticks: clip.timeline_duration().ticks(),
        media_id: media_fields.media_id,
        source_start_ticks: media_fields.source_start_ticks,
        source_end_ticks: media_fields.source_end_ticks,
        source_duration_ticks: media_fields.source_duration_ticks,
        speed: media_fields.speed,
        has_picture: media_fields.has_picture,
        has_sound: media_fields.has_sound,
        volume: property_view_of(&clip.volume_multiplier),
        opacity: property_view_of(&clip.opacity),
        position_x: property_view_of(&transform.position_x_in_pixels),
        position_y: property_view_of(&transform.position_y_in_pixels),
        scale: property_view_of(&transform.scale_factor),
        rotation: property_view_of(&transform.rotation_in_degrees),
        crop: CropView {
            left: transform.crop.left_fraction,
            top: transform.crop.top_fraction,
            right: transform.crop.right_fraction,
            bottom: transform.crop.bottom_fraction,
        },
        text,
        color_adjustment: first_color_adjustment_of(clip),
        effect_count: clip.effects.len(),
    }
}

/// The fields only media clips have.
struct MediaFields {
    media_id: Option<MediaAssetId>,
    source_start_ticks: Option<u64>,
    source_end_ticks: Option<u64>,
    source_duration_ticks: Option<u64>,
    speed: f64,
    has_picture: bool,
    has_sound: bool,
}

impl MediaFields {
    fn for_text() -> Self {
        Self {
            media_id: None,
            source_start_ticks: None,
            source_end_ticks: None,
            source_duration_ticks: None,
            speed: 1.0,
            has_picture: true,
            has_sound: false,
        }
    }
}

fn first_color_adjustment_of(clip: &Clip) -> Option<ColorAdjustmentView> {
    clip.effects.iter().find_map(|effect| match &effect.kind {
        EffectKind::ColorAdjustment(adjustment) => Some(ColorAdjustmentView {
            effect_id: effect.id,
            is_enabled: effect.is_enabled,
            exposure: adjustment.exposure_in_stops,
            contrast: adjustment.contrast_multiplier,
            saturation: adjustment.saturation_multiplier,
            temperature: adjustment.temperature_shift,
        }),
        EffectKind::TrackedBlur(_) => None,
    })
}
