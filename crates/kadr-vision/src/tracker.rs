//! The region tracker: follows one region of a video, frame after frame.
//!
//! [`RegionTracker`] documents how it works.

use thiserror::Error;

use crate::correlation_filter::{
    CONTEXT_FACTOR, CorrelationFilter, CorrelationPeak, SquareWindow, WINDOW_SIDE_IN_CELLS,
};
use crate::geometry::{Point, Region};
use crate::grey_image::{GreyImage, ImagePyramid};
use crate::optical_flow::{RegionMotion, estimate_region_motion};
use crate::template_matching::AppearanceTemplate;

/// Widest image worth tracking on: wider frames are shrunk to about this width.
pub const TRACKING_IMAGE_MAXIMUM_WIDTH_IN_PIXELS: usize = 640;

/// A PSR at least this high is a sure match. Bolme et al. observe 20 to 60 for
/// good tracking; occlusions bring it under 7.
pub const CONFIDENT_PEAK_TO_SIDELOBE_RATIO: f32 = 12.0;

/// Below this PSR, the region is lost.
pub const LOST_PEAK_TO_SIDELOBE_RATIO: f32 = 7.0;

/// After this many uncertain frames in a row, the region is considered lost.
pub const MAXIMUM_UNCERTAIN_FRAMES_IN_A_ROW: u32 = 15;

/// Largest size change accepted from one frame to the next.
const MAXIMUM_SCALE_CHANGE_PER_FRAME: f32 = 1.05;

/// Shortest side a region can have to be tracked.
pub const MINIMUM_REGION_SIDE_IN_PIXELS: f32 = 8.0;

/// Pyramids stop before a level's shorter side drops below this.
const PYRAMID_SMALLEST_SIDE_IN_PIXELS: usize = 16;

/// A predicted motion smaller than this many filter cells is not worth a second
/// detection: the filter finds it from the previous position anyway.
const MINIMUM_PREDICTED_MOTION_IN_CELLS: f32 = 2.0;

/// Template matching must reach this similarity for its candidate to be checked.
const MINIMUM_RECOVERY_SIMILARITY: f32 = 0.6;

/// The search for a lost region starts this many region sides around it…
const RECOVERY_SEARCH_START_IN_REGION_SIDES: f32 = 1.0;

/// …grows by this much every lost frame…
const RECOVERY_SEARCH_GROWTH_IN_REGION_SIDES_PER_FRAME: f32 = 0.1;

/// …up to this many region sides.
const RECOVERY_SEARCH_MAXIMUM_IN_REGION_SIDES: f32 = 3.0;

/// Template cells around a confident detection where the selection's appearance
/// is looked for, to correct a slow drift.
const ANCHOR_SEARCH_RADIUS_IN_CELLS: usize = 3;

/// Sizes tried around a confident detection's, to correct a drift of the size:
/// 4 % smaller, the same, 4 % larger (equal steps on a logarithmic scale, as
/// `AppearanceTemplate::best_match_near` expects).
const ANCHOR_SCALE_FACTORS: [f32; 3] = [1.0 / 1.04, 1.0, 1.04];

/// The selection's appearance must match at least this well near a confident
/// detection to correct it.
const ANCHOR_MINIMUM_SIMILARITY: f32 = 0.7;

/// How far a confident detection moves towards where the selection's appearance
/// matches best (0 = not at all, 1 = all the way), every frame.
const ANCHOR_CORRECTION_GAIN: f32 = 0.35;

/// Below this similarity with the selection near a confident detection, the
/// detection is suspect: the filter may have drifted onto something else.
const DRIFT_SIMILARITY: f32 = 0.5;

/// A suspect region is moved to where the selection's appearance is found
/// further away only if it matches at least this well.
const RE_ANCHOR_MINIMUM_SIMILARITY: f32 = 0.85;

/// The memorised appearance is refreshed after this many confident frames.
const APPEARANCE_REFRESH_INTERVAL_IN_FRAMES: u32 = 10;

