//! The settings of a tracked blur (docs/ux.md, "Blurring a face"): style,
//! strength, shape and margin, turning it off and on, and removing it.
//!
//! The region samples are never touched here: only the tracking analysis
//! (`tracking_job.rs`) writes them.
//!
//! A change names only the settings it changes ([`BlurChanges`]): the others are
//! read from the project as it is when the change is applied, so two quick
//! changes never undo each other (the same rule as `clip_settings.rs`).

use kadr_core::color::{Color, ColorParseError};
use kadr_core::commands::{RemoveEffect, ReplaceEffect, SetEffectEnabled};
use kadr_core::effect::{BlurShape, BlurStyle, EffectKind, TrackedBlur};
use kadr_core::error::EditError;
use kadr_core::history::EditSession;
use kadr_core::ids::{ClipId, EffectId};
use kadr_core::project::Project;
use kadr_core::validation::ValidationError;
use serde::{Deserialize, Serialize};

use super::EditOutcome;

/// How a region is hidden, in the inspector's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BlurStyleChoice {
    /// A strong soft blur.
    Blur,
    /// Big squares.
    Pixels,
    /// A plain colour.
    Mask,
}

/// Shape of the hidden area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BlurShapeChoice {
    /// An ellipse inside the box, for faces.
    Ellipse,
    /// The box itself, for plates and screens.
    Rectangle,
}

impl From<BlurShapeChoice> for BlurShape {
    fn from(choice: BlurShapeChoice) -> Self {
        match choice {
            BlurShapeChoice::Ellipse => BlurShape::Ellipse,
            BlurShapeChoice::Rectangle => BlurShape::Rectangle,
        }
    }
}

/// Everything the inspector shows and changes about a tracked blur.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlurSettings {
    /// Blur, pixels or mask.
    pub style: BlurStyleChoice,
    /// Blur radius or square size, in source pixels (kept for the mask, unused).
    pub strength: f32,
    /// The mask's colour, `#RRGGBB`.
    pub mask_color: String,
    /// Ellipse or rectangle.
    pub shape: BlurShapeChoice,
    /// Extra hidden space around the box, in percent of its size.
    pub margin_in_percent: f32,
}

/// A change of some of a blur's settings; the others stay as they are in the
/// project. Unknown fields are refused, so a misspelt setting is not silently
/// ignored.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BlurChanges {
    /// Blur, pixels or mask.
    pub style: Option<BlurStyleChoice>,
    /// Blur radius or square size, in source pixels.
    pub strength: Option<f32>,
    /// The mask's colour, `#RRGGBB`.
    pub mask_color: Option<String>,
    /// Ellipse or rectangle.
    pub shape: Option<BlurShapeChoice>,
    /// Extra hidden space around the box, in percent of its size.
    pub margin_in_percent: Option<f32>,
}

impl BlurChanges {
    /// `settings` with these changes.
    fn applied_to(&self, settings: BlurSettings) -> BlurSettings {
        BlurSettings {
            style: self.style.unwrap_or(settings.style),
            strength: self.strength.unwrap_or(settings.strength),
            mask_color: self.mask_color.clone().unwrap_or(settings.mask_color),
            shape: self.shape.unwrap_or(settings.shape),
            margin_in_percent: self.margin_in_percent.unwrap_or(settings.margin_in_percent),
        }
    }
}

/// The strength shown for a mask, which has none: the blur radius it gets back
/// if the style is switched to blur.
const DEFAULT_BLUR_RADIUS_IN_PIXELS: f32 = 30.0;

impl BlurSettings {
    /// The settings of `blur`.
    pub fn of(blur: &TrackedBlur) -> Self {
        let (style, strength, mask_color) = match blur.style {
            BlurStyle::GaussianBlur { radius_in_pixels } => {
                (BlurStyleChoice::Blur, radius_in_pixels, Color::BLACK)
            }
            BlurStyle::Pixelate {
                block_size_in_pixels,
            } => (
                BlurStyleChoice::Pixels,
                block_size_in_pixels as f32,
                Color::BLACK,
            ),
            BlurStyle::SolidFill { color } => {
                (BlurStyleChoice::Mask, DEFAULT_BLUR_RADIUS_IN_PIXELS, color)
            }
        };
        Self {
            style,
            strength,
            mask_color: mask_color.to_hex().chars().take(7).collect(),
            shape: match blur.shape {
                BlurShape::Ellipse => BlurShapeChoice::Ellipse,
                BlurShape::Rectangle => BlurShapeChoice::Rectangle,
            },
            margin_in_percent: blur.margin_in_percent,
        }
    }

    /// `blur` with these settings; its samples are kept.
    ///
    /// # Errors
    ///
    /// When the mask colour is not `#RRGGBB`.
    pub fn applied_to(&self, blur: &TrackedBlur) -> Result<TrackedBlur, ColorParseError> {
        let style = match self.style {
            BlurStyleChoice::Blur => BlurStyle::GaussianBlur {
                radius_in_pixels: self.strength,
            },
            BlurStyleChoice::Pixels => BlurStyle::Pixelate {
                block_size_in_pixels: self.strength.round().max(0.0) as u32,
            },
            BlurStyleChoice::Mask => BlurStyle::SolidFill {
                color: Color::from_hex(&self.mask_color)?,
            },
        };
        Ok(TrackedBlur {
            style,
            shape: self.shape.into(),
            margin_in_percent: self.margin_in_percent,
            ..blur.clone()
        })
    }
}

