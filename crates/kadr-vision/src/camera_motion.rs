//! How the camera moved between two frames: its shift, its roll and its change of
//! size (video stabilisation).
//!
//! A shaky camera moves the whole picture: everything slides, turns a little and
//! grows or shrinks a little together. [`CameraMotionEstimator`] measures that
//! common motion from one frame to the next, as a [`SimilarityTransform`] about the
//! frame's centre. `camera_path.rs` then adds these motions up into the camera's
//! path, smooths it, and works out how to move each frame so that it follows the
//! smoothed path instead.
//!
//! References:
//! - J. Shi and C. Tomasi, "Good Features to Track", CVPR 1994;
//! - J.-Y. Bouguet, "Pyramidal Implementation of the Lucas Kanade Feature Tracker",
//!   Intel, 2001;
//! - Z. Kalal, K. Mikolajczyk and J. Matas, "Forward-Backward Error: Automatic
//!   Detection of Tracking Failures", ICPR 2010;
//! - S. Umeyama, "Least-Squares Estimation of Transformation Parameters Between Two
//!   Point Patterns", IEEE PAMI 1991;
//! - M. J. Swain and D. H. Ballard, "Color Indexing", IJCV 1991.
//!
//! # How it works
//!
//! 1. **Points worth following.** Each frame is divided into a grid of 16 cells
//!    along its longer side and 9 along its shorter one, inside a 16-pixel border
//!    (outside the image, [`GreyImage::sample`] repeats the edge pixels, which do
//!    not move like the picture). In each cell, the point kept is the pixel whose
//!    surroundings are the most textured: the one whose 7 × 7 structure tensor
//!    `[ΣIx² ΣIxIy; ΣIxIy ΣIy²]` (`Ix`, `Iy` the brightness slopes) has the largest
//!    smallest eigenvalue. That is Shi and Tomasi's score: it is small on a uniform
//!    area and along a straight edge, whose motion is ambiguous, and large on
//!    corners and textures. A cell keeps its point only when the score is at least
//!    half a grey level² per window pixel and at least 1 % of the frame's best
//!    cell, so flat cells (sky, a wall) give no point. One point per cell spreads
//!    the points over the whole picture, which makes the roll and the change of
//!    size measurable.
//! 2. **Following them.** Each point found in the previous frame is followed into
//!    the current one by pyramidal Lucas-Kanade optical flow (`optical_flow.rs`),
//!    starting from the previous frame's measured shift (a camera moving steadily
//!    keeps moving the same way), then followed back. A point that does not come
//!    back within a pixel of where it started was badly followed and is dropped.
//! 3. **A first guess that ignores what moves on its own.** The median shift of the
//!    points, on each axis, is the shift of the majority. The points moving like it
//!    (within 1 pixel, plus a fraction of their distance to the centre, which is
//!    what a roll or a change of size explains: 0.7 % and 2 % are both tried, and
//!    the one whose result explains the most points is kept) are kept; a
//!    person walking across the frame or a passing car is left out.
//! 4. **The best similarity.** On those points, the least-squares similarity has a
//!    closed form (Umeyama). With `p` the points in the previous frame and `q` in
//!    the current one, `p̃` and `q̃` measured from their centroids:
//!    `α = Σ(p̃·q̃) / Σ|p̃|²` and `β = Σ(p̃ × q̃) / Σ|p̃|²`; the change of size is
//!    `hypot(α, β)`, the roll `atan2(β, α)` (clockwise, as y points down), and the
//!    shift what is left to bring the centroids together.
//! 5. **Refined.** The first similarity can be bent by a group of points moving
//!    together at one side of the picture (a large object entering the frame),
//!    which the tolerant test of step 3 lets through. It is fitted again on the
//!    points it explains within 3 pixels, which leaves such a group out. Then the
//!    points that this second similarity explains within 1 pixel are taken, and
//!    the similarity is fitted a last time on them. When they are
//!    bunched together (their spread is under 15 % of the frame's diagonal), they
//!    cannot tell a roll from a shift: only the median shift is kept, with no roll
//!    and no change of size.
//! 6. **Not measured.** The motion is reported as unknown when too few points were
//!    followed (fewer than 8, or under a quarter of those tried), when too few agree
//!    with the result (fewer than 8, or under 40 % of the followed ones) and no
//!    majority moves together either (otherwise that majority's shift is kept,
//!    without roll or change of size), when the
//!    result is implausible for one frame (a roll over 5°, a change of size over
//!    5 %), or when the two frames do not show the same scene: a cut. A cut is
//!    recognised by the brightness histograms of the two frames (32 levels): the
//!    share of pixels they have in common, `Σ min(h₁, h₂) / N`, is under one half
//!    (Swain and Ballard's histogram intersection). Missing a cut would be the worst
//!    mistake (a huge correction around it), while an unmeasured frame only
//!    restarts the smoothing.
//!
//! The camera's own motion is the majority's: a subject filling more than half the
//! frame becomes the reference. Rolling-shutter wobble, parallax and motion blur
//! are not modelled.

