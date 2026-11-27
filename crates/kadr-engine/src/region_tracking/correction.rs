//! Correcting a tracked blur: the region redrawn at one moment, and the tracking
//! started again from there.
//!
//! Where the tracker was unsure or wrong, the user (or Claude) redraws the region
//! on one frame. The analysis then runs again from that frame, forwards and
//! backwards, but only as far as the nearest **anchors** on each side: the first
//! selection and the earlier corrections, the frames where someone said where
//! the region is. Beyond them, nothing changes.
//!
//! Within that stretch, each frame takes the region of the new analysis unless
//! the old one was surer there (confident beats uncertain, which beats lost).
//! When both are as sure, the analysis started from the nearer anchor wins: near
//! the correction, the new one; near an older anchor, the old one. So a
//! correction can only make the blur surer, and a good part of the old track
//! stays as it was.

use kadr_core::effect::{
    EffectKind, PixelRegion, RegionSampleOrigin, TrackedBlur, TrackingConfidence,
};
use kadr_core::error::EditError;
use kadr_core::ids::{ClipId, EffectId};
use kadr_core::project::Project;
use kadr_core::time::{FrameRate, TimeSpan, Timestamp};

use super::track_samples::simplified_samples;
use super::{
    FrameRange, RegionTrackingError, RegionTrackingReport, RegionTrackingRequest, TrackedFrame,
};

impl RegionTrackingRequest {
    /// The request to correct the tracked blur `effect_id` of clip `clip_id`:
    /// `corrected_region` is where the region really is in the clip's picture
    /// as shown at `timeline_time`, in the file's pixels.
    ///
    /// Only the stretch between the nearest anchors around that moment (the
    /// first selection, earlier corrections, or the ends of the clip) is
    /// analysed again. Merge the report with [`corrected_tracked_blur`].
    ///
    /// # Errors
    ///
    /// The errors of [`RegionTrackingRequest::for_clip`], and
    /// [`RegionTrackingError::NotATrackedBlur`] when the clip has no tracked
    /// blur with this identifier.
    pub fn for_correction(
        project: &Project,
        clip_id: ClipId,
        effect_id: EffectId,
        timeline_time: Timestamp,
        corrected_region: PixelRegion,
    ) -> Result<Self, RegionTrackingError> {
        let blur = find_tracked_blur(project, clip_id, effect_id)?;
        let mut request = Self::for_clip(project, clip_id, timeline_time, corrected_region)?;
        request.selection_origin = RegionSampleOrigin::ManualCorrection;
        let clip_frames = FrameRange::of(&request);
        let frame_rate = request.frame_rate;
        let (previous_anchor, next_anchor) = anchors_around(blur, frame_rate, clip_frames.selected);
        if let Some(previous) = previous_anchor.filter(|previous| *previous > clip_frames.first) {
            request.source_start = frame_rate.timestamp_of_frame(previous);
        }
        if let Some(next) = next_anchor.filter(|next| *next < clip_frames.last) {
            // One tick past the anchor's start: the anchor's frame is the last one analysed.
            request.source_end = frame_rate.timestamp_of_frame(next) + TimeSpan::from_ticks(1);
        }
        Ok(request)
    }
}

/// The tracked blur `effect_id` of clip `clip_id`.
///
/// # Errors
///
/// When the clip does not exist, has no effect with this identifier, or that
/// effect is not a tracked blur.
pub fn find_tracked_blur(
    project: &Project,
    clip_id: ClipId,
    effect_id: EffectId,
) -> Result<&TrackedBlur, RegionTrackingError> {
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
        _ => Err(RegionTrackingError::NotATrackedBlur { clip_id, effect_id }),
    }
}

