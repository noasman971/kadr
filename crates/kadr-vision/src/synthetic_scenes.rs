//! Synthetic scenes for testing and measuring the tracker and the camera motion.
//!
//! A textured ellipse (the stand-in for a face) moves, grows or hides behind an
//! obstacle over a textured background ([`SceneFrame`]). Every pixel is computed
//! from formulas, so the true position of the object is known exactly in every
//! frame and the tracking error can be measured.
//!
//! A [`ShakenCameraFrame`] films a textured background through a known similarity
//! transform (a shaky camera's shift, roll and change of size), with an optional
//! object moving on its own and noise, so that the measured camera motion can be
//! compared with the true one.
//!
//! Textures are value noise: random brightness on a grid, smoothly interpolated,
//! at three scales. They are deterministic: the same seed gives the same texture.

use crate::geometry::{Point, Region, SimilarityTransform};
use crate::grey_image::GreyImage;

/// Typical size of a background detail, in pixels.
const BACKGROUND_FEATURE_SIZE_IN_PIXELS: f32 = 14.0;

/// Typical size of an object detail, relative to the object's width.
const OBJECT_FEATURE_SIZE_RELATIVE_TO_WIDTH: f32 = 0.15;

/// Seed of the obstacle's texture.
const OBSTACLE_TEXTURE_SEED: u32 = 0x0b57_ac1e;

/// A textured ellipse filling `region`. Its texture grows and shrinks with it,
/// like a face coming closer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneObject {
    /// Where the object is and how big.
    pub region: Region,
    /// Seed of its texture.
    pub texture_seed: u32,
}

/// The content of one synthetic frame.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneFrame {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// Seed of the background texture.
    pub background_seed: u32,
    /// How far the background has moved to the left (a camera pan), in pixels.
    pub background_offset_in_pixels: f32,
    /// The object, if it is in the scene.
    pub object: Option<SceneObject>,
    /// An opaque rectangle drawn over everything, hiding what is behind it.
    pub obstacle: Option<Region>,
}

impl SceneFrame {
    /// Draws the frame.
    pub fn render(&self) -> GreyImage {
        GreyImage::from_fn(self.width, self.height, |column, row| {
            let (x, y) = (column as f32 + 0.5, row as f32 + 0.5);
            if let Some(obstacle) = &self.obstacle
                && (x - obstacle.center_x).abs() <= obstacle.width / 2.0
                && (y - obstacle.center_y).abs() <= obstacle.height / 2.0
            {
                return 90.0 + 40.0 * textured_value(x, y, 9.0, OBSTACLE_TEXTURE_SEED);
            }
            let background = 30.0
                + 170.0
                    * textured_value(
                        x + self.background_offset_in_pixels,
                        y,
                        BACKGROUND_FEATURE_SIZE_IN_PIXELS,
                        self.background_seed,
                    );
            match &self.object {
                Some(object) => object.drawn_over(background, x, y),
                None => background,
            }
        })
    }
}

/// One frame filmed by a shaky camera: a textured background seen through a known
/// similarity transform, an optional object moving on its own, and noise.
///
/// The motion of the camera from a frame filmed with `previous` to one filmed with
/// `current` is `previous.inverse().followed_by(&current)`: it maps a point of the
/// first frame's content to where the second frame shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct ShakenCameraFrame {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// Seed of the background texture.
    pub background_seed: u32,
    /// Where the camera shows the background: the background's point `p` (in the
    /// pixels of a frame filmed with the identity) is seen at `camera.apply_to(p)`.
    pub camera: SimilarityTransform,
    /// An object drawn over the background where this frame shows it: it does not
    /// move with the camera.
    pub moving_object: Option<SceneObject>,
    /// Largest brightness the noise adds or removes, in grey levels (0 = none).
    pub noise_amplitude: f32,
    /// Seed of the noise: another seed gives other noise, as in the next frame of
    /// a real video.
    pub noise_seed: u32,
}

impl ShakenCameraFrame {
    /// Draws the frame.
    pub fn render(&self) -> GreyImage {
        let seen_from = self.camera.inverse();
        GreyImage::from_fn(self.width, self.height, |column, row| {
            let (x, y) = (column as f32 + 0.5, row as f32 + 0.5);
            let seen = seen_from.apply_to(Point { x, y });
            let background = 30.0
                + 170.0
                    * textured_value(
                        seen.x,
                        seen.y,
                        BACKGROUND_FEATURE_SIZE_IN_PIXELS,
                        self.background_seed,
                    );
            let value = match &self.moving_object {
                Some(object) => object.drawn_over(background, x, y),
                None => background,
            };
            let noise = 2.0 * lattice_value(column as i32, row as i32, self.noise_seed) - 1.0;
            value + self.noise_amplitude * noise
        })
    }
}

impl SceneObject {
    /// The brightness at (`x`, `y`) with the object drawn over `background`.
    fn drawn_over(&self, background: f32, x: f32, y: f32) -> f32 {
        let coverage = ellipse_coverage(&self.region, x, y);
        if coverage <= 0.0 {
            return background;
        }
        let feature_size = self.region.width * OBJECT_FEATURE_SIZE_RELATIVE_TO_WIDTH;
        let local_x = (x - self.region.left()) / feature_size;
        let local_y = (y - self.region.top()) / feature_size;
        let object_value = 40.0 + 200.0 * textured_value(local_x, local_y, 1.0, self.texture_seed);
        background * (1.0 - coverage) + object_value * coverage
    }
}

