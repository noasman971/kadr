//! How the project is described to Claude: plain JSON in seconds and pixels.
//!
//! Kadr stores exact ticks and positions measured from the canvas centre; Claude
//! gets seconds (rounded to the microsecond) and **canvas pixels with the origin
//! at the top-left corner**, x to the right and y down, the same convention as
//! picture coordinates.

use kadr_core::animation::{AnimatedValue, Interpolation};
use kadr_core::clip::{AnimatableProperty, Clip, ClipContent, TextAlignment};
use kadr_core::effect::{BlurShape, BlurStyle, Effect, EffectKind, TrackingConfidence};
use kadr_core::media_asset::{MediaAsset, MediaAssetDescription, MediaKind, PictureSize};
use kadr_core::project::{Project, ProjectSettings};
use kadr_core::time::FrameRate;
use kadr_core::timeline::{Track, TrackKind};
use serde::Serialize;

use crate::time_input::{seconds_in, seconds_of};

/// A media file, imported or only probed.
#[derive(Debug, Serialize)]
pub(crate) struct MediaView {
    /// Present once imported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) media_id: Option<u64>,
    pub(crate) name: String,
    pub(crate) kind: &'static str,
    pub(crate) file_path: String,
    /// Absent for still images, which can last any time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) duration_in_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) width_in_pixels: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) height_in_pixels: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) frames_per_second: Option<f64>,
    pub(crate) has_sound: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) used_by_clip_ids: Option<Vec<u64>>,
}

impl MediaView {
    /// Describes a file that is not imported yet.
    pub(crate) fn of_description(description: &MediaAssetDescription) -> Self {
        Self {
            media_id: None,
            name: description.display_name.clone(),
            kind: media_kind_name(description.kind),
            file_path: description.file_path.display().to_string(),
            duration_in_seconds: description.duration.map(seconds_in),
            width_in_pixels: description.picture_size.map(|size| size.width_in_pixels),
            height_in_pixels: description.picture_size.map(|size| size.height_in_pixels),
            frames_per_second: description.frame_rate.map(rounded_frames_per_second),
            has_sound: description.has_audio(),
            used_by_clip_ids: None,
        }
    }

    /// Describes an imported file, with the clips that show it.
    pub(crate) fn of_asset(project: &Project, media_asset: &MediaAsset) -> Self {
        let used_by_clip_ids = project
            .clips_using_media_asset(media_asset.id)
            .into_iter()
            .map(|clip_id| clip_id.raw())
            .collect();
        Self {
            media_id: Some(media_asset.id.raw()),
            used_by_clip_ids: Some(used_by_clip_ids),
            ..Self::of_description(&media_asset.description)
        }
    }
}

/// The name of a media kind for Claude.
pub(crate) fn media_kind_name(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Video => "video",
        MediaKind::Audio => "audio",
        MediaKind::Image => "image",
    }
}

/// The name of a track kind for Claude.
pub(crate) fn track_kind_name(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Video => "video",
        TrackKind::Audio => "audio",
    }
}

/// Frames per second, rounded to three decimals (29.97 rather than 29.97002997).
fn rounded_frames_per_second(frame_rate: FrameRate) -> f64 {
    (frame_rate.as_frames_per_second() * 1000.0).round() / 1000.0
}

/// The project settings.
#[derive(Debug, Serialize)]
pub(crate) struct SettingsView {
    pub(crate) width_in_pixels: u32,
    pub(crate) height_in_pixels: u32,
    pub(crate) orientation: &'static str,
    pub(crate) frames_per_second: f64,
    pub(crate) audio_sample_rate_in_hertz: u32,
    pub(crate) background_color: String,
}

impl SettingsView {
    pub(crate) fn of(settings: &ProjectSettings) -> Self {
        let PictureSize {
            width_in_pixels,
            height_in_pixels,
        } = settings.canvas_size;
        let orientation = match width_in_pixels.cmp(&height_in_pixels) {
            std::cmp::Ordering::Greater => "landscape",
            std::cmp::Ordering::Less => "vertical",
            std::cmp::Ordering::Equal => "square",
        };
        Self {
            width_in_pixels,
            height_in_pixels,
            orientation,
            frames_per_second: rounded_frames_per_second(settings.frame_rate),
            audio_sample_rate_in_hertz: settings.audio_sample_rate_in_hertz,
            background_color: settings.background_color.to_hex(),
        }
    }
}