/// How sure the tracker is about a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackingState {
    /// The region was found clearly.
    Confident,
    /// The region was probably found, but the match is weak (partial occlusion,
    /// motion blur, fast change of appearance), it was just found again, or it
    /// no longer looks like the selection although the filter is sure of it (it
    /// may have drifted onto something else).
    Uncertain,
    /// The region is not visible: hidden, out of the frame, or lost. Its position
    /// is where it was last seen.
    Lost,
}

/// The tracker's answer for one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackedRegion {
    /// Where the region is (or was last seen, when lost).
    pub region: Region,
    /// How sure the tracker is.
    pub state: TrackingState,
    /// The PSR of the best correlation peak, 0 when none was found.
    pub peak_to_sidelobe_ratio: f32,
}

/// Why tracking cannot start or continue.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum TrackingError {
    /// The selected region is too small.
    #[error(
        "the region is {width_in_pixels} × {height_in_pixels} pixels: each side must be at least \
         {minimum_side_in_pixels} pixels to be tracked"
    )]
    RegionTooSmall {
        /// The region's width.
        width_in_pixels: f32,
        /// The region's height.
        height_in_pixels: f32,
        /// The shortest side accepted.
        minimum_side_in_pixels: f32,
    },
    /// The selected region's centre is outside the image.
    #[error(
        "the region's centre ({center_x}, {center_y}) is outside the {image_width} × {image_height} image"
    )]
    RegionOutsideImage {
        /// Horizontal position of the region's centre.
        center_x: f32,
        /// Vertical position of the region's centre.
        center_y: f32,
        /// The image's width in pixels.
        image_width: usize,
        /// The image's height in pixels.
        image_height: usize,
    },
    /// The region and its surroundings are uniform.
    #[error("the region and its surroundings are uniform: there is nothing to follow")]
    RegionWithoutTexture,
    /// The region itself is uniform, even if its surroundings are not.
    #[error("the region itself is uniform: it has no detail to recognise it by when it is lost")]
    RegionWithoutDetail,
    /// A frame does not have the size of the first one.
    #[error(
        "the frame is {width} × {height} pixels, but tracking started on a {expected_width} × {expected_height} frame"
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

/// The factor by which to shrink frames `frame_width_in_pixels` wide before
/// tracking (1 for frames no wider than [`TRACKING_IMAGE_MAXIMUM_WIDTH_IN_PIXELS`]).
pub fn shrink_factor_for_tracking(frame_width_in_pixels: usize) -> usize {
    frame_width_in_pixels
        .div_ceil(TRACKING_IMAGE_MAXIMUM_WIDTH_IN_PIXELS)
        .max(1)
}

/// A detection: where the filter puts the region, and how clearly.
#[derive(Debug, Clone, Copy)]
struct Detection {
    region: Region,
    peak: CorrelationPeak,
}

