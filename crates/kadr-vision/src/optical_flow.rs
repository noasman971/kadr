//! Pyramidal Lucas-Kanade optical flow: how far did the region move?
//!
//! The correlation filter only sees motions up to about half the region's size
//! per frame. For faster motions (a quick pan, someone running across the frame),
//! the tracker predicts the motion first, then lets the filter look around the
//! predicted position. The prediction starts from the region's motion in the
//! previous frame (constant speed), and optical flow corrects it with what the
//! current frame shows.
//!
//! References:
//! - B. D. Lucas and T. Kanade, "An Iterative Image Registration Technique with an
//!   Application to Stereo Vision", IJCAI 1981;
//! - J.-Y. Bouguet, "Pyramidal Implementation of the Lucas Kanade Feature Tracker",
//!   Intel, 2001;
//! - Z. Kalal, K. Mikolajczyk and J. Matas, "Forward-Backward Error: Automatic
//!   Detection of Tracking Failures", ICPR 2010.
//!
//! # How it works
//!
//! 1. A grid of points is laid over the region.
//! 2. Each point is followed on its own. Around it, a small window of the previous
//!    frame should reappear, shifted by `d`, in the current frame. Lucas-Kanade
//!    finds `d` by repeated linear steps: with `Ix`, `Iy` the brightness slopes and
//!    `e` the brightness difference, solve `[ΣIx² ΣIxIy; ΣIxIy ΣIy²] · step = [ΣIx·e; ΣIy·e]`.
//! 3. To follow larger motions, this runs on a coarse pyramid level first (where
//!    the motion is smaller), and each finer level starts from the coarser answer.
//!    The coarsest level is chosen so that the region still spans
//!    [`MINIMUM_REGION_SIDE_AT_COARSEST_LEVEL_IN_PIXELS`] there: below that, the
//!    windows would mostly see the background, which does not move like the region.
//! 4. Each point is then followed back from the current frame to the previous one.
//!    If it does not come back where it started, it was badly followed (an edge, an
//!    occlusion, a uniform area) and is ignored.
//! 5. The region's motion is the median of the remaining points' motions, which
//!    ignores the few points that followed the background.
//! 6. Its change of size is the median, over every pair of points, of how much
//!    their distance changed: if the region comes 3 % closer, every distance
//!    grows by 3 %. This is the "Median Flow" estimate of Kalal et al. Nothing is
//!    learnt from frame to frame, so a small error on one frame does not become
//!    the reference for the next one.
//! 7. Only the points that move **with** the region count for the size: points on
//!    the background, or on parts moving inside the region (a mouth, a scrolling
//!    text), have their own motion, and their distances to the others change for
//!    reasons that have nothing to do with size. A point moves with the region
//!    when its motion is the median motion, give or take what a change of size of
//!    a few percent explains at its distance from the centre. When the remaining
//!    pairs disagree (their ratios are spread), the size is left as it is: a size
//!    that drifts lets the blur slide onto the background.

use crate::geometry::{Point, Region};
use crate::grey_image::{GreyImage, ImagePyramid};

/// Points along each side of the grid laid over the region.
const GRID_POINTS_PER_SIDE: usize = 4;

/// The grid covers the middle of the region: from 20 % to 80 % of each side.
const GRID_MARGIN_FRACTION: f32 = 0.2;

/// Half-side of the window followed around each point (15 × 15 pixels).
const WINDOW_HALF_SIDE_IN_PIXELS: i32 = 7;

/// Coarsest pyramid level ever used. Level 3 shrinks motions eight times.
const COARSEST_LEVEL: usize = 3;

/// The region's shorter side must span at least this much on the coarsest level used.
const MINIMUM_REGION_SIDE_AT_COARSEST_LEVEL_IN_PIXELS: f32 = 16.0;

/// Most Lucas-Kanade steps per level.
const MAXIMUM_STEPS_PER_LEVEL: usize = 10;

/// A step smaller than this ends the iterations.
const CONVERGED_STEP_IN_PIXELS: f32 = 0.03;

