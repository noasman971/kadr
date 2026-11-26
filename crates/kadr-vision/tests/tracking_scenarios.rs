//! Tracking scenarios on synthetic videos.
//!
//! A textured ellipse (the stand-in for a face) moves, speeds up, grows, shrinks,
//! hides behind an obstacle or leaves the frame, over a textured background. The
//! true position of the object is known in every frame, so the tracking error is
//! measured exactly. Run with `--nocapture` to see the measured errors.

// The helpers below unwrap: clippy's test exemption only covers `#[test]` functions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use kadr_vision::synthetic_scenes::{SceneFrame, SceneObject};
use kadr_vision::{GreyImage, Region, RegionTracker, TrackedRegion, TrackingState};

const OBJECT_TEXTURE_SEED: u32 = 0x5eed_0b1e;
const BACKGROUND_TEXTURE_SEED: u32 = 0x00ba_c6e0;

/// What one frame of a scenario shows.
#[derive(Debug, Clone, Copy)]
struct ScenarioFrame {
    /// Where the object truly is, if it is in the scene.
    object: Option<Region>,
    /// An obstacle hiding what is behind it.
    obstacle: Option<Region>,
    /// How far the background has moved to the left.
    background_offset_in_pixels: f32,
}

impl ScenarioFrame {
    fn object_at(object: Region) -> Self {
        Self {
            object: Some(object),
            obstacle: None,
            background_offset_in_pixels: 0.0,
        }
    }

    fn render(&self, width: usize, height: usize) -> GreyImage {
        SceneFrame {
            width,
            height,
            background_seed: BACKGROUND_TEXTURE_SEED,
            background_offset_in_pixels: self.background_offset_in_pixels,
            object: self.object.map(|region| SceneObject {
                region,
                texture_seed: OBJECT_TEXTURE_SEED,
            }),
            obstacle: self.obstacle,
        }
        .render()
    }
}

/// Starts the tracker on the first frame with the true region, then tracks
/// every following frame. The answer for the first frame is the selection itself.
fn track_scenario(frames: &[ScenarioFrame], width: usize, height: usize) -> Vec<TrackedRegion> {
    let selection = frames[0]
        .object
        .expect("the object is visible on the first frame");
    let mut tracker = RegionTracker::start(frames[0].render(width, height), selection).unwrap();
    let mut track = vec![TrackedRegion {
        region: selection,
        state: TrackingState::Confident,
        peak_to_sidelobe_ratio: f32::INFINITY,
    }];
    for frame in &frames[1..] {
        track.push(tracker.track(frame.render(width, height)).unwrap());
    }
    track
}

/// Centre errors of the frames where the object is visible.
fn center_errors(frames: &[ScenarioFrame], track: &[TrackedRegion]) -> Vec<f32> {
    frames
        .iter()
        .zip(track)
        .filter_map(|(frame, tracked)| {
            frame
                .object
                .map(|object| object.center_distance_to(&tracked.region))
        })
        .collect()
}

fn report(scenario: &str, frames: &[ScenarioFrame], track: &[TrackedRegion]) -> (f32, f32) {
    let errors = center_errors(frames, track);
    let mean = errors.iter().sum::<f32>() / errors.len() as f32;
    let maximum = errors.iter().copied().fold(0.0, f32::max);
    let confident = track
        .iter()
        .filter(|tracked| tracked.state == TrackingState::Confident)
        .count();
    eprintln!(
        "{scenario}: {} frames, centre error mean {mean:.2} px, max {maximum:.2} px, {confident} confident",
        frames.len()
    );
    (mean, maximum)
}

fn states_of(track: &[TrackedRegion]) -> String {
    track
        .iter()
        .map(|tracked| match tracked.state {
            TrackingState::Confident => 'C',
            TrackingState::Uncertain => 'u',
            TrackingState::Lost => '.',
        })
        .collect()
}