/// Follows one region, frame after frame.
///
/// The user selects a region (a face, a licence plate) on one frame; the tracker
/// is started on that frame, then given the following frames one by one (or the
/// preceding ones, in reverse order, to track backwards). For each frame:
///
/// 1. **Follow points.** Lucas-Kanade optical flow (`optical_flow.rs`) follows a
///    grid of points of the region, starting from the motion of the previous
///    frame (constant speed). The points tell how far the region moved, which
///    predicts its position for fast motions, and how much it grew or shrank
///    (the median change of distance between pairs of points).
/// 2. **Detect.** The MOSSE correlation filter (`correlation_filter.rs`) looks
///    for the region around its previous position, and around the predicted one
///    when it differs noticeably. The clearest answer wins.
/// 3. **Decide**, from the peak-to-sidelobe ratio (PSR) of that answer:
///    - at least [`CONFIDENT_PEAK_TO_SIDELOBE_RATIO`]: **confident**; the region
///      moves to the filter's answer, takes the size the points measured, is
///      checked against the selection (step 4), and the filter learns its
///      current appearance;
///    - at least [`LOST_PEAK_TO_SIDELOBE_RATIO`]: **uncertain**; the region moves
///      but keeps its size, and the filter does not learn (it could be learning an
///      obstacle). After [`MAXIMUM_UNCERTAIN_FRAMES_IN_A_ROW`] uncertain frames in a
///      row, the region is considered lost;
///    - below: **lost**.
/// 4. **Check against the selection.** The filter learns the region's
///    appearance as it goes, so its small errors add up and the region can
///    slowly slide off the object while the PSR stays high. The selection's
///    appearance never changes: it is searched a few cells around the answer, at
///    three sizes (template matching, `template_matching.rs`):
///    - where it matches clearly (a similarity of at
///      least 0.7), the region is pulled a third of the way towards it, every
///      frame. A slow drift of position or size never builds up;
///    - where it matches a little (a head turning), the filter's answer stands;
///    - where it does not match at all (below 0.5), the region may have drifted
///      onto something else. The selection is searched further away, as for a
///      lost region; if it is found clearly, the region jumps there and the
///      filter starts afresh. Either way the frame is **uncertain**, so the
///      range is reported for checking.
/// 5. **Recover.** While lost, the region stays where it was last seen. Each
///    frame, the filter looks there again, and template matching
///    (`template_matching.rs`) searches a wider area, growing with time, for
///    the appearance memorised while tracking was sure. A candidate is only
///    accepted when the filter confirms it with a confident PSR.
///
/// The tracker answers in the coordinates of the images it is given. To track a
/// full-HD video, shrink its frames with [`GreyImage::from_luma_shrunk`] and
/// [`shrink_factor_for_tracking`] first: following a face needs no more than
/// about 640 pixels of width, and it is several times faster.
pub struct RegionTracker {
    filter: CorrelationFilter,
    region: Region,
    previous_frame: ImagePyramid,
    frame_width: usize,
    frame_height: usize,
    initial_appearance: AppearanceTemplate,
    recent_appearance: AppearanceTemplate,
    confident_frames_since_appearance_refresh: u32,
    uncertain_frames_in_a_row: u32,
    /// `Some(count)` while the region is lost.
    lost_frames_in_a_row: Option<u32>,
    /// How far the region's centre moved in the last frame: the constant-speed
    /// prediction for the next one.
    last_motion: (f32, f32),
}

impl std::fmt::Debug for RegionTracker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RegionTracker")
            .field("region", &self.region)
            .field("frame_width", &self.frame_width)
            .field("frame_height", &self.frame_height)
            .field("uncertain_frames_in_a_row", &self.uncertain_frames_in_a_row)
            .field("lost_frames_in_a_row", &self.lost_frames_in_a_row)
            .finish_non_exhaustive()
    }
}

impl RegionTracker {
    /// Starts tracking `region`, selected on `first_frame`.
    pub fn start(first_frame: GreyImage, region: Region) -> Result<Self, TrackingError> {
        check_region_can_be_tracked(&first_frame, &region)?;
        let (frame_width, frame_height) = (first_frame.width(), first_frame.height());
        let pyramid = ImagePyramid::new(first_frame, PYRAMID_SMALLEST_SIDE_IN_PIXELS);
        let filter = CorrelationFilter::train(&pyramid, filter_window(region.center(), &region))
            .ok_or(TrackingError::RegionWithoutTexture)?;
        let appearance = AppearanceTemplate::capture(&pyramid, &region)
            .ok_or(TrackingError::RegionWithoutDetail)?;
        Ok(Self {
            filter,
            region,
            previous_frame: pyramid,
            frame_width,
            frame_height,
            recent_appearance: appearance.clone(),
            initial_appearance: appearance,
            confident_frames_since_appearance_refresh: 0,
            uncertain_frames_in_a_row: 0,
            lost_frames_in_a_row: None,
            last_motion: (0.0, 0.0),
        })
    }

    /// Where the region is now, or was last seen.
    pub fn region(&self) -> Region {
        self.region
    }

