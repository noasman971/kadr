//! The inspector's changes to one clip: picture, sound, speed, colour, colour LUT
//! and text (docs/ux.md, "Adjust a clip"). Each function is one undo step.
//!
//! A change names only the settings it changes ([`CropChanges`], [`ColorChanges`],
//! [`LutChanges`], [`TextChanges`]); the others are read from the project as it is
//! when the change is applied. The interface's copy of the project can be behind (an earlier change
//! not answered yet, or a change made by Claude): a request carrying every setting
//! from that copy would put old values back.

use kadr_core::clip::{
    AnimatableProperty, ClipContent, CropInsets, PlaybackSpeed, TextAlignment, TextContent,
    TextStyle,
};
use kadr_core::color::Color;
use kadr_core::commands::{
    AddEffect, RemoveEffect, ReplaceEffect, SetClipCrop, SetClipPropertyValue, SetClipSpeed,
    SetClipText, SetKeyframe,
};
use std::path::PathBuf;

use kadr_core::effect::{ColorAdjustment, ColorLut, EffectKind};
use kadr_core::error::EditError;
use kadr_core::history::EditSession;
use kadr_core::ids::{ClipId, EffectId};
use kadr_core::project::Project;
use kadr_core::time::Timestamp;
use serde::Deserialize;

use super::{EditOutcome, keyframes};

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
    /// The clip property this value is.
    pub(super) fn property(self) -> AnimatableProperty {
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

/// The colour LUT settings to change. A setting left out keeps its current value.
///
/// The file never comes from the front end: `choose_clip_lut_file` reads it first,
/// then fills `file_path`. A request naming `filePath` is refused like any unknown
/// name.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LutChanges {
    /// The `.cube` file, already read by the back end.
    #[serde(skip_deserializing)]
    pub file_path: Option<PathBuf>,
    /// From 0 (no look) to 1 (the full look).
    pub strength: Option<f32>,
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

/// Sets one value of a clip. When the value is animated (as the project is when
/// the change is applied), it sets the keyframe at `time` (the playhead)
/// instead, adding one there if needed: the person changes what they see.
///
/// # Errors
///
/// When the clip does not exist, the value is out of range, or it is animated
/// and `time` is missing or not on the clip.
pub fn set_clip_value(
    session: &mut EditSession,
    clip_id: ClipId,
    value: ClipValue,
    amount: f32,
    time: Option<Timestamp>,
) -> Result<EditOutcome, EditError> {
    let property = value.property();
    session.transaction(value.undo_label(), |transaction| {
        let keyframe = {
            let (_, clip) = find_clip(transaction.project(), clip_id)?;
            match (clip.animated_property(property).is_animated(), time) {
                (true, Some(time)) => {
                    let offset = keyframes::offset_on_clip(clip, time)?;
                    Some(keyframes::keyframe_with_value(
                        clip, property, offset, amount,
                    ))
                }
                (true, None) => return Err(EditError::PropertyIsAnimated { clip_id, property }),
                (false, _) => None,
            }
        };
        match keyframe {
            Some(keyframe) => transaction.execute(SetKeyframe::new(clip_id, property, keyframe))?,
            None => transaction.execute(SetClipPropertyValue::new(clip_id, property, amount))?,
        };
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

/// Removes the clip's colour adjustment and its colour LUT: back to the picture
/// as it was ("off until first touched").
///
/// # Errors
///
/// When the clip does not exist.
pub fn reset_clip_color(
    session: &mut EditSession,
    clip_id: ClipId,
) -> Result<EditOutcome, EditError> {
    let adjustment = color_adjustment_of(session.project(), clip_id)?;
    let lut = color_lut_of(session.project(), clip_id)?;
    session.transaction("resetColor", |transaction| {
        let effect_ids = adjustment
            .map(|(effect_id, _)| effect_id)
            .into_iter()
            .chain(lut.map(|(effect_id, _)| effect_id));
        for effect_id in effect_ids {
            transaction.execute(RemoveEffect::new(clip_id, effect_id))?;
        }
        Ok(EditOutcome::default())
    })
}

/// Changes the clip's colour LUT: the settings given change, the others stay as
/// they are. A file given adds a LUT (at full strength unless one is given) to a
/// clip that has none; a strength alone changes nothing on such a clip.
///
/// # Errors
///
/// When the clip does not exist, or the change is refused (a strength outside
/// 0–1, a clip without pictures).
pub fn set_clip_lut(
    session: &mut EditSession,
    clip_id: ClipId,
    changes: &LutChanges,
) -> Result<EditOutcome, EditError> {
    let label = if changes.file_path.is_some() {
        "chooseLut"
    } else {
        "changeLutStrength"
    };
    session.transaction(label, |transaction| {
        let existing = color_lut_of(transaction.project(), clip_id)?;
        match (existing, &changes.file_path) {
            (Some((effect_id, current)), _) => {
                let changed = ColorLut {
                    file_path: changes.file_path.clone().unwrap_or(current.file_path),
                    strength_fraction: changes.strength.unwrap_or(current.strength_fraction),
                };
                transaction.execute(ReplaceEffect::new(
                    clip_id,
                    effect_id,
                    EffectKind::ColorLut(changed),
                ))?;
            }
            (None, Some(file_path)) => {
                let added = ColorLut {
                    file_path: file_path.clone(),
                    strength_fraction: changes.strength.unwrap_or(1.0),
                };
                transaction.execute(AddEffect::new(clip_id, EffectKind::ColorLut(added)))?;
            }
            // The LUT was removed meanwhile (by Claude, or an undo): nothing to change.
            (None, None) => {}
        }
        Ok(EditOutcome::default())
    })
}

/// Removes the clip's colour LUT.
///
/// # Errors
///
/// When the clip does not exist.
pub fn remove_clip_lut(
    session: &mut EditSession,
    clip_id: ClipId,
) -> Result<EditOutcome, EditError> {
    let existing = color_lut_of(session.project(), clip_id)?;
    session.transaction("removeLut", |transaction| {
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

/// The clip's first colour LUT effect and its settings, if it has one.
fn color_lut_of(
    project: &Project,
    clip_id: ClipId,
) -> Result<Option<(EffectId, ColorLut)>, EditError> {
    let (_, clip) = find_clip(project, clip_id)?;
    Ok(clip.effects.iter().find_map(|effect| match &effect.kind {
        EffectKind::ColorLut(color_lut) => Some((effect.id, color_lut.clone())),
        EffectKind::ColorAdjustment(_) | EffectKind::TrackedBlur(_) => None,
    }))
}

/// The clip's first colour adjustment effect and its settings, if it has one.
fn color_adjustment_of(
    project: &Project,
    clip_id: ClipId,
) -> Result<Option<(EffectId, ColorAdjustment)>, EditError> {
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

/// The clip and its track, or the error naming the clips there are.
pub(super) fn find_clip(
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
