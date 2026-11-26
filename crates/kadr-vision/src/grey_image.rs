//! Grey-level images and image pyramids.
//!
//! The tracker only needs brightness: it reads a video's Y (luma) plane, usually
//! shrunk to about 640 pixels wide, which is plenty to follow a face and makes
//! the analysis several times faster. Values are `f32` from 0 to 255.
//!
//! Coordinates follow the convention of [`crate::geometry`]: pixel
//! `(column, row)` covers the square from `(column, row)` to
//! `(column + 1, row + 1)`.
//!
//! An [`ImagePyramid`] holds the image at full, half, quarter… size. A large
//! window is read from a smaller level, so that resampling it into the tracker's
//! small working window does not alias, and the optical flow follows large
//! motions from coarse to fine.

/// Weights of the four pixels averaged along each side when halving an image.
const HALVING_WEIGHTS: [f32; 4] = [1.0 / 8.0, 3.0 / 8.0, 3.0 / 8.0, 1.0 / 8.0];

/// A grey-level image.
#[derive(Debug, Clone, PartialEq)]
pub struct GreyImage {
    width: usize,
    height: usize,
    values: Vec<f32>,
}

impl GreyImage {
    /// Builds an image from 8-bit brightness values, row after row, `row_stride`
    /// bytes apart (`row_stride >= width`).
    ///
    /// Returns `None` if `luma` is too short or a side is zero.
    pub fn from_luma(width: usize, height: usize, row_stride: usize, luma: &[u8]) -> Option<Self> {
        Self::from_luma_shrunk(width, height, row_stride, luma, 1)
    }

    /// Builds an image `shrink_factor` times smaller than the 8-bit brightness
    /// values given, each pixel the average of a `shrink_factor × shrink_factor`
    /// block. Columns and rows left over at the right and bottom edges are
    /// ignored, so every coordinate is divided by exactly `shrink_factor`.
    ///
    /// Returns `None` if `luma` is too short, `shrink_factor` is zero, or the
    /// result would be empty.
    pub fn from_luma_shrunk(
        width: usize,
        height: usize,
        row_stride: usize,
        luma: &[u8],
        shrink_factor: usize,
    ) -> Option<Self> {
        if shrink_factor == 0
            || width < shrink_factor
            || height < shrink_factor
            || row_stride < width
        {
            return None;
        }
        if luma.len() < row_stride * (height - 1) + width {
            return None;
        }
        let (shrunk_width, shrunk_height) = (width / shrink_factor, height / shrink_factor);
        let normalization = 1.0 / (shrink_factor * shrink_factor) as f32;
        let mut values = Vec::with_capacity(shrunk_width * shrunk_height);
        let mut block_sums = vec![0_u32; shrunk_width];
        for shrunk_row in 0..shrunk_height {
            block_sums.fill(0);
            for row in shrunk_row * shrink_factor..(shrunk_row + 1) * shrink_factor {
                let line = &luma[row * row_stride..row * row_stride + shrunk_width * shrink_factor];
                for (sum, block) in block_sums.iter_mut().zip(line.chunks_exact(shrink_factor)) {
                    *sum += block.iter().map(|&value| u32::from(value)).sum::<u32>();
                }
            }
            values.extend(block_sums.iter().map(|&sum| sum as f32 * normalization));
        }
        Some(Self {
            width: shrunk_width,
            height: shrunk_height,
            values,
        })
    }

    /// Builds an image from a function giving the value of pixel `(column, row)`.
    pub fn from_fn(width: usize, height: usize, value_at: impl Fn(usize, usize) -> f32) -> Self {
        let values = (0..height)
            .flat_map(|row| (0..width).map(move |column| (column, row)))
            .map(|(column, row)| value_at(column, row))
            .collect();
        Self {
            width,
            height,
            values,
        }
    }

