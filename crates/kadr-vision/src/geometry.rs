//! Positions and regions in an image, in pixels.
//!
//! Coordinates are measured from the image's top-left corner. Pixel
//! `(column, row)` covers the square from `(column, row)` to
//! `(column + 1, row + 1)`, so its centre is at `(column + 0.5, row + 0.5)`. With
//! this convention, an image shrunk by a factor `k` has every coordinate divided
//! by `k`, exactly.
//!
//! A region is stored as its centre and size, which is what the tracker moves and
//! scales.
//!
//! A [`SimilarityTransform`] moves a whole picture: a rotation and a change of size
//! about a centre, then a shift. Rotations are clockwise for positive angles, as
//! the y axis points down: `R(θ)` maps `(x, y)` to
//! `(x cos θ − y sin θ, x sin θ + y cos θ)`.

/// A point in an image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    /// Horizontal position, right is positive.
    pub x: f32,
    /// Vertical position, down is positive.
    pub y: f32,
}

/// A rectangular region of an image, axis-aligned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region {
    /// Horizontal position of the centre.
    pub center_x: f32,
    /// Vertical position of the centre.
    pub center_y: f32,
    /// Width in pixels.
    pub width: f32,
    /// Height in pixels.
    pub height: f32,
}

impl Region {
    /// The region whose top-left corner is at (`left`, `top`).
    pub fn from_top_left(left: f32, top: f32, width: f32, height: f32) -> Self {
        Self {
            center_x: left + width / 2.0,
            center_y: top + height / 2.0,
            width,
            height,
        }
    }

    /// Distance from the left edge of the image to the region's left edge.
    pub fn left(&self) -> f32 {
        self.center_x - self.width / 2.0
    }

    /// Distance from the top edge of the image to the region's top edge.
    pub fn top(&self) -> f32 {
        self.center_y - self.height / 2.0
    }

    /// The centre as a point.
    pub fn center(&self) -> Point {
        Point {
            x: self.center_x,
            y: self.center_y,
        }
    }

    /// The same region moved by (`delta_x`, `delta_y`).
    pub fn moved_by(&self, delta_x: f32, delta_y: f32) -> Self {
        Self {
            center_x: self.center_x + delta_x,
            center_y: self.center_y + delta_y,
            ..*self
        }
    }

    /// The same region, `factor` times larger, around the same centre.
    pub fn scaled_by(&self, factor: f32) -> Self {
        Self {
            width: self.width * factor,
            height: self.height * factor,
            ..*self
        }
    }

    /// Distance between the centres of two regions.
    pub fn center_distance_to(&self, other: &Region) -> f32 {
        (self.center_x - other.center_x).hypot(self.center_y - other.center_y)
    }

    /// The longer side.
    pub fn longer_side(&self) -> f32 {
        self.width.max(self.height)
    }

    /// The same region in an image shrunk by `shrink_factor`
    /// (see [`GreyImage::from_luma_shrunk`](crate::GreyImage::from_luma_shrunk)).
    pub fn in_shrunk_image(&self, shrink_factor: usize) -> Self {
        self.with_every_coordinate_multiplied_by(1.0 / shrink_factor.max(1) as f32)
    }

    /// The same region in the full-size image, for a region found in an image
    /// shrunk by `shrink_factor`.
    pub fn in_full_size_image(&self, shrink_factor: usize) -> Self {
        self.with_every_coordinate_multiplied_by(shrink_factor.max(1) as f32)
    }

    fn with_every_coordinate_multiplied_by(&self, factor: f32) -> Self {
        Self {
            center_x: self.center_x * factor,
            center_y: self.center_y * factor,
            width: self.width * factor,
            height: self.height * factor,
        }
    }
}