/// `blur` with the analysis of a correction merged in (see the module
/// documentation). `request` is the one made by
/// [`RegionTrackingRequest::for_correction`] and `report` its analysis.
pub fn corrected_tracked_blur(
    blur: &TrackedBlur,
    request: &RegionTrackingRequest,
    report: &RegionTrackingReport,
) -> TrackedBlur {
    let (Some(first_frame), Some(last_frame)) = (report.frames.first(), report.frames.last())
    else {
        return blur.clone();
    };
    let frame_rate = request.frame_rate;
    let correction_frame = frame_rate.frame_index_at(request.selection_time);
    let (previous_anchor, next_anchor) = anchors_around(blur, frame_rate, correction_frame);
    let merged_frames: Vec<TrackedFrame> = report
        .frames
        .iter()
        .map(|new_frame| {
            let frame_index = frame_rate.frame_index_at(new_frame.source_time);
            let other_anchor = if frame_index < correction_frame {
                previous_anchor
            } else {
                next_anchor
            };
            merged_frame(blur, new_frame, frame_index, correction_frame, other_anchor)
        })
        .collect();

    let (span_start, span_end) = (first_frame.source_time, last_frame.source_time);
    let mut region_samples: Vec<_> = blur
        .region_samples
        .iter()
        .filter(|sample| sample.source_time < span_start)
        .copied()
        .collect();
    region_samples.extend(simplified_samples(&merged_frames));
    region_samples.extend(
        blur.region_samples
            .iter()
            .filter(|sample| sample.source_time > span_end)
            .copied(),
    );
    TrackedBlur {
        region_samples,
        ..blur.clone()
    }
}

/// The frame of the merged track at `frame_index`: the old anchor if there is
/// one there, otherwise the surer of the old and new answers, the nearer
/// anchor's answer when both are as sure.
fn merged_frame(
    blur: &TrackedBlur,
    new_frame: &TrackedFrame,
    frame_index: u64,
    correction_frame: u64,
    other_anchor: Option<u64>,
) -> TrackedFrame {
    if new_frame.origin != RegionSampleOrigin::Tracker {
        return *new_frame;
    }
    if let Some(anchor) = blur.region_samples.iter().find(|sample| {
        sample.origin != RegionSampleOrigin::Tracker && sample.source_time == new_frame.source_time
    }) {
        return TrackedFrame {
            source_time: anchor.source_time,
            region: anchor.region,
            confidence: anchor.confidence,
            origin: anchor.origin,
            peak_to_sidelobe_ratio: None,
        };
    }
    let Some((old_region, old_confidence)) = old_answer_at(blur, new_frame.source_time) else {
        return *new_frame;
    };
    let new_is_kept = match unsureness(new_frame.confidence).cmp(&unsureness(old_confidence)) {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Greater => false,
        std::cmp::Ordering::Equal => other_anchor.is_none_or(|anchor| {
            frame_index.abs_diff(correction_frame) <= frame_index.abs_diff(anchor)
        }),
    };
    if new_is_kept {
        return *new_frame;
    }
    TrackedFrame {
        source_time: new_frame.source_time,
        region: old_region,
        confidence: old_confidence,
        origin: RegionSampleOrigin::Tracker,
        peak_to_sidelobe_ratio: None,
    }
}

/// The old track at `source_time`: the sample there if there is one (its own
/// confidence, where [`TrackedBlur::region_at`] would take the less sure of it
/// and the next one), otherwise the interpolation between the samples around.
fn old_answer_at(
    blur: &TrackedBlur,
    source_time: Timestamp,
) -> Option<(PixelRegion, TrackingConfidence)> {
    blur.region_samples
        .iter()
        .find(|sample| sample.source_time == source_time)
        .map(|sample| (sample.region, sample.confidence))
        .or_else(|| blur.region_at(source_time))
}

/// The frames of the nearest anchors (selection or corrections) strictly
/// before and strictly after `frame_index`.
fn anchors_around(
    blur: &TrackedBlur,
    frame_rate: FrameRate,
    frame_index: u64,
) -> (Option<u64>, Option<u64>) {
    let anchor_frames = blur
        .region_samples
        .iter()
        .filter(|sample| sample.origin != RegionSampleOrigin::Tracker)
        .map(|sample| frame_rate.frame_index_at(sample.source_time));
    let mut previous = None;
    let mut next = None;
    for anchor in anchor_frames {
        if anchor < frame_index && previous.is_none_or(|previous| anchor > previous) {
            previous = Some(anchor);
        }
        if anchor > frame_index && next.is_none_or(|next| anchor < next) {
            next = Some(anchor);
        }
    }
    (previous, next)
}

