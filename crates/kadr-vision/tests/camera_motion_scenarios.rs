//! Camera motion scenarios on synthetic videos.
//!
//! A textured background is filmed by a camera that shifts, rolls and comes closer
//! by known amounts, sometimes with an object moving on its own, noise, a cut or
//! a uniform frame. The true motion between frames is known, so the measured one
//! is checked exactly. Run with `--nocapture` to see the measured errors.

// The helpers below unwrap: clippy's test exemption only covers `#[test]` functions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use kadr_vision::camera_motion::CameraMotionEstimator;
use kadr_vision::synthetic_scenes::{SceneObject, ShakenCameraFrame};
use kadr_vision::{GreyImage, Point, Region, SimilarityTransform};

/// The frames are the size the analysis works at: a 1080p video shrunk three times.
const WIDTH: usize = 640;
const HEIGHT: usize = 360;
const CENTER: Point = Point {
    x: WIDTH as f32 / 2.0,
    y: HEIGHT as f32 / 2.0,
};

const BACKGROUND_SEED: u32 = 0x00ba_c6e0;
const OTHER_BACKGROUND_SEED: u32 = 0x0c07_5eed;
const OBJECT_SEED: u32 = 0x5eed_0b1e;

/// Largest errors accepted on a measured motion, in tracking pixels, degrees and
/// relative size.
const MAXIMUM_SHIFT_ERROR_IN_PIXELS: f32 = 0.05;
const MAXIMUM_ROLL_ERROR_IN_DEGREES: f32 = 0.01;
const MAXIMUM_SCALE_ERROR: f32 = 5e-4;

/// What one frame of a scenario shows.
#[derive(Debug, Clone, Copy)]
struct ScenarioFrame {
    camera: SimilarityTransform,
    background_seed: u32,
    moving_object: Option<Region>,
    noise_amplitude: f32,
    is_uniform: bool,
}

impl ScenarioFrame {
    fn filmed_with(camera: SimilarityTransform) -> Self {
        Self {
            camera,
            background_seed: BACKGROUND_SEED,
            moving_object: None,
            noise_amplitude: 0.0,
            is_uniform: false,
        }
    }

    fn render(&self, noise_seed: u32) -> GreyImage {
        if self.is_uniform {
            return GreyImage::from_fn(WIDTH, HEIGHT, |_, _| 110.0);
        }
        ShakenCameraFrame {
            width: WIDTH,
            height: HEIGHT,
            background_seed: self.background_seed,
            camera: self.camera,
            moving_object: self.moving_object.map(|region| SceneObject {
                region,
                texture_seed: OBJECT_SEED,
            }),
            noise_amplitude: self.noise_amplitude,
            noise_seed,
        }
        .render()
    }
}

/// A camera shifted, rolled and scaled about the frame's centre.
fn camera(shift_x: f32, shift_y: f32, rotation_in_degrees: f32, scale: f32) -> SimilarityTransform {
    SimilarityTransform {
        center: CENTER,
        shift_x_in_pixels: shift_x,
        shift_y_in_pixels: shift_y,
        rotation_in_degrees,
        scale_factor: scale,
    }
}

/// Cameras that each add `motions[i]` to the previous one, starting still.
fn cameras_moved_by(motions: &[SimilarityTransform]) -> Vec<SimilarityTransform> {
    let mut cameras = vec![SimilarityTransform::identity_about(CENTER)];
    for motion in motions {
        let last = *cameras.last().unwrap();
        cameras.push(last.followed_by(motion));
    }
    cameras
}

/// The measured motion of each frame after the first.
fn measure(frames: &[ScenarioFrame]) -> Vec<Option<SimilarityTransform>> {
    let mut estimator = CameraMotionEstimator::new();
    let mut motions = Vec::new();
    for (index, frame) in frames.iter().enumerate() {
        let motion = estimator.push_frame(frame.render(index as u32)).unwrap();
        if index == 0 {
            assert_eq!(motion, None, "the first frame has no motion");
        } else {
            motions.push(motion);
        }
    }
    motions
}

/// The true motion from `previous` to `current`, about the frame's centre.
fn true_motion(previous: &ScenarioFrame, current: &ScenarioFrame) -> SimilarityTransform {
    previous
        .camera
        .inverse()
        .followed_by(&current.camera)
        .about(CENTER)
}