/// A rotation and a change of size about a centre, followed by a shift: point `p`
/// goes to `center + scale_factor · R(rotation) · (p − center) + shift`, with `R`
/// turning clockwise (the y axis points down).
///
/// The same motion can be written about any centre (see
/// [`SimilarityTransform::about`]): only the shift changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimilarityTransform {
    /// The point the rotation and the change of size are about.
    pub center: Point,
    /// Shift to the right (positive), in pixels.
    pub shift_x_in_pixels: f32,
    /// Shift down (positive), in pixels.
    pub shift_y_in_pixels: f32,
    /// Clockwise rotation, in degrees.
    pub rotation_in_degrees: f32,
    /// Change of size (1 = same size, 1.02 = 2 % larger).
    pub scale_factor: f32,
}

impl SimilarityTransform {
    /// The transform that moves nothing, written about `center`.
    pub fn identity_about(center: Point) -> Self {
        Self {
            center,
            shift_x_in_pixels: 0.0,
            shift_y_in_pixels: 0.0,
            rotation_in_degrees: 0.0,
            scale_factor: 1.0,
        }
    }

    /// Where `point` goes.
    pub fn apply_to(&self, point: Point) -> Point {
        let (x, y) = self.turned_and_scaled(point.x - self.center.x, point.y - self.center.y);
        Point {
            x: self.center.x + x + self.shift_x_in_pixels,
            y: self.center.y + y + self.shift_y_in_pixels,
        }
    }

    /// The transform that brings every point back where it was, about the same
    /// centre.
    pub fn inverse(&self) -> Self {
        let undone = Self {
            rotation_in_degrees: -self.rotation_in_degrees,
            scale_factor: 1.0 / self.scale_factor,
            ..*self
        };
        let (x, y) = undone.turned_and_scaled(self.shift_x_in_pixels, self.shift_y_in_pixels);
        Self {
            shift_x_in_pixels: -x,
            shift_y_in_pixels: -y,
            ..undone
        }
    }

    /// This transform, then `next`: the result moves `p` to
    /// `next.apply_to(self.apply_to(p))`, about this transform's centre.
    pub fn followed_by(&self, next: &SimilarityTransform) -> Self {
        let next = next.about(self.center);
        let (x, y) = next.turned_and_scaled(self.shift_x_in_pixels, self.shift_y_in_pixels);
        Self {
            center: self.center,
            shift_x_in_pixels: x + next.shift_x_in_pixels,
            shift_y_in_pixels: y + next.shift_y_in_pixels,
            rotation_in_degrees: self.rotation_in_degrees + next.rotation_in_degrees,
            scale_factor: self.scale_factor * next.scale_factor,
        }
    }

    /// The same motion written about `center`: the rotation and the change of size
    /// are unchanged, the shift becomes `shift + (I − scale · R)(self.center − center)`.
    pub fn about(&self, center: Point) -> Self {
        let (from_x, from_y) = (self.center.x - center.x, self.center.y - center.y);
        let (x, y) = self.turned_and_scaled(from_x, from_y);
        Self {
            center,
            shift_x_in_pixels: self.shift_x_in_pixels + from_x - x,
            shift_y_in_pixels: self.shift_y_in_pixels + from_y - y,
            ..*self
        }
    }

    /// The same motion in the full-size image, for a transform measured in an image
    /// shrunk by `shrink_factor` (see
    /// [`GreyImage::from_luma_shrunk`](crate::GreyImage::from_luma_shrunk)): the
    /// centre and the shift are multiplied by `shrink_factor`.
    pub fn in_full_size_image(&self, shrink_factor: usize) -> Self {
        let factor = shrink_factor.max(1) as f32;
        Self {
            center: Point {
                x: self.center.x * factor,
                y: self.center.y * factor,
            },
            shift_x_in_pixels: self.shift_x_in_pixels * factor,
            shift_y_in_pixels: self.shift_y_in_pixels * factor,
            ..*self
        }
    }

