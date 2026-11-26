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

#[cfg(test)]
mod tests {
    use super::*;

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
