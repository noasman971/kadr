//! The inspector's changes to one clip: picture, sound, speed, colour and text
//! (docs/ux.md, "Adjust a clip"). Each function is one undo step.

use kadr_core::clip::{
    AnimatableProperty, ClipContent, CropInsets, PlaybackSpeed, TextAlignment, TextStyle,
};
use kadr_core::color::Color;
use kadr_core::commands::{
    AddEffect, RemoveEffect, ReplaceEffect, SetClipCrop, SetClipPropertyValue, SetClipSpeed,
    SetClipText,
};
use kadr_core::effect::{ColorAdjustment, EffectKind};
use kadr_core::error::EditError;
use kadr_core::history::EditSession;
use kadr_core::ids::ClipId;
use kadr_core::project::Project;
use serde::Deserialize;

use super::EditOutcome;

/// A value of a clip the inspector changes with a number field or a slider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClipValue {
    /// Sound level multiplier (0–4).
    Volume,
    /// 0 invisible to 1 opaque.
    Opacity,
    /// Pixels from the canvas centre.
    PositionX,
    /// Pixels from the canvas centre.
    PositionY,
    /// Size multiplier.
    Scale,
    /// Degrees, clockwise.
    Rotation,
}

impl ClipValue {
    fn property(self) -> AnimatableProperty {
        match self {
            Self::Volume => AnimatableProperty::Volume,
            Self::Opacity => AnimatableProperty::Opacity,
            Self::PositionX => AnimatableProperty::PositionX,
            Self::PositionY => AnimatableProperty::PositionY,
            Self::Scale => AnimatableProperty::Scale,
            Self::Rotation => AnimatableProperty::Rotation,
        }
    }

    /// The undo label (a key the front end translates).
    fn undo_label(self) -> &'static str {
        match self {
            Self::Volume => "changeVolume",
            Self::Opacity => "changeOpacity",
            Self::PositionX | Self::PositionY => "changePosition",
            Self::Scale => "changeScale",
            Self::Rotation => "changeRotation",
        }
    }
}

/// The colour settings of the inspector.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorSettings {
    /// Photographic stops.
    pub exposure: f32,
    /// Multiplier, 1 unchanged.
    pub contrast: f32,
    /// Multiplier, 1 unchanged.
    pub saturation: f32,
    /// −1 (cooler) to 1 (warmer).
    pub temperature: f32,
}

/// The text settings of the inspector.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextSettings {
    /// The words, with line breaks.
    pub text: String,
    /// Font family.
    pub font_family: String,
    /// Size in canvas pixels.
    pub font_size: f32,
    /// `#RRGGBB` or `#RRGGBBAA`; an unreadable colour keeps the current one.
    pub color: String,
    /// Alignment of the lines.
    pub alignment: TextAlignmentChoice,
}

/// Alignment of a title's lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextAlignmentChoice {
    /// Lines start on the left.
    Left,
    /// Lines are centred.
    Center,
    /// Lines end on the right.
    Right,
}

/// Sets one value of a clip.
///
/// # Errors
///
/// When the clip does not exist, the value is animated, or out of range.
pub fn set_clip_value(
    session: &mut EditSession,
    clip_id: ClipId,
    value: ClipValue,
    amount: f32,
) -> Result<EditOutcome, EditError> {
    session.transaction(value.undo_label(), |transaction| {
        transaction.execute(SetClipPropertyValue::new(clip_id, value.property(), amount))?;
        Ok(EditOutcome::default())
    })
}

/// Crops the clip's picture (fractions of each side, 0 to under 1).
///
/// # Errors
///
/// When the clip does not exist or the crop leaves nothing.
pub fn set_clip_crop(
    session: &mut EditSession,
    clip_id: ClipId,
    crop: CropInsets,
) -> Result<EditOutcome, EditError> {
    session.transaction("changeCrop", |transaction| {
        transaction.execute(SetClipCrop::new(clip_id, crop))?;
        Ok(EditOutcome::default())
    })
}

/// Puts the picture settings back: centred, full size, upright, opaque, uncropped.
/// Animated values are left as they are.
///
/// # Errors
///
/// When the clip does not exist.
pub fn reset_clip_picture(
    session: &mut EditSession,
    clip_id: ClipId,
) -> Result<EditOutcome, EditError> {
    let animated = animated_properties(session.project(), clip_id)?;
    session.transaction("resetPicture", |transaction| {
        let defaults = [
            (AnimatableProperty::PositionX, 0.0),
            (AnimatableProperty::PositionY, 0.0),
            (AnimatableProperty::Scale, 1.0),
            (AnimatableProperty::Rotation, 0.0),
            (AnimatableProperty::Opacity, 1.0),
        ];
        for (property, default_value) in defaults {
            if !animated.contains(&property) {
                transaction.execute(SetClipPropertyValue::new(clip_id, property, default_value))?;
            }
        }
        transaction.execute(SetClipCrop::new(clip_id, CropInsets::default()))?;
        Ok(EditOutcome::default())
    })
}