/// A point that comes back further than this from where it started is ignored.
const MAXIMUM_FORWARD_BACKWARD_ERROR_IN_PIXELS: f32 = 1.0;

/// Smallest eigenvalue of the slope matrix, per window pixel, for a point to be
/// followed. Below it, the window is uniform or a straight edge: its motion is
/// ambiguous.
pub(crate) const MINIMUM_TEXTURE_PER_PIXEL: f32 = 0.5;

/// Fewest well-followed points for the motion to be trusted.
const MINIMUM_RELIABLE_POINTS: usize = 4;

/// Pairs of points closer than this fraction of the region's shorter side are
/// left out of the size estimate: a tiny error on their distance would be a large
/// error on its ratio.
const MINIMUM_PAIR_DISTANCE_FRACTION: f32 = 0.2;

/// A point moves with the region when its motion differs from the median motion
/// by at most this much…
const INLIER_TOLERANCE_IN_PIXELS: f32 = 0.5;

/// …plus this fraction of its distance from the region's centre (what a 5 %
/// change of size explains).
const INLIER_TOLERANCE_PER_DISTANCE: f32 = 0.05;

/// Above this spread of the pairs' size ratios (median absolute deviation of
/// their logarithms, about 1 %), they disagree and the size is left as it is.
const MAXIMUM_SCALE_SPREAD: f32 = 0.01;

/// How the region moved from one frame to the next.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RegionMotion {
    /// Horizontal motion of the centre, in pixels.
    pub(crate) motion_x: f32,
    /// Vertical motion of the centre, in pixels.
    pub(crate) motion_y: f32,
    /// How much the region grew (1 = same size, 1.02 = 2 % larger).
    pub(crate) scale_change: f32,
}

/// Estimates how `region` moved and changed size from `previous` to `current`,
/// starting from `expected_motion` (for example the motion of the previous frame).
///
/// Returns `None` if too few points could be followed reliably.
pub(crate) fn estimate_region_motion(
    previous: &ImagePyramid,
    current: &ImagePyramid,
    region: &Region,
    expected_motion: (f32, f32),
) -> Option<RegionMotion> {
    let coarsest_level = coarsest_level_for(region)
        .min(previous.level_count() - 1)
        .min(current.level_count() - 1);
    let followed_points = follow_points_both_ways(
        previous,
        current,
        &grid_points(region),
        expected_motion,
        coarsest_level,
    );
    if followed_points.len() < MINIMUM_RELIABLE_POINTS {
        return None;
    }
    let mut horizontal_motions: Vec<f32> = followed_points
        .iter()
        .map(|(from, to)| to.x - from.x)
        .collect();
    let mut vertical_motions: Vec<f32> = followed_points
        .iter()
        .map(|(from, to)| to.y - from.y)
        .collect();
    let (motion_x, motion_y) = (
        median(&mut horizontal_motions),
        median(&mut vertical_motions),
    );
    let moving_with_the_region: Vec<(Point, Point)> = followed_points
        .into_iter()
        .filter(|(from, to)| {
            let deviation = (to.x - from.x - motion_x).hypot(to.y - from.y - motion_y);
            let distance_from_center = (from.x - region.center_x).hypot(from.y - region.center_y);
            deviation
                <= INLIER_TOLERANCE_IN_PIXELS + INLIER_TOLERANCE_PER_DISTANCE * distance_from_center
        })
        .collect();
    let minimum_pair_distance = MINIMUM_PAIR_DISTANCE_FRACTION * region.width.min(region.height);
    let scale_change = if moving_with_the_region.len() >= MINIMUM_RELIABLE_POINTS {
        agreed_distance_ratio(&moving_with_the_region, minimum_pair_distance).unwrap_or(1.0)
    } else {
        1.0
    };
    Some(RegionMotion {
        motion_x,
        motion_y,
        scale_change,
    })
}