    /// Width in pixels.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> usize {
        self.height
    }

    /// The value of one pixel.
    pub fn value(&self, column: usize, row: usize) -> f32 {
        self.values[row * self.width + column]
    }

    /// The value at any point, interpolated between the four nearest pixel
    /// centres. Points outside the image take the value of the nearest edge pixel.
    pub fn sample(&self, x: f32, y: f32) -> f32 {
        let column = (x - 0.5).clamp(0.0, (self.width - 1) as f32);
        let row = (y - 0.5).clamp(0.0, (self.height - 1) as f32);
        let (left, top) = (column.floor(), row.floor());
        let (weight_right, weight_bottom) = (column - left, row - top);
        let (left, top) = (left as usize, top as usize);
        let right = (left + 1).min(self.width - 1);
        let bottom = (top + 1).min(self.height - 1);
        let top_mix =
            self.value(left, top) * (1.0 - weight_right) + self.value(right, top) * weight_right;
        let bottom_mix = self.value(left, bottom) * (1.0 - weight_right)
            + self.value(right, bottom) * weight_right;
        top_mix * (1.0 - weight_bottom) + bottom_mix * weight_bottom
    }

    /// The horizontal and vertical brightness slopes at a point, per pixel
    /// (central differences).
    pub fn gradient(&self, x: f32, y: f32) -> (f32, f32) {
        (
            (self.sample(x + 1.0, y) - self.sample(x - 1.0, y)) / 2.0,
            (self.sample(x, y + 1.0) - self.sample(x, y - 1.0)) / 2.0,
        )
    }

    /// The image at half size. Each pixel is a weighted average of the 4 × 4
    /// pixels around its 2 × 2 block (weights 1, 3, 3, 1 along each side): a
    /// gentle blur that removes the details too fine for the smaller image, which
    /// would otherwise turn into false patterns (aliasing). An odd last column or
    /// row is dropped, so coordinates are halved exactly.
    pub fn half_size(&self) -> GreyImage {
        let (half_width, half_height) = ((self.width / 2).max(1), (self.height / 2).max(1));
        let clamped_column = |column: isize| column.clamp(0, self.width as isize - 1) as usize;
        let clamped_row = |row: isize| row.clamp(0, self.height as isize - 1) as usize;
        // Horizontal pass on every row, then vertical pass on the result.
        let horizontally = GreyImage::from_fn(half_width, self.height, |column, row| {
            let left = 2 * column as isize;
            HALVING_WEIGHTS
                .iter()
                .zip(-1..)
                .map(|(weight, offset)| weight * self.value(clamped_column(left + offset), row))
                .sum()
        });
        GreyImage::from_fn(half_width, half_height, |column, row| {
            let top = 2 * row as isize;
            HALVING_WEIGHTS
                .iter()
                .zip(-1..)
                .map(|(weight, offset)| {
                    weight * horizontally.value(column, clamped_row(top + offset))
                })
                .sum()
        })
    }
}

/// An image at full, half, quarter… size.
#[derive(Debug, Clone)]
pub(crate) struct ImagePyramid {
    levels: Vec<GreyImage>,
}

impl ImagePyramid {
    /// Halves the image until its shorter side would drop below `smallest_side`.
    pub(crate) fn new(image: GreyImage, smallest_side: usize) -> Self {
        let mut levels = vec![image];
        while let Some(last) = levels.last() {
            if last.width().min(last.height()) / 2 < smallest_side {
                break;
            }
            let smaller = last.half_size();
            levels.push(smaller);
        }
        Self { levels }
    }

    /// Number of levels (1 = only the full-size image).
    pub(crate) fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// The image at `level` (0 = full size), clamped to the smallest level.
    pub(crate) fn level(&self, level: usize) -> &GreyImage {
        &self.levels[self.clamped(level)]
    }

    /// How many full-size pixels one pixel of `level` spans (1, 2, 4…).
    pub(crate) fn scale_of_level(&self, level: usize) -> f32 {
        (1_u32 << self.clamped(level)) as f32
    }

    /// The smallest level that still has at least one pixel per `step_in_pixels`
    /// full-size pixels: sampling it every `step_in_pixels` skips no detail it holds.
    pub(crate) fn level_for_step(&self, step_in_pixels: f32) -> usize {
        if step_in_pixels < 2.0 {
            return 0;
        }
        self.clamped(step_in_pixels.log2().floor() as usize)
    }