/// How much of the pixel at (`x`, `y`) the ellipse filling `region` covers,
/// from 0 to 1, with a one-pixel soft edge so that sub-pixel motion is smooth.
fn ellipse_coverage(region: &Region, x: f32, y: f32) -> f32 {
    let normalized_x = (x - region.center_x) / (region.width / 2.0);
    let normalized_y = (y - region.center_y) / (region.height / 2.0);
    let normalized_radius = normalized_x.hypot(normalized_y);
    let distance_inside_in_pixels =
        (1.0 - normalized_radius) * region.width.min(region.height) / 2.0;
    (distance_inside_in_pixels + 0.5).clamp(0.0, 1.0)
}

/// A smooth random texture, from 0 to 1, with details about
/// `feature_size_in_pixels` wide.
pub fn textured_value(x: f32, y: f32, feature_size_in_pixels: f32, seed: u32) -> f32 {
    let (x, y) = (x / feature_size_in_pixels, y / feature_size_in_pixels);
    0.5 * value_noise(x, y, seed)
        + 0.3 * value_noise(x * 2.0, y * 2.0, seed.wrapping_add(1))
        + 0.2 * value_noise(x * 4.0, y * 4.0, seed.wrapping_add(2))
}

fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (left, top) = (x.floor(), y.floor());
    let (weight_right, weight_bottom) = (smooth_step(x - left), smooth_step(y - top));
    let (column, row) = (left as i32, top as i32);
    let top_mix = lattice_value(column, row, seed) * (1.0 - weight_right)
        + lattice_value(column + 1, row, seed) * weight_right;
    let bottom_mix = lattice_value(column, row + 1, seed) * (1.0 - weight_right)
        + lattice_value(column + 1, row + 1, seed) * weight_right;
    top_mix * (1.0 - weight_bottom) + bottom_mix * weight_bottom
}

fn smooth_step(fraction: f32) -> f32 {
    fraction * fraction * (3.0 - 2.0 * fraction)
}

/// A pseudo-random value from 0 to 1 for one grid point (integer hashing).
fn lattice_value(column: i32, row: i32, seed: u32) -> f32 {
    let mut hash = (column as u32).wrapping_mul(0x8da6_b343)
        ^ (row as u32).wrapping_mul(0xd816_3841)
        ^ seed.wrapping_mul(0xcb1a_b31f);
    hash ^= hash >> 13;
    hash = hash.wrapping_mul(0x5bd1_e995);
    hash ^= hash >> 15;
    (hash & 0xffff) as f32 / 65_535.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene_with(object: Option<SceneObject>, obstacle: Option<Region>) -> SceneFrame {
        SceneFrame {
            width: 64,
            height: 48,
            background_seed: 1,
            background_offset_in_pixels: 0.0,
            object,
            obstacle,
        }
    }

    #[test]
    fn textures_are_deterministic_and_within_range() {
        let values: Vec<f32> = (0..500)
            .map(|index| textured_value(index as f32 * 0.7, 3.0, 5.0, 42))
            .collect();
        assert!(values.iter().all(|value| (0.0..=1.0).contains(value)));
        assert_eq!(
            textured_value(12.5, 7.0, 5.0, 42),
            textured_value(12.5, 7.0, 5.0, 42)
        );
        assert_ne!(
            textured_value(12.5, 7.0, 5.0, 42),
            textured_value(12.5, 7.0, 5.0, 43)
        );
    }

    #[test]
    fn the_object_and_the_obstacle_change_the_pixels_they_cover() {
        let object = SceneObject {
            region: Region::from_top_left(20.0, 10.0, 20.0, 24.0),
            texture_seed: 7,
        };
        let empty = scene_with(None, None).render();
        let with_object = scene_with(Some(object), None).render();
        assert_ne!(
            empty.value(30, 22),
            with_object.value(30, 22),
            "inside the ellipse"
        );
        assert_eq!(empty.value(2, 2), with_object.value(2, 2), "far outside");
        let hidden = scene_with(Some(object), Some(object.region.scaled_by(1.5))).render();
        let obstacle_only = scene_with(None, Some(object.region.scaled_by(1.5))).render();
        assert_eq!(hidden.value(30, 22), obstacle_only.value(30, 22));
    }

    #[test]
    fn a_shaken_camera_shows_the_background_where_its_transform_puts_it() {
        let center = Point { x: 32.0, y: 24.0 };
        let still = ShakenCameraFrame {
            width: 64,
            height: 48,
            background_seed: 1,
            camera: SimilarityTransform::identity_about(center),
            moving_object: None,
            noise_amplitude: 0.0,
            noise_seed: 0,
        };
        let shaken = ShakenCameraFrame {
            camera: SimilarityTransform {
                shift_x_in_pixels: 3.0,
                shift_y_in_pixels: -2.0,
                ..still.camera
            },
            ..still.clone()
        };
        let (still, shaken_image) = (still.render(), shaken.render());
        // The content of pixel (10, 20) moved 3 pixels right and 2 up.
        assert!((still.value(10, 20) - shaken_image.value(13, 18)).abs() < 1e-3);
        let noisy = ShakenCameraFrame {
            noise_amplitude: 4.0,
            noise_seed: 9,
            ..shaken
        }
        .render();
        let differences: Vec<f32> = (0..64)
            .map(|column| noisy.value(column, 5) - shaken_image.value(column, 5))
            .collect();
        assert!(differences.iter().all(|difference| difference.abs() <= 4.0));
        assert!(differences.iter().any(|difference| difference.abs() > 1.0));
    }
}