    /// `scale_factor · R(rotation) · (x, y)`.
    fn turned_and_scaled(&self, x: f32, y: f32) -> (f32, f32) {
        let (sine, cosine) = self.rotation_in_degrees.to_radians().sin_cos();
        (
            self.scale_factor * (x * cosine - y * sine),
            self.scale_factor * (x * sine + y * cosine),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_points_close(first: Point, second: Point) {
        assert!(
            (first.x - second.x).abs() < 1e-3 && (first.y - second.y).abs() < 1e-3,
            "{first:?} and {second:?}"
        );
    }

    const SHAKE: SimilarityTransform = SimilarityTransform {
        center: Point { x: 320.0, y: 180.0 },
        shift_x_in_pixels: 6.5,
        shift_y_in_pixels: -3.0,
        rotation_in_degrees: 2.0,
        scale_factor: 1.03,
    };

    const SOME_POINTS: [Point; 3] = [
        Point { x: 0.0, y: 0.0 },
        Point { x: 500.0, y: 90.0 },
        Point { x: 210.0, y: 340.0 },
    ];

    #[test]
    fn a_positive_rotation_turns_clockwise() {
        let quarter_turn = SimilarityTransform {
            rotation_in_degrees: 90.0,
            ..SimilarityTransform::identity_about(Point { x: 0.0, y: 0.0 })
        };
        // Right of the centre goes below it (y points down).
        assert_points_close(
            quarter_turn.apply_to(Point { x: 10.0, y: 0.0 }),
            Point { x: 0.0, y: 10.0 },
        );
        assert_eq!(SHAKE.apply_to(SHAKE.center), Point { x: 326.5, y: 177.0 });
    }

    #[test]
    fn the_inverse_and_compositions_agree_with_applying_the_transforms() {
        let other = SimilarityTransform {
            center: Point { x: 10.0, y: 20.0 },
            shift_x_in_pixels: -4.0,
            shift_y_in_pixels: 8.0,
            rotation_in_degrees: -5.0,
            scale_factor: 0.97,
        };
        let both = SHAKE.followed_by(&other);
        assert_eq!(both.center, SHAKE.center);
        for point in SOME_POINTS {
            assert_points_close(SHAKE.inverse().apply_to(SHAKE.apply_to(point)), point);
            assert_points_close(both.apply_to(point), other.apply_to(SHAKE.apply_to(point)));
            let elsewhere = SHAKE.about(other.center);
            assert_eq!(elsewhere.center, other.center);
            assert_points_close(elsewhere.apply_to(point), SHAKE.apply_to(point));
        }
    }

    #[test]
    fn a_transform_of_a_shrunk_image_applies_to_the_full_size_image() {
        let shrink_factor = 3;
        let full_size = SHAKE.in_full_size_image(shrink_factor);
        for point in SOME_POINTS {
            let shrunk = Point {
                x: point.x / 3.0,
                y: point.y / 3.0,
            };
            let moved = SHAKE.apply_to(shrunk);
            assert_points_close(
                full_size.apply_to(point),
                Point {
                    x: moved.x * 3.0,
                    y: moved.y * 3.0,
                },
            );
        }
    }

    #[test]
    fn top_left_and_centre_agree() {
        let region = Region::from_top_left(10.0, 20.0, 40.0, 60.0);
        assert_eq!((region.center_x, region.center_y), (30.0, 50.0));
        assert_eq!((region.left(), region.top()), (10.0, 20.0));
    }

    #[test]
    fn moving_and_scaling_keep_what_they_should() {
        let region = Region::from_top_left(0.0, 0.0, 10.0, 20.0);
        let moved = region.moved_by(3.0, -4.0);
        assert_eq!(moved.center_distance_to(&region), 5.0);
        let scaled = region.scaled_by(2.0);
        assert_eq!(
            (scaled.width, scaled.height, scaled.center_x),
            (20.0, 40.0, 5.0)
        );
        assert_eq!(scaled.longer_side(), 40.0);
    }

    #[test]
    fn shrinking_and_enlarging_round_trip() {
        let region = Region::from_top_left(300.0, 120.0, 90.0, 60.0);
        let shrunk = region.in_shrunk_image(3);
        assert_eq!(
            (shrunk.left(), shrunk.top(), shrunk.width),
            (100.0, 40.0, 30.0)
        );
        assert_eq!(shrunk.in_full_size_image(3), region);
    }
}