    /// Finds the region in the next frame.
    pub fn track(&mut self, frame: GreyImage) -> Result<TrackedRegion, TrackingError> {
        if (frame.width(), frame.height()) != (self.frame_width, self.frame_height) {
            return Err(TrackingError::FrameSizeChanged {
                width: frame.width(),
                height: frame.height(),
                expected_width: self.frame_width,
                expected_height: self.frame_height,
            });
        }
        let pyramid = ImagePyramid::new(frame, PYRAMID_SMALLEST_SIDE_IN_PIXELS);
        let tracked = match self.lost_frames_in_a_row {
            Some(_) => self.look_for_lost_region(&pyramid),
            None => self.follow_region(&pyramid),
        };
        self.previous_frame = pyramid;
        Ok(tracked)
    }

    fn follow_region(&mut self, pyramid: &ImagePyramid) -> TrackedRegion {
        let motion = estimate_region_motion(
            &self.previous_frame,
            pyramid,
            &self.region,
            self.last_motion,
        );
        let detection = self
            .centers_to_search(motion)
            .into_iter()
            .filter_map(|center| self.detect_around(pyramid, center))
            .max_by(|first, second| {
                first
                    .peak
                    .peak_to_sidelobe_ratio
                    .total_cmp(&second.peak.peak_to_sidelobe_ratio)
            });
        let Some(detection) = detection else {
            return self.lose_region(0.0);
        };
        let peak_to_sidelobe_ratio = detection.peak.peak_to_sidelobe_ratio;
        if peak_to_sidelobe_ratio >= CONFIDENT_PEAK_TO_SIDELOBE_RATIO {
            let scale_change = motion.map_or(1.0, |motion| {
                motion.scale_change.clamp(
                    1.0 / MAXIMUM_SCALE_CHANGE_PER_FRAME,
                    MAXIMUM_SCALE_CHANGE_PER_FRAME,
                )
            });
            self.check_against_selection(
                pyramid,
                detection.region.scaled_by(scale_change),
                peak_to_sidelobe_ratio,
            )
        } else if peak_to_sidelobe_ratio >= LOST_PEAK_TO_SIDELOBE_RATIO
            && self.uncertain_frames_in_a_row < MAXIMUM_UNCERTAIN_FRAMES_IN_A_ROW
        {
            // Moved, but the size stays: the points may be following an obstacle.
            self.uncertain_frames_in_a_row += 1;
            self.move_region_to(detection.region);
            self.answer(TrackingState::Uncertain, peak_to_sidelobe_ratio)
        } else {
            self.lose_region(peak_to_sidelobe_ratio)
        }
    }

    /// The previous centre, plus the predicted one if it differs noticeably: the
    /// points' motion, or the last frame's motion when the points were lost.
    fn centers_to_search(&self, motion: Option<RegionMotion>) -> Vec<Point> {
        let mut centers = vec![self.region.center()];
        let (motion_x, motion_y) = motion.map_or(self.last_motion, |motion| {
            (motion.motion_x, motion.motion_y)
        });
        let cell_size_in_pixels =
            CONTEXT_FACTOR * self.region.longer_side() / WINDOW_SIDE_IN_CELLS as f32;
        if motion_x.hypot(motion_y) >= MINIMUM_PREDICTED_MOTION_IN_CELLS * cell_size_in_pixels {
            centers.push(self.region.moved_by(motion_x, motion_y).center());
        }
        centers
    }

    /// Moves the region, and remembers how far it went for the next prediction.
    fn move_region_to(&mut self, region: Region) {
        let moved = self.clamped_to_frame(region);
        self.last_motion = (
            moved.center_x - self.region.center_x,
            moved.center_y - self.region.center_y,
        );
        self.region = moved;
    }

    /// The correlation filter's answer around `center`, at the current size.
    fn detect_around(&mut self, pyramid: &ImagePyramid, center: Point) -> Option<Detection> {
        let peak = self
            .filter
            .find_peak(pyramid, filter_window(center, &self.region))?;
        let region = self
            .region
            .moved_by(
                center.x - self.region.center_x,
                center.y - self.region.center_y,
            )
            .moved_by(peak.displacement_x_in_pixels, peak.displacement_y_in_pixels);
        Some(Detection { region, peak })
    }