/// Errors of one measured motion: shift, roll, size.
fn errors_of(measured: &SimilarityTransform, truth: &SimilarityTransform) -> (f32, f32, f32) {
    assert_eq!(
        measured.center, CENTER,
        "motions are about the frame's centre"
    );
    (
        (measured.shift_x_in_pixels - truth.shift_x_in_pixels)
            .hypot(measured.shift_y_in_pixels - truth.shift_y_in_pixels),
        (measured.rotation_in_degrees - truth.rotation_in_degrees).abs(),
        (measured.scale_factor - truth.scale_factor).abs(),
    )
}

/// Checks every motion of the scenario against the truth, within the thresholds.
fn check_scenario(scenario: &str, frames: &[ScenarioFrame]) {
    let measured = measure(frames);
    let mut worst = (0.0_f32, 0.0_f32, 0.0_f32);
    for (index, motion) in measured.iter().enumerate() {
        let truth = true_motion(&frames[index], &frames[index + 1]);
        let motion =
            motion.unwrap_or_else(|| panic!("{scenario}: frame {} not measured", index + 1));
        let (shift, roll, scale) = errors_of(&motion, &truth);
        worst = (worst.0.max(shift), worst.1.max(roll), worst.2.max(scale));
        assert!(
            shift <= MAXIMUM_SHIFT_ERROR_IN_PIXELS
                && roll <= MAXIMUM_ROLL_ERROR_IN_DEGREES
                && scale <= MAXIMUM_SCALE_ERROR,
            "{scenario}: frame {}: measured {motion:?}, truth {truth:?}",
            index + 1
        );
    }
    eprintln!(
        "{scenario}: {} motions, worst shift error {:.4} px, roll {:.5}°, scale {:.6}",
        measured.len(),
        worst.0,
        worst.1,
        worst.2
    );
}

fn frames_for(motions: &[SimilarityTransform]) -> Vec<ScenarioFrame> {
    cameras_moved_by(motions)
        .into_iter()
        .map(ScenarioFrame::filmed_with)
        .collect()
}

#[test]
fn a_pure_shift_is_measured() {
    let motions = [
        camera(2.3, -1.4, 0.0, 1.0),
        camera(-5.6, 3.25, 0.0, 1.0),
        camera(0.4, 0.7, 0.0, 1.0),
        camera(12.0, -8.5, 0.0, 1.0),
        camera(-0.15, 0.05, 0.0, 1.0),
    ];
    check_scenario("pure shift", &frames_for(&motions));
}

#[test]
fn a_clockwise_roll_is_measured_with_its_sign() {
    let motions = [
        camera(0.0, 0.0, 0.8, 1.0),
        camera(0.0, 0.0, -0.5, 1.0),
        camera(0.0, 0.0, 1.5, 1.0),
    ];
    let frames = frames_for(&motions);
    check_scenario("roll", &frames);
    // The camera turned clockwise: the picture's content turns clockwise too, so
    // a point right of the centre moves down.
    let first = measure(&frames[..2])[0].unwrap();
    assert!(first.rotation_in_degrees > 0.7, "{first:?}");
    let moved = first.apply_to(Point {
        x: CENTER.x + 200.0,
        y: CENTER.y,
    });
    assert!(moved.y > CENTER.y + 2.0, "{moved:?}");
}

#[test]
fn a_change_of_size_is_measured() {
    let motions = [
        camera(0.0, 0.0, 0.0, 1.02),
        camera(0.0, 0.0, 0.0, 0.985),
        camera(0.0, 0.0, 0.0, 1.004),
    ];
    check_scenario("change of size", &frames_for(&motions));
}

#[test]
fn shift_roll_and_size_together() {
    let motions = [
        camera(3.5, -2.0, 0.6, 1.01),
        camera(-4.25, 1.5, -0.9, 0.992),
        camera(1.0, 6.0, 0.3, 1.015),
        camera(-7.0, -3.0, -0.4, 0.99),
    ];
    check_scenario("shift, roll and size", &frames_for(&motions));
}