/// 0 for confident, 1 for uncertain, 2 for lost.
fn unsureness(confidence: TrackingConfidence) -> u8 {
    match confidence {
        TrackingConfidence::Confident => 0,
        TrackingConfidence::Uncertain => 1,
        TrackingConfidence::Lost => 2,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use kadr_core::effect::RegionSample;
    use kadr_core::media_asset::PictureSize;

    use super::*;

    const FRAME_RATE: FrameRate = FrameRate::FPS_30;

    fn region_at_left(left: f32) -> PixelRegion {
        PixelRegion {
            left_in_pixels: left,
            top_in_pixels: 100.0,
            width_in_pixels: 80.0,
            height_in_pixels: 100.0,
        }
    }

    fn sample(
        frame_index: u64,
        left: f32,
        confidence: TrackingConfidence,
        origin: RegionSampleOrigin,
    ) -> RegionSample {
        RegionSample {
            source_time: FRAME_RATE.timestamp_of_frame(frame_index),
            region: region_at_left(left),
            confidence,
            origin,
        }
    }

    /// A blur selected at frame 10, confident until frame 40, lost from 41 to 90.
    fn blur_lost_after_frame_40() -> TrackedBlur {
        TrackedBlur {
            region_samples: vec![
                sample(
                    0,
                    100.0,
                    TrackingConfidence::Confident,
                    RegionSampleOrigin::Tracker,
                ),
                sample(
                    10,
                    110.0,
                    TrackingConfidence::Confident,
                    RegionSampleOrigin::UserSelection,
                ),
                sample(
                    40,
                    140.0,
                    TrackingConfidence::Confident,
                    RegionSampleOrigin::Tracker,
                ),
                sample(
                    41,
                    140.0,
                    TrackingConfidence::Lost,
                    RegionSampleOrigin::Tracker,
                ),
                sample(
                    90,
                    140.0,
                    TrackingConfidence::Lost,
                    RegionSampleOrigin::Tracker,
                ),
            ],
            ..TrackedBlur::with_default_style()
        }
    }

    fn correction_request(first: u64, selected: u64, last_excluded: u64) -> RegionTrackingRequest {
        RegionTrackingRequest {
            video_path: PathBuf::from("video.mp4"),
            frame_rate: FRAME_RATE,
            picture_size: PictureSize {
                width_in_pixels: 640,
                height_in_pixels: 360,
            },
            source_start: FRAME_RATE.timestamp_of_frame(first),
            source_end: FRAME_RATE.timestamp_of_frame(last_excluded),
            selection_time: FRAME_RATE.timestamp_of_frame(selected),
            selection: region_at_left(300.0),
            selection_origin: RegionSampleOrigin::ManualCorrection,
        }
    }

    /// The new analysis: `confidence_at(frame)` for each frame, a region at
    /// left 300 everywhere, the correction at `selected`.
    fn report_of(
        request: &RegionTrackingRequest,
        confidence_at: impl Fn(u64) -> TrackingConfidence,
    ) -> RegionTrackingReport {
        let range = FrameRange::of(request);
        let frames: Vec<TrackedFrame> = (range.first..=range.last)
            .map(|frame_index| TrackedFrame {
                source_time: FRAME_RATE.timestamp_of_frame(frame_index),
                region: region_at_left(300.0),
                confidence: confidence_at(frame_index),
                origin: if frame_index == range.selected {
                    RegionSampleOrigin::ManualCorrection
                } else {
                    RegionSampleOrigin::Tracker
                },
                peak_to_sidelobe_ratio: Some(20.0),
            })
            .collect();
        RegionTrackingReport {
            region_samples: simplified_samples(&frames),
            unsure_ranges: Vec::new(),
            frames,
            shrink_factor: 1,
        }
    }

    #[test]
    fn a_correction_in_a_lost_range_replaces_it_and_keeps_the_sure_part_before() {
        let blur = blur_lost_after_frame_40();
        // Corrected at frame 60; the previous anchor is the selection at 10.
        let request = correction_request(10, 60, 91);
        let report = report_of(&request, |frame| {
            if frame < 30 {
                TrackingConfidence::Lost
            } else {
                TrackingConfidence::Confident
            }
        });
        let corrected = corrected_tracked_blur(&blur, &request, &report);

        let left_at = |frame: u64| {
            corrected
                .region_at(FRAME_RATE.timestamp_of_frame(frame))
                .map(|(region, confidence)| (region.left_in_pixels, confidence))
                .unwrap()
        };
        // Before the selection: untouched.
        assert_eq!(left_at(0).0, 100.0);
        // The selection is kept as it was.
        assert_eq!(left_at(10), (110.0, TrackingConfidence::Confident));
        // 10 to 29: the old track was sure, the new one lost: the old one stays.
        assert_eq!(left_at(20).0, 120.0);
        // 30 to 34: both sure, nearer the selection (10) than the correction (60).
        assert_eq!(left_at(34).0, 134.0);
        // 35 to 40: both sure, nearer the correction (or as near).
        assert_eq!(left_at(38), (300.0, TrackingConfidence::Confident));
        // The lost range is gone: the new analysis was sure there.
        assert_eq!(left_at(70), (300.0, TrackingConfidence::Confident));
        assert!(corrected.unsure_ranges().is_empty());
        let corrections: Vec<_> = corrected
            .region_samples
            .iter()
            .filter(|sample| sample.origin == RegionSampleOrigin::ManualCorrection)
            .collect();
        assert_eq!(corrections.len(), 1);
        assert_eq!(
            corrections[0].source_time,
            FRAME_RATE.timestamp_of_frame(60)
        );
        assert!(
            corrected
                .region_samples
                .windows(2)
                .all(|pair| pair[0].source_time < pair[1].source_time)
        );
    }

    #[test]
    fn a_less_sure_new_analysis_does_not_replace_a_sure_old_one() {
        let blur = blur_lost_after_frame_40();
        let request = correction_request(10, 60, 91);
        let report = report_of(&request, |_| TrackingConfidence::Uncertain);
        let corrected = corrected_tracked_blur(&blur, &request, &report);
        let (region, confidence) = corrected
            .region_at(FRAME_RATE.timestamp_of_frame(39))
            .unwrap();
        assert_eq!(
            (region.left_in_pixels, confidence),
            (139.0, TrackingConfidence::Confident)
        );
        // Lost before, uncertain now: surer, so replaced.
        let (_, confidence) = corrected
            .region_at(FRAME_RATE.timestamp_of_frame(80))
            .unwrap();
        assert_eq!(confidence, TrackingConfidence::Uncertain);
    }

    #[test]
    fn anchors_are_found_on_each_side() {
        let mut blur = blur_lost_after_frame_40();
        blur.region_samples.push(sample(
            95,
            150.0,
            TrackingConfidence::Confident,
            RegionSampleOrigin::ManualCorrection,
        ));
        assert_eq!(anchors_around(&blur, FRAME_RATE, 60), (Some(10), Some(95)));
        assert_eq!(anchors_around(&blur, FRAME_RATE, 10), (None, Some(95)));
        assert_eq!(anchors_around(&blur, FRAME_RATE, 5), (None, Some(10)));
        assert_eq!(anchors_around(&blur, FRAME_RATE, 99), (Some(95), None));
    }

    #[test]
    fn the_analysed_stretch_ends_on_the_anchor_frames() {
        // The request for a correction at 60 between anchors 10 and 95 covers
        // frames 10 to 95, both included.
        let start = FRAME_RATE.timestamp_of_frame(10);
        let end = FRAME_RATE.timestamp_of_frame(95) + TimeSpan::from_ticks(1);
        let mut request = correction_request(0, 60, 0);
        request.source_start = start;
        request.source_end = end;
        let range = FrameRange::of(&request);
        assert_eq!((range.first, range.selected, range.last), (10, 60, 95));
    }
}