    /// Checks a detection the filter is sure of against the selection's
    /// appearance, the one reference that cannot drift (see [`RegionTracker`]).
    fn check_against_selection(
        &mut self,
        pyramid: &ImagePyramid,
        measured: Region,
        peak_to_sidelobe_ratio: f32,
    ) -> TrackedRegion {
        let nearby = self.initial_appearance.best_match_near(
            pyramid,
            &measured,
            ANCHOR_SEARCH_RADIUS_IN_CELLS,
            &ANCHOR_SCALE_FACTORS,
        );
        let similarity = nearby.map_or(-1.0, |found| found.similarity);
        if let Some(found) = nearby.filter(|found| found.similarity >= ANCHOR_MINIMUM_SIMILARITY) {
            let anchored = pulled_towards(measured, found.region, ANCHOR_CORRECTION_GAIN);
            self.accept_confident_detection(pyramid, anchored, true);
            return self.answer(TrackingState::Confident, peak_to_sidelobe_ratio);
        }
        if similarity >= DRIFT_SIMILARITY {
            // Changed, but still like the selection: a turning head, a new expression.
            self.accept_confident_detection(pyramid, measured, true);
            return self.answer(TrackingState::Confident, peak_to_sidelobe_ratio);
        }
        // The filter is sure, but this no longer looks like the selection: it may
        // have drifted onto something else.
        if let Some(found) = self.find_selection_further_away(pyramid, &measured) {
            return self.re_anchor(pyramid, found, peak_to_sidelobe_ratio);
        }
        // Keep following, but say so; the recent appearance is not refreshed with
        // what may be the wrong thing.
        self.accept_confident_detection(pyramid, measured, false);
        self.answer(TrackingState::Uncertain, peak_to_sidelobe_ratio)
    }

    /// Looks for the selection's appearance up to the recovery search's largest
    /// radius around `region`, and returns it if it is found clearly.
    fn find_selection_further_away(
        &self,
        pyramid: &ImagePyramid,
        region: &Region,
    ) -> Option<Region> {
        let search_radius = region.longer_side() * RECOVERY_SEARCH_MAXIMUM_IN_REGION_SIDES;
        let candidate = self
            .initial_appearance
            .search(pyramid, region, search_radius)
            .filter(|candidate| candidate.similarity >= RE_ANCHOR_MINIMUM_SIMILARITY)?;
        let around_candidate = region.moved_by(
            candidate.center.x - region.center_x,
            candidate.center.y - region.center_y,
        );
        self.initial_appearance
            .best_match_near(pyramid, &around_candidate, 1, &ANCHOR_SCALE_FACTORS)
            .filter(|found| found.similarity >= RE_ANCHOR_MINIMUM_SIMILARITY)
            .map(|found| found.region)
    }

    /// Moves the region to where the selection was found again, and starts the
    /// filter afresh there: what it learnt since it drifted is wrong.
    fn re_anchor(
        &mut self,
        pyramid: &ImagePyramid,
        found: Region,
        peak_to_sidelobe_ratio: f32,
    ) -> TrackedRegion {
        self.region = self.clamped_to_frame(found);
        self.last_motion = (0.0, 0.0);
        if let Some(filter) =
            CorrelationFilter::train(pyramid, filter_window(self.region.center(), &self.region))
        {
            self.filter = filter;
        }
        if let Some(appearance) = AppearanceTemplate::capture(pyramid, &self.region) {
            self.recent_appearance = appearance;
        }
        self.confident_frames_since_appearance_refresh = 0;
        // Like a region found again after a loss: uncertain until the next frame confirms.
        self.uncertain_frames_in_a_row = 1;
        self.answer(TrackingState::Uncertain, peak_to_sidelobe_ratio)
    }