#[test]
fn follows_a_region_moving_steadily() {
    let frames: Vec<ScenarioFrame> = (0..80)
        .map(|index| {
            let time = index as f32;
            ScenarioFrame::object_at(Region::from_top_left(
                60.0 + 2.5 * time,
                70.0 + 1.2 * time,
                40.0,
                48.0,
            ))
        })
        .collect();
    let track = track_scenario(&frames, 320, 240);
    let (mean, maximum) = report("steady motion", &frames, &track);
    assert!(mean < 1.0 && maximum < 2.5, "{}", states_of(&track));
    assert!(
        track
            .iter()
            .all(|tracked| tracked.state == TrackingState::Confident),
        "{}",
        states_of(&track)
    );
}

#[test]
fn tracks_backwards_as_well_as_forwards() {
    let mut frames: Vec<ScenarioFrame> = (0..60)
        .map(|index| {
            let time = index as f32;
            ScenarioFrame::object_at(Region::from_top_left(
                50.0 + 3.0 * time,
                90.0 - 0.8 * time,
                44.0,
                52.0,
            ))
        })
        .collect();
    frames.reverse();
    let track = track_scenario(&frames, 320, 240);
    let (mean, maximum) = report("backwards", &frames, &track);
    assert!(mean < 1.0 && maximum < 2.5, "{}", states_of(&track));
}

#[test]
fn follows_a_fast_accelerating_region() {
    // From 4 to 36 pixels per frame: well beyond half the region's width, which
    // is all the correlation filter sees without the motion prediction.
    let mut left = 20.0;
    let mut frames = Vec::new();
    for index in 0..20 {
        let speed = (4.0 * (index + 1) as f32).min(36.0);
        let top = 150.0 + 20.0 * (index as f32 * 0.3).sin();
        frames.push(ScenarioFrame::object_at(Region::from_top_left(
            left, top, 48.0, 56.0,
        )));
        left += speed;
    }
    let track = track_scenario(&frames, 640, 360);
    let (_, maximum) = report("fast motion", &frames, &track);
    assert!(maximum < 4.0, "{}", states_of(&track));
    assert!(
        track
            .iter()
            .all(|tracked| tracked.state != TrackingState::Lost),
        "{}",
        states_of(&track)
    );
}

#[test]
fn follows_a_region_coming_closer() {
    let frames: Vec<ScenarioFrame> = (0..80)
        .map(|index| {
            let scale = 1.8_f32.powf(index as f32 / 79.0);
            let region = Region {
                center_x: 150.0 + 0.5 * index as f32,
                center_y: 115.0,
                width: 40.0 * scale,
                height: 48.0 * scale,
            };
            ScenarioFrame::object_at(region)
        })
        .collect();
    let track = track_scenario(&frames, 320, 240);
    let (_, maximum) = report("coming closer", &frames, &track);
    let (last_frame, last_tracked) = (frames[79].object.unwrap(), track[79].region);
    let size_error = (last_tracked.width / last_frame.width - 1.0).abs();
    eprintln!(
        "coming closer: final width {} for {}",
        last_tracked.width, last_frame.width
    );
    assert!(maximum < 4.0, "{}", states_of(&track));
    assert!(
        size_error < 0.1,
        "final width {} for {}",
        last_tracked.width,
        last_frame.width
    );
}

#[test]
fn follows_a_region_moving_away() {
    let frames: Vec<ScenarioFrame> = (0..80)
        .map(|index| {
            let scale = 0.6_f32.powf(index as f32 / 79.0);
            let region = Region {
                center_x: 160.0,
                center_y: 120.0 - 0.3 * index as f32,
                width: 64.0 * scale,
                height: 76.0 * scale,
            };
            ScenarioFrame::object_at(region)
        })
        .collect();
    let track = track_scenario(&frames, 320, 240);
    let (_, maximum) = report("moving away", &frames, &track);
    let (last_frame, last_tracked) = (frames[79].object.unwrap(), track[79].region);
    let size_error = (last_tracked.width / last_frame.width - 1.0).abs();
    eprintln!(
        "moving away: final width {} for {}",
        last_tracked.width, last_frame.width
    );
    assert!(maximum < 4.0, "{}", states_of(&track));
    assert!(
        size_error < 0.1,
        "final width {} for {}",
        last_tracked.width,
        last_frame.width
    );
}

