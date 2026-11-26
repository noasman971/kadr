//! Adding a finished analysis to the open project (docs/ux.md, "Blurring a
//! face"): a new tracked blur, or a corrected one, each as one undo step.
//!
//! A correction is merged with the blur **as it is now** (its style may have
//! changed during the analysis) by `kadr_engine::corrected_tracked_blur`: the
//! analysed stretch, between the neighbouring anchors, keeps the surer answer
//! frame by frame (docs/tracking-and-blur.md, "Correcting a tracked blur").

use kadr_core::commands::{AddEffect, CreatedObjectsExt, ReplaceEffect};
use kadr_core::effect::{EffectKind, TrackedBlur};
use kadr_core::error::EditError;
use kadr_core::ids::{ClipId, EffectId};
use kadr_engine::{RegionTrackingReport, RegionTrackingRequest, corrected_tracked_blur};

use crate::command_error::CommandResult;
use crate::editing::tracked_blur_of;
use crate::open_project::OpenProject;
use crate::tracking_job::TrackingEvent;

/// Undo label of a blur added by following a box.
pub const BLUR_REGION_UNDO_LABEL: &str = "blurRegion";
/// Undo label of a blur corrected from a redrawn box.
pub const CORRECT_BLUR_UNDO_LABEL: &str = "correctBlur";

/// What an analysis is for.
#[derive(Debug, Clone, PartialEq)]
pub enum TrackingPurpose {
    /// A new tracked blur, in this style (its samples are empty).
    NewBlur {
        /// Style, shape and margin of the new blur.
        style: TrackedBlur,
    },
    /// The correction of an existing tracked blur from a redrawn box.
    Correction {
        /// The blur corrected.
        effect_id: EffectId,
    },
}