    fn accept_confident_detection(
        &mut self,
        pyramid: &ImagePyramid,
        region: Region,
        may_refresh_appearance: bool,
    ) {
        self.move_region_to(region);
        self.uncertain_frames_in_a_row = 0;
        self.filter
            .learn(pyramid, filter_window(self.region.center(), &self.region));
        if !may_refresh_appearance {
            return;
        }
        self.confident_frames_since_appearance_refresh += 1;
        if self.confident_frames_since_appearance_refresh >= APPEARANCE_REFRESH_INTERVAL_IN_FRAMES {
            self.confident_frames_since_appearance_refresh = 0;
            if let Some(appearance) = AppearanceTemplate::capture(pyramid, &self.region) {
                self.recent_appearance = appearance;
            }
        }
    }

    fn lose_region(&mut self, peak_to_sidelobe_ratio: f32) -> TrackedRegion {
        self.lost_frames_in_a_row = Some(0);
        self.uncertain_frames_in_a_row = 0;
        self.last_motion = (0.0, 0.0);
        self.answer(TrackingState::Lost, peak_to_sidelobe_ratio)
    }

    fn look_for_lost_region(&mut self, pyramid: &ImagePyramid) -> TrackedRegion {
        let searches_before = self.lost_frames_in_a_row.unwrap_or(0);
        self.lost_frames_in_a_row = Some(searches_before + 1);
        // Where it was last seen: the obstacle may simply have gone.
        let at_last_position = self.detect_around(pyramid, self.region.center());
        let last_position_ratio =
            at_last_position.map_or(0.0, |detection| detection.peak.peak_to_sidelobe_ratio);
        if let Some(detection) = at_last_position.filter(is_confident) {
            return self.end_loss(detection);
        }
        // Further away, by appearance.
        let search_radius = self.region.longer_side()
            * (RECOVERY_SEARCH_START_IN_REGION_SIDES
                + RECOVERY_SEARCH_GROWTH_IN_REGION_SIDES_PER_FRAME * searches_before as f32)
                .min(RECOVERY_SEARCH_MAXIMUM_IN_REGION_SIDES);
        let best_match = [&self.initial_appearance, &self.recent_appearance]
            .into_iter()
            .filter_map(|appearance| appearance.search(pyramid, &self.region, search_radius))
            .max_by(|first, second| first.similarity.total_cmp(&second.similarity));
        if let Some(candidate) =
            best_match.filter(|candidate| candidate.similarity >= MINIMUM_RECOVERY_SIMILARITY)
            && let Some(detection) = self
                .detect_around(pyramid, candidate.center)
                .filter(is_confident)
        {
            return self.end_loss(detection);
        }
        self.answer(TrackingState::Lost, last_position_ratio)
    }

    fn end_loss(&mut self, detection: Detection) -> TrackedRegion {
        self.lost_frames_in_a_row = None;
        // Reported uncertain, so it counts towards the uncertain frames in a row.
        self.uncertain_frames_in_a_row = 1;
        self.region = self.clamped_to_frame(detection.region);
        self.last_motion = (0.0, 0.0);
        // Found again, but not learnt yet: uncertain until the next frame confirms.
        self.answer(
            TrackingState::Uncertain,
            detection.peak.peak_to_sidelobe_ratio,
        )
    }

    fn answer(&self, state: TrackingState, peak_to_sidelobe_ratio: f32) -> TrackedRegion {
        TrackedRegion {
            region: self.region,
            state,
            peak_to_sidelobe_ratio,
        }
    }

    /// Keeps the centre inside the frame and the size between the minimum side
    /// and the frame's size.
    fn clamped_to_frame(&self, region: Region) -> Region {
        let (frame_width, frame_height) = (self.frame_width as f32, self.frame_height as f32);
        let mut factor = 1.0_f32;
        if region.longer_side() > frame_width.max(frame_height) {
            factor = frame_width.max(frame_height) / region.longer_side();
        }
        let shorter_side = region.width.min(region.height) * factor;
        if shorter_side < MINIMUM_REGION_SIDE_IN_PIXELS {
            factor *= MINIMUM_REGION_SIDE_IN_PIXELS / shorter_side;
        }
        Region {
            center_x: region.center_x.clamp(0.0, frame_width),
            center_y: region.center_y.clamp(0.0, frame_height),
            ..region.scaled_by(factor)
        }
    }
}