use thiserror::Error;

use crate::geometry::{Point, SimilarityTransform};
use crate::grey_image::{GreyImage, ImagePyramid};
use crate::optical_flow::{MINIMUM_TEXTURE_PER_PIXEL, follow_points_both_ways, median};

/// Pyramids stop before a level's shorter side drops below this (5 levels for a
/// 640 × 360 frame).
const PYRAMID_SMALLEST_SIDE_IN_PIXELS: usize = 16;

/// Coarsest pyramid level used to follow points. Level 3 shrinks motions eight
/// times, so a 40-pixel pan is a 5-pixel motion there.
const COARSEST_LEVEL: usize = 3;

/// Cells of the point grid along the frame's longer side…
const CELLS_ALONG_LONGER_SIDE: usize = 16;

/// …and along its shorter side.
const CELLS_ALONG_SHORTER_SIDE: usize = 9;

/// No point is taken closer than this to the frame's edges.
const POINT_BORDER_IN_PIXELS: usize = 16;

/// Half-side of the window whose structure tensor scores a point (7 × 7 pixels).
const SCORE_WINDOW_HALF_SIDE_IN_PIXELS: usize = 3;

/// Side of that window.
const SCORE_WINDOW_SIDE_IN_PIXELS: usize = 2 * SCORE_WINDOW_HALF_SIDE_IN_PIXELS + 1;

/// A cell's point is dropped when its score is under this fraction of the
/// frame's best cell.
const MINIMUM_SCORE_FRACTION_OF_BEST: f32 = 0.01;

/// Fewest followed points for the motion to be measured…
const MINIMUM_FOLLOWED_POINTS: usize = 8;

/// …and the smallest share of the points tried they must be.
const MINIMUM_FOLLOWED_FRACTION: f32 = 0.25;

/// The first guess keeps the points whose motion is within this of the median…
const MEDIAN_TOLERANCE_IN_PIXELS: f32 = 1.0;

/// …plus a fraction of their distance to the frame's centre. Two fractions are
/// tried. The strict one (what a roll of 0.4° or a change of size of 0.7 % in
/// one frame explains) keeps out an object moving at one side of the picture,
/// which the tolerant one would take for a part of the camera's motion. The
/// tolerant one (a roll of about a degree, a change of size of 2 %) is needed
/// for fast rolls and zooms.
const MEDIAN_TOLERANCES_PER_DISTANCE: [f32; 2] = [0.007, 0.02];

/// A point agrees with a fitted similarity when it lands within this of it.
const MODEL_TOLERANCE_IN_PIXELS: f32 = 1.0;

/// Points within this distance of the first fit are used for the second one. Wide
/// enough to keep the background when the first fit is bent by an object moving
/// at one side of the picture, narrow enough to leave that object out.
const ROUGH_MODEL_TOLERANCE_IN_PIXELS: f32 = 3.0;

/// Fewest points agreeing with the result for the motion to be measured…
const MINIMUM_AGREEING_POINTS: usize = 8;

/// …and the smallest share of the followed points they must be.
const MINIMUM_AGREEING_FRACTION: f32 = 0.4;

/// Points whose spread (root mean square distance to their centroid) is under
/// this fraction of the frame's diagonal only give a shift.
const SMALL_SPREAD_FRACTION_OF_DIAGONAL: f32 = 0.15;

/// Largest roll accepted from one frame to the next.
const MAXIMUM_ROLL_PER_FRAME_IN_DEGREES: f32 = 5.0;

/// Largest change of size accepted from one frame to the next (5 %, either way).
const MAXIMUM_SCALE_CHANGE_PER_FRAME: f32 = 1.05;

/// Brightness levels of the histograms compared to recognise a cut.
const HISTOGRAM_BIN_COUNT: usize = 32;

/// Two frames whose histograms share less than this are a cut.
const MINIMUM_HISTOGRAM_INTERSECTION: f32 = 0.5;

/// Brightest grey level (values go from 0 to this).
const BRIGHTEST_GREY_LEVEL: f32 = 255.0;

/// Why a frame cannot be compared with the previous one.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum CameraMotionError {
    /// A frame does not have the size of the first one.
    #[error(
        "the frame is {width} × {height} pixels, but the analysis started on a {expected_width} × {expected_height} frame"
    )]
    FrameSizeChanged {
        /// The frame's width.
        width: usize,
        /// The frame's height.
        height: usize,
        /// The first frame's width.
        expected_width: usize,
        /// The first frame's height.
        expected_height: usize,
    },
}

