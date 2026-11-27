//! Where a clip's source picture is on the canvas.
//!
//! Interfaces draw on top of the preview (a blur's box) and turn a pointer
//! position back into the source video's pixels, the coordinates a tracked blur
//! is stored in. [`clip_source_placement`] gives them the renderer's own mapping
//! for a clip at one moment, keyframe animations included, so what they draw sits
//! exactly on what the export shows.

use kadr_core::clip::ClipContent;
use kadr_core::error::EditError;
use kadr_core::ids::ClipId;
use kadr_core::media_asset::PictureSize;
use kadr_core::project::Project;
use kadr_core::time::Timestamp;
use kadr_render::LayerPlacement;

pub use kadr_render::SourceToCanvasMapping;

/// How a clip's source picture lands on the canvas at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipSourcePlacement {
    /// The size of the source picture, in pixels.
    pub picture_size: PictureSize,
    /// The moment of the source file shown.
    pub source_time: Timestamp,
    /// Source pixels to canvas pixels.
    pub mapping: SourceToCanvasMapping,
}

/// Where clip `clip_id`'s source picture is on the canvas at `timeline_time`.
///
/// `None` when the clip shows no source picture then: a title or a sound, a
/// moment outside the clip, or a clip its own crop or scale makes invisible.
///
/// # Errors
///
/// When the clip does not exist (the error lists the existing ones).
pub fn clip_source_placement(
    project: &Project,
    clip_id: ClipId,
    timeline_time: Timestamp,
) -> Result<Option<ClipSourcePlacement>, EditError> {
    let (_, clip) = project.require_clip(clip_id)?;
    let ClipContent::Media(segment) = &clip.content else {
        return Ok(None);
    };
    let Some(picture_size) = project
        .media_asset(segment.media_asset_id)
        .and_then(|asset| asset.description.picture_size)
    else {
        return Ok(None);
    };
    let (Some(source_time), Some(time_offset)) = (
        clip.source_time_at(timeline_time),
        timeline_time.span_since(clip.timeline_start),
    ) else {
        return Ok(None);
    };
    let placement = LayerPlacement::of_clip_at(clip, time_offset);
    Ok(placement
        .source_to_canvas_mapping(project.settings().canvas_size, picture_size)
        .map(|mapping| ClipSourcePlacement {
            picture_size,
            source_time,
            mapping,
        }))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use kadr_core::commands::{AddClip, AddTrack, CreatedObjectsExt, ImportMedia, TrackPosition};
    use kadr_core::history::EditSession;
    use kadr_core::media_asset::{MediaAssetDescription, MediaKind};
    use kadr_core::project::ProjectSettings;
    use kadr_core::time::{FrameRate, TimeSpan};
    use kadr_core::timeline::TrackKind;

    use super::*;

    #[test]
    fn a_fitted_video_maps_its_corners_onto_the_canvas() {
        let mut session =
            EditSession::new(Project::new(ProjectSettings::landscape_1080p())).unwrap();
        let media_id = session
            .execute(ImportMedia::new(MediaAssetDescription {
                file_path: PathBuf::from("C:/videos/small.mp4"),
                display_name: "small.mp4".to_string(),
                kind: MediaKind::Video,
                duration: Some(TimeSpan::from_whole_seconds(4)),
                picture_size: Some(PictureSize {
                    width_in_pixels: 960,
                    height_in_pixels: 540,
                }),
                frame_rate: Some(FrameRate::FPS_30),
                audio_format: None,
            }))
            .unwrap()
            .first_media_asset_id()
            .unwrap();
        let track_id = session
            .execute(AddTrack::new(TrackKind::Video, TrackPosition::Top))
            .unwrap()
            .first_track_id()
            .unwrap();
        let clip_id = session
            .execute(AddClip::media(
                track_id,
                media_id,
                Timestamp::ZERO,
                Timestamp::from_whole_seconds(4),
                Timestamp::ZERO,
            ))
            .unwrap()
            .first_clip_id()
            .unwrap();

        let placement =
            clip_source_placement(session.project(), clip_id, Timestamp::from_whole_seconds(1))
                .unwrap()
                .unwrap();
        assert_eq!(placement.source_time, Timestamp::from_whole_seconds(1));
        assert_eq!(placement.mapping.canvas_point_of(0.0, 0.0), (0.0, 0.0));
        assert_eq!(
            placement.mapping.canvas_point_of(960.0, 540.0),
            (1920.0, 1080.0)
        );
        assert!(
            clip_source_placement(session.project(), clip_id, Timestamp::from_whole_seconds(9))
                .unwrap()
                .is_none()
        );
    }
}
