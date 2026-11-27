//! Export sizes: the presets every interface offers, and drawing a project at
//! another size of the same shape.
//!
//! An export at another size of the **same shape** (a 1280 × 720 project
//! exported in 1920 × 1080) renders a scaled copy of the project: positions and
//! text sizes, measured in canvas pixels, grow with the canvas. Another shape
//! (a vertical export of a horizontal project) is refused here: it changes the
//! framing, so the interface first changes the project itself, as an edit the
//! user can see and undo.

use kadr_core::animation::Keyframe;
use kadr_core::clip::{AnimatableProperty, ClipContent};
use kadr_core::commands::{
    ChangeProjectSettings, SetClipPropertyValue, SetClipText, SetKeyframe, SetSubtitleStyle,
};
use kadr_core::error::EditError;
use kadr_core::history::{EditSession, Transaction};
use kadr_core::media_asset::PictureSize;
use kadr_core::project::Project;

/// A ready-made export size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExportPreset {
    /// 1920 × 1080 landscape.
    Youtube1080p,
    /// 3840 × 2160 landscape.
    Youtube4k,
    /// 1080 × 1920 vertical: TikTok, Instagram Reels, YouTube Shorts.
    VerticalReelsShorts,
    /// 1080 × 1080.
    Square1080,
}

impl ExportPreset {
    /// Every preset, in the order interfaces list them.
    pub const ALL: [Self; 4] = [
        Self::Youtube1080p,
        Self::Youtube4k,
        Self::VerticalReelsShorts,
        Self::Square1080,
    ];

    /// The picture size of the preset.
    pub const fn picture_size(self) -> PictureSize {
        let (width_in_pixels, height_in_pixels) = match self {
            Self::Youtube1080p => (1920, 1080),
            Self::Youtube4k => (3840, 2160),
            Self::VerticalReelsShorts => (1080, 1920),
            Self::Square1080 => (1080, 1080),
        };
        PictureSize {
            width_in_pixels,
            height_in_pixels,
        }
    }
}

/// Why a project cannot be exported at a size.
#[derive(Debug, thiserror::Error)]
pub enum ExportSizeError {
    /// The export size has another shape than the project's canvas.
    #[error(
        "the project is {}×{} but the export is {}×{}, another shape",
        project_size.width_in_pixels,
        project_size.height_in_pixels,
        export_size.width_in_pixels,
        export_size.height_in_pixels
    )]
    OtherShape {
        /// The project's canvas.
        project_size: PictureSize,
        /// The size asked for.
        export_size: PictureSize,
    },
    /// Scaling the project failed (the size is outside the engine's limits, for example).
    #[error(transparent)]
    Edit(#[from] EditError),
}

/// Whether two picture sizes have exactly the same shape.
pub fn have_same_shape(first: PictureSize, second: PictureSize) -> bool {
    u64::from(first.width_in_pixels) * u64::from(second.height_in_pixels)
        == u64::from(first.height_in_pixels) * u64::from(second.width_in_pixels)
}

/// The project to export at `export_size`: the project itself at its own size, or
/// a copy scaled to a bigger or smaller canvas of the same shape.
///
/// # Errors
///
/// [`ExportSizeError::OtherShape`] for another shape; [`ExportSizeError::Edit`]
/// when the engine refuses the scaled project.
pub fn project_at_export_size(
    project: &Project,
    export_size: PictureSize,
) -> Result<Project, ExportSizeError> {
    let project_size = project.settings().canvas_size;
    if export_size == project_size {
        return Ok(project.clone());
    }
    if !have_same_shape(export_size, project_size) {
        return Err(ExportSizeError::OtherShape {
            project_size,
            export_size,
        });
    }
    Ok(project_scaled_to_canvas(project, export_size)?)
}

/// A copy of `project` drawn on a canvas of `new_size` (same shape): positions
/// and text sizes, measured in canvas pixels, are scaled by the same factor.
fn project_scaled_to_canvas(
    project: &Project,
    new_size: PictureSize,
) -> Result<Project, EditError> {
    let old_size = project.settings().canvas_size;
    let mut settings = *project.settings();
    settings.canvas_size = new_size;
    let mut session = EditSession::new(project.clone()).map_err(EditError::from)?;
    session.transaction("Scale for export", |transaction| {
        transaction.execute(ChangeProjectSettings::new(settings))?;
        refit_clips_to_canvas(transaction, old_size, new_size)
    })?;
    Ok(session.into_project())
}