/// Follows each of `points` from `previous` to `current`, coarse to fine from
/// `coarsest_level` and starting from `expected_motion`, then back. Returns where
/// each point started and where it arrived, for the points that came back within
/// [`MAXIMUM_FORWARD_BACKWARD_ERROR_IN_PIXELS`] of where they started, in the
/// order of `points`.
pub(crate) fn follow_points_both_ways(
    previous: &ImagePyramid,
    current: &ImagePyramid,
    points: &[Point],
    expected_motion: (f32, f32),
    coarsest_level: usize,
) -> Vec<(Point, Point)> {
    let mut followed_points: Vec<(Point, Point)> = Vec::new();
    for &point in points {
        let Some(forward) = follow_point(previous, current, point, expected_motion, coarsest_level)
        else {
            continue;
        };
        let arrival = Point {
            x: point.x + forward.0,
            y: point.y + forward.1,
        };
        let Some(backward) = follow_point(
            current,
            previous,
            arrival,
            (-forward.0, -forward.1),
            coarsest_level,
        ) else {
            continue;
        };
        let round_trip_error = (forward.0 + backward.0).hypot(forward.1 + backward.1);
        if round_trip_error <= MAXIMUM_FORWARD_BACKWARD_ERROR_IN_PIXELS {
            followed_points.push((point, arrival));
        }
    }
    followed_points
}

/// The median change of distance over the pairs, when the pairs agree on it
/// (`None` when they disagree or there is no pair).
fn agreed_distance_ratio(followed_points: &[(Point, Point)], minimum_distance: f32) -> Option<f32> {
    let mut ratios = distance_ratios(followed_points, minimum_distance);
    if ratios.is_empty() {
        return None;
    }
    let ratio = median(&mut ratios);
    let mut deviations: Vec<f32> = ratios
        .iter()
        .map(|other| (other / ratio).ln().abs())
        .collect();
    (median(&mut deviations) <= MAXIMUM_SCALE_SPREAD).then_some(ratio)
}

/// How much the distance changed, for every pair of points at least
/// `minimum_distance` apart.
fn distance_ratios(followed_points: &[(Point, Point)], minimum_distance: f32) -> Vec<f32> {
    let distance = |first: Point, second: Point| (first.x - second.x).hypot(first.y - second.y);
    followed_points
        .iter()
        .enumerate()
        .flat_map(|(index, first)| {
            followed_points[index + 1..]
                .iter()
                .map(move |second| (first, second))
        })
        .filter_map(|((from_first, to_first), (from_second, to_second))| {
            let before = distance(*from_first, *from_second);
            (before >= minimum_distance).then(|| distance(*to_first, *to_second) / before)
        })
        .collect()
}

/// The coarsest level on which the region still spans enough pixels.
fn coarsest_level_for(region: &Region) -> usize {
    let shorter_side = region.width.min(region.height);
    let mut level = 0;
    while level < COARSEST_LEVEL
        && shorter_side / (1_u32 << (level + 1)) as f32
            >= MINIMUM_REGION_SIDE_AT_COARSEST_LEVEL_IN_PIXELS
    {
        level += 1;
    }
    level
}

fn grid_points(region: &Region) -> Vec<Point> {
    let fraction_at = |index: usize| {
        GRID_MARGIN_FRACTION
            + (1.0 - 2.0 * GRID_MARGIN_FRACTION) * index as f32 / (GRID_POINTS_PER_SIDE - 1) as f32
    };
    (0..GRID_POINTS_PER_SIDE)
        .flat_map(|row| (0..GRID_POINTS_PER_SIDE).map(move |column| (column, row)))
        .map(|(column, row)| Point {
            x: region.left() + region.width * fraction_at(column),
            y: region.top() + region.height * fraction_at(row),
        })
        .collect()
}