/// Changes the clip's speed. Its length on the timeline changes with it; later
/// clips stay where they are, so a longer clip that would overlap is refused.
///
/// # Errors
///
/// When the clip does not exist, is not a media clip, or would overlap.
pub fn set_clip_speed(
    session: &mut EditSession,
    clip_id: ClipId,
    multiplier: f64,
) -> Result<EditOutcome, EditError> {
    let clamped = if multiplier.is_finite() {
        multiplier.clamp(
            PlaybackSpeed::SLOWEST_MULTIPLIER,
            PlaybackSpeed::FASTEST_MULTIPLIER,
        )
    } else {
        1.0
    };
    let speed = PlaybackSpeed::from_multiplier(clamped).unwrap_or(PlaybackSpeed::NORMAL);
    session.transaction("changeSpeed", |transaction| {
        transaction.execute(SetClipSpeed::new(clip_id, speed))?;
        Ok(EditOutcome::default())
    })
}

/// Sets the clip's colour adjustment, adding one on the first change.
///
/// # Errors
///
/// When the clip does not exist.
pub fn set_clip_color(
    session: &mut EditSession,
    clip_id: ClipId,
    settings: ColorSettings,
) -> Result<EditOutcome, EditError> {
    let existing = color_adjustment_effect(session.project(), clip_id)?;
    let kind = EffectKind::ColorAdjustment(ColorAdjustment {
        exposure_in_stops: settings.exposure,
        contrast_multiplier: settings.contrast,
        saturation_multiplier: settings.saturation,
        temperature_shift: settings.temperature,
    });
    session.transaction("changeColor", |transaction| {
        match existing {
            Some(effect_id) => transaction.execute(ReplaceEffect::new(clip_id, effect_id, kind))?,
            None => transaction.execute(AddEffect::new(clip_id, kind))?,
        };
        Ok(EditOutcome::default())
    })
}

/// Removes the clip's colour adjustment ("off until first touched").
///
/// # Errors
///
/// When the clip does not exist.
pub fn reset_clip_color(
    session: &mut EditSession,
    clip_id: ClipId,
) -> Result<EditOutcome, EditError> {
    let existing = color_adjustment_effect(session.project(), clip_id)?;
    session.transaction("resetColor", |transaction| {
        if let Some(effect_id) = existing {
            transaction.execute(RemoveEffect::new(clip_id, effect_id))?;
        }
        Ok(EditOutcome::default())
    })
}

/// Changes a title's words and style.
///
/// # Errors
///
/// When the clip does not exist or is not a title.
pub fn set_clip_text(
    session: &mut EditSession,
    clip_id: ClipId,
    settings: &TextSettings,
) -> Result<EditOutcome, EditError> {
    let current_color = current_text_style(session.project(), clip_id)?.color;
    let style = TextStyle {
        font_family: settings.font_family.clone(),
        font_size_in_pixels: settings.font_size,
        color: Color::from_hex(&settings.color).unwrap_or(current_color),
        alignment: match settings.alignment {
            TextAlignmentChoice::Left => TextAlignment::Left,
            TextAlignmentChoice::Center => TextAlignment::Center,
            TextAlignmentChoice::Right => TextAlignment::Right,
        },
    };
    session.transaction("changeText", |transaction| {
        transaction.execute(SetClipText::new(clip_id, settings.text.clone()).with_style(style))?;
        Ok(EditOutcome::default())
    })
}

/// The clip's first colour adjustment effect, if it has one.
fn color_adjustment_effect(
    project: &Project,
    clip_id: ClipId,
) -> Result<Option<kadr_core::ids::EffectId>, EditError> {
    let (_, clip) = find_clip(project, clip_id)?;
    Ok(clip
        .effects
        .iter()
        .find(|effect| matches!(effect.kind, EffectKind::ColorAdjustment(_)))
        .map(|effect| effect.id))
}

/// The properties animated with keyframes, which a single value would not replace.
fn animated_properties(
    project: &Project,
    clip_id: ClipId,
) -> Result<Vec<AnimatableProperty>, EditError> {
    let (_, clip) = find_clip(project, clip_id)?;
    Ok(AnimatableProperty::ALL
        .into_iter()
        .filter(|property| clip.animated_property(*property).is_animated())
        .collect())
}

fn current_text_style(project: &Project, clip_id: ClipId) -> Result<TextStyle, EditError> {
    let (_, clip) = find_clip(project, clip_id)?;
    match &clip.content {
        ClipContent::Text(text) => Ok(text.style.clone()),
        ClipContent::Media(_) => Err(EditError::NotATextClip { clip_id }),
    }
}

fn find_clip(
    project: &Project,
    clip_id: ClipId,
) -> Result<(&kadr_core::timeline::Track, &kadr_core::clip::Clip), EditError> {
    project
        .timeline()
        .find_clip(clip_id)
        .ok_or_else(|| EditError::ClipNotFound {
            clip_id,
            existing_clip_ids: project
                .timeline()
                .all_clips()
                .map(|(_, clip)| clip.id)
                .collect(),
        })
}
