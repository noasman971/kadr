//! The camera's path, smoothed: how to move each frame so that a shaky video looks
//! steady, and how much to enlarge the picture so that its moving edges never show
//! (video stabilisation).
//!
//! References:
//! - Y. Matsushita, E. Ofek, W. Ge, X. Tang and H.-Y. Shum, "Full-Frame Video
//!   Stabilization with Motion Inpainting", IEEE PAMI 2006 (Gaussian smoothing of
//!   the camera's motion);
//! - G. Martius, vid.stab (the fixed zoom that keeps every frame's edges out of
//!   view).
//!
//! # How it works
//!
//! Every transform here is about the picture's centre `C = (W / 2, H / 2)`, with
//! rotations clockwise (the y axis points down), in the source picture's pixels.
//!
//! 1. **The path.** [`camera_motion`](crate::camera_motion) measures the motion
//!    `M_k` from each frame to the next. Composed, they give each frame's
//!    [`CameraPose`]: `P_k = M_k ∘ P_(k−1)`, starting from the identity, written
//!    `P(u) = C + σ·R(a)·(u − C) + T`. It tells where the content seen in the
//!    path's first frame is in frame `k`. A motion that could not be measured (a
//!    cut, a blurred frame) keeps the pose (`P_k = P_(k−1)`) and starts a new
//!    **run**: the frames before and after it are not compared.
//! 2. **Smoothing.** The shift `T_x`, `T_y`, the angle `a` and the logarithm of
//!    the size `ln σ` are each smoothed on their own, inside each run, by a
//!    Gaussian over the frames' times in the file: weights `exp(−Δt² / 2s²)`, `s`
//!    the smoothing in seconds, up to `3s` away. A shake at frequency `f` is kept
//!    at `exp(−(2πfs)² / 2)` of its size: with `s = 0.5 s`, 1 Hz is cut to 0.7 %
//!    while a slow move at 0.05 Hz keeps 98.8 %.
//! 3. **The ends of a run.** Past each end, the path is continued by **point
//!    reflection** about the run's *fitted* end: the sample at `t_e + d` gets the
//!    value `2·f_e − p(t_e − d)`, where `f_e` is the value at `t_e` of the straight
//!    line that best fits the run's samples within `3s` of that end. A straight
//!    move then stays exactly straight, and the end frames are steadied like the
//!    others. Reflecting about the end *sample* itself was tried first: it forces
//!    the smoothed path through one shaky sample, and on the benchmark the first
//!    and last half second of every run kept most of their shake. A run shorter
//!    than `6s` is smoothed with `s = length / 6`, so that the reflections at its
//!    two ends never both reach a frame.
//! 4. **The correction.** With `S` the smoothed pose and `P` the measured one,
//!    the frame must move by `K = S ∘ P⁻¹`: its content goes where the smoothed
//!    camera would show it. With `ρ = σ_S / σ_P`, `Δa = a_S − a_P` and
//!    `e = T_S − ρ·R(Δa)·T_P`, `K(v) = C + ρ·R(Δa)·(v − C) + e`
//!    ([`StabilizingCorrection`]).
//! 5. **The zoom.** A moved frame leaves its edges in view. Enlarging it by `z`
//!    about the centre hides them when `z` is at least
//!    [`StabilizingCorrection::zoom_needed`]: with `e′ = R(−Δa)·e / ρ`,
//!    `h_x = |cos Δa|·W/2 + |sin Δa|·H/2` and `h_y = |sin Δa|·W/2 + |cos Δa|·H/2`
//!    (half the size of the turned frame), the frame needs
//!    `max(1, h_x / (ρ·(W/2 − |e′_x|)), h_y / (ρ·(H/2 − |e′_y|)))`. One zoom is
//!    used for the whole video, so the picture never breathes: the largest need of
//!    all its frames, at most [`MAXIMUM_ZOOM_FACTOR`], rounded up to 0.1 %
//!    ([`fitted_zoom_factor`]).
//! 6. **Partial correction.** A frame whose correction needs more than the zoom
//!    (above the cap, or with a zoom fitted for other settings) gets only part of
//!    it: `K_λ = (λ·e, λ·Δa, ρ^λ)`, with the largest `λ` in `[0, 1]` that the zoom
//!    still covers, found by 16 halvings (`λ = 0` always fits). Its edges never
//!    show; it is only less steady ([`StabilizingCorrection::fitted_inside_zoom`]).
//!
//! The path is read through the [`CameraPath`] trait, so that it can stay in the
//! caller's own types. A frame's correction only reads the samples a few
//! smoothings around it.
//!
//! ```
//! use kadr_vision::camera_path::{
//!     CameraPathPoint, CameraPose, fitted_zoom_factor, stabilizing_correction_at,
//! };
//!
//! // A camera shaking left and right by 4 pixels, 30 frames per second.
//! let path: Vec<CameraPathPoint> = (0..300)
//!     .map(|index| {
//!         let time_in_seconds = index as f64 / 30.0;
//!         CameraPathPoint {
//!             time_in_seconds,
//!             pose: CameraPose {
//!                 shift_x_in_pixels: 4.0 * (time_in_seconds * 15.0).sin(),
//!                 ..CameraPose::IDENTITY
//!             },
//!             motion_was_measured: index > 0,
//!         }
//!     })
//!     .collect();
//! let correction = stabilizing_correction_at(path.as_slice(), 150, 0.5);
//! // The correction brings the frame back to where the camera steadily is (0).
//! let measured = path[150].pose.shift_x_in_pixels;
//! assert!((measured + correction.shift_x_in_pixels).abs() < 0.05);
//! let zoom = fitted_zoom_factor(path.as_slice(), 0.5, 1920.0, 1080.0);
//! assert!(zoom > 1.0 && zoom < 1.01);
//! ```

use std::ops::ControlFlow;

use crate::geometry::{Point, SimilarityTransform};

/// Largest zoom used to hide the moving edges (20 %). Frames that would need more
/// are only partly corrected.
pub const MAXIMUM_ZOOM_FACTOR: f64 = 1.2;

/// The smoothing reads samples up to this many smoothings away.
const SMOOTHING_REACH_IN_SMOOTHINGS: f64 = 3.0;

/// The line fitted at an end of a run reads the samples up to this many
/// smoothings from that end.
const END_FIT_SPAN_IN_SMOOTHINGS: f64 = SMOOTHING_REACH_IN_SMOOTHINGS;

/// Runs shorter than this many smoothings are smoothed less (twice the reach, so
/// the reflections at both ends never reach the same frame).
const SHORT_RUN_IN_SMOOTHINGS: f64 = 2.0 * SMOOTHING_REACH_IN_SMOOTHINGS;