/// Follows one point from `from` to `to`, coarse to fine, starting from
/// `initial_motion`. Returns its motion.
fn follow_point(
    from: &ImagePyramid,
    to: &ImagePyramid,
    point: Point,
    initial_motion: (f32, f32),
    coarsest_level: usize,
) -> Option<(f32, f32)> {
    let mut motion = initial_motion;
    for level in (0..=coarsest_level).rev() {
        let scale = from.scale_of_level(level);
        let point_at_level = Point {
            x: point.x / scale,
            y: point.y / scale,
        };
        let motion_at_level = (motion.0 / scale, motion.1 / scale);
        let refined = refine_motion(
            from.level(level),
            to.level(level),
            point_at_level,
            motion_at_level,
        )?;
        motion = (refined.0 * scale, refined.1 * scale);
    }
    Some(motion)
}

/// Lucas-Kanade steps on one level, starting from `initial_motion`.
fn refine_motion(
    from: &GreyImage,
    to: &GreyImage,
    point: Point,
    initial_motion: (f32, f32),
) -> Option<(f32, f32)> {
    let offsets: Vec<(f32, f32)> = (-WINDOW_HALF_SIDE_IN_PIXELS..=WINDOW_HALF_SIDE_IN_PIXELS)
        .flat_map(|dy| {
            (-WINDOW_HALF_SIDE_IN_PIXELS..=WINDOW_HALF_SIDE_IN_PIXELS)
                .map(move |dx| (dx as f32, dy as f32))
        })
        .collect();
    let window: Vec<(f32, f32, f32)> = offsets
        .iter()
        .map(|&(dx, dy)| {
            let (x, y) = (point.x + dx, point.y + dy);
            let (slope_x, slope_y) = from.gradient(x, y);
            (from.sample(x, y), slope_x, slope_y)
        })
        .collect();
    let (mut xx, mut xy, mut yy) = (0.0_f32, 0.0_f32, 0.0_f32);
    for &(_, slope_x, slope_y) in &window {
        xx += slope_x * slope_x;
        xy += slope_x * slope_y;
        yy += slope_y * slope_y;
    }
    let smallest_eigenvalue = (xx + yy - ((xx - yy).powi(2) + 4.0 * xy * xy).sqrt()) / 2.0;
    if smallest_eigenvalue < MINIMUM_TEXTURE_PER_PIXEL * window.len() as f32 {
        return None;
    }
    let determinant = xx * yy - xy * xy;
    let mut motion = initial_motion;
    for _ in 0..MAXIMUM_STEPS_PER_LEVEL {
        let (mut mismatch_x, mut mismatch_y) = (0.0_f32, 0.0_f32);
        for (&(dx, dy), &(value, slope_x, slope_y)) in offsets.iter().zip(&window) {
            let difference = value - to.sample(point.x + dx + motion.0, point.y + dy + motion.1);
            mismatch_x += difference * slope_x;
            mismatch_y += difference * slope_y;
        }
        let step = (
            (yy * mismatch_x - xy * mismatch_y) / determinant,
            (xx * mismatch_y - xy * mismatch_x) / determinant,
        );
        motion = (motion.0 + step.0, motion.1 + step.1);
        if step.0.hypot(step.1) < CONVERGED_STEP_IN_PIXELS {
            break;
        }
    }
    motion
        .0
        .is_finite()
        .then_some(motion)
        .filter(|motion| motion.1.is_finite())
}