#[test]
fn ignores_the_background_when_the_camera_pans() {
    // The camera follows the object: the object stays still, the background slides.
    let frames: Vec<ScenarioFrame> = (0..60)
        .map(|index| ScenarioFrame {
            object: Some(Region::from_top_left(140.0, 90.0, 40.0, 48.0)),
            obstacle: None,
            background_offset_in_pixels: 6.0 * index as f32,
        })
        .collect();
    let track = track_scenario(&frames, 320, 240);
    let (_, maximum) = report("camera pan", &frames, &track);
    assert!(maximum < 2.0, "{}", states_of(&track));
}

#[test]
fn reports_a_hidden_region_as_lost_then_finds_it_again() {
    const HIDDEN_FROM: usize = 30;
    const HIDDEN_UNTIL: usize = 45;
    let object_at =
        |index: usize| Region::from_top_left(100.0 + 1.0 * index as f32, 90.0, 40.0, 48.0);
    let obstacle = Region {
        width: 80.0,
        height: 70.0,
        ..object_at((HIDDEN_FROM + HIDDEN_UNTIL) / 2)
    };
    let frames: Vec<ScenarioFrame> = (0..80)
        .map(|index| {
            let is_hidden = (HIDDEN_FROM..HIDDEN_UNTIL).contains(&index);
            ScenarioFrame {
                object: (!is_hidden).then(|| object_at(index)),
                obstacle: is_hidden.then_some(obstacle),
                background_offset_in_pixels: 0.0,
            }
        })
        .collect();
    let track = track_scenario(&frames, 320, 240);
    report("hidden then visible", &frames, &track);
    let states = states_of(&track);
    eprintln!("hidden then visible: {states}");
    // While hidden: never confident, and lost for most of it.
    let while_hidden = &track[HIDDEN_FROM + 1..HIDDEN_UNTIL];
    assert!(
        while_hidden
            .iter()
            .all(|tracked| tracked.state != TrackingState::Confident),
        "{states}"
    );
    let lost_while_hidden = while_hidden
        .iter()
        .filter(|tracked| tracked.state == TrackingState::Lost)
        .count();
    assert!(lost_while_hidden * 2 >= while_hidden.len(), "{states}");
    // It does not wander off while hidden.
    assert!(
        while_hidden
            .iter()
            .all(|tracked| tracked.region.center_distance_to(&object_at(HIDDEN_FROM)) < 25.0),
        "{states}"
    );
    // Found again within a few frames, at the right place.
    let found_again = (HIDDEN_UNTIL..HIDDEN_UNTIL + 5)
        .find(|&index| track[index].state == TrackingState::Confident)
        .unwrap_or_else(|| panic!("not found again: {states}"));
    for (index, tracked) in track.iter().enumerate().skip(found_again) {
        let error = object_at(index).center_distance_to(&tracked.region);
        assert!(error < 2.5, "frame {index}: error {error}, {states}");
    }
}

#[test]
fn reports_a_region_leaving_the_frame_as_lost() {
    let frames: Vec<ScenarioFrame> = (0..70)
        .map(|index| {
            let region = Region::from_top_left(150.0 + 4.0 * index as f32, 100.0, 40.0, 48.0);
            // Fully outside once its left edge passes the right edge of the frame.
            ScenarioFrame {
                object: (region.left() < 320.0).then_some(region),
                obstacle: None,
                background_offset_in_pixels: 0.0,
            }
        })
        .collect();
    let track = track_scenario(&frames, 320, 240);
    let states = states_of(&track);
    eprintln!("leaving the frame: {states}");
    let first_frame_fully_out = frames
        .iter()
        .position(|frame| frame.object.is_none())
        .unwrap();
    assert!(
        track[first_frame_fully_out + 3..]
            .iter()
            .all(|tracked| tracked.state == TrackingState::Lost),
        "{states}"
    );
}