/// A track with a short line per clip.
#[derive(Debug, Serialize)]
pub(crate) struct TrackView {
    pub(crate) track_id: u64,
    pub(crate) kind: &'static str,
    pub(crate) name: String,
    /// 0 is the bottom track; for video, higher tracks are drawn on top.
    pub(crate) position_from_bottom: usize,
    pub(crate) is_muted: bool,
    pub(crate) is_hidden: bool,
    pub(crate) clips: Vec<ClipSummaryView>,
}

impl TrackView {
    pub(crate) fn of(project: &Project, track: &Track, position_from_bottom: usize) -> Self {
        Self {
            track_id: track.id.raw(),
            kind: track_kind_name(track.kind),
            name: track.name.clone(),
            position_from_bottom,
            is_muted: track.is_muted,
            is_hidden: track.is_hidden,
            clips: track
                .clips()
                .iter()
                .map(|clip| ClipSummaryView::of(project, clip))
                .collect(),
        }
    }
}

/// The essentials of a clip, for the timeline overview.
#[derive(Debug, Serialize)]
pub(crate) struct ClipSummaryView {
    pub(crate) clip_id: u64,
    pub(crate) kind: &'static str,
    /// The media file name, or the words of a text.
    pub(crate) shows: String,
    pub(crate) timeline_start_in_seconds: f64,
    pub(crate) timeline_end_in_seconds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) media_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source_start_in_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source_end_in_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) speed: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) effects: Vec<&'static str>,
}

impl ClipSummaryView {
    pub(crate) fn of(project: &Project, clip: &Clip) -> Self {
        let mut summary = Self {
            clip_id: clip.id.raw(),
            kind: clip_kind_name(project, clip),
            shows: String::new(),
            timeline_start_in_seconds: seconds_of(clip.timeline_start),
            timeline_end_in_seconds: seconds_of(clip.timeline_end()),
            media_id: None,
            source_start_in_seconds: None,
            source_end_in_seconds: None,
            speed: None,
            effects: clip
                .effects
                .iter()
                .map(|effect| effect.kind.display_name())
                .collect(),
        };
        match &clip.content {
            ClipContent::Media(segment) => {
                summary.shows = project
                    .media_asset(segment.media_asset_id)
                    .map(|asset| asset.description.display_name.clone())
                    .unwrap_or_default();
                summary.media_id = Some(segment.media_asset_id.raw());
                summary.source_start_in_seconds = Some(seconds_of(segment.source_start));
                summary.source_end_in_seconds = Some(seconds_of(segment.source_end));
                summary.speed = Some(segment.speed.as_multiplier());
            }
            ClipContent::Text(text_content) => summary.shows = text_content.text.clone(),
        }
        summary
    }
}

/// "video", "audio", "image" or "text".
pub(crate) fn clip_kind_name(project: &Project, clip: &Clip) -> &'static str {
    match &clip.content {
        ClipContent::Text(_) => "text",
        ClipContent::Media(segment) => project
            .media_asset(segment.media_asset_id)
            .map_or("video", |asset| media_kind_name(asset.description.kind)),
    }
}

/// Everything about one clip.
#[derive(Debug, Serialize)]
pub(crate) struct ClipView {
    #[serde(flatten)]
    pub(crate) summary: ClipSummaryView,
    pub(crate) track_id: u64,
    pub(crate) duration_in_seconds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) text_style: Option<TextStyleView>,
    /// Null when animated (see `animations`).
    pub(crate) volume: Option<f32>,
    /// Null when animated (see `animations`).
    pub(crate) opacity: Option<f32>,
    pub(crate) transform: TransformView,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) animations: Vec<AnimationView>,
    #[serde(rename = "effects")]
    pub(crate) effect_details: Vec<EffectView>,
}