/// Halvings of the interval searched for the part of a correction that fits.
const ZOOM_BISECTION_STEPS: usize = 16;

/// The fitted zoom is rounded up to a whole number of thousandths (0.1 %).
const ZOOM_STEPS_PER_UNIT: f64 = 1000.0;

/// Needs within this relative amount above a thousandth are rounded down to it: a
/// straight path's corrections are only zero to the last digits, and should not
/// cost a 0.1 % zoom.
const ZOOM_ROUNDING_TOLERANCE: f64 = 1e-6;

/// A correction needing this little (relative) more than the zoom is still applied
/// whole: zooms are often stored with 7 digits (`f32`), and the fitted zoom was
/// rounded within [`ZOOM_ROUNDING_TOLERANCE`]. Its edge could then show by 1e-5 of
/// half the picture, 0.005 pixels at 1080p, inside the edge pixels' own half.
const ZOOM_FITTING_TOLERANCE: f64 = 1e-5;

/// Samples this close (relative) to the smoothing's reach are read on both sides:
/// their time differences are computed from rounded numbers, and a sample read on
/// one side only would make the average lean (a pan would get a correction).
const REACH_TOLERANCE: f64 = 1e-9;

/// Where the content seen in the path's first frame is in a later frame, about
/// the picture's centre `C`: the first frame's point `u` is at
/// `C + scale_factor · R(rotation) · (u − C) + shift`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraPose {
    /// Shift to the right (positive), in pixels.
    pub shift_x_in_pixels: f64,
    /// Shift down (positive), in pixels.
    pub shift_y_in_pixels: f64,
    /// Clockwise rotation since the first frame, in degrees (not wrapped: a full
    /// turn is 360).
    pub rotation_in_degrees: f64,
    /// Size relative to the first frame (1 = same size).
    pub scale_factor: f64,
}

impl CameraPose {
    /// The first frame's pose: nothing moved.
    pub const IDENTITY: Self = Self {
        shift_x_in_pixels: 0.0,
        shift_y_in_pixels: 0.0,
        rotation_in_degrees: 0.0,
        scale_factor: 1.0,
    };

    /// The pose after the camera then moved by `motion` (`motion ∘ self`), for a
    /// pose about `picture_center`. `motion` may be written about another centre,
    /// for example the centre of a shrunk tracking image made full-size with
    /// [`SimilarityTransform::in_full_size_image`]: it is re-expressed first.
    pub fn followed_by(&self, motion: &SimilarityTransform, picture_center: Point) -> Self {
        let motion = motion.about(picture_center);
        let rotation_in_degrees = f64::from(motion.rotation_in_degrees);
        let scale_factor = f64::from(motion.scale_factor);
        let (x, y) = turned_and_scaled(
            self.shift_x_in_pixels,
            self.shift_y_in_pixels,
            rotation_in_degrees,
            scale_factor,
        );
        Self {
            shift_x_in_pixels: x + f64::from(motion.shift_x_in_pixels),
            shift_y_in_pixels: y + f64::from(motion.shift_y_in_pixels),
            rotation_in_degrees: self.rotation_in_degrees + rotation_in_degrees,
            scale_factor: self.scale_factor * scale_factor,
        }
    }
}

/// One frame of a camera path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraPathPoint {
    /// When the frame starts in the file, in seconds (only differences matter).
    pub time_in_seconds: f64,
    /// The camera's pose in this frame.
    pub pose: CameraPose,
    /// Whether the motion from the previous frame was measured. A frame where it
    /// was not starts a new run; the first frame always does.
    pub motion_was_measured: bool,
}

/// Read access to a camera path: its frames in strictly increasing time.
///
/// Implemented for slices and vectors of [`CameraPathPoint`]; a caller keeping the
/// path in its own types implements it to convert one sample at a time.
pub trait CameraPath {
    /// Number of frames in the path.
    fn sample_count(&self) -> usize;
    /// Frame `index` (`index < sample_count()`).
    fn sample_at(&self, index: usize) -> CameraPathPoint;
}

impl CameraPath for [CameraPathPoint] {
    fn sample_count(&self) -> usize {
        self.len()
    }

    fn sample_at(&self, index: usize) -> CameraPathPoint {
        self[index]
    }
}

impl CameraPath for Vec<CameraPathPoint> {
    fn sample_count(&self) -> usize {
        self.len()
    }

    fn sample_at(&self, index: usize) -> CameraPathPoint {
        self[index]
    }
}

/// How a frame's picture moves inside its window to follow the smoothed path, about
/// the picture's centre `C`: its point `v` is drawn at
/// `C + scale_factor · R(rotation) · (v − C) + shift`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StabilizingCorrection {
    /// Shift to the right (positive), in pixels.
    pub shift_x_in_pixels: f64,
    /// Shift down (positive), in pixels.
    pub shift_y_in_pixels: f64,
    /// Clockwise rotation, in degrees.
    pub rotation_in_degrees: f64,
    /// Change of size (1 = same size).
    pub scale_factor: f64,
}

/// A correction made to fit inside a zoom: see
/// [`StabilizingCorrection::fitted_inside_zoom`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FittedCorrection {
    /// The applied part of the correction, then the zoom: `Z ∘ K_λ`.
    pub correction: StabilizingCorrection,
    /// The part `λ` of the correction applied, from 0 to 1 (1 = all of it).
    pub fraction_applied: f64,
}

impl StabilizingCorrection {
    /// No correction.
    pub const IDENTITY: Self = Self {
        shift_x_in_pixels: 0.0,
        shift_y_in_pixels: 0.0,
        rotation_in_degrees: 0.0,
        scale_factor: 1.0,
    };

    /// The correction `K = S ∘ P⁻¹` that moves a frame filmed at the `measured`
    /// pose to where the `smoothed` pose would show it.
    pub fn between(measured: &CameraPose, smoothed: &CameraPose) -> Self {
        let scale_factor = smoothed.scale_factor / measured.scale_factor;
        let rotation_in_degrees = smoothed.rotation_in_degrees - measured.rotation_in_degrees;
        let (x, y) = turned_and_scaled(
            measured.shift_x_in_pixels,
            measured.shift_y_in_pixels,
            rotation_in_degrees,
            scale_factor,
        );
        Self {
            shift_x_in_pixels: smoothed.shift_x_in_pixels - x,
            shift_y_in_pixels: smoothed.shift_y_in_pixels - y,
            rotation_in_degrees,
            scale_factor,
        }
    }

