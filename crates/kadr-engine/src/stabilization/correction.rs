//! From a stored camera path to the correction of one frame.
//!
//! Everything here is computed from the effect alone, with no state kept
//! between frames: the correction of a frame only depends on the stored path,
//! the smoothing, the stored enlargement and the frame's time in the file.

use std::sync::Arc;

use kadr_core::clip::Clip;
use kadr_core::effect::{CameraPathSample, EffectKind, Stabilization};
use kadr_core::media_asset::PictureSize;
use kadr_core::time::Timestamp;
use kadr_render::SourceCorrection;
use kadr_vision::camera_path::{
    CameraPath, CameraPathPoint, CameraPose, fitted_zoom_factor, stabilizing_correction_at,
    stabilizing_corrections,
};

/// A stored camera path, read by the smoothing algorithm.
struct StoredPath<'a>(&'a [CameraPathSample]);

impl CameraPath for StoredPath<'_> {
    fn sample_count(&self) -> usize {
        self.0.len()
    }

    fn sample_at(&self, index: usize) -> CameraPathPoint {
        let sample = &self.0[index];
        CameraPathPoint {
            time_in_seconds: sample.source_time.as_seconds(),
            pose: CameraPose {
                shift_x_in_pixels: f64::from(sample.shift_x_in_pixels),
                shift_y_in_pixels: f64::from(sample.shift_y_in_pixels),
                rotation_in_degrees: f64::from(sample.rotation_in_degrees),
                scale_factor: f64::from(sample.scale_factor),
            },
            motion_was_measured: sample.motion_was_measured,
        }
    }
}

/// Width and height of the analysed pictures.
fn analysed_size_of(stabilization: &Stabilization) -> (f64, f64) {
    let size = stabilization.analysed_picture_size;
    (
        f64::from(size.width_in_pixels),
        f64::from(size.height_in_pixels),
    )
}

/// The same stabilisation with another smoothing, and the enlargement that
/// hides the picture's moving edges with it. The enlargement is always derived
/// this way, never set on its own. The camera path is shared, not copied.
pub fn stabilization_with_smoothing(
    stabilization: &Stabilization,
    smoothing_in_seconds: f32,
) -> Stabilization {
    let (width, height) = analysed_size_of(stabilization);
    let zoom_factor = if stabilization.camera_path.is_empty() {
        Stabilization::MINIMUM_ZOOM_FACTOR
    } else {
        fitted_zoom_factor(
            &StoredPath(&stabilization.camera_path),
            f64::from(smoothing_in_seconds),
            width,
            height,
        ) as f32
    };
    Stabilization {
        smoothing_in_seconds,
        zoom_factor: zoom_factor.clamp(
            Stabilization::MINIMUM_ZOOM_FACTOR,
            Stabilization::MAXIMUM_ZOOM_FACTOR,
        ),
        analysed_picture_size: stabilization.analysed_picture_size,
        camera_path: Arc::clone(&stabilization.camera_path),
    }
}

/// How one frame is moved to steady it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameCorrection {
    /// The move of the source picture inside its layer, enlargement included.
    pub source_correction: SourceCorrection,
    /// How much of the full correction is applied, from 0 to 1. Below 1 when
    /// the shake is larger than the enlargement can hide: the frame is then
    /// only partly steadied, rather than showing an edge.
    pub fraction_applied: f32,
}

/// The correction of the frame shown at `source_time` (a moment of the file),
/// for a decoded picture of `picture_size`.
///
/// The frame's sample is the last one at or before `source_time`
/// ([`Stabilization::sample_index_at`]): decode the frame at
/// [`Stabilization::frame_time_for`] so the picture and its correction match.
/// `picture_size` may differ from the analysed size (a proxy): shifts are
/// scaled to it.
pub fn stabilization_correction_at(
    stabilization: &Stabilization,
    source_time: Timestamp,
    picture_size: PictureSize,
) -> FrameCorrection {
    if stabilization.camera_path.is_empty() {
        return FrameCorrection {
            source_correction: SourceCorrection::IDENTITY,
            fraction_applied: 1.0,
        };
    }
    let (analysed_width, analysed_height) = analysed_size_of(stabilization);
    let fitted = stabilizing_correction_at(
        &StoredPath(&stabilization.camera_path),
        stabilization.sample_index_at(source_time),
        f64::from(stabilization.smoothing_in_seconds),
    )
    .fitted_inside_zoom(
        f64::from(stabilization.zoom_factor),
        analysed_width,
        analysed_height,
    );
    let horizontal_pixel_ratio = f64::from(picture_size.width_in_pixels) / analysed_width;
    let vertical_pixel_ratio = f64::from(picture_size.height_in_pixels) / analysed_height;
    FrameCorrection {
        source_correction: SourceCorrection {
            shift_x_in_pixels: (fitted.correction.shift_x_in_pixels * horizontal_pixel_ratio)
                as f32,
            shift_y_in_pixels: (fitted.correction.shift_y_in_pixels * vertical_pixel_ratio) as f32,
            rotation_in_degrees: fitted.correction.rotation_in_degrees as f32,
            scale_factor: fitted.correction.scale_factor as f32,
        },
        fraction_applied: fitted.fraction_applied as f32,
    }
}