/// Measures the camera's motion from each frame to the next.
///
/// Frames are given in order with [`CameraMotionEstimator::push_frame`], usually
/// shrunk to about 640 pixels wide
/// ([`shrink_factor_for_tracking`](crate::shrink_factor_for_tracking)). Only the
/// previous frame is kept.
///
/// ```
/// use kadr_vision::camera_motion::CameraMotionEstimator;
/// use kadr_vision::synthetic_scenes::ShakenCameraFrame;
/// use kadr_vision::{Point, SimilarityTransform};
///
/// let center = Point { x: 160.0, y: 120.0 };
/// let frame_with = |camera| {
///     ShakenCameraFrame {
///         width: 320,
///         height: 240,
///         background_seed: 7,
///         camera,
///         moving_object: None,
///         noise_amplitude: 0.0,
///         noise_seed: 0,
///     }
///     .render()
/// };
/// let shaken = SimilarityTransform {
///     shift_x_in_pixels: 2.5,
///     shift_y_in_pixels: -1.0,
///     ..SimilarityTransform::identity_about(center)
/// };
///
/// let mut estimator = CameraMotionEstimator::new();
/// assert_eq!(estimator.push_frame(frame_with(SimilarityTransform::identity_about(center)))?, None);
/// let motion = estimator.push_frame(frame_with(shaken))?.expect("measured");
/// assert!((motion.shift_x_in_pixels - 2.5).abs() < 0.1);
/// assert!((motion.shift_y_in_pixels + 1.0).abs() < 0.1);
/// # Ok::<(), kadr_vision::camera_motion::CameraMotionError>(())
/// ```
#[derive(Debug, Clone, Default)]
pub struct CameraMotionEstimator {
    previous_frame: Option<AnalysedFrame>,
    /// The last measured shift, where the following starts from.
    expected_motion: (f32, f32),
}

/// What is kept of the previous frame.
#[derive(Debug, Clone)]
struct AnalysedFrame {
    pyramid: ImagePyramid,
    points: Vec<Point>,
    histogram: BrightnessHistogram,
}

impl CameraMotionEstimator {
    /// An estimator that has seen no frame yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Compares `frame` with the previous one.
    ///
    /// Returns how the camera moved: the transform maps a point of the previous
    /// frame's content to where it is in `frame`, about the frame's centre
    /// (`(width / 2, height / 2)`), in this frame's pixels. Returns `None` for the
    /// first frame and when the motion cannot be measured (a cut, a uniform or
    /// blurred frame, too much changing at once).
    pub fn push_frame(
        &mut self,
        frame: GreyImage,
    ) -> Result<Option<SimilarityTransform>, CameraMotionError> {
        if let Some(previous) = &self.previous_frame {
            let expected = previous.pyramid.level(0);
            if (frame.width(), frame.height()) != (expected.width(), expected.height()) {
                return Err(CameraMotionError::FrameSizeChanged {
                    width: frame.width(),
                    height: frame.height(),
                    expected_width: expected.width(),
                    expected_height: expected.height(),
                });
            }
        }
        let histogram = BrightnessHistogram::of(&frame);
        let pyramid = ImagePyramid::new(frame, PYRAMID_SMALLEST_SIDE_IN_PIXELS);
        let motion = self.previous_frame.as_ref().and_then(|previous| {
            measure_motion(previous, &pyramid, &histogram, self.expected_motion)
        });
        self.expected_motion = motion.map_or((0.0, 0.0), |motion| {
            (motion.shift_x_in_pixels, motion.shift_y_in_pixels)
        });
        let points = points_worth_following(pyramid.level(0));
        self.previous_frame = Some(AnalysedFrame {
            pyramid,
            points,
            histogram,
        });
        Ok(motion)
    }
}

/// The camera's motion from `previous` to `current`, if it can be measured.
fn measure_motion(
    previous: &AnalysedFrame,
    current: &ImagePyramid,
    current_histogram: &BrightnessHistogram,
    expected_motion: (f32, f32),
) -> Option<SimilarityTransform> {
    if previous.histogram.intersection_with(current_histogram) < MINIMUM_HISTOGRAM_INTERSECTION {
        return None;
    }
    let coarsest_level = COARSEST_LEVEL
        .min(previous.pyramid.level_count() - 1)
        .min(current.level_count() - 1);
    let followed = follow_points_both_ways(
        &previous.pyramid,
        current,
        &previous.points,
        expected_motion,
        coarsest_level,
    );
    let image = current.level(0);
    let (width, height) = (image.width() as f32, image.height() as f32);
    fit_camera_motion(
        &followed,
        previous.points.len(),
        Point {
            x: width / 2.0,
            y: height / 2.0,
        },
        width.hypot(height),
    )
}