/// `region` moved `gain` of the way towards `target`, and resized as much on a
/// logarithmic scale (`gain` = 1 gives `target`).
fn pulled_towards(region: Region, target: Region, gain: f32) -> Region {
    Region {
        center_x: region.center_x + gain * (target.center_x - region.center_x),
        center_y: region.center_y + gain * (target.center_y - region.center_y),
        width: region.width * (target.width / region.width).powf(gain),
        height: region.height * (target.height / region.height).powf(gain),
    }
}

fn is_confident(detection: &Detection) -> bool {
    detection.peak.peak_to_sidelobe_ratio >= CONFIDENT_PEAK_TO_SIDELOBE_RATIO
}

/// The correlation filter's window: centred on `center`, sized from `region`.
fn filter_window(center: Point, region: &Region) -> SquareWindow {
    SquareWindow {
        center,
        side_in_pixels: CONTEXT_FACTOR * region.longer_side(),
        region_width_in_pixels: region.width,
        region_height_in_pixels: region.height,
    }
}

fn check_region_can_be_tracked(frame: &GreyImage, region: &Region) -> Result<(), TrackingError> {
    let is_large_enough = region.width >= MINIMUM_REGION_SIDE_IN_PIXELS
        && region.height >= MINIMUM_REGION_SIDE_IN_PIXELS;
    if !is_large_enough {
        return Err(TrackingError::RegionTooSmall {
            width_in_pixels: region.width,
            height_in_pixels: region.height,
            minimum_side_in_pixels: MINIMUM_REGION_SIDE_IN_PIXELS,
        });
    }
    let is_inside = (0.0..=frame.width() as f32).contains(&region.center_x)
        && (0.0..=frame.height() as f32).contains(&region.center_y);
    if !is_inside {
        return Err(TrackingError::RegionOutsideImage {
            center_x: region.center_x,
            center_y: region.center_y,
            image_width: frame.width(),
            image_height: frame.height(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_shrunk_to_about_640_pixels_wide() {
        assert_eq!(shrink_factor_for_tracking(320), 1);
        assert_eq!(shrink_factor_for_tracking(640), 1);
        assert_eq!(shrink_factor_for_tracking(1280), 2);
        assert_eq!(shrink_factor_for_tracking(1920), 3);
        assert_eq!(shrink_factor_for_tracking(3840), 6);
        assert_eq!(shrink_factor_for_tracking(0), 1);
    }

    #[test]
    fn bad_selections_are_refused_with_a_reason() {
        let frame =
            GreyImage::from_fn(100, 80, |column, row| ((column * 7 + row * 13) % 50) as f32);
        let too_small = Region::from_top_left(10.0, 10.0, 5.0, 30.0);
        assert!(matches!(
            RegionTracker::start(frame.clone(), too_small),
            Err(TrackingError::RegionTooSmall { .. })
        ));
        let outside = Region::from_top_left(120.0, 10.0, 20.0, 20.0);
        assert!(matches!(
            RegionTracker::start(frame, outside),
            Err(TrackingError::RegionOutsideImage { .. })
        ));
        let uniform = GreyImage::from_fn(100, 80, |_, _| 128.0);
        let fine = Region::from_top_left(30.0, 30.0, 20.0, 20.0);
        assert_eq!(
            RegionTracker::start(uniform, fine).unwrap_err(),
            TrackingError::RegionWithoutTexture
        );
    }

    #[test]
    fn a_uniform_region_on_a_busy_background_is_refused_with_its_own_reason() {
        use crate::synthetic_scenes::textured_value;
        let frame = GreyImage::from_fn(200, 160, |column, row| {
            let is_inside_the_plate = (80..120).contains(&column) && (60..100).contains(&row);
            if is_inside_the_plate {
                20.0
            } else {
                40.0 + 180.0 * textured_value(column as f32, row as f32, 8.0, 5)
            }
        });
        // Inside the plate, away from its edges; the filter's window still sees
        // the textured background around it.
        let plate = Region::from_top_left(85.0, 65.0, 30.0, 30.0);
        assert_eq!(
            RegionTracker::start(frame, plate).unwrap_err(),
            TrackingError::RegionWithoutDetail
        );
    }

    /// The object of the drift tests, and a textured plate that does not look
    /// like it, two region sizes to its right.
    fn object_and_plate(with_plate: bool) -> (GreyImage, Region, Region) {
        use crate::synthetic_scenes::{SceneFrame, SceneObject};
        let object = Region::from_top_left(60.0, 90.0, 40.0, 48.0);
        let plate = object.moved_by(96.0, 0.0);
        let frame = SceneFrame {
            width: 320,
            height: 240,
            background_seed: 3,
            background_offset_in_pixels: 0.0,
            object: Some(SceneObject {
                region: object,
                texture_seed: 4,
            }),
            obstacle: with_plate.then_some(plate),
        }
        .render();
        (frame, object, plate)
    }

    /// Makes the tracker believe the user selected `selection` instead: what it
    /// follows then no longer looks like the selection, as after a drift.
    fn pretend_selection_was(tracker: &mut RegionTracker, frame: &GreyImage, selection: &Region) {
        let pyramid = ImagePyramid::new(frame.clone(), PYRAMID_SMALLEST_SIDE_IN_PIXELS);
        tracker.initial_appearance = AppearanceTemplate::capture(&pyramid, selection).unwrap();
    }

    #[test]
    fn a_sure_match_that_no_longer_looks_like_the_selection_is_uncertain() {
        let (with_plate, object, plate) = object_and_plate(true);
        let (without_plate, _, _) = object_and_plate(false);
        let mut tracker = RegionTracker::start(without_plate.clone(), object).unwrap();
        pretend_selection_was(&mut tracker, &with_plate, &plate);
        let tracked = tracker.track(without_plate).unwrap();
        assert!(
            tracked.peak_to_sidelobe_ratio >= CONFIDENT_PEAK_TO_SIDELOBE_RATIO,
            "{tracked:?}"
        );
        assert_eq!(tracked.state, TrackingState::Uncertain, "{tracked:?}");
        assert!(
            tracked.region.center_distance_to(&object) < 1.0,
            "{tracked:?}"
        );
    }

    #[test]
    fn a_drifted_region_is_moved_back_to_the_selection_nearby() {
        let (frame, object, plate) = object_and_plate(true);
        let mut tracker = RegionTracker::start(frame.clone(), object).unwrap();
        pretend_selection_was(&mut tracker, &frame, &plate);
        let re_anchored = tracker.track(frame.clone()).unwrap();
        assert_eq!(
            re_anchored.state,
            TrackingState::Uncertain,
            "{re_anchored:?}"
        );
        assert!(
            re_anchored.region.center_distance_to(&plate) < 1.5,
            "{re_anchored:?}"
        );
        // The filter started afresh there: the next frame is sure again.
        let confirmed = tracker.track(frame).unwrap();
        assert_eq!(confirmed.state, TrackingState::Confident, "{confirmed:?}");
        assert!(
            confirmed.region.center_distance_to(&plate) < 1.5,
            "{confirmed:?}"
        );
    }

    #[test]
    fn frames_of_another_size_are_refused() {
        let frame =
            GreyImage::from_fn(100, 80, |column, row| ((column * 7 + row * 13) % 50) as f32);
        let mut tracker =
            RegionTracker::start(frame, Region::from_top_left(30.0, 30.0, 20.0, 20.0)).unwrap();
        let other = GreyImage::from_fn(90, 80, |_, _| 0.0);
        assert!(matches!(
            tracker.track(other),
            Err(TrackingError::FrameSizeChanged { .. })
        ));
    }
}
