//! The inspector's changes to one clip: picture, sound, speed, colour and text
//! (docs/ux.md, "Adjust a clip"). Each function is one undo step.
//!
//! A change names only the settings it changes ([`CropChanges`], [`ColorChanges`],
//! [`TextChanges`]); the others are read from the project as it is when the change
//! is applied. The interface's copy of the project can be behind (an earlier change
//! not answered yet, or a change made by Claude): a request carrying every setting
//! from that copy would put old values back.

use kadr_core::clip::{
    AnimatableProperty, ClipContent, CropInsets, PlaybackSpeed, TextAlignment, TextContent,
    TextStyle,
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

/// The crop sides to change, as fractions of each side (0 to under 1). A side left
/// out keeps its current value.
///
/// Unknown names are refused, so a misspelt side is reported instead of changing
/// nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CropChanges {
    /// Fraction removed on the left.
    pub left: Option<f32>,
    /// Fraction removed at the top.
    pub top: Option<f32>,
    /// Fraction removed on the right.
    pub right: Option<f32>,
    /// Fraction removed at the bottom.
    pub bottom: Option<f32>,
}

impl CropChanges {
    fn applied_to(self, crop: CropInsets) -> CropInsets {
        CropInsets {
            left_fraction: self.left.unwrap_or(crop.left_fraction),
            top_fraction: self.top.unwrap_or(crop.top_fraction),
            right_fraction: self.right.unwrap_or(crop.right_fraction),
            bottom_fraction: self.bottom.unwrap_or(crop.bottom_fraction),
        }
    }
}

/// The colour settings to change. A setting left out keeps its current value, or
/// its neutral one when the clip has no colour adjustment yet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColorChanges {
    /// Photographic stops.
    pub exposure: Option<f32>,
    /// Multiplier, 1 unchanged.
    pub contrast: Option<f32>,
    /// Multiplier, 1 unchanged.
    pub saturation: Option<f32>,
    /// −1 (cooler) to 1 (warmer).
    pub temperature: Option<f32>,
}

impl ColorChanges {
    fn applied_to(self, adjustment: ColorAdjustment) -> ColorAdjustment {
        ColorAdjustment {
            exposure_in_stops: self.exposure.unwrap_or(adjustment.exposure_in_stops),
            contrast_multiplier: self.contrast.unwrap_or(adjustment.contrast_multiplier),
            saturation_multiplier: self.saturation.unwrap_or(adjustment.saturation_multiplier),
            temperature_shift: self.temperature.unwrap_or(adjustment.temperature_shift),
        }
    }
}

/// The words and style settings of a title to change. A setting left out keeps
/// its current value.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextChanges {
    /// The words, with line breaks.
    pub text: Option<String>,
    /// Font family.
    pub font_family: Option<String>,
    /// Size in canvas pixels.
    pub font_size: Option<f32>,
    /// `#RRGGBB` or `#RRGGBBAA`; an unreadable colour keeps the current one.
    pub color: Option<String>,
    /// Alignment of the lines.
    pub alignment: Option<TextAlignmentChoice>,
}

impl TextChanges {
    /// The title's words and style once these changes are made.
    fn applied_to(&self, current: &TextContent) -> (String, TextStyle) {
        let style = &current.style;
        let changed_style = TextStyle {
            font_family: self
                .font_family
                .clone()
                .unwrap_or_else(|| style.font_family.clone()),
            font_size_in_pixels: self.font_size.unwrap_or(style.font_size_in_pixels),
            color: self
                .color
                .as_deref()
                .and_then(|hex| Color::from_hex(hex).ok())
                .unwrap_or(style.color),
            alignment: self
                .alignment
                .map_or(style.alignment, TextAlignmentChoice::alignment),
        };
        let words = self.text.clone().unwrap_or_else(|| current.text.clone());
        (words, changed_style)
    }
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

impl TextAlignmentChoice {
    fn alignment(self) -> TextAlignment {
        match self {
            Self::Left => TextAlignment::Left,
            Self::Center => TextAlignment::Center,
            Self::Right => TextAlignment::Right,
        }
    }
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

/// Crops the clip's picture: the sides given change, the others stay as they are.
///
/// # Errors
///
/// When the clip does not exist or the crop leaves nothing.
pub fn set_clip_crop(
    session: &mut EditSession,
    clip_id: ClipId,
    changes: CropChanges,
) -> Result<EditOutcome, EditError> {
    session.transaction("changeCrop", |transaction| {
        let (_, clip) = find_clip(transaction.project(), clip_id)?;
        let crop = changes.applied_to(clip.transform.crop);
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

/// Changes the clip's colour adjustment, adding one on the first change: the
/// settings given change, the others stay as they are.
///
/// # Errors
///
/// When the clip does not exist.
pub fn set_clip_color(
    session: &mut EditSession,
    clip_id: ClipId,
    changes: ColorChanges,
) -> Result<EditOutcome, EditError> {
    session.transaction("changeColor", |transaction| {
        let existing = color_adjustment_of(transaction.project(), clip_id)?;
        let current = existing.map_or(ColorAdjustment::NEUTRAL, |(_, adjustment)| adjustment);
        let kind = EffectKind::ColorAdjustment(changes.applied_to(current));
        match existing {
            Some((effect_id, _)) => {
                transaction.execute(ReplaceEffect::new(clip_id, effect_id, kind))?
            }
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
    let existing = color_adjustment_of(session.project(), clip_id)?;
    session.transaction("resetColor", |transaction| {
        if let Some((effect_id, _)) = existing {
            transaction.execute(RemoveEffect::new(clip_id, effect_id))?;
        }
        Ok(EditOutcome::default())
    })
}

/// Changes a title's words or style: the settings given change, the others stay
/// as they are.
///
/// # Errors
///
/// When the clip does not exist or is not a title.
pub fn set_clip_text(
    session: &mut EditSession,
    clip_id: ClipId,
    changes: &TextChanges,
) -> Result<EditOutcome, EditError> {
    session.transaction("changeText", |transaction| {
        let (words, style) = changes.applied_to(text_of(transaction.project(), clip_id)?);
        transaction.execute(SetClipText::new(clip_id, words).with_style(style))?;
        Ok(EditOutcome::default())
    })
}

/// The clip's first colour adjustment effect and its settings, if it has one.
fn color_adjustment_of(
    project: &Project,
    clip_id: ClipId,
) -> Result<Option<(kadr_core::ids::EffectId, ColorAdjustment)>, EditError> {
    let (_, clip) = find_clip(project, clip_id)?;
    Ok(clip.effects.iter().find_map(|effect| match effect.kind {
        EffectKind::ColorAdjustment(adjustment) => Some((effect.id, adjustment)),
        _ => None,
    }))
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

/// A title's words and style.
fn text_of(project: &Project, clip_id: ClipId) -> Result<&TextContent, EditError> {
    let (_, clip) = find_clip(project, clip_id)?;
    match &clip.content {
        ClipContent::Text(text) => Ok(text),
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