/// The similarity that best explains how most `followed` points moved, about
/// `center`, out of `tried_point_count` points tried; `None` when the motion
/// cannot be trusted (steps 3 to 6 of the module documentation).
fn fit_camera_motion(
    followed: &[(Point, Point)],
    tried_point_count: usize,
    center: Point,
    diagonal_in_pixels: f32,
) -> Option<SimilarityTransform> {
    if followed.len() < MINIMUM_FOLLOWED_POINTS
        || (followed.len() as f32) < MINIMUM_FOLLOWED_FRACTION * tried_point_count as f32
    {
        return None;
    }
    let median_shift = median_shift_of(followed);
    // Two first guesses, a strict and a tolerant one, each refined the same way;
    // the one that explains the most points wins.
    let agreeing = MEDIAN_TOLERANCES_PER_DISTANCE
        .iter()
        .filter_map(|&tolerance_per_distance| {
            let moving_like_the_median =
                points_moving_like(followed, median_shift, center, tolerance_per_distance);
            points_agreeing_on_a_similarity(followed, &moving_like_the_median, center)
        })
        .max_by_key(Vec::len)
        .filter(|agreeing| is_enough_of(agreeing.len(), followed.len()));
    let similarity = agreeing.and_then(|agreeing| {
        if spread_of(&agreeing) < SMALL_SPREAD_FRACTION_OF_DIAGONAL * diagonal_in_pixels {
            Some(shift_only(&agreeing, center))
        } else {
            least_squares_similarity(&agreeing, center)
        }
    });
    if let Some(motion) = similarity.filter(is_plausible_for_one_frame) {
        return Some(motion);
    }
    // No plausible similarity explains enough points, but most of them may still
    // move together: their shift is then a better answer than "not measured",
    // which would let the whole shake through at this frame.
    let majority = points_moving_like(
        followed,
        median_shift,
        center,
        MEDIAN_TOLERANCES_PER_DISTANCE[0],
    );
    is_enough_of(majority.len(), followed.len()).then(|| shift_only(&majority, center))
}

/// Whether `agreeing_count` points out of `followed_count` are enough to trust
/// what they say.
fn is_enough_of(agreeing_count: usize, followed_count: usize) -> bool {
    agreeing_count >= MINIMUM_AGREEING_POINTS
        && agreeing_count as f32 >= MINIMUM_AGREEING_FRACTION * followed_count as f32
}

/// The `followed` points whose motion is within 1 pixel of `median_shift`, plus
/// `tolerance_per_distance` times their distance to `center`.
fn points_moving_like(
    followed: &[(Point, Point)],
    median_shift: (f32, f32),
    center: Point,
    tolerance_per_distance: f32,
) -> Vec<(Point, Point)> {
    followed
        .iter()
        .copied()
        .filter(|(from, to)| {
            let deviation = (to.x - from.x - median_shift.0).hypot(to.y - from.y - median_shift.1);
            let distance_to_center = (from.x - center.x).hypot(from.y - center.y);
            deviation <= MEDIAN_TOLERANCE_IN_PIXELS + tolerance_per_distance * distance_to_center
        })
        .collect()
}

/// From a first guess of the points that follow the camera, the `followed`
/// points that agree within 1 pixel on one similarity (steps 4 and 5 of the
/// module documentation); `None` when the guess is too small to fit one.
fn points_agreeing_on_a_similarity(
    followed: &[(Point, Point)],
    first_guess: &[(Point, Point)],
    center: Point,
) -> Option<Vec<(Point, Point)>> {
    if first_guess.len() < MINIMUM_AGREEING_POINTS {
        return None;
    }
    let first_fit = least_squares_similarity(first_guess, center)?;
    // A first fit can still be bent by points that do not follow the camera. A
    // second fit on the points roughly agreeing with it leaves them out, before
    // the strict selection.
    let roughly_agreeing = points_within(followed, &first_fit, ROUGH_MODEL_TOLERANCE_IN_PIXELS);
    if roughly_agreeing.len() < MINIMUM_AGREEING_POINTS {
        return None;
    }
    let second_fit = least_squares_similarity(&roughly_agreeing, center)?;
    Some(points_within(
        followed,
        &second_fit,
        MODEL_TOLERANCE_IN_PIXELS,
    ))
}

/// The `followed` points that `motion` brings within `tolerance_in_pixels` of
/// where they were found.
fn points_within(
    followed: &[(Point, Point)],
    motion: &SimilarityTransform,
    tolerance_in_pixels: f32,
) -> Vec<(Point, Point)> {
    followed
        .iter()
        .copied()
        .filter(|(from, to)| {
            let landing = motion.apply_to(*from);
            (landing.x - to.x).hypot(landing.y - to.y) <= tolerance_in_pixels
        })
        .collect()
}

/// The median motion on each axis.
fn median_shift_of(followed: &[(Point, Point)]) -> (f32, f32) {
    let mut horizontal: Vec<f32> = followed.iter().map(|(from, to)| to.x - from.x).collect();
    let mut vertical: Vec<f32> = followed.iter().map(|(from, to)| to.y - from.y).collect();
    (median(&mut horizontal), median(&mut vertical))
}

/// The pure shift that moves the points the most like they moved: the median
/// motion, with no roll and no change of size.
fn shift_only(followed: &[(Point, Point)], center: Point) -> SimilarityTransform {
    let (shift_x, shift_y) = median_shift_of(followed);
    SimilarityTransform {
        shift_x_in_pixels: shift_x,
        shift_y_in_pixels: shift_y,
        ..SimilarityTransform::identity_about(center)
    }
}