    /// The smallest zoom about the centre (at least 1) that keeps the moved
    /// picture's edges outside a `picture_width_in_pixels × picture_height_in_pixels`
    /// window; infinite when no zoom can (a shift of half the picture or more).
    pub fn zoom_needed(&self, picture_width_in_pixels: f64, picture_height_in_pixels: f64) -> f64 {
        if !(picture_width_in_pixels > 0.0 && picture_height_in_pixels > 0.0) {
            return 1.0;
        }
        let scale = self.scale_factor;
        if !(scale > 0.0 && scale.is_finite()) {
            return f64::INFINITY;
        }
        let (sine, cosine) = self.rotation_in_degrees.to_radians().sin_cos();
        // e′ = R(−Δa)·e / ρ: the shift seen from the moved picture.
        let shift_x = (cosine * self.shift_x_in_pixels + sine * self.shift_y_in_pixels) / scale;
        let shift_y = (cosine * self.shift_y_in_pixels - sine * self.shift_x_in_pixels) / scale;
        let (half_width, half_height) = (
            picture_width_in_pixels / 2.0,
            picture_height_in_pixels / 2.0,
        );
        let reach_x = cosine.abs() * half_width + sine.abs() * half_height;
        let reach_y = sine.abs() * half_width + cosine.abs() * half_height;
        let room_x = scale * (half_width - shift_x.abs());
        let room_y = scale * (half_height - shift_y.abs());
        if !(room_x > 0.0 && room_y > 0.0) {
            return f64::INFINITY;
        }
        (reach_x / room_x).max(reach_y / room_y).max(1.0)
    }

    /// The part `fraction` of this correction (`K_λ`: the shift and the rotation
    /// times `fraction`, the size raised to it). 0 gives the identity, 1 the whole
    /// correction.
    pub fn partly_applied(&self, fraction: f64) -> Self {
        Self {
            shift_x_in_pixels: fraction * self.shift_x_in_pixels,
            shift_y_in_pixels: fraction * self.shift_y_in_pixels,
            rotation_in_degrees: fraction * self.rotation_in_degrees,
            scale_factor: self.scale_factor.powf(fraction),
        }
    }

    /// This correction, then a zoom by `zoom_factor` about the centre (`Z ∘ K`).
    pub fn enlarged_by(&self, zoom_factor: f64) -> Self {
        Self {
            shift_x_in_pixels: zoom_factor * self.shift_x_in_pixels,
            shift_y_in_pixels: zoom_factor * self.shift_y_in_pixels,
            rotation_in_degrees: self.rotation_in_degrees,
            scale_factor: zoom_factor * self.scale_factor,
        }
    }

    /// The largest part of this correction that a zoom of `zoom_factor` still
    /// covers in a `picture_width_in_pixels × picture_height_in_pixels` picture,
    /// then that zoom: the picture to draw, whose edges never show. Zooms under 1
    /// count as 1.
    pub fn fitted_inside_zoom(
        &self,
        zoom_factor: f64,
        picture_width_in_pixels: f64,
        picture_height_in_pixels: f64,
    ) -> FittedCorrection {
        let zoom_factor = if zoom_factor.is_finite() {
            zoom_factor.max(1.0)
        } else {
            1.0
        };
        let need_of = |fraction: f64| {
            self.partly_applied(fraction)
                .zoom_needed(picture_width_in_pixels, picture_height_in_pixels)
        };
        let fits = |fraction: f64| need_of(fraction) <= zoom_factor;
        let fraction_applied = if need_of(1.0) <= zoom_factor * (1.0 + ZOOM_FITTING_TOLERANCE) {
            1.0
        } else {
            let (mut fitting, mut too_large) = (0.0_f64, 1.0_f64);
            for _ in 0..ZOOM_BISECTION_STEPS {
                let middle = (fitting + too_large) / 2.0;
                if fits(middle) {
                    fitting = middle;
                } else {
                    too_large = middle;
                }
            }
            fitting
        };
        FittedCorrection {
            correction: self
                .partly_applied(fraction_applied)
                .enlarged_by(zoom_factor),
            fraction_applied,
        }
    }

    fn is_finite(&self) -> bool {
        self.shift_x_in_pixels.is_finite()
            && self.shift_y_in_pixels.is_finite()
            && self.rotation_in_degrees.is_finite()
            && self.scale_factor.is_finite()
    }
}

/// The smoothed pose of frame `index` (steps 2 and 3 of the module
/// documentation), or `None` for an index outside the path. A smoothing that is
/// not a positive number of seconds leaves the pose as measured.
pub fn smoothed_pose_at<P: CameraPath + ?Sized>(
    path: &P,
    index: usize,
    smoothing_in_seconds: f64,
) -> Option<CameraPose> {
    if index >= path.sample_count() {
        return None;
    }
    let run = Run::around(path, index, smoothing_in_seconds);
    Some(smoothed_pose_in_run(path, &run, index))
}

/// The correction that moves frame `index` onto the smoothed path (step 4 of the
/// module documentation), before any zoom. The identity for an index outside the
/// path, a smoothing that is not a positive number, or values that are not finite.
pub fn stabilizing_correction_at<P: CameraPath + ?Sized>(
    path: &P,
    index: usize,
    smoothing_in_seconds: f64,
) -> StabilizingCorrection {
    if index >= path.sample_count() {
        return StabilizingCorrection::IDENTITY;
    }
    let run = Run::around(path, index, smoothing_in_seconds);
    correction_in_run(path, &run, index)
}

/// The correction of every frame of the path, in order: the same as
/// [`stabilizing_correction_at`] for each index, computed run by run.
pub fn stabilizing_corrections<P: CameraPath + ?Sized>(
    path: &P,
    smoothing_in_seconds: f64,
) -> Vec<StabilizingCorrection> {
    let mut corrections = Vec::with_capacity(path.sample_count());
    visit_corrections(path, smoothing_in_seconds, |correction| {
        corrections.push(correction);
        ControlFlow::Continue(())
    });
    corrections
}