/// The median of `values` (the mean of the two middle ones for an even count),
/// sorting them in place. `values` must not be empty.
pub(crate) fn median(values: &mut [f32]) -> f32 {
    values.sort_by(f32::total_cmp);
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic_scenes::textured_value;

    fn shifted_pyramid(shift_x: f32, shift_y: f32) -> ImagePyramid {
        let image = GreyImage::from_fn(320, 240, |column, row| {
            let (x, y) = (column as f32 + 0.5 - shift_x, row as f32 + 0.5 - shift_y);
            30.0 + 190.0 * textured_value(x, y, 10.0, 21)
        });
        ImagePyramid::new(image, 16)
    }

    #[test]
    fn motions_are_measured_from_the_expected_one() {
        let region = Region::from_top_left(120.0, 80.0, 60.0, 70.0);
        let cases = [
            // (true shift, expected motion)
            ((1.3, -0.6), (0.0, 0.0)),
            ((-5.0, 3.5), (0.0, 0.0)),
            ((38.0, 5.0), (35.0, 7.0)),
        ];
        for ((shift_x, shift_y), expected_motion) in cases {
            let motion = estimate_region_motion(
                &shifted_pyramid(0.0, 0.0),
                &shifted_pyramid(shift_x, shift_y),
                &region,
                expected_motion,
            )
            .unwrap();
            assert!(
                (motion.motion_x - shift_x).abs() < 0.3 && (motion.motion_y - shift_y).abs() < 0.3,
                "shift ({shift_x}, {shift_y}), measured {motion:?}"
            );
            assert!((motion.scale_change - 1.0).abs() < 0.005, "{motion:?}");
        }
    }

    #[test]
    fn a_zoom_is_measured_as_a_change_of_size() {
        let zoomed = |zoom: f32| {
            let image = GreyImage::from_fn(320, 240, |column, row| {
                let (x, y) = (
                    (column as f32 + 0.5 - 160.0) / zoom,
                    (row as f32 + 0.5 - 120.0) / zoom,
                );
                30.0 + 190.0 * textured_value(x, y, 10.0, 21)
            });
            ImagePyramid::new(image, 16)
        };
        let region = Region {
            center_x: 160.0,
            center_y: 120.0,
            width: 60.0,
            height: 70.0,
        };
        for zoom in [1.03, 0.96] {
            let motion =
                estimate_region_motion(&zoomed(1.0), &zoomed(zoom), &region, (0.0, 0.0)).unwrap();
            assert!(
                (motion.scale_change / zoom - 1.0).abs() < 0.005,
                "zoom {zoom}: {motion:?}"
            );
            assert!(
                motion.motion_x.abs() < 0.2 && motion.motion_y.abs() < 0.2,
                "{motion:?}"
            );
        }
    }

    #[test]
    fn a_still_background_inside_the_region_does_not_change_its_size() {
        // The left half of the region moves 6 pixels to the right, the right half
        // stays still (a background showing through): the distances between the
        // halves change, but the region does not grow.
        let moving_left_half = |shift_x: f32| {
            let image = GreyImage::from_fn(320, 240, |column, row| {
                let x = column as f32 + 0.5;
                let shifted_x = if x < 150.0 { x - shift_x } else { x };
                30.0 + 190.0 * textured_value(shifted_x, row as f32 + 0.5, 10.0, 21)
            });
            ImagePyramid::new(image, 16)
        };
        let region = Region::from_top_left(110.0, 80.0, 60.0, 70.0);
        let motion = estimate_region_motion(
            &moving_left_half(0.0),
            &moving_left_half(6.0),
            &region,
            (0.0, 0.0),
        )
        .unwrap();
        // Only the moving half counts, and it did not change size.
        assert!((motion.scale_change - 1.0).abs() < 0.002, "{motion:?}");
    }

    #[test]
    fn small_regions_stay_on_fine_levels() {
        assert_eq!(
            coarsest_level_for(&Region::from_top_left(0.0, 0.0, 20.0, 40.0)),
            0
        );
        assert_eq!(
            coarsest_level_for(&Region::from_top_left(0.0, 0.0, 60.0, 70.0)),
            1
        );
        assert_eq!(
            coarsest_level_for(&Region::from_top_left(0.0, 0.0, 400.0, 400.0)),
            3
        );
    }

    #[test]
    fn a_uniform_region_gives_no_motion() {
        let flat = ImagePyramid::new(GreyImage::from_fn(200, 200, |_, _| 70.0), 16);
        let region = Region::from_top_left(50.0, 50.0, 60.0, 60.0);
        assert!(estimate_region_motion(&flat, &flat, &region, (0.0, 0.0)).is_none());
    }

    #[test]
    fn the_median_ignores_outliers() {
        assert_eq!(median(&mut [1.0, 100.0, 2.0]), 2.0);
        assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), 2.5);
    }
}