/// The similarity that maps the starting points closest to their arrivals, in the
/// least-squares sense (Umeyama), about `center`. `None` when the starting points
/// all coincide.
fn least_squares_similarity(
    followed: &[(Point, Point)],
    center: Point,
) -> Option<SimilarityTransform> {
    let count = followed.len() as f64;
    if followed.is_empty() {
        return None;
    }
    let (mut from_x, mut from_y, mut to_x, mut to_y) = (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64);
    for (from, to) in followed {
        from_x += f64::from(from.x);
        from_y += f64::from(from.y);
        to_x += f64::from(to.x);
        to_y += f64::from(to.y);
    }
    let (from_x, from_y, to_x, to_y) = (from_x / count, from_y / count, to_x / count, to_y / count);
    let (mut dot_sum, mut cross_sum, mut square_sum) = (0.0_f64, 0.0_f64, 0.0_f64);
    for (from, to) in followed {
        let (px, py) = (f64::from(from.x) - from_x, f64::from(from.y) - from_y);
        let (qx, qy) = (f64::from(to.x) - to_x, f64::from(to.y) - to_y);
        dot_sum += px * qx + py * qy;
        cross_sum += px * qy - py * qx;
        square_sum += px * px + py * py;
    }
    if square_sum <= f64::EPSILON {
        return None;
    }
    let (alpha, beta) = (dot_sum / square_sum, cross_sum / square_sum);
    // q = c + [α −β; β α](p − c) + t, so t = q̄ − c − [α −β; β α](p̄ − c).
    let (centroid_x, centroid_y) = (from_x - f64::from(center.x), from_y - f64::from(center.y));
    let shift_x = to_x - f64::from(center.x) - (alpha * centroid_x - beta * centroid_y);
    let shift_y = to_y - f64::from(center.y) - (beta * centroid_x + alpha * centroid_y);
    Some(SimilarityTransform {
        center,
        shift_x_in_pixels: shift_x as f32,
        shift_y_in_pixels: shift_y as f32,
        rotation_in_degrees: beta.atan2(alpha).to_degrees() as f32,
        scale_factor: alpha.hypot(beta) as f32,
    })
}

/// Root mean square distance of the starting points to their centroid.
fn spread_of(followed: &[(Point, Point)]) -> f32 {
    let count = followed.len().max(1) as f32;
    let centroid_x = followed.iter().map(|(from, _)| from.x).sum::<f32>() / count;
    let centroid_y = followed.iter().map(|(from, _)| from.y).sum::<f32>() / count;
    let square_sum: f32 = followed
        .iter()
        .map(|(from, _)| (from.x - centroid_x).powi(2) + (from.y - centroid_y).powi(2))
        .sum();
    (square_sum / count).sqrt()
}

/// Whether a camera can turn and change size this much in one frame.
fn is_plausible_for_one_frame(motion: &SimilarityTransform) -> bool {
    motion.rotation_in_degrees.abs() <= MAXIMUM_ROLL_PER_FRAME_IN_DEGREES
        && motion.scale_factor.ln().abs() <= MAXIMUM_SCALE_CHANGE_PER_FRAME.ln()
        && motion.shift_x_in_pixels.is_finite()
        && motion.shift_y_in_pixels.is_finite()
}

/// The most textured point of each cell of the grid (step 1 of the module
/// documentation), at the centre of its pixel.
fn points_worth_following(image: &GreyImage) -> Vec<Point> {
    let (width, height) = (image.width(), image.height());
    let (columns_of_cells, rows_of_cells) = if width >= height {
        (CELLS_ALONG_LONGER_SIDE, CELLS_ALONG_SHORTER_SIDE)
    } else {
        (CELLS_ALONG_SHORTER_SIDE, CELLS_ALONG_LONGER_SIDE)
    };
    let Some(scores) = CornerScores::of(image) else {
        return Vec::new();
    };
    if scores.width < columns_of_cells || scores.height < rows_of_cells {
        return Vec::new();
    }
    let mut best_of_each_cell: Vec<(f32, Point)> =
        Vec::with_capacity(columns_of_cells * rows_of_cells);
    for cell_row in 0..rows_of_cells {
        let rows = cell_row * scores.height / rows_of_cells
            ..(cell_row + 1) * scores.height / rows_of_cells;
        for cell_column in 0..columns_of_cells {
            let columns = cell_column * scores.width / columns_of_cells
                ..(cell_column + 1) * scores.width / columns_of_cells;
            if let Some(best) = scores.best_in(rows.clone(), columns) {
                best_of_each_cell.push(best);
            }
        }
    }
    let best_score = best_of_each_cell
        .iter()
        .map(|(score, _)| *score)
        .fold(0.0_f32, f32::max);
    let minimum_score = (MINIMUM_TEXTURE_PER_PIXEL
        * (SCORE_WINDOW_SIDE_IN_PIXELS * SCORE_WINDOW_SIDE_IN_PIXELS) as f32)
        .max(MINIMUM_SCORE_FRACTION_OF_BEST * best_score);
    best_of_each_cell
        .into_iter()
        .filter(|(score, _)| *score >= minimum_score)
        .map(|(_, point)| point)
        .collect()
}