#[test]
fn a_moving_object_covering_a_third_is_ignored() {
    // An ellipse of 360 × 272 pixels covers a third of the 640 × 360 frame, and
    // walks to the right while the camera shakes the other way.
    let motions = [
        camera(-2.0, 1.0, 0.2, 1.0),
        camera(-1.5, -0.5, -0.3, 1.005),
        camera(-2.5, 0.75, 0.1, 0.997),
        camera(-1.0, 0.25, 0.0, 1.0),
    ];
    let frames: Vec<ScenarioFrame> = cameras_moved_by(&motions)
        .into_iter()
        .enumerate()
        .map(|(index, camera)| ScenarioFrame {
            moving_object: Some(Region::from_top_left(
                60.0 + 14.0 * index as f32,
                40.0 + 3.0 * index as f32,
                360.0,
                272.0,
            )),
            ..ScenarioFrame::filmed_with(camera)
        })
        .collect();
    let covered = std::f32::consts::FRAC_PI_4 * 360.0 * 272.0 / (WIDTH * HEIGHT) as f32;
    assert!(covered > 0.33, "the object covers {covered}");
    check_scenario("moving object", &frames);
}

#[test]
fn a_uniform_frame_is_not_measured() {
    let mut frames = frames_for(&[camera(1.0, 1.0, 0.0, 1.0), camera(1.0, 1.0, 0.0, 1.0)]);
    frames[1].is_uniform = true;
    let measured = measure(&frames);
    assert_eq!(measured[0], None, "into the uniform frame");
    assert_eq!(measured[1], None, "out of the uniform frame");
}

#[test]
fn a_cut_is_not_measured() {
    // Two shots of different places: the motion across the cut is unknown, and
    // the second shot's own motion is measured again from the next frame.
    let mut frames = frames_for(&[
        camera(2.0, 0.0, 0.0, 1.0),
        camera(2.0, 0.0, 0.0, 1.0),
        camera(-3.0, 1.0, 0.0, 1.0),
    ]);
    for frame in &mut frames[2..] {
        frame.background_seed = OTHER_BACKGROUND_SEED;
    }
    let measured = measure(&frames);
    assert!(measured[0].is_some(), "before the cut");
    assert_eq!(measured[1], None, "across the cut");
    let after = measured[2].expect("after the cut");
    let (shift, _, _) = errors_of(&after, &true_motion(&frames[2], &frames[3]));
    assert!(shift <= MAXIMUM_SHIFT_ERROR_IN_PIXELS, "{after:?}");
}

#[test]
fn a_fast_pan_is_followed_through_the_prediction() {
    // The pan speeds up to 40 pixels per frame (a whole frame in 16 frames) and
    // keeps that speed. Each frame starts from the previous frame's motion, so
    // the optical flow only has to find the change of speed.
    let motions: Vec<SimilarityTransform> = [-8.0, -16.0, -24.0, -32.0, -40.0, -40.0, -40.0, -40.0]
        .into_iter()
        .map(|shift_x| camera(shift_x, 1.5, 0.0, 1.0))
        .collect();
    check_scenario("fast pan", &frames_for(&motions));
}

#[test]
fn noise_does_not_bias_the_motion() {
    // Sensor noise of ±6 grey levels, different in every frame.
    let motions = [
        camera(1.5, -0.75, 0.2, 1.003),
        camera(-2.25, 1.0, -0.1, 0.998),
        camera(0.5, 0.5, 0.05, 1.0),
        camera(3.0, -1.5, -0.25, 1.006),
        camera(-1.0, 2.0, 0.15, 0.996),
        camera(0.25, -0.5, 0.0, 1.0),
    ];
    let frames: Vec<ScenarioFrame> = frames_for(&motions)
        .into_iter()
        .map(|frame| ScenarioFrame {
            noise_amplitude: 6.0,
            ..frame
        })
        .collect();
    check_scenario("noise", &frames);
    // On average, the errors cancel out: no bias in any direction.
    let measured = measure(&frames);
    let (mut sum_x, mut sum_y) = (0.0_f32, 0.0_f32);
    for (index, motion) in measured.iter().enumerate() {
        let truth = true_motion(&frames[index], &frames[index + 1]);
        let motion = motion.unwrap();
        sum_x += motion.shift_x_in_pixels - truth.shift_x_in_pixels;
        sum_y += motion.shift_y_in_pixels - truth.shift_y_in_pixels;
    }
    let count = measured.len() as f32;
    eprintln!(
        "noise: mean error ({:.4}, {:.4}) px",
        sum_x / count,
        sum_y / count
    );
    assert!((sum_x / count).abs() < 0.03 && (sum_y / count).abs() < 0.03);
}