/// The stabilisation `clip` applies: its first enabled one (the project's rules
/// allow one per clip).
pub(crate) fn applied_stabilization_of(clip: &Clip) -> Option<&Stabilization> {
    clip.effects
        .iter()
        .filter(|effect| effect.is_enabled)
        .find_map(|effect| match &effect.kind {
            EffectKind::Stabilization(stabilization) => Some(stabilization),
            _ => None,
        })
}

/// What a stabilisation does to its video, for reports to the user and Claude.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StabilizationSummary {
    /// The enlargement hiding the moving edges (1 = none).
    pub zoom_factor: f32,
    /// The largest move of a frame, in the analysed pictures' pixels.
    pub largest_shift_in_pixels: f32,
    /// The largest rotation of a frame, in degrees.
    pub largest_rotation_in_degrees: f32,
    /// Frames whose shake is larger than the enlargement hides: they are only
    /// partly steadied.
    pub partly_corrected_frame_count: usize,
    /// Frames, after the first, whose motion could not be measured.
    pub unmeasured_frame_count: usize,
}

/// Summarises `stabilization` over its whole analysed part.
pub fn stabilization_summary(stabilization: &Stabilization) -> StabilizationSummary {
    let (width, height) = analysed_size_of(stabilization);
    let zoom_factor = f64::from(stabilization.zoom_factor);
    let corrections = stabilizing_corrections(
        &StoredPath(&stabilization.camera_path),
        f64::from(stabilization.smoothing_in_seconds),
    );
    let mut summary = StabilizationSummary {
        zoom_factor: stabilization.zoom_factor,
        largest_shift_in_pixels: 0.0,
        largest_rotation_in_degrees: 0.0,
        partly_corrected_frame_count: 0,
        unmeasured_frame_count: stabilization
            .camera_path
            .iter()
            .skip(1)
            .filter(|sample| !sample.motion_was_measured)
            .count(),
    };
    for correction in &corrections {
        let shift = correction
            .shift_x_in_pixels
            .hypot(correction.shift_y_in_pixels) as f32;
        summary.largest_shift_in_pixels = summary.largest_shift_in_pixels.max(shift);
        summary.largest_rotation_in_degrees = summary
            .largest_rotation_in_degrees
            .max(correction.rotation_in_degrees.abs() as f32);
        let fitted = correction.fitted_inside_zoom(zoom_factor, width, height);
        if fitted.fraction_applied < 1.0 {
            summary.partly_corrected_frame_count += 1;
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use kadr_core::time::FrameRate;

    const SIZE: PictureSize = PictureSize {
        width_in_pixels: 1920,
        height_in_pixels: 1080,
    };

    /// A camera panning right at 60 px/s, with `shake_in_pixels` of vertical
    /// shake at 5 Hz, for three seconds at 30 frames per second.
    fn shaky_pan(shake_in_pixels: f32) -> Stabilization {
        let camera_path: Vec<CameraPathSample> = (0..90u64)
            .map(|frame_index| {
                let seconds = frame_index as f32 / 30.0;
                CameraPathSample {
                    source_time: FrameRate::FPS_30.timestamp_of_frame(frame_index),
                    shift_x_in_pixels: 60.0 * seconds,
                    shift_y_in_pixels: shake_in_pixels
                        * (2.0 * std::f32::consts::PI * 5.0 * seconds).sin(),
                    rotation_in_degrees: 0.0,
                    scale_factor: 1.0,
                    motion_was_measured: frame_index != 0,
                }
            })
            .collect();
        let unfitted = Stabilization {
            smoothing_in_seconds: Stabilization::DEFAULT_SMOOTHING_IN_SECONDS,
            zoom_factor: 1.0,
            analysed_picture_size: SIZE,
            camera_path: camera_path.into(),
        };
        stabilization_with_smoothing(&unfitted, Stabilization::DEFAULT_SMOOTHING_IN_SECONDS)
    }

    #[test]
    fn a_shake_is_cancelled_and_the_pan_is_kept() {
        let stabilization = shaky_pan(8.0);
        // Frame 45 is in the middle: the 5 Hz shake there is sin(2π·5·1.5) = 0,
        // so look at frame 46, where it is 8·sin(2π·5·46/30) ≈ −6.93 px.
        let time = FrameRate::FPS_30.timestamp_of_frame(46);
        let frame = stabilization_correction_at(&stabilization, time, SIZE);
        let zoom = stabilization.zoom_factor;
        assert_eq!(frame.fraction_applied, 1.0);
        // The content is 6.93 px too high there: the picture is moved back down
        // by as much (times the enlargement)…
        let expected_shift_y = 6.93 * zoom;
        assert!(
            (frame.source_correction.shift_y_in_pixels - expected_shift_y).abs() < 0.3,
            "{frame:?}"
        );
        // … and not along the pan.
        assert!(
            frame.source_correction.shift_x_in_pixels.abs() < 0.3,
            "{frame:?}"
        );
        assert_eq!(frame.source_correction.scale_factor, zoom);
        // 8 px of shake on 1080 lines needs about 1.5 % of enlargement, a little
        // more near the ends of the path.
        assert!(zoom > 1.01 && zoom < 1.04, "{zoom}");
    }

    #[test]
    fn a_smaller_decoded_picture_gets_smaller_shifts() {
        let stabilization = shaky_pan(8.0);
        let time = FrameRate::FPS_30.timestamp_of_frame(46);
        let full = stabilization_correction_at(&stabilization, time, SIZE);
        let half = stabilization_correction_at(
            &stabilization,
            time,
            PictureSize {
                width_in_pixels: 960,
                height_in_pixels: 540,
            },
        );
        let (full, half) = (full.source_correction, half.source_correction);
        assert!((half.shift_y_in_pixels - full.shift_y_in_pixels / 2.0).abs() < 1e-3);
        assert_eq!(half.scale_factor, full.scale_factor);
        assert_eq!(half.rotation_in_degrees, full.rotation_in_degrees);
    }

    #[test]
    fn the_first_and_last_frames_are_only_enlarged() {
        let stabilization = shaky_pan(8.0);
        for frame_index in [0, 89, 500] {
            let time = FrameRate::FPS_30.timestamp_of_frame(frame_index);
            let correction =
                stabilization_correction_at(&stabilization, time, SIZE).source_correction;
            assert!(correction.shift_x_in_pixels.abs() < 1e-3, "{correction:?}");
            assert!(correction.shift_y_in_pixels.abs() < 1e-3, "{correction:?}");
            assert_eq!(correction.scale_factor, stabilization.zoom_factor);
        }
    }

    #[test]
    fn a_shake_too_large_for_the_enlargement_is_partly_corrected() {
        let stabilization = shaky_pan(400.0);
        assert_eq!(
            stabilization.zoom_factor,
            Stabilization::MAXIMUM_ZOOM_FACTOR
        );
        let summary = stabilization_summary(&stabilization);
        assert!(summary.partly_corrected_frame_count > 0);
        assert!(summary.largest_shift_in_pixels > 100.0);
        assert_eq!(summary.unmeasured_frame_count, 0);
        // Even then, no frame shows an edge: every correction fits the zoom.
        for frame_index in 0..90 {
            let time = FrameRate::FPS_30.timestamp_of_frame(frame_index);
            let frame = stabilization_correction_at(&stabilization, time, SIZE);
            let correction = frame.source_correction;
            let slack = f64::from(correction.scale_factor - 1.0) * 540.0;
            assert!(
                f64::from(correction.shift_y_in_pixels.abs()) <= slack + 0.5,
                "frame {frame_index}: {frame:?}"
            );
        }
    }

    #[test]
    fn changing_the_smoothing_shares_the_path_and_refits_the_zoom() {
        let stabilization = shaky_pan(8.0);
        let weaker = stabilization_with_smoothing(&stabilization, 0.1);
        assert!(Arc::ptr_eq(&weaker.camera_path, &stabilization.camera_path));
        assert_eq!(weaker.smoothing_in_seconds, 0.1);
        assert!(weaker.zoom_factor <= stabilization.zoom_factor);
        let summary = stabilization_summary(&stabilization);
        assert_eq!(summary.zoom_factor, stabilization.zoom_factor);
        assert_eq!(summary.partly_corrected_frame_count, 0);
    }
}