/// The tracked blur `effect_id` of clip `clip_id`.
///
/// # Errors
///
/// When the clip or the effect does not exist, or the effect is not a tracked blur.
pub fn tracked_blur_of(
    project: &Project,
    clip_id: ClipId,
    effect_id: EffectId,
) -> Result<&TrackedBlur, EditError> {
    let (_, clip) = project.require_clip(clip_id)?;
    let effect = clip
        .effects
        .iter()
        .find(|effect| effect.id == effect_id)
        .ok_or_else(|| EditError::EffectNotFound {
            clip_id,
            effect_id,
            existing_effect_ids: clip.effects.iter().map(|effect| effect.id).collect(),
        })?;
    match &effect.kind {
        EffectKind::TrackedBlur(blur) => Ok(blur),
        EffectKind::ColorAdjustment(_) => Err(invalid_blur(
            clip_id,
            effect_id,
            "it is not a tracked blur".to_string(),
        )),
    }
}

fn invalid_blur(clip_id: ClipId, effect_id: EffectId, reason: String) -> EditError {
    EditError::WouldMakeProjectInvalid(ValidationError::InvalidEffect {
        clip_id,
        effect_id,
        reason,
    })
}

/// Changes some of a blur's settings (one undo step); the others are read from
/// the project now, not from what the interface last showed.
///
/// # Errors
///
/// When the blur does not exist or a value is out of range.
pub fn set_blur_settings(
    session: &mut EditSession,
    clip_id: ClipId,
    effect_id: EffectId,
    changes: &BlurChanges,
) -> Result<EditOutcome, EditError> {
    let blur = tracked_blur_of(session.project(), clip_id, effect_id)?;
    let changed = changes
        .applied_to(BlurSettings::of(blur))
        .applied_to(blur)
        .map_err(|error| invalid_blur(clip_id, effect_id, error.to_string()))?;
    session.transaction("changeBlur", |transaction| {
        transaction.execute(ReplaceEffect::new(
            clip_id,
            effect_id,
            EffectKind::TrackedBlur(changed),
        ))?;
        Ok(EditOutcome::default())
    })
}

/// Turns a blur off (kept, not drawn) or back on.
///
/// # Errors
///
/// When the blur does not exist.
pub fn set_blur_enabled(
    session: &mut EditSession,
    clip_id: ClipId,
    effect_id: EffectId,
    is_enabled: bool,
) -> Result<EditOutcome, EditError> {
    tracked_blur_of(session.project(), clip_id, effect_id)?;
    let label = if is_enabled { "showBlur" } else { "hideBlur" };
    session.transaction(label, |transaction| {
        transaction.execute(SetEffectEnabled::new(clip_id, effect_id, is_enabled))?;
        Ok(EditOutcome::default())
    })
}

/// Removes a blur (undoable).
///
/// # Errors
///
/// When the blur does not exist.
pub fn remove_blur(
    session: &mut EditSession,
    clip_id: ClipId,
    effect_id: EffectId,
) -> Result<EditOutcome, EditError> {
    tracked_blur_of(session.project(), clip_id, effect_id)?;
    session.transaction("removeBlur", |transaction| {
        transaction.execute(RemoveEffect::new(clip_id, effect_id))?;
        Ok(EditOutcome::default())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_go_both_ways_and_keep_the_samples() {
        let mut blur = TrackedBlur::with_default_style();
        blur.region_samples = Vec::new();
        let settings = BlurSettings::of(&blur);
        assert_eq!(settings.style, BlurStyleChoice::Blur);
        assert_eq!(settings.strength, 30.0);
        assert_eq!(settings.shape, BlurShapeChoice::Ellipse);
        assert_eq!(settings.mask_color, "#000000");

        let pixels = BlurSettings {
            style: BlurStyleChoice::Pixels,
            strength: 16.4,
            shape: BlurShapeChoice::Rectangle,
            margin_in_percent: 35.0,
            ..settings.clone()
        }
        .applied_to(&blur)
        .unwrap();
        assert_eq!(
            pixels.style,
            BlurStyle::Pixelate {
                block_size_in_pixels: 16
            }
        );
        assert_eq!(pixels.shape, BlurShape::Rectangle);
        assert_eq!(pixels.margin_in_percent, 35.0);
        assert_eq!(pixels.region_samples, blur.region_samples);

        let mask = BlurSettings {
            style: BlurStyleChoice::Mask,
            mask_color: "#FF8800".to_string(),
            ..settings
        }
        .applied_to(&blur)
        .unwrap();
        assert_eq!(BlurSettings::of(&mask).mask_color, "#FF8800");
    }

    #[test]
    fn a_change_keeps_the_settings_it_does_not_name() {
        let settings = BlurSettings::of(&TrackedBlur::with_default_style());
        let wider = BlurChanges {
            margin_in_percent: Some(40.0),
            ..BlurChanges::default()
        }
        .applied_to(settings.clone());
        assert_eq!(wider.margin_in_percent, 40.0);
        assert_eq!(
            BlurSettings {
                margin_in_percent: settings.margin_in_percent,
                ..wider
            },
            settings
        );
    }

    #[test]
    fn misspelt_settings_are_refused() {
        let misspelt = serde_json::from_value::<BlurChanges>(serde_json::json!({ "margin": 40 }));
        assert!(misspelt.is_err());
        let fine: BlurChanges =
            serde_json::from_value(serde_json::json!({ "marginInPercent": 40 })).unwrap();
        assert_eq!(fine.margin_in_percent, Some(40.0));
    }
}