/// Shi and Tomasi's score of every pixel inside the border: the smallest
/// eigenvalue of its 7 × 7 structure tensor.
struct CornerScores {
    /// Scores, row after row, of the `width × height` pixels inside the border.
    scores: Vec<f32>,
    width: usize,
    height: usize,
}

impl CornerScores {
    /// The scores of `image`, or `None` when nothing is left inside the border.
    fn of(image: &GreyImage) -> Option<Self> {
        let (image_width, image_height) = (image.width(), image.height());
        let border = POINT_BORDER_IN_PIXELS;
        let half = SCORE_WINDOW_HALF_SIDE_IN_PIXELS;
        if image_width <= 2 * border || image_height <= 2 * border {
            return None;
        }
        let (width, height) = (image_width - 2 * border, image_height - 2 * border);
        // Slope products are needed `half` pixels around the scored area.
        let (products_width, products_height) = (width + 2 * half, height + 2 * half);
        let first = border - half;
        let values = image.values();
        let mut products = vec![[0.0_f32; 3]; products_width * products_height];
        for row in 0..products_height {
            let image_row = first + row;
            for column in 0..products_width {
                let image_column = first + column;
                let at = image_row * image_width + image_column;
                let slope_x = (values[at + 1] - values[at - 1]) / 2.0;
                let slope_y = (values[at + image_width] - values[at - image_width]) / 2.0;
                products[row * products_width + column] =
                    [slope_x * slope_x, slope_x * slope_y, slope_y * slope_y];
            }
        }
        let horizontal = window_sums_along_rows(&products, products_width, width);
        let mut scores = Vec::with_capacity(width * height);
        // Running sums down the columns, in f64 so that adding and removing rows
        // leaves no rounding behind.
        let mut sums = vec![[0.0_f64; 3]; width];
        for row in 0..SCORE_WINDOW_SIDE_IN_PIXELS {
            add_row(&mut sums, &horizontal[row * width..(row + 1) * width], 1.0);
        }
        for row in 0..height {
            if row > 0 {
                let entering = row + SCORE_WINDOW_SIDE_IN_PIXELS - 1;
                add_row(
                    &mut sums,
                    &horizontal[entering * width..(entering + 1) * width],
                    1.0,
                );
                let leaving = row - 1;
                add_row(
                    &mut sums,
                    &horizontal[leaving * width..(leaving + 1) * width],
                    -1.0,
                );
            }
            scores.extend(sums.iter().map(|&[xx, xy, yy]| {
                ((xx + yy - ((xx - yy).powi(2) + 4.0 * xy * xy).sqrt()) / 2.0) as f32
            }));
        }
        Some(Self {
            scores,
            width,
            height,
        })
    }

    /// The best score of the pixels in `rows` × `columns` (inside the border), and
    /// the centre of its pixel in the image. `None` for an empty cell.
    fn best_in(
        &self,
        rows: std::ops::Range<usize>,
        columns: std::ops::Range<usize>,
    ) -> Option<(f32, Point)> {
        let mut best: Option<(f32, usize, usize)> = None;
        for row in rows {
            for column in columns.clone() {
                let score = self.scores[row * self.width + column];
                if best.is_none_or(|(best_score, _, _)| score > best_score) {
                    best = Some((score, column, row));
                }
            }
        }
        best.map(|(score, column, row)| {
            (
                score,
                Point {
                    x: (POINT_BORDER_IN_PIXELS + column) as f32 + 0.5,
                    y: (POINT_BORDER_IN_PIXELS + row) as f32 + 0.5,
                },
            )
        })
    }
}

/// For each row of `products` (`products_width` wide), the sums over the 7
/// pixels centred on each of the `width` middle columns.
fn window_sums_along_rows(
    products: &[[f32; 3]],
    products_width: usize,
    width: usize,
) -> Vec<[f32; 3]> {
    let mut sums = Vec::with_capacity(products.len() / products_width * width);
    for row in products.chunks_exact(products_width) {
        let mut running = [0.0_f64; 3];
        for product in &row[..SCORE_WINDOW_SIDE_IN_PIXELS] {
            add_product(&mut running, product, 1.0);
        }
        for column in 0..width {
            if column > 0 {
                add_product(
                    &mut running,
                    &row[column + SCORE_WINDOW_SIDE_IN_PIXELS - 1],
                    1.0,
                );
                add_product(&mut running, &row[column - 1], -1.0);
            }
            sums.push(running.map(|sum| sum as f32));
        }
    }
    sums
}