/// Moves and resizes what is measured in canvas pixels when the canvas goes from
/// `old_size` to `new_size`: horizontal positions follow the width, vertical ones
/// the height, and text sizes the smaller of the two changes, so titles keep
/// their place on the picture and still fit, like videos (which are fitted to the
/// canvas whatever its size). Video scales are relative to the canvas, blur
/// regions are in source pixels and crops are fractions: they stay as they are.
/// Subtitle styles are refitted with
/// [`SubtitleStyle::refitted_from_canvas_to_canvas`](kadr_core::subtitle::SubtitleStyle::refitted_from_canvas_to_canvas).
///
/// Call it inside the transaction that changes the canvas size.
///
/// # Errors
///
/// The engine's refusal of one of the changes (the transaction is then undone).
pub fn refit_clips_to_canvas(
    transaction: &mut Transaction<'_>,
    old_size: PictureSize,
    new_size: PictureSize,
) -> Result<(), EditError> {
    let horizontal_factor = new_size.width_in_pixels as f32 / old_size.width_in_pixels as f32;
    let vertical_factor = new_size.height_in_pixels as f32 / old_size.height_in_pixels as f32;
    let text_factor = horizontal_factor.min(vertical_factor);
    let clips: Vec<_> = transaction
        .project()
        .timeline()
        .all_clips()
        .map(|(_, clip)| clip.clone())
        .collect();
    for clip in clips {
        for (property, factor) in [
            (AnimatableProperty::PositionX, horizontal_factor),
            (AnimatableProperty::PositionY, vertical_factor),
        ] {
            let value = clip.animated_property(property);
            if value.is_animated() {
                for keyframe in value.keyframes() {
                    let scaled = Keyframe {
                        value: keyframe.value * factor,
                        ..*keyframe
                    };
                    transaction.execute(SetKeyframe::new(clip.id, property, scaled))?;
                }
            } else if value.constant_value != 0.0 {
                let scaled = value.constant_value * factor;
                transaction.execute(SetClipPropertyValue::new(clip.id, property, scaled))?;
            }
        }
        if let ClipContent::Text(text_content) = &clip.content {
            let mut style = text_content.style.clone();
            style.font_size_in_pixels *= text_factor;
            transaction
                .execute(SetClipText::new(clip.id, text_content.text.clone()).with_style(style))?;
        }
    }
    // Subtitles keep their size and margin relative to the defaults of the
    // canvas: on a vertical canvas they move clear of the apps' buttons.
    let subtitle_styles: Vec<_> = transaction
        .project()
        .timeline()
        .subtitle_tracks()
        .filter_map(|track| Some((track.id, track.subtitle_style.clone()?)))
        .collect();
    for (track_id, style) in subtitle_styles {
        let refitted = style.refitted_from_canvas_to_canvas(old_size, new_size);
        transaction.execute(SetSubtitleStyle::new(track_id, refitted))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kadr_core::clip::TextStyle;
    use kadr_core::commands::{AddClip, AddTrack, CreatedObjectsExt, TrackPosition};
    use kadr_core::project::ProjectSettings;
    use kadr_core::time::{TimeSpan, Timestamp};
    use kadr_core::timeline::TrackKind;

    /// A 1280 × 720 project with a title moved 100 px right, in 40 px type.
    fn project_with_a_title() -> (Project, kadr_core::ids::ClipId) {
        let mut settings = ProjectSettings::landscape_1080p();
        settings.canvas_size = PictureSize {
            width_in_pixels: 1280,
            height_in_pixels: 720,
        };
        let mut session = EditSession::new(Project::new(settings)).unwrap();
        let track_id = session
            .execute(AddTrack::new(TrackKind::Video, TrackPosition::Top))
            .unwrap()
            .first_track_id()
            .unwrap();
        let clip_id = session
            .execute(AddClip::text(
                track_id,
                "Lisbonne",
                TimeSpan::from_whole_seconds(3),
                Timestamp::ZERO,
            ))
            .unwrap()
            .first_clip_id()
            .unwrap();
        let style = TextStyle {
            font_size_in_pixels: 40.0,
            ..TextStyle::default()
        };
        session
            .execute(SetClipText::new(clip_id, "Lisbonne").with_style(style))
            .unwrap();
        session
            .execute(SetClipPropertyValue::new(
                clip_id,
                AnimatableProperty::PositionX,
                100.0,
            ))
            .unwrap();
        (session.into_project(), clip_id)
    }

    #[test]
    fn presets_have_their_sizes_and_shapes() {
        assert_eq!(ExportPreset::Youtube4k.picture_size().width_in_pixels, 3840);
        assert!(have_same_shape(
            ExportPreset::Youtube1080p.picture_size(),
            ExportPreset::Youtube4k.picture_size()
        ));
        assert!(!have_same_shape(
            ExportPreset::Youtube1080p.picture_size(),
            ExportPreset::VerticalReelsShorts.picture_size()
        ));
        assert_eq!(ExportPreset::ALL.len(), 4);
    }

    #[test]
    fn the_same_size_gives_the_project_itself() {
        let (project, _) = project_with_a_title();
        let exported = project_at_export_size(&project, project.settings().canvas_size).unwrap();
        assert_eq!(exported, project);
    }

    #[test]
    fn a_bigger_size_of_the_same_shape_scales_positions_and_text() {
        let (project, clip_id) = project_with_a_title();
        let exported =
            project_at_export_size(&project, ExportPreset::Youtube1080p.picture_size()).unwrap();
        assert_eq!(
            exported.settings().canvas_size,
            ExportPreset::Youtube1080p.picture_size()
        );
        let (_, clip) = exported.timeline().find_clip(clip_id).unwrap();
        let position_x = clip.animated_property(AnimatableProperty::PositionX);
        assert!((position_x.constant_value - 150.0).abs() < 0.001);
        let ClipContent::Text(text) = &clip.content else {
            panic!("the title is still a text clip");
        };
        assert!((text.style.font_size_in_pixels - 60.0).abs() < 0.001);
        // The project itself is untouched.
        assert_eq!(project.settings().canvas_size.width_in_pixels, 1280);
    }

    #[test]
    fn refitting_to_another_shape_keeps_titles_in_place_and_fitting() {
        let (project, clip_id) = project_with_a_title();
        let mut session = EditSession::new(project).unwrap();
        let old_size = session.project().settings().canvas_size;
        let new_size = ExportPreset::VerticalReelsShorts.picture_size();
        session
            .execute(SetClipPropertyValue::new(
                clip_id,
                AnimatableProperty::PositionY,
                270.0,
            ))
            .unwrap();
        session
            .transaction("Refit", |transaction| {
                let mut settings = *transaction.project().settings();
                settings.canvas_size = new_size;
                transaction.execute(ChangeProjectSettings::new(settings))?;
                refit_clips_to_canvas(transaction, old_size, new_size)
            })
            .unwrap();
        let (_, clip) = session.project().timeline().find_clip(clip_id).unwrap();
        // 1280 × 720 → 1080 × 1920: x × 0.84375, y × 2.666…, text × 0.84375.
        let x = clip
            .animated_property(AnimatableProperty::PositionX)
            .constant_value;
        let y = clip
            .animated_property(AnimatableProperty::PositionY)
            .constant_value;
        assert!((x - 84.375).abs() < 0.01);
        assert!((y - 720.0).abs() < 0.01);
        let ClipContent::Text(text) = &clip.content else {
            panic!("the title is still a text clip");
        };
        assert!((text.style.font_size_in_pixels - 33.75).abs() < 0.01);
    }

    #[test]
    fn another_shape_is_refused() {
        let (project, _) = project_with_a_title();
        let error =
            project_at_export_size(&project, ExportPreset::VerticalReelsShorts.picture_size())
                .unwrap_err();
        assert!(matches!(error, ExportSizeError::OtherShape { .. }));
        assert!(error.to_string().contains("1280×720"));
    }
}