impl ClipView {
    pub(crate) fn of(project: &Project, track: &Track, clip: &Clip) -> Self {
        let mut summary = ClipSummaryView::of(project, clip);
        summary.effects = Vec::new();
        let text_style = match &clip.content {
            ClipContent::Text(text_content) => Some(TextStyleView {
                font_family: text_content.style.font_family.clone(),
                font_size_in_pixels: text_content.style.font_size_in_pixels,
                color: text_content.style.color.to_hex(),
                alignment: alignment_name(text_content.style.alignment),
            }),
            ClipContent::Media(_) => None,
        };
        Self {
            summary,
            track_id: track.id.raw(),
            duration_in_seconds: seconds_in(clip.timeline_duration()),
            text_style,
            volume: constant_or_none(&clip.volume_multiplier),
            opacity: constant_or_none(&clip.opacity),
            transform: TransformView::of(project.settings(), clip),
            animations: animations_of(project.settings(), clip),
            effect_details: clip.effects.iter().map(EffectView::of).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct TextStyleView {
    pub(crate) font_family: String,
    pub(crate) font_size_in_pixels: f32,
    pub(crate) color: String,
    pub(crate) alignment: &'static str,
}

/// The name of an alignment for Claude.
pub(crate) fn alignment_name(alignment: TextAlignment) -> &'static str {
    match alignment {
        TextAlignment::Left => "left",
        TextAlignment::Center => "center",
        TextAlignment::Right => "right",
    }
}

/// Where the clip sits on the canvas. Values are null when animated.
#[derive(Debug, Serialize)]
pub(crate) struct TransformView {
    /// Canvas pixels from the left edge to the clip's centre.
    pub(crate) center_x_in_pixels: Option<f32>,
    /// Canvas pixels from the top edge to the clip's centre.
    pub(crate) center_y_in_pixels: Option<f32>,
    /// 1 = the picture fits inside the canvas.
    pub(crate) scale: Option<f32>,
    pub(crate) rotation_in_degrees: Option<f32>,
    pub(crate) crop_left: f32,
    pub(crate) crop_top: f32,
    pub(crate) crop_right: f32,
    pub(crate) crop_bottom: f32,
}

impl TransformView {
    fn of(settings: &ProjectSettings, clip: &Clip) -> Self {
        let transform = &clip.transform;
        let canvas_center_x = settings.canvas_size.width_in_pixels as f32 / 2.0;
        let canvas_center_y = settings.canvas_size.height_in_pixels as f32 / 2.0;
        Self {
            center_x_in_pixels: constant_or_none(&transform.position_x_in_pixels)
                .map(|offset| canvas_center_x + offset),
            center_y_in_pixels: constant_or_none(&transform.position_y_in_pixels)
                .map(|offset| canvas_center_y + offset),
            scale: constant_or_none(&transform.scale_factor),
            rotation_in_degrees: constant_or_none(&transform.rotation_in_degrees),
            crop_left: transform.crop.left_fraction,
            crop_top: transform.crop.top_fraction,
            crop_right: transform.crop.right_fraction,
            crop_bottom: transform.crop.bottom_fraction,
        }
    }
}

/// The value of a property that is not animated.
fn constant_or_none(value: &AnimatedValue) -> Option<f32> {
    (!value.is_animated()).then_some(value.constant_value)
}

/// The keyframes of one animated property.
#[derive(Debug, Serialize)]
pub(crate) struct AnimationView {
    pub(crate) property: &'static str,
    pub(crate) keyframes: Vec<KeyframeView>,
}

#[derive(Debug, Serialize)]
pub(crate) struct KeyframeView {
    /// Measured from the start of the clip.
    pub(crate) time_in_clip_in_seconds: f64,
    /// In the unit of the tool parameter of the same name (canvas pixels from the
    /// top-left corner for positions).
    pub(crate) value: f32,
    /// How the value travels to the next keyframe (see `set_keyframes`).
    pub(crate) curve: &'static str,
    /// `[x1, y1, x2, y2]` when `curve` is `"bezier"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) bezier_handles: Option<[f32; 4]>,
}

pub(crate) fn animations_of(settings: &ProjectSettings, clip: &Clip) -> Vec<AnimationView> {
    AnimatableProperty::ALL
        .into_iter()
        .filter(|property| clip.animated_property(*property).is_animated())
        .map(|property| AnimationView {
            property: property_name(property),
            keyframes: clip
                .animated_property(property)
                .keyframes()
                .iter()
                .map(|keyframe| KeyframeView {
                    time_in_clip_in_seconds: seconds_in(keyframe.time_offset),
                    value: value_shown_to_claude(settings, property, keyframe.value),
                    curve: curve_name(keyframe.interpolation_to_next),
                    bezier_handles: bezier_handles_of(keyframe.interpolation_to_next),
                })
                .collect(),
        })
        .collect()
}

/// The name of a keyframe curve in tool parameters and answers.
pub(crate) fn curve_name(interpolation: Interpolation) -> &'static str {
    match interpolation {
        Interpolation::Linear => "linear",
        Interpolation::EaseIn => "ease_in",
        Interpolation::EaseOut => "ease_out",
        Interpolation::EaseInOut => "ease_in_out",
        Interpolation::Hold => "hold",
        Interpolation::CubicBezier(_) => "bezier",
    }
}

/// `[x1, y1, x2, y2]` of a Bézier curve, as in CSS `cubic-bezier()`.
fn bezier_handles_of(interpolation: Interpolation) -> Option<[f32; 4]> {
    match interpolation {
        Interpolation::CubicBezier(curve) => Some([
            curve.start_handle.time_fraction,
            curve.start_handle.progress_fraction,
            curve.end_handle.time_fraction,
            curve.end_handle.progress_fraction,
        ]),
        _ => None,
    }
}

/// How far Claude's value of `property` is from Kadr's: positions are shown from
/// the canvas's top-left corner, but stored from its centre.
pub(crate) fn claude_value_offset(settings: &ProjectSettings, property: AnimatableProperty) -> f32 {
    match property {
        AnimatableProperty::PositionX => settings.canvas_size.width_in_pixels as f32 / 2.0,
        AnimatableProperty::PositionY => settings.canvas_size.height_in_pixels as f32 / 2.0,
        AnimatableProperty::Volume
        | AnimatableProperty::Opacity
        | AnimatableProperty::Scale
        | AnimatableProperty::Rotation => 0.0,
    }
}

/// A stored value of `property`, in the unit Claude uses.
fn value_shown_to_claude(
    settings: &ProjectSettings,
    property: AnimatableProperty,
    value: f32,
) -> f32 {
    value + claude_value_offset(settings, property)
}

/// The property name used in tool parameters.
pub(crate) fn property_name(property: AnimatableProperty) -> &'static str {
    match property {
        AnimatableProperty::Volume => "volume",
        AnimatableProperty::Opacity => "opacity",
        AnimatableProperty::PositionX => "center_x_in_pixels",
        AnimatableProperty::PositionY => "center_y_in_pixels",
        AnimatableProperty::Scale => "scale",
        AnimatableProperty::Rotation => "rotation_in_degrees",
    }
}