fn add_product(running: &mut [f64; 3], product: &[f32; 3], sign: f64) {
    for (sum, value) in running.iter_mut().zip(product) {
        *sum += sign * f64::from(*value);
    }
}

fn add_row(sums: &mut [[f64; 3]], row: &[[f32; 3]], sign: f64) {
    for (sum, product) in sums.iter_mut().zip(row) {
        add_product(sum, product, sign);
    }
}

/// How many pixels of a frame have each brightness level.
#[derive(Debug, Clone, PartialEq)]
struct BrightnessHistogram {
    counts: [u32; HISTOGRAM_BIN_COUNT],
    pixel_count: u32,
}

impl BrightnessHistogram {
    fn of(image: &GreyImage) -> Self {
        let mut counts = [0_u32; HISTOGRAM_BIN_COUNT];
        let bins_per_level = HISTOGRAM_BIN_COUNT as f32 / (BRIGHTEST_GREY_LEVEL + 1.0);
        for value in image.values() {
            let bin = (value.max(0.0) * bins_per_level) as usize;
            counts[bin.min(HISTOGRAM_BIN_COUNT - 1)] += 1;
        }
        Self {
            counts,
            pixel_count: image.values().len() as u32,
        }
    }

    /// The share of pixels the two histograms have in common, from 0 (nothing
    /// alike) to 1 (the same brightness distribution).
    fn intersection_with(&self, other: &BrightnessHistogram) -> f32 {
        let common: u32 = self
            .counts
            .iter()
            .zip(&other.counts)
            .map(|(first, second)| *first.min(second))
            .sum();
        common as f32 / self.pixel_count.max(other.pixel_count).max(1) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic_scenes::textured_value;

    fn textured_image(width: usize, height: usize) -> GreyImage {
        GreyImage::from_fn(width, height, |column, row| {
            30.0 + 190.0 * textured_value(column as f32 + 0.5, row as f32 + 0.5, 12.0, 5)
        })
    }

    /// Point pairs on a grid over a 640 × 360 frame, moved by `motion`.
    fn pairs_moved_by(motion: &SimilarityTransform) -> Vec<(Point, Point)> {
        (0..12)
            .flat_map(|row| (0..16).map(move |column| (column, row)))
            .map(|(column, row)| {
                let from = Point {
                    x: 30.0 + 38.0 * column as f32,
                    y: 25.0 + 28.0 * row as f32,
                };
                (from, motion.apply_to(from))
            })
            .collect()
    }

    const CENTER: Point = Point { x: 320.0, y: 180.0 };

    fn assert_close(measured: &SimilarityTransform, truth: &SimilarityTransform) {
        assert!(
            (measured.shift_x_in_pixels - truth.shift_x_in_pixels).abs() < 1e-3
                && (measured.shift_y_in_pixels - truth.shift_y_in_pixels).abs() < 1e-3
                && (measured.rotation_in_degrees - truth.rotation_in_degrees).abs() < 1e-4
                && (measured.scale_factor - truth.scale_factor).abs() < 1e-5,
            "measured {measured:?}, truth {truth:?}"
        );
    }

    #[test]
    fn corner_points_are_spread_and_avoid_the_border() {
        let image = textured_image(640, 360);
        let points = points_worth_following(&image);
        assert!(points.len() >= 140, "{} points", points.len());
        assert!(points.len() <= CELLS_ALONG_LONGER_SIDE * CELLS_ALONG_SHORTER_SIDE);
        let border = POINT_BORDER_IN_PIXELS as f32;
        assert!(points.iter().all(|point| point.x > border
            && point.x < 640.0 - border
            && point.y > border
            && point.y < 360.0 - border));
        // One point per cell: no two points share a cell of the 16 × 9 grid.
        let cell_along = |position: f32, side: usize, cells: usize| {
            let pixel = (position - border) as usize;
            (0..cells)
                .find(|cell| pixel < (cell + 1) * side / cells)
                .unwrap()
        };
        let cell_of = |point: &Point| (cell_along(point.x, 608, 16), cell_along(point.y, 328, 9));
        let mut cells: Vec<(usize, usize)> = points.iter().map(cell_of).collect();
        cells.sort_unstable();
        cells.dedup();
        assert_eq!(cells.len(), points.len());
        // A portrait frame gets 9 cells across and 16 down.
        let portrait = points_worth_following(&textured_image(360, 640));
        assert!(portrait.len() >= 140, "{} points", portrait.len());
        let mut portrait_cells: Vec<(usize, usize)> = portrait
            .iter()
            .map(|point| (cell_along(point.x, 328, 9), cell_along(point.y, 608, 16)))
            .collect();
        portrait_cells.sort_unstable();
        portrait_cells.dedup();
        assert_eq!(portrait_cells.len(), portrait.len());
    }

    #[test]
    fn flat_cells_give_no_point() {
        // The left half is uniform, the right half textured.
        let image = GreyImage::from_fn(640, 360, |column, row| {
            if column < 320 {
                100.0
            } else {
                30.0 + 190.0 * textured_value(column as f32 + 0.5, row as f32 + 0.5, 12.0, 5)
            }
        });
        let points = points_worth_following(&image);
        // The cells straddling the middle may keep a point on the textured side.
        assert!(
            points.iter().all(|point| point.x > 320.0 - 3.0),
            "{points:?}"
        );
        assert!(points.len() >= 70, "{} points", points.len());
        assert!(points_worth_following(&GreyImage::from_fn(640, 360, |_, _| 80.0)).is_empty());
        assert!(points_worth_following(&GreyImage::from_fn(20, 20, |_, _| 80.0)).is_empty());
    }

    #[test]
    fn the_similarity_fit_recovers_a_known_transform() {
        let truth = SimilarityTransform {
            center: CENTER,
            shift_x_in_pixels: 4.25,
            shift_y_in_pixels: -2.5,
            rotation_in_degrees: 0.8,
            scale_factor: 1.012,
        };
        let pairs = pairs_moved_by(&truth);
        let fitted = least_squares_similarity(&pairs, CENTER).unwrap();
        assert_close(&fitted, &truth);
        let measured = fit_camera_motion(&pairs, pairs.len(), CENTER, 734.0).unwrap();
        assert_close(&measured, &truth);
        // The same motion about another centre.
        let elsewhere = Point { x: 100.0, y: 50.0 };
        assert_close(
            &least_squares_similarity(&pairs, elsewhere).unwrap(),
            &truth.about(elsewhere),
        );
    }

    #[test]
    fn the_fit_ignores_a_third_of_outliers() {
        let truth = SimilarityTransform {
            center: CENTER,
            shift_x_in_pixels: -3.0,
            shift_y_in_pixels: 1.5,
            rotation_in_degrees: -0.6,
            scale_factor: 0.99,
        };
        let pairs: Vec<(Point, Point)> = pairs_moved_by(&truth)
            .into_iter()
            .enumerate()
            .map(|(index, (from, to))| {
                if index % 3 == 0 {
                    // Something moving on its own, a different way at each point.
                    let wander = (index as f32 * 1.7).sin() * 9.0;
                    (
                        from,
                        Point {
                            x: to.x + 12.0 + wander,
                            y: to.y - 7.0 + wander * 0.5,
                        },
                    )
                } else {
                    (from, to)
                }
            })
            .collect();
        let measured = fit_camera_motion(&pairs, pairs.len(), CENTER, 734.0).unwrap();
        assert_close(&measured, &truth);
    }

    #[test]
    fn bunched_inliers_give_a_translation_only() {
        // Every point is in a small patch: a roll there looks like a shift.
        let truth = SimilarityTransform {
            center: CENTER,
            shift_x_in_pixels: 2.0,
            shift_y_in_pixels: 1.0,
            rotation_in_degrees: 0.3,
            scale_factor: 1.0,
        };
        let pairs: Vec<(Point, Point)> = (0..36)
            .map(|index| {
                let from = Point {
                    x: 500.0 + 6.0 * (index % 6) as f32,
                    y: 80.0 + 6.0 * (index / 6) as f32,
                };
                (from, truth.apply_to(from))
            })
            .collect();
        let measured = fit_camera_motion(&pairs, pairs.len(), CENTER, 734.0).unwrap();
        assert_eq!(measured.rotation_in_degrees, 0.0);
        assert_eq!(measured.scale_factor, 1.0);
        let (expected_x, expected_y) = median_shift_of(&pairs);
        assert_eq!(
            (measured.shift_x_in_pixels, measured.shift_y_in_pixels),
            (expected_x, expected_y)
        );
    }

    #[test]
    fn too_few_or_disagreeing_points_give_no_motion() {
        let truth = SimilarityTransform::identity_about(CENTER);
        let pairs = pairs_moved_by(&truth);
        assert!(fit_camera_motion(&pairs[..7], 7, CENTER, 734.0).is_none());
        // Most points tried were lost.
        assert!(fit_camera_motion(&pairs[..40], 200, CENTER, 734.0).is_none());
        // A roll of 8° in one frame is not a shake.
        let rolled = SimilarityTransform {
            rotation_in_degrees: 8.0,
            ..truth
        };
        assert!(fit_camera_motion(&pairs_moved_by(&rolled), 192, CENTER, 734.0).is_none());
    }

    #[test]
    fn a_change_of_brightness_everywhere_is_a_cut() {
        let image = textured_image(320, 180);
        let darker = GreyImage::from_fn(320, 180, |column, row| image.value(column, row) * 0.4);
        let same = BrightnessHistogram::of(&image);
        assert_eq!(same.intersection_with(&same), 1.0);
        assert!(same.intersection_with(&BrightnessHistogram::of(&darker)) < 0.5);
    }
}