/// The zoom that keeps every frame's edges out of view once corrected, for a
/// `picture_width_in_pixels × picture_height_in_pixels` picture (step 5 of the
/// module documentation): the largest need of the path's frames, rounded up to
/// 0.1 %, at least 1 and at most [`MAXIMUM_ZOOM_FACTOR`].
pub fn fitted_zoom_factor<P: CameraPath + ?Sized>(
    path: &P,
    smoothing_in_seconds: f64,
    picture_width_in_pixels: f64,
    picture_height_in_pixels: f64,
) -> f64 {
    let mut largest_need = 1.0_f64;
    visit_corrections(path, smoothing_in_seconds, |correction| {
        largest_need = largest_need
            .max(correction.zoom_needed(picture_width_in_pixels, picture_height_in_pixels));
        if largest_need >= MAXIMUM_ZOOM_FACTOR {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    if largest_need >= MAXIMUM_ZOOM_FACTOR {
        return MAXIMUM_ZOOM_FACTOR;
    }
    let rounded_up = (largest_need * (1.0 - ZOOM_ROUNDING_TOLERANCE) * ZOOM_STEPS_PER_UNIT).ceil()
        / ZOOM_STEPS_PER_UNIT;
    rounded_up.clamp(1.0, MAXIMUM_ZOOM_FACTOR)
}

/// Calls `visit` with the correction of every frame, in order, until it breaks.
fn visit_corrections<P: CameraPath + ?Sized>(
    path: &P,
    smoothing_in_seconds: f64,
    mut visit: impl FnMut(StabilizingCorrection) -> ControlFlow<()>,
) {
    let sample_count = path.sample_count();
    let mut first_index = 0;
    while first_index < sample_count {
        let mut last_index = first_index;
        while last_index + 1 < sample_count && path.sample_at(last_index + 1).motion_was_measured {
            last_index += 1;
        }
        let run = Run::whole(path, first_index, last_index, smoothing_in_seconds);
        for index in first_index..=last_index {
            if visit(correction_in_run(path, &run, index)).is_break() {
                return;
            }
        }
        first_index = last_index + 1;
    }
}

/// The correction of frame `index`, inside `run`.
fn correction_in_run<P: CameraPath + ?Sized>(
    path: &P,
    run: &Run,
    index: usize,
) -> StabilizingCorrection {
    let smoothed = smoothed_pose_in_run(path, run, index);
    let correction = StabilizingCorrection::between(&path.sample_at(index).pose, &smoothed);
    if correction.is_finite() {
        correction
    } else {
        StabilizingCorrection::IDENTITY
    }
}

/// The frames that smoothing a frame may read: its run, or the part of it within
/// [`SHORT_RUN_IN_SMOOTHINGS`] smoothings when the run is longer.
#[derive(Debug, Clone, Copy)]
struct Run {
    first_index: usize,
    last_index: usize,
    /// Whether `first_index` is the run's first frame, where the path is
    /// reflected (rather than a frame cut from a longer run).
    starts_at_first_index: bool,
    /// Whether `last_index` is the run's last frame.
    ends_at_last_index: bool,
    /// The smoothing used in this run, shortened for a short run; 0 for none.
    smoothing_in_seconds: f64,
}

impl Run {
    /// The whole run from `first_index` to `last_index`.
    fn whole<P: CameraPath + ?Sized>(
        path: &P,
        first_index: usize,
        last_index: usize,
        smoothing_in_seconds: f64,
    ) -> Self {
        let length_in_seconds = path.sample_at(last_index).time_in_seconds
            - path.sample_at(first_index).time_in_seconds;
        Self {
            first_index,
            last_index,
            starts_at_first_index: true,
            ends_at_last_index: true,
            smoothing_in_seconds: smoothing_for_run(smoothing_in_seconds, Some(length_in_seconds)),
        }
    }

    /// The run of frame `index`, read only as far as smoothing needs: when the
    /// run reaches more than [`SHORT_RUN_IN_SMOOTHINGS`] smoothings past the
    /// frame, it is long enough for the whole smoothing, and its far end is
    /// beyond the frames the smoothing reads.
    fn around<P: CameraPath + ?Sized>(path: &P, index: usize, smoothing_in_seconds: f64) -> Self {
        let smoothing = smoothing_for_run(smoothing_in_seconds, None);
        let reach = SHORT_RUN_IN_SMOOTHINGS * smoothing;
        let time = path.sample_at(index).time_in_seconds;
        let mut first_index = index;
        while first_index > 0
            && path.sample_at(first_index).motion_was_measured
            && time - path.sample_at(first_index - 1).time_in_seconds <= reach
        {
            first_index -= 1;
        }
        let starts_at_first_index =
            first_index == 0 || !path.sample_at(first_index).motion_was_measured;
        let mut last_index = index;
        while last_index + 1 < path.sample_count()
            && path.sample_at(last_index + 1).motion_was_measured
            && path.sample_at(last_index + 1).time_in_seconds - time <= reach
        {
            last_index += 1;
        }
        let ends_at_last_index = last_index + 1 == path.sample_count()
            || !path.sample_at(last_index + 1).motion_was_measured;
        if starts_at_first_index && ends_at_last_index {
            return Self::whole(path, first_index, last_index, smoothing_in_seconds);
        }
        Self {
            first_index,
            last_index,
            starts_at_first_index,
            ends_at_last_index,
            smoothing_in_seconds: smoothing,
        }
    }
}

/// The smoothing to use in a run: `smoothing_in_seconds`, shortened to a sixth of
/// a short run's length (when known), and 0 when it is not a positive number.
fn smoothing_for_run(smoothing_in_seconds: f64, run_length_in_seconds: Option<f64>) -> f64 {
    if !(smoothing_in_seconds > 0.0 && smoothing_in_seconds.is_finite()) {
        return 0.0;
    }
    match run_length_in_seconds {
        Some(length) => smoothing_in_seconds.min(length.max(0.0) / SHORT_RUN_IN_SMOOTHINGS),
        None => smoothing_in_seconds,
    }
}

/// The smoothed pose of frame `index` inside `run`.
fn smoothed_pose_in_run<P: CameraPath + ?Sized>(path: &P, run: &Run, index: usize) -> CameraPose {
    let center = path.sample_at(index);
    let smoothing = run.smoothing_in_seconds;
    if smoothing <= 0.0 {
        return center.pose;
    }
    let reach = SMOOTHING_REACH_IN_SMOOTHINGS * smoothing * (1.0 + REACH_TOLERANCE);
    let falloff = 0.5 / (smoothing * smoothing);
    let weight = |time_difference: f64| (-time_difference * time_difference * falloff).exp();
    let mut sums = WeightedChannels::default();
    sums.add(1.0, PoseChannels::of(&center.pose));
    for before in (run.first_index..index).rev() {
        let sample = path.sample_at(before);
        let difference = center.time_in_seconds - sample.time_in_seconds;
        if difference > reach {
            break;
        }
        sums.add(weight(difference), PoseChannels::of(&sample.pose));
    }
    for after in index + 1..=run.last_index {
        let sample = path.sample_at(after);
        let difference = sample.time_in_seconds - center.time_in_seconds;
        if difference > reach {
            break;
        }
        sums.add(weight(difference), PoseChannels::of(&sample.pose));
    }
    if run.starts_at_first_index {
        // The samples after the first one, reflected to before it.
        let first = path.sample_at(run.first_index);
        let to_first = center.time_in_seconds - first.time_in_seconds;
        if to_first <= reach {
            let first_channels = fitted_end_of(path, run, true);
            for reflected in run.first_index + 1..=run.last_index {
                let sample = path.sample_at(reflected);
                let difference = to_first + (sample.time_in_seconds - first.time_in_seconds);
                if difference > reach {
                    break;
                }
                sums.add(
                    weight(difference),
                    first_channels.reflected(&PoseChannels::of(&sample.pose)),
                );
            }
        }
    }
    if run.ends_at_last_index {
        // The samples before the last one, reflected to after it.
        let last = path.sample_at(run.last_index);
        let to_last = last.time_in_seconds - center.time_in_seconds;
        if to_last <= reach {
            let last_channels = fitted_end_of(path, run, false);
            for reflected in (run.first_index..run.last_index).rev() {
                let sample = path.sample_at(reflected);
                let difference = to_last + (last.time_in_seconds - sample.time_in_seconds);
                if difference > reach {
                    break;
                }
                sums.add(
                    weight(difference),
                    last_channels.reflected(&PoseChannels::of(&sample.pose)),
                );
            }
        }
    }
    sums.average().pose()
}

/// Where the path stands at the start (`at_start`) or the end of `run`, without
/// its shake: the value there of the straight line that best fits, by least
/// squares, the run's samples within [`END_FIT_SPAN_IN_SMOOTHINGS`] smoothings of
/// that end. The end sample itself when the samples cannot define a line.
fn fitted_end_of<P: CameraPath + ?Sized>(path: &P, run: &Run, at_start: bool) -> PoseChannels {
    let end = path.sample_at(if at_start {
        run.first_index
    } else {
        run.last_index
    });
    let span = END_FIT_SPAN_IN_SMOOTHINGS * run.smoothing_in_seconds * (1.0 + REACH_TOLERANCE);
    // Sums for the least-squares line of each value against the time to the end.
    let (mut count, mut time_sum, mut squared_time_sum) = (0.0, 0.0, 0.0);
    let (mut value_sums, mut time_value_sums) = ([0.0; 4], [0.0; 4]);
    for step in 0..=run.last_index - run.first_index {
        let sample = path.sample_at(if at_start {
            run.first_index + step
        } else {
            run.last_index - step
        });
        let time_to_end = (sample.time_in_seconds - end.time_in_seconds).abs();
        if time_to_end > span {
            break;
        }
        count += 1.0;
        time_sum += time_to_end;
        squared_time_sum += time_to_end * time_to_end;
        for (channel, value) in PoseChannels::of(&sample.pose).0.into_iter().enumerate() {
            value_sums[channel] += value;
            time_value_sums[channel] += time_to_end * value;
        }
    }
    let determinant = count * squared_time_sum - time_sum * time_sum;
    if count < 3.0 || determinant <= 0.0 {
        return PoseChannels::of(&end.pose);
    }
    // The line's value at the end (time 0) is its intercept.
    PoseChannels(std::array::from_fn(|channel| {
        (squared_time_sum * value_sums[channel] - time_sum * time_value_sums[channel]) / determinant
    }))
}

/// The four values of a pose that are smoothed on their own: the shift, the angle
/// and the logarithm of the size (sizes multiply, so their logarithms add).
#[derive(Debug, Clone, Copy, Default)]
struct PoseChannels([f64; 4]);

impl PoseChannels {
    fn of(pose: &CameraPose) -> Self {
        Self([
            pose.shift_x_in_pixels,
            pose.shift_y_in_pixels,
            pose.rotation_in_degrees,
            pose.scale_factor.ln(),
        ])
    }

    fn pose(&self) -> CameraPose {
        let [shift_x, shift_y, rotation, log_scale] = self.0;
        CameraPose {
            shift_x_in_pixels: shift_x,
            shift_y_in_pixels: shift_y,
            rotation_in_degrees: rotation,
            scale_factor: log_scale.exp(),
        }
    }

    /// `other` reflected about these values: `2·self − other`.
    fn reflected(&self, other: &PoseChannels) -> Self {
        Self(std::array::from_fn(|channel| {
            2.0 * self.0[channel] - other.0[channel]
        }))
    }
}

/// A running weighted sum of pose channels.
#[derive(Debug, Clone, Copy, Default)]
struct WeightedChannels {
    sums: [f64; 4],
    total_weight: f64,
}

impl WeightedChannels {
    fn add(&mut self, weight: f64, channels: PoseChannels) {
        for (sum, value) in self.sums.iter_mut().zip(channels.0) {
            *sum += weight * value;
        }
        self.total_weight += weight;
    }

    fn average(&self) -> PoseChannels {
        PoseChannels(self.sums.map(|sum| sum / self.total_weight))
    }
}

/// `scale_factor · R(rotation) · (x, y)`, R clockwise (y down).
fn turned_and_scaled(x: f64, y: f64, rotation_in_degrees: f64, scale_factor: f64) -> (f64, f64) {
    let (sine, cosine) = rotation_in_degrees.to_radians().sin_cos();
    (
        scale_factor * (x * cosine - y * sine),
        scale_factor * (x * sine + y * cosine),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    const FRAMES_PER_SECOND: f64 = 30.0;
    const WIDTH: f64 = 1920.0;
    const HEIGHT: f64 = 1080.0;

    fn pose(shift_x: f64, shift_y: f64, rotation_in_degrees: f64, scale_factor: f64) -> CameraPose {
        CameraPose {
            shift_x_in_pixels: shift_x,
            shift_y_in_pixels: shift_y,
            rotation_in_degrees,
            scale_factor,
        }
    }

    /// A path of `frame_count` frames at 30 fps, posed by `pose_at(time)`, whose
    /// motion was not measured at the first frame and at `breaks`.
    fn path_of(
        frame_count: usize,
        breaks: &[usize],
        pose_at: impl Fn(f64) -> CameraPose,
    ) -> Vec<CameraPathPoint> {
        (0..frame_count)
            .map(|index| {
                let time_in_seconds = index as f64 / FRAMES_PER_SECOND;
                CameraPathPoint {
                    time_in_seconds,
                    pose: pose_at(time_in_seconds),
                    motion_was_measured: index > 0 && !breaks.contains(&index),
                }
            })
            .collect()
    }

    /// A shake made of a few frequencies from 2 to 7 Hz.
    fn shake(time: f64) -> CameraPose {
        pose(
            6.0 * (TAU * 3.0 * time).sin() + 2.0 * (TAU * 7.3 * time + 0.4).sin(),
            4.0 * (TAU * 5.0 * time + 1.0).sin(),
            0.5 * (TAU * 2.0 * time).sin(),
            (0.004 * (TAU * 4.0 * time).sin()).exp(),
        )
    }

    /// Where `pose` moves the first frame's point (`x`, `y`), for a picture centred
    /// on `center`.
    fn moved_by_pose(pose: &CameraPose, center: Point, x: f64, y: f64) -> (f64, f64) {
        let (center_x, center_y) = (f64::from(center.x), f64::from(center.y));
        let (moved_x, moved_y) = turned_and_scaled(
            x - center_x,
            y - center_y,
            pose.rotation_in_degrees,
            pose.scale_factor,
        );
        (
            center_x + moved_x + pose.shift_x_in_pixels,
            center_y + moved_y + pose.shift_y_in_pixels,
        )
    }

    /// Where the picture point drawn at window point (`x`, `y`) comes from, once
    /// `correction` moved the picture (its inverse).
    fn source_of(correction: &StabilizingCorrection, x: f64, y: f64) -> (f64, f64) {
        let (center_x, center_y) = (WIDTH / 2.0, HEIGHT / 2.0);
        let (back_x, back_y) = turned_and_scaled(
            x - center_x - correction.shift_x_in_pixels,
            y - center_y - correction.shift_y_in_pixels,
            -correction.rotation_in_degrees,
            1.0 / correction.scale_factor,
        );
        (center_x + back_x, center_y + back_y)
    }

    fn largest_difference(first: &CameraPose, second: &CameraPose) -> [f64; 4] {
        [
            (first.shift_x_in_pixels - second.shift_x_in_pixels).abs(),
            (first.shift_y_in_pixels - second.shift_y_in_pixels).abs(),
            (first.rotation_in_degrees - second.rotation_in_degrees).abs(),
            (first.scale_factor.ln() - second.scale_factor.ln()).abs(),
        ]
    }

    #[test]
    fn composing_motions_gives_the_pose() {
        // A 1919 × 1079 picture tracked at 639 × 359: the tracking image's centre,
        // made full-size, is a pixel away from the picture's centre.
        let picture_center = Point { x: 959.5, y: 539.5 };
        let tracking_center = Point { x: 319.5, y: 179.5 };
        let motions = [
            SimilarityTransform {
                center: tracking_center,
                shift_x_in_pixels: 2.0,
                shift_y_in_pixels: -1.25,
                rotation_in_degrees: 0.7,
                scale_factor: 1.01,
            },
            SimilarityTransform {
                center: tracking_center,
                shift_x_in_pixels: -3.5,
                shift_y_in_pixels: 0.5,
                rotation_in_degrees: -1.1,
                scale_factor: 0.995,
            },
        ]
        .map(|motion| motion.in_full_size_image(3));
        let pose = CameraPose::IDENTITY
            .followed_by(&motions[0], picture_center)
            .followed_by(&motions[1], picture_center);
        assert!((pose.rotation_in_degrees - (0.7 - 1.1)).abs() < 1e-6);
        assert!((pose.scale_factor - 1.01 * 0.995).abs() < 1e-6);
        for (x, y) in [(0.0, 0.0), (1919.0, 0.0), (700.0, 900.0), (959.5, 539.5)] {
            let moved = motions[1].apply_to(motions[0].apply_to(Point {
                x: x as f32,
                y: y as f32,
            }));
            let (pose_x, pose_y) = moved_by_pose(&pose, picture_center, x, y);
            assert!(
                (pose_x - f64::from(moved.x)).abs() < 5e-3
                    && (pose_y - f64::from(moved.y)).abs() < 5e-3,
                "({x}, {y}): pose gives ({pose_x}, {pose_y}), the motions {moved:?}"
            );
        }
    }

    #[test]
    fn a_straight_pan_needs_no_correction() {
        // Steady motion on every channel: a pan, a slow roll and a slow zoom.
        let path = path_of(300, &[], |time| {
            pose(
                100.0 + 300.0 * time,
                -50.0 * time,
                0.2 * time,
                (0.01 * time).exp(),
            )
        });
        for smoothing in [0.5, 3.0] {
            for (index, correction) in stabilizing_corrections(&path, smoothing).iter().enumerate()
            {
                assert!(
                    correction.shift_x_in_pixels.abs() < 1e-6
                        && correction.shift_y_in_pixels.abs() < 1e-6
                        && correction.rotation_in_degrees.abs() < 1e-9
                        && (correction.scale_factor - 1.0).abs() < 1e-9,
                    "frame {index} at {smoothing} s: {correction:?}"
                );
            }
            assert_eq!(fitted_zoom_factor(&path, smoothing, WIDTH, HEIGHT), 1.0);
        }
    }

    #[test]
    fn a_shake_on_a_pan_is_removed() {
        let pan = |time: f64| pose(200.0 * time, 0.0, 0.0, 1.0);
        let path = path_of(600, &[], |time| {
            let (pan, shake) = (pan(time), shake(time));
            pose(
                pan.shift_x_in_pixels + shake.shift_x_in_pixels,
                shake.shift_y_in_pixels,
                shake.rotation_in_degrees,
                shake.scale_factor,
            )
        });
        let shake_size = [8.0, 4.0, 0.5, 0.004];
        let smoothing = 0.5;
        let reach = SMOOTHING_REACH_IN_SMOOTHINGS * smoothing;
        let mut largest_inside = [0.0_f64; 4];
        for (index, sample) in path.iter().enumerate() {
            let smoothed = smoothed_pose_at(&path, index, smoothing).unwrap();
            let differences = largest_difference(&smoothed, &pan(sample.time_in_seconds));
            let is_inside = sample.time_in_seconds >= reach
                && sample.time_in_seconds <= path[599].time_in_seconds - reach;
            for channel in 0..4 {
                // Near the ends, a slow drift of at most one shake's size is left.
                assert!(
                    differences[channel] <= shake_size[channel] * 1.02,
                    "frame {index}, channel {channel}: {smoothed:?}"
                );
                if is_inside {
                    largest_inside[channel] = largest_inside[channel].max(differences[channel]);
                }
            }
        }
        eprintln!("left of the shake inside the run: {largest_inside:?}");
        // Inside the run, the shake is gone: what is left is a few thousandths.
        assert!(
            largest_inside[0] < 0.02 && largest_inside[1] < 0.02,
            "{largest_inside:?}"
        );
        assert!(
            largest_inside[2] < 0.002 && largest_inside[3] < 2e-5,
            "{largest_inside:?}"
        );
    }

    #[test]
    fn the_ends_of_a_run_are_steadied_like_its_middle() {
        // A slow pan with a shake, in two runs (a break at frame 250).
        let camera = |time: f64| {
            let shaking = shake(time);
            pose(
                40.0 * time + shaking.shift_x_in_pixels,
                shaking.shift_y_in_pixels,
                shaking.rotation_in_degrees,
                shaking.scale_factor,
            )
        };
        let path = path_of(600, &[250], camera);
        let corrections = stabilizing_corrections(&path, 0.5);
        let step_of = |from: &CameraPose, to: &CameraPose| {
            (to.shift_x_in_pixels - from.shift_x_in_pixels)
                .hypot(to.shift_y_in_pixels - from.shift_y_in_pixels)
        };
        for (first, last) in [(0, 249), (250, 599)] {
            for frames in [first..first + 15, last - 15..last] {
                for frame in frames {
                    let smoothed = [frame, frame + 1]
                        .map(|index| smoothed_pose_at(&path, index, 0.5).unwrap());
                    let filmed = step_of(&path[frame].pose, &path[frame + 1].pose);
                    let steadied = step_of(&smoothed[0], &smoothed[1]);
                    // The pan alone moves the picture by 40 / 60 pixel per frame; the
                    // shake, by up to 6 pixels.
                    assert!(
                        steadied < 2.0,
                        "frame {frame}: {steadied:.2} px after, {filmed:.2} px before"
                    );
                }
            }
            for end in [first, last] {
                assert_eq!(
                    stabilizing_correction_at(&path, end, 0.5),
                    corrections[end],
                    "frame {end}"
                );
                assert_ne!(corrections[end], StabilizingCorrection::IDENTITY);
            }
        }
    }

    #[test]
    fn a_run_break_barely_moves_the_correction() {
        // A pan shaking up, down and sideways, whose motion could not be measured
        // at frame 300: the pose is held there, and the path goes on from it. The
        // slow shake moves the picture by up to 7 pixels, but by less than 3 pixels
        // from one frame to the next: a correction dropped at the break would jump.
        let camera = |time: f64| {
            pose(
                150.0 * time + 6.0 * (TAU * 1.5 * time).sin(),
                4.0 * (TAU * 2.5 * time + 1.0).sin(),
                0.0,
                1.0,
            )
        };
        let unbroken = path_of(600, &[], camera);
        let mut broken = unbroken.clone();
        let (held, lost) = (unbroken[299].pose, unbroken[300].pose);
        assert!(
            largest_difference(&held, &lost)[0] > 1.0,
            "the lost motion is visible"
        );
        for sample in &mut broken[300..] {
            let pose = &mut sample.pose;
            pose.shift_x_in_pixels -= lost.shift_x_in_pixels - held.shift_x_in_pixels;
            pose.shift_y_in_pixels -= lost.shift_y_in_pixels - held.shift_y_in_pixels;
            pose.rotation_in_degrees -= lost.rotation_in_degrees - held.rotation_in_degrees;
            pose.scale_factor *= held.scale_factor / lost.scale_factor;
        }
        broken[300].motion_was_measured = false;
        let steps = |path: &[CameraPathPoint]| -> Vec<f64> {
            let corrections = stabilizing_corrections(path, 0.5);
            corrections
                .windows(2)
                .map(|pair| {
                    (pair[1].shift_x_in_pixels - pair[0].shift_x_in_pixels)
                        .hypot(pair[1].shift_y_in_pixels - pair[0].shift_y_in_pixels)
                })
                .collect()
        };
        let (broken_steps, unbroken_steps) = (steps(&broken), steps(&unbroken));
        // Both sides of the break are corrected towards the same smooth path.
        assert!(broken_steps[299] < 1.0, "{}", broken_steps[299]);
        let largest = |steps: &[f64]| steps.iter().copied().fold(0.0, f64::max);
        let largest_correction = stabilizing_corrections(&unbroken, 0.5)
            .iter()
            .map(|correction| {
                correction
                    .shift_x_in_pixels
                    .hypot(correction.shift_y_in_pixels)
            })
            .fold(0.0, f64::max);
        eprintln!(
            "largest step of the correction: {:.3} px with the break, {:.3} px without; largest correction {largest_correction:.3} px",
            largest(&broken_steps),
            largest(&unbroken_steps)
        );
        assert!(largest(&unbroken_steps) < 0.5 * largest_correction);
        assert!(largest(&broken_steps) <= 1.5 * largest(&unbroken_steps));
    }

    #[test]
    fn short_runs_shrink_the_smoothing() {
        // 20 frames: two thirds of a second, under 6 × 0.5 s.
        let path = path_of(20, &[], shake);
        let length = path[19].time_in_seconds - path[0].time_in_seconds;
        let with = |smoothing: f64| stabilizing_corrections(&path, smoothing);
        assert_eq!(with(0.5), with(10.0));
        assert_eq!(with(0.5), with(length / 6.0));
        assert_ne!(with(0.5), with(length / 12.0));
        for index in 0..20 {
            assert_eq!(
                stabilizing_correction_at(&path, index, 0.5),
                with(0.5)[index]
            );
        }
        // A frame on its own, or a run of two frames, is left as it is.
        let lonely = path_of(4, &[1, 2], shake);
        assert!(
            stabilizing_corrections(&lonely, 0.5)
                .iter()
                .all(|correction| *correction == StabilizingCorrection::IDENTITY)
        );
    }

    #[test]
    fn the_whole_path_and_single_frames_agree() {
        let path = path_of(400, &[90, 91, 260], shake);
        let all = stabilizing_corrections(&path, 0.4);
        for (index, correction) in all.iter().enumerate() {
            assert_eq!(
                stabilizing_correction_at(&path, index, 0.4),
                *correction,
                "frame {index}"
            );
        }
        assert_eq!(
            stabilizing_correction_at(&path, 400, 0.4),
            StabilizingCorrection::IDENTITY,
            "past the end"
        );
        assert_eq!(smoothed_pose_at(&path, 400, 0.4), None);
        assert!(
            stabilizing_corrections(&path, -1.0)
                .iter()
                .chain(&stabilizing_corrections(&path, f64::NAN))
                .all(|correction| *correction == StabilizingCorrection::IDENTITY),
            "an invalid smoothing leaves the path as it is"
        );
    }

    #[test]
    fn a_constant_offset_of_the_path_changes_nothing() {
        // The same shake measured from another starting angle and size: every
        // frame's angle and size are offset the same way.
        let path = path_of(300, &[], shake);
        let offset = path_of(300, &[], |time| {
            let shake = shake(time);
            pose(
                shake.shift_x_in_pixels,
                shake.shift_y_in_pixels,
                shake.rotation_in_degrees + 7.0,
                shake.scale_factor * 1.3,
            )
        });
        // A shaking shift seen from another starting place, under a constant roll
        // and size.
        let shifting = path_of(300, &[], |time| {
            let shake = shake(time);
            pose(shake.shift_x_in_pixels, shake.shift_y_in_pixels, 3.0, 1.1)
        });
        let shifted = path_of(300, &[], |time| {
            let shake = shake(time);
            pose(
                shake.shift_x_in_pixels + 250.0,
                shake.shift_y_in_pixels - 120.0,
                3.0,
                1.1,
            )
        });
        for (first, second) in [(&path, &offset), (&shifting, &shifted)] {
            let pairs = stabilizing_corrections(first, 0.5)
                .into_iter()
                .zip(stabilizing_corrections(second, 0.5));
            for (index, (expected, measured)) in pairs.enumerate() {
                assert!(
                    (expected.shift_x_in_pixels - measured.shift_x_in_pixels).abs() < 1e-9
                        && (expected.shift_y_in_pixels - measured.shift_y_in_pixels).abs() < 1e-9
                        && (expected.rotation_in_degrees - measured.rotation_in_degrees).abs()
                            < 1e-9
                        && (expected.scale_factor - measured.scale_factor).abs() < 1e-9,
                    "frame {index}: {expected:?} and {measured:?}"
                );
            }
        }
    }

    #[test]
    fn the_fitted_zoom_covers_every_frame_and_respects_the_cap() {
        let path = path_of(300, &[150], shake);
        let corrections = stabilizing_corrections(&path, 0.5);
        let zoom = fitted_zoom_factor(&path, 0.5, WIDTH, HEIGHT);
        let largest_need = corrections
            .iter()
            .map(|correction| correction.zoom_needed(WIDTH, HEIGHT))
            .fold(1.0, f64::max);
        eprintln!("largest need {largest_need:.5}, fitted zoom {zoom}");
        assert!(zoom > 1.0 && zoom <= MAXIMUM_ZOOM_FACTOR);
        assert!(zoom >= largest_need * (1.0 - 1e-6) && zoom - largest_need < 0.001);
        assert!(
            (zoom * 1000.0 - (zoom * 1000.0).round()).abs() < 1e-9,
            "{zoom}"
        );
        // Every frame gets its whole correction, even with the zoom stored in 7 digits.
        for correction in &corrections {
            let fitted = correction.fitted_inside_zoom(f64::from(zoom as f32), WIDTH, HEIGHT);
            assert_eq!(fitted.fraction_applied, 1.0, "{correction:?}");
        }
        // Ten times the shake would need more than the cap: the zoom stops there,
        // and the largest corrections are only partly applied.
        let violent = path_of(300, &[], |time| {
            let shake = shake(time);
            pose(
                10.0 * shake.shift_x_in_pixels,
                10.0 * shake.shift_y_in_pixels,
                10.0 * shake.rotation_in_degrees,
                shake.scale_factor,
            )
        });
        assert_eq!(
            fitted_zoom_factor(&violent, 0.5, WIDTH, HEIGHT),
            MAXIMUM_ZOOM_FACTOR
        );
        let fractions: Vec<f64> = stabilizing_corrections(&violent, 0.5)
            .iter()
            .map(|correction| {
                correction
                    .fitted_inside_zoom(MAXIMUM_ZOOM_FACTOR, WIDTH, HEIGHT)
                    .fraction_applied
            })
            .collect();
        assert!(
            fractions
                .iter()
                .all(|fraction| (0.0..=1.0).contains(fraction))
        );
        assert!(fractions.iter().any(|fraction| *fraction < 1.0));
        assert!(fractions.contains(&1.0));
    }

    #[test]
    fn a_partly_applied_correction_fits_inside_the_picture() {
        let correction = StabilizingCorrection {
            shift_x_in_pixels: 120.0,
            shift_y_in_pixels: -80.0,
            rotation_in_degrees: 4.0,
            scale_factor: 1.03,
        };
        let zoom = 1.1;
        assert!(correction.zoom_needed(WIDTH, HEIGHT) > zoom);
        let fitted = correction.fitted_inside_zoom(zoom, WIDTH, HEIGHT);
        let fraction = fitted.fraction_applied;
        assert!(fraction > 0.1 && fraction < 1.0, "{fitted:?}");
        assert_eq!(
            fitted.correction,
            correction.partly_applied(fraction).enlarged_by(zoom)
        );
        // The window's corners all show the picture: no edge in view.
        for (x, y) in [(0.0, 0.0), (WIDTH, 0.0), (0.0, HEIGHT), (WIDTH, HEIGHT)] {
            let (source_x, source_y) = source_of(&fitted.correction, x, y);
            assert!(
                (-1e-9..=WIDTH + 1e-9).contains(&source_x)
                    && (-1e-9..=HEIGHT + 1e-9).contains(&source_y),
                "corner ({x}, {y}) shows ({source_x}, {source_y})"
            );
        }
        // It is the largest part that fits, to 1/65536.
        let slightly_more = correction.partly_applied(fraction + 1.0 / 65_536.0);
        assert!(slightly_more.zoom_needed(WIDTH, HEIGHT) > zoom);
        // A correction the zoom covers is applied whole.
        let small = correction.partly_applied(0.1);
        let whole = small.fitted_inside_zoom(zoom, WIDTH, HEIGHT);
        assert_eq!(whole.fraction_applied, 1.0);
        assert_eq!(whole.correction, small.enlarged_by(zoom));
        // A shift of half the picture cannot be hidden: nothing of it is applied.
        let impossible = StabilizingCorrection {
            shift_x_in_pixels: WIDTH,
            ..StabilizingCorrection::IDENTITY
        };
        assert_eq!(impossible.zoom_needed(WIDTH, HEIGHT), f64::INFINITY);
        assert!(
            impossible
                .fitted_inside_zoom(zoom, WIDTH, HEIGHT)
                .fraction_applied
                < 0.1
        );
    }

    #[test]
    fn the_zoom_needed_matches_the_turned_frame() {
        assert_eq!(
            StabilizingCorrection::IDENTITY.zoom_needed(WIDTH, HEIGHT),
            1.0
        );
        // A shift of 96 pixels to the right shows 96 pixels of edge on the left:
        // the picture must grow until its left half covers 960 + 96 pixels.
        let shifted = StabilizingCorrection {
            shift_x_in_pixels: 96.0,
            ..StabilizingCorrection::IDENTITY
        };
        assert!((shifted.zoom_needed(WIDTH, HEIGHT) - 960.0 / 864.0).abs() < 1e-12);
        // A roll needs the turned frame's half-height to fit in the picture's.
        let rolled = StabilizingCorrection {
            rotation_in_degrees: 1.0,
            ..StabilizingCorrection::IDENTITY
        };
        let (sine, cosine) = 1.0_f64.to_radians().sin_cos();
        let expected = (sine * 960.0 + cosine * 540.0) / 540.0;
        assert!((rolled.zoom_needed(WIDTH, HEIGHT) - expected).abs() < 1e-12);
    }
}