/// Adds the analysis `report` of `request` to the project, as one undo step.
///
/// # Errors
///
/// When the clip or the corrected blur no longer exists, or the engine refuses
/// the change; the project is then unchanged.
pub fn add_analysis_to_project(
    open_project: &mut OpenProject,
    clip_id: ClipId,
    purpose: &TrackingPurpose,
    request: &RegionTrackingRequest,
    report: &RegionTrackingReport,
) -> CommandResult<TrackingEvent> {
    let (effect_id, blur) = match purpose {
        TrackingPurpose::NewBlur { style } => {
            let blur = report.tracked_blur_like(style);
            let kind = EffectKind::TrackedBlur(blur.clone());
            let effect_id = open_project.edit(|session, _| {
                session.transaction(BLUR_REGION_UNDO_LABEL, |transaction| {
                    transaction
                        .execute(AddEffect::new(clip_id, kind))?
                        .first_effect_id()
                        .ok_or_else(|| EditError::InternalInconsistency {
                            details: "the blur created was not reported".to_string(),
                        })
                })
            })?;
            (effect_id, blur)
        }
        TrackingPurpose::Correction { effect_id } => {
            let current = tracked_blur_of(open_project.project(), clip_id, *effect_id)?;
            let corrected = corrected_tracked_blur(current, request, report);
            let kind = EffectKind::TrackedBlur(corrected.clone());
            open_project.edit(|session, _| {
                session.transaction(CORRECT_BLUR_UNDO_LABEL, |transaction| {
                    transaction.execute(ReplaceEffect::new(clip_id, *effect_id, kind))?;
                    Ok(())
                })
            })?;
            (*effect_id, corrected)
        }
    };
    Ok(TrackingEvent::finished(
        open_project.view(),
        clip_id,
        effect_id,
        matches!(purpose, TrackingPurpose::Correction { .. }),
        &blur.unsure_ranges(),
    ))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use kadr_core::effect::{PixelRegion, RegionSampleOrigin, TrackingConfidence};
    use kadr_core::media_asset::{MediaAssetDescription, MediaKind, PictureSize};
    use kadr_core::project::ProjectSettings;
    use kadr_core::time::{FrameRate, TimeSpan, Timestamp};
    use kadr_engine::region_tracking::TrackedFrame;

    use super::*;
    use crate::editing::{EditRequest, apply_edit_request, import_media_files};

    const FRAME_RATE: FrameRate = FrameRate::FPS_30;

    fn project_with_a_video(folder: &std::path::Path) -> (OpenProject, ClipId) {
        let mut open_project = OpenProject::create(
            folder.join("Blur.kadr"),
            ProjectSettings::landscape_1080p(),
            false,
        )
        .unwrap();
        let video = MediaAssetDescription {
            file_path: PathBuf::from("C:/videos/interview.mp4"),
            display_name: "interview.mp4".to_string(),
            kind: MediaKind::Video,
            duration: Some(TimeSpan::from_whole_seconds(4)),
            picture_size: Some(PictureSize {
                width_in_pixels: 1920,
                height_in_pixels: 1080,
            }),
            frame_rate: Some(FRAME_RATE),
            audio_format: None,
        };
        let media_ids = open_project
            .edit(|session, _| import_media_files(session, vec![video]))
            .unwrap();
        let outcome = open_project
            .edit(|session, adopts| {
                apply_edit_request(
                    session,
                    adopts,
                    &EditRequest::AddMediaAtEnd {
                        media_id: media_ids[0],
                    },
                )
            })
            .unwrap();
        (open_project, outcome.created_clip_ids[0])
    }

    fn region_at_left(left: f32) -> PixelRegion {
        PixelRegion {
            left_in_pixels: left,
            top_in_pixels: 200.0,
            width_in_pixels: 120.0,
            height_in_pixels: 150.0,
        }
    }

    /// An analysis of `request`'s frames: left 300 everywhere, `confidence_at`
    /// each frame, the selection kept at its frame.
    fn report_of(
        request: &RegionTrackingRequest,
        confidence_at: impl Fn(u64) -> TrackingConfidence,
    ) -> RegionTrackingReport {
        let first = FRAME_RATE.frame_index_at(request.source_start);
        let last = FRAME_RATE
            .frame_count_covering(TimeSpan::from_ticks(request.source_end.ticks()))
            .saturating_sub(1);
        let selected = FRAME_RATE.frame_index_at(request.selection_time);
        let frames: Vec<TrackedFrame> = (first..=last)
            .map(|frame_index| TrackedFrame {
                source_time: FRAME_RATE.timestamp_of_frame(frame_index),
                region: if frame_index == selected {
                    request.selection
                } else {
                    region_at_left(300.0)
                },
                confidence: confidence_at(frame_index),
                origin: if frame_index == selected {
                    request.selection_origin
                } else {
                    RegionSampleOrigin::Tracker
                },
                peak_to_sidelobe_ratio: Some(20.0),
            })
            .collect();
        let region_samples = frames
            .iter()
            .map(|frame| kadr_core::effect::RegionSample {
                source_time: frame.source_time,
                region: frame.region,
                confidence: frame.confidence,
                origin: frame.origin,
            })
            .collect();
        RegionTrackingReport {
            frames,
            region_samples,
            unsure_ranges: Vec::new(),
            shrink_factor: 3,
        }
    }

    #[test]
    fn a_new_blur_then_a_correction_are_two_undo_steps() {
        let folder = tempfile::tempdir().unwrap();
        let (mut open_project, clip_id) = project_with_a_video(folder.path());

        // Selected at 0.5 s, lost from 2 s to the end.
        let request = RegionTrackingRequest::for_clip(
            open_project.project(),
            clip_id,
            Timestamp::from_milliseconds(500),
            region_at_left(300.0),
        )
        .unwrap();
        let report = report_of(&request, |frame| {
            if frame >= 60 {
                TrackingConfidence::Lost
            } else {
                TrackingConfidence::Confident
            }
        });
        let purpose = TrackingPurpose::NewBlur {
            style: TrackedBlur::with_default_style(),
        };
        let TrackingEvent::Finished {
            effect_id,
            lost_range_count,
            is_correction,
            ..
        } = add_analysis_to_project(&mut open_project, clip_id, &purpose, &request, &report)
            .unwrap()
        else {
            panic!("expected a finished event");
        };
        assert_eq!((lost_range_count, is_correction), (1, false));
        assert_eq!(
            open_project.view().undo_label.as_deref(),
            Some("blurRegion")
        );

        // Corrected at 2.5 s: confident from there on.
        let correction = RegionTrackingRequest::for_correction(
            open_project.project(),
            clip_id,
            effect_id,
            Timestamp::from_milliseconds(2_500),
            region_at_left(900.0),
        )
        .unwrap();
        assert_eq!(correction.source_start, FRAME_RATE.timestamp_of_frame(15));
        let report = report_of(&correction, |frame| {
            if frame >= 70 {
                TrackingConfidence::Confident
            } else {
                TrackingConfidence::Lost
            }
        });
        let purpose = TrackingPurpose::Correction { effect_id };
        let TrackingEvent::Finished {
            lost_range_count,
            is_correction,
            ..
        } = add_analysis_to_project(&mut open_project, clip_id, &purpose, &correction, &report)
            .unwrap()
        else {
            panic!("expected a finished event");
        };
        assert!(is_correction);
        assert_eq!(lost_range_count, 1);
        assert_eq!(
            open_project.view().undo_label.as_deref(),
            Some("correctBlur")
        );
        let blur = tracked_blur_of(open_project.project(), clip_id, effect_id).unwrap();
        let (region, confidence) = blur.region_at(FRAME_RATE.timestamp_of_frame(80)).unwrap();
        assert_eq!(confidence, TrackingConfidence::Confident);
        assert_eq!(region.left_in_pixels, 300.0);
        // The redrawn box is kept exactly.
        assert_eq!(
            blur.region_at(FRAME_RATE.timestamp_of_frame(75)).unwrap().0,
            region_at_left(900.0)
        );
        assert_eq!(
            blur.region_samples
                .iter()
                .filter(|sample| sample.origin == RegionSampleOrigin::ManualCorrection)
                .count(),
            1
        );

        // Undo removes the correction only.
        open_project.undo().unwrap();
        let blur = tracked_blur_of(open_project.project(), clip_id, effect_id).unwrap();
        assert_eq!(
            blur.region_at(FRAME_RATE.timestamp_of_frame(75)).unwrap().1,
            TrackingConfidence::Lost
        );
    }
}