    fn clamped(&self, level: usize) -> usize {
        level.min(self.levels.len() - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luma_rows_are_read_without_their_padding() {
        let luma = [1, 2, 99, 3, 4, 99];
        let image = GreyImage::from_luma(2, 2, 3, &luma).unwrap();
        assert_eq!(image.value(1, 1), 4.0);
        assert!(GreyImage::from_luma(4, 2, 3, &luma).is_none());
    }

    #[test]
    fn shrinking_averages_blocks_and_drops_leftovers() {
        // 5 × 2 pixels, shrunk by 2: one row of two blocks, the fifth column dropped.
        let luma = [0, 2, 10, 10, 200, 4, 6, 20, 20, 200];
        let image = GreyImage::from_luma_shrunk(5, 2, 5, &luma, 2).unwrap();
        assert_eq!((image.width(), image.height()), (2, 1));
        assert_eq!((image.value(0, 0), image.value(1, 0)), (3.0, 15.0));
        assert!(GreyImage::from_luma_shrunk(5, 2, 5, &luma, 0).is_none());
        assert!(GreyImage::from_luma_shrunk(5, 2, 5, &luma, 3).is_none());
    }

    #[test]
    fn sampling_interpolates_between_pixel_centres_and_clamps() {
        let image = GreyImage::from_fn(2, 1, |column, _| if column == 0 { 0.0 } else { 100.0 });
        assert_eq!(image.sample(0.5, 0.5), 0.0, "centre of the first pixel");
        assert_eq!(image.sample(0.75, 0.5), 25.0);
        assert_eq!(image.sample(-5.0, 3.0), 0.0);
        assert_eq!(image.gradient(1.0, 0.5).0, 50.0);
    }

    #[test]
    fn a_pyramid_halves_until_the_smallest_side() {
        let image = GreyImage::from_fn(320, 240, |column, row| (column + row) as f32);
        let pyramid = ImagePyramid::new(image, 32);
        // 240 → 120 → 60 → stop (30 < 32).
        assert_eq!(pyramid.level_count(), 3);
        assert_eq!(pyramid.level(2).width(), 80);
        assert_eq!(
            pyramid.level(9).width(),
            80,
            "clamped to the smallest level"
        );
        assert_eq!(pyramid.scale_of_level(9), 4.0);
        assert_eq!(pyramid.level_for_step(1.5), 0);
        assert_eq!(pyramid.level_for_step(3.0), 1);
        assert_eq!(pyramid.level_for_step(100.0), 2);
    }

    #[test]
    fn halving_keeps_flat_images_flat_and_removes_fine_stripes() {
        let flat = GreyImage::from_fn(9, 7, |_, _| 42.0).half_size();
        assert_eq!((flat.width(), flat.height()), (4, 3));
        assert!((0..4).all(|column| (flat.value(column, 1) - 42.0).abs() < 1e-4));
        // Stripes one pixel wide cannot be shown at half size: they become grey.
        let stripes = GreyImage::from_fn(
            16,
            16,
            |column, _| if column % 2 == 0 { 0.0 } else { 200.0 },
        );
        let halved = stripes.half_size();
        assert!((halved.value(3, 3) - 100.0).abs() < 1e-3);
    }

    #[test]
    fn every_level_shows_the_same_point_at_the_same_coordinates() {
        // A brightness ramp: every level must agree on the value at a given point
        // (away from the borders, where the halving blur sees clamped pixels).
        let image = GreyImage::from_fn(128, 128, |column, row| (column * 3 + row) as f32);
        let pyramid = ImagePyramid::new(image, 8);
        for (x, y) in [(40.0, 50.0), (63.5, 47.25), (80.0, 70.0)] {
            let full_size = pyramid.level(0).sample(x, y);
            for level in 1..pyramid.level_count() {
                let scale = pyramid.scale_of_level(level);
                let at_level = pyramid.level(level).sample(x / scale, y / scale);
                assert!(
                    (at_level - full_size).abs() < 1e-3,
                    "level {level} at ({x}, {y})"
                );
            }
        }
    }
}