/// One effect of a clip.
#[derive(Debug, Serialize)]
pub(crate) struct EffectView {
    pub(crate) effect_id: u64,
    #[serde(rename = "type")]
    pub(crate) effect_type: &'static str,
    pub(crate) is_enabled: bool,
    #[serde(flatten)]
    pub(crate) settings: serde_json::Value,
}

impl EffectView {
    pub(crate) fn of(effect: &Effect) -> Self {
        let (effect_type, settings) = match &effect.kind {
            EffectKind::ColorAdjustment(adjustment) => (
                "color_adjustment",
                serde_json::json!({
                    "exposure_in_stops": adjustment.exposure_in_stops,
                    "contrast": adjustment.contrast_multiplier,
                    "saturation": adjustment.saturation_multiplier,
                    "temperature": adjustment.temperature_shift,
                }),
            ),
            EffectKind::ColorLut(color_lut) => (
                "color_lut",
                serde_json::json!({
                    "lut_file_path": color_lut.file_path.display().to_string(),
                    "strength": color_lut.strength_fraction,
                }),
            ),
            EffectKind::TrackedBlur(tracked_blur) => {
                let count_with = |confidence: TrackingConfidence| {
                    tracked_blur
                        .region_samples
                        .iter()
                        .filter(|sample| sample.confidence == confidence)
                        .count()
                };
                (
                    "tracked_blur",
                    serde_json::json!({
                        "style": blur_style_name(tracked_blur.style),
                        "shape": match tracked_blur.shape {
                            BlurShape::Rectangle => "rectangle",
                            BlurShape::Ellipse => "ellipse",
                        },
                        "margin_in_percent": tracked_blur.margin_in_percent,
                        "tracked_frame_count": tracked_blur.region_samples.len(),
                        "uncertain_frame_count": count_with(TrackingConfidence::Uncertain),
                        "lost_frame_count": count_with(TrackingConfidence::Lost),
                    }),
                )
            }
            // Its settings are not described to Claude yet.
            EffectKind::Stabilization(_) => ("stabilization", serde_json::json!({})),
        };
        Self {
            effect_id: effect.id.raw(),
            effect_type,
            is_enabled: effect.is_enabled,
            settings,
        }
    }
}

fn blur_style_name(style: BlurStyle) -> &'static str {
    match style {
        BlurStyle::GaussianBlur { .. } => "gaussian_blur",
        BlurStyle::Pixelate { .. } => "pixelate",
        BlurStyle::SolidFill { .. } => "solid_fill",
    }
}
