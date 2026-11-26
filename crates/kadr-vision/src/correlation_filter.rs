//! MOSSE, the correlation filter that follows the region from one frame to the next.
//!
//! Reference: D. S. Bolme, J. R. Beveridge, B. A. Draper and Y. M. Lui, "Visual
//! Object Tracking using Adaptive Correlation Filters", CVPR 2010.
//!
//! # How it works
//!
//! 1. **Window.** A square window centred on the region, [`CONTEXT_FACTOR`] times
//!    its longer side, is resampled to [`WINDOW_SIDE_IN_CELLS`]² cells. The extra
//!    context lets the filter find the region after it moved, and the fixed cell
//!    count makes a small and a large region cost the same.
//! 2. **Preprocessing.** Each cell takes the logarithm of its brightness (which
//!    tames lighting changes), the window is normalised to a mean of 0 and a
//!    standard deviation of 1, then multiplied by a cosine window that fades its
//!    edges to zero: the Fourier transform treats the window as periodic, and hard
//!    edges would create false matches.
//! 3. **Training.** The filter `H` is the image whose correlation with the
//!    training windows is as close as possible to `G`, a sharp gaussian peak at the
//!    centre. In the frequency domain the answer has a closed form, computed
//!    frequency by frequency: `H* = Σ G·F* / (Σ F·F* + λ)`, where the `F` are the
//!    training windows' transforms and `λ` keeps rare frequencies from being
//!    amplified. The first frame gives five training windows, slightly rotated,
//!    so that the filter tolerates a tilting head.
//! 4. **Detection.** On a new frame, the window at the previous position is
//!    transformed and multiplied by `H*`; back in the image domain, the response
//!    peaks where the region now is. The peak's offset from the centre is the motion.
//! 5. **Confidence.** The peak-to-sidelobe ratio (PSR) compares the peak with the
//!    rest of the response: `(peak − mean) / standard deviation`, computed outside
//!    an 11 × 11 square around the peak. A clear, unique peak gives a high PSR; an
//!    occlusion flattens the response and the PSR drops.
//! 6. **Update.** When the match is sure, the filter's numerator and denominator
//!    are blended with the new window's ([`LEARNING_RATE`]), so the filter follows
//!    slow changes of appearance. When unsure it is left alone, so that it does not
//!    learn the obstacle.

use rustfft::num_complex::Complex;

use crate::fourier::Fourier2d;
use crate::geometry::Point;
use crate::grey_image::ImagePyramid;

/// Side of the working window, in cells. A power of two keeps the transforms fast.
pub(crate) const WINDOW_SIDE_IN_CELLS: usize = 64;

/// The window's side, relative to the region's longer side.
pub(crate) const CONTEXT_FACTOR: f32 = 2.0;

/// Width of the gaussian peak the filter is trained to produce.
const TARGET_PEAK_SIGMA_IN_CELLS: f32 = 2.0;

/// How fast the filter learns a new appearance: the weight of the newest frame
/// (0 = never learns, 1 = only remembers the last frame). Bolme et al. use 0.125.
const LEARNING_RATE: f32 = 0.125;

/// `λ`, relative to the average energy per frequency of the training windows.
const REGULARIZATION_RELATIVE_TO_MEAN_ENERGY: f32 = 0.01;

/// Half-side of the square around the peak left out of the sidelobe (11 × 11 cells).
const PEAK_EXCLUSION_HALF_SIDE_IN_CELLS: usize = 5;

/// Weight of the background, far from the region, in the filter's window (the
/// region itself weighs 1). The window is twice as large as the region, so the
/// background would otherwise dominate it: a still, detailed background then
/// holds the filter back while the region moves, and it drifts.
pub(crate) const BACKGROUND_WEIGHT: f32 = 0.0;

/// How fast the weight falls from 1 at the region's edge to the background's,
/// in half-sides of the region (a gaussian tail).
pub(crate) const TARGET_MASK_TAIL_IN_HALF_SIDES: f32 = 0.5;

/// Below this standard deviation of the log brightness, a window is uniform:
/// there is nothing to follow in it.
const MINIMUM_WINDOW_STANDARD_DEVIATION: f32 = 1e-3;

/// The training windows of the first frame, by rotation in radians. Only
/// rotations: training on scaled copies too would make the filter answer equally
/// at every scale, and the tracker could no longer tell when the region grows.
const TRAINING_ROTATIONS_IN_RADIANS: [f32; 5] = [0.0, 0.05, -0.05, 0.1, -0.1];

/// A square part of the image, in image pixels, around a region.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SquareWindow {
    /// Centre of the window.
    pub(crate) center: Point,
    /// Side of the window.
    pub(crate) side_in_pixels: f32,
    /// Width of the region at the centre of the window.
    pub(crate) region_width_in_pixels: f32,
    /// Height of the region at the centre of the window.
    pub(crate) region_height_in_pixels: f32,
}

/// Where the filter's response peaks, and how clearly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CorrelationPeak {
    /// How far the region is from the window's centre, horizontally, in image pixels.
    pub(crate) displacement_x_in_pixels: f32,
    /// How far the region is from the window's centre, vertically, in image pixels.
    pub(crate) displacement_y_in_pixels: f32,
    /// Height of the peak: about 1 for a perfect match.
    pub(crate) peak_value: f32,
    /// Peak-to-sidelobe ratio: how much the peak stands out.
    pub(crate) peak_to_sidelobe_ratio: f32,
}

/// A trained MOSSE filter.
pub(crate) struct CorrelationFilter {
    fourier: Fourier2d,
    cosine_window: Vec<f32>,
    target_spectrum: Vec<Complex<f32>>,
    numerator: Vec<Complex<f32>>,
    denominator: Vec<f32>,
    regularization: f32,
}

impl std::fmt::Debug for CorrelationFilter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CorrelationFilter")
            .field("fourier", &self.fourier)
            .field("regularization", &self.regularization)
            .finish_non_exhaustive()
    }
}

impl CorrelationFilter {
    /// Trains a filter on the window of the first frame.
    ///
    /// Returns `None` if the window is uniform.
    pub(crate) fn train(pyramid: &ImagePyramid, window: SquareWindow) -> Option<Self> {
        let cell_count = WINDOW_SIDE_IN_CELLS * WINDOW_SIDE_IN_CELLS;
        let mut fourier = Fourier2d::new(WINDOW_SIDE_IN_CELLS);
        let cosine_window = cosine_window(WINDOW_SIDE_IN_CELLS);
        let target_spectrum = fourier.forward_real(&gaussian_peak(WINDOW_SIDE_IN_CELLS));
        let mut numerator = vec![Complex::new(0.0, 0.0); cell_count];
        let mut denominator = vec![0.0; cell_count];
        let weight = 1.0 / TRAINING_ROTATIONS_IN_RADIANS.len() as f32;
        for rotation_in_radians in TRAINING_ROTATIONS_IN_RADIANS {
            let values = prepare_window(pyramid, window, rotation_in_radians, &cosine_window)?;
            let spectrum = fourier.forward_real(&values);
            blend_into(
                &mut numerator,
                &mut denominator,
                &target_spectrum,
                &spectrum,
                1.0,
                weight,
            );
        }
        let mean_energy = denominator.iter().sum::<f32>() / cell_count as f32;
        Some(Self {
            fourier,
            cosine_window,
            target_spectrum,
            numerator,
            denominator,
            regularization: REGULARIZATION_RELATIVE_TO_MEAN_ENERGY * mean_energy,
        })
    }

    /// Correlates the filter with `window` and finds the peak of the response.
    ///
    /// Returns `None` if the window is uniform.
    pub(crate) fn find_peak(
        &mut self,
        pyramid: &ImagePyramid,
        window: SquareWindow,
    ) -> Option<CorrelationPeak> {
        let values = prepare_window(pyramid, window, 0.0, &self.cosine_window)?;
        let spectrum = self.fourier.forward_real(&values);
        let regularization = self.regularization;
        let filtered: Vec<Complex<f32>> = spectrum
            .iter()
            .zip(&self.numerator)
            .zip(&self.denominator)
            .map(|((frequency, numerator), denominator)| {
                frequency * numerator / (denominator + regularization)
            })
            .collect();
        let response = self.fourier.inverse_real(filtered);
        let cell_size_in_pixels = window.side_in_pixels / WINDOW_SIDE_IN_CELLS as f32;
        Some(locate_peak(&response, cell_size_in_pixels))
    }

    /// Learns the region's appearance in `window`, centred on the region.
    pub(crate) fn learn(&mut self, pyramid: &ImagePyramid, window: SquareWindow) {
        let Some(values) = prepare_window(pyramid, window, 0.0, &self.cosine_window) else {
            return;
        };
        let spectrum = self.fourier.forward_real(&values);
        blend_into(
            &mut self.numerator,
            &mut self.denominator,
            &self.target_spectrum,
            &spectrum,
            1.0 - LEARNING_RATE,
            LEARNING_RATE,
        );
    }
}

/// `numerator = kept · numerator + added · G·F*` and
/// `denominator = kept · denominator + added · F·F*`.
fn blend_into(
    numerator: &mut [Complex<f32>],
    denominator: &mut [f32],
    target_spectrum: &[Complex<f32>],
    spectrum: &[Complex<f32>],
    kept: f32,
    added: f32,
) {
    for (((numerator, denominator), target), frequency) in numerator
        .iter_mut()
        .zip(denominator.iter_mut())
        .zip(target_spectrum)
        .zip(spectrum)
    {
        *numerator = *numerator * kept + target * frequency.conj() * added;
        *denominator = *denominator * kept + frequency.norm_sqr() * added;
    }
}

/// Resamples the window to cells (optionally rotated around its centre), then
/// preprocesses it: log brightness, mean 0, standard deviation 1, cosine window.
fn prepare_window(
    pyramid: &ImagePyramid,
    window: SquareWindow,
    rotation_in_radians: f32,
    cosine_window: &[f32],
) -> Option<Vec<f32>> {
    let cell_size_in_pixels = window.side_in_pixels / WINDOW_SIDE_IN_CELLS as f32;
    let level = pyramid.level_for_step(cell_size_in_pixels);
    let (image, level_scale) = (pyramid.level(level), pyramid.scale_of_level(level));
    let (sine, cosine) = rotation_in_radians.sin_cos();
    let half_side = WINDOW_SIDE_IN_CELLS as f32 / 2.0;
    let mut values = Vec::with_capacity(WINDOW_SIDE_IN_CELLS * WINDOW_SIDE_IN_CELLS);
    for row in 0..WINDOW_SIDE_IN_CELLS {
        let offset_y = (row as f32 - half_side) * cell_size_in_pixels;
        for column in 0..WINDOW_SIDE_IN_CELLS {
            let offset_x = (column as f32 - half_side) * cell_size_in_pixels;
            let x = window.center.x + offset_x * cosine - offset_y * sine;
            let y = window.center.y + offset_x * sine + offset_y * cosine;
            values.push(image.sample(x / level_scale, y / level_scale).ln_1p());
        }
    }
    let cell_count = values.len() as f32;
    let mean = values.iter().sum::<f32>() / cell_count;
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f32>()
        / cell_count;
    let standard_deviation = variance.sqrt();
    if standard_deviation < MINIMUM_WINDOW_STANDARD_DEVIATION {
        return None;
    }
    let region_mask = region_mask(&window, cell_size_in_pixels);
    for ((value, weight), region_weight) in values.iter_mut().zip(cosine_window).zip(region_mask) {
        *value = (*value - mean) / standard_deviation * weight * region_weight;
    }
    Some(values)
}

/// For every cell, how much it belongs to the region: 1 inside, falling to
/// [`BACKGROUND_WEIGHT`] outside.
fn region_mask(window: &SquareWindow, cell_size_in_pixels: f32) -> impl Iterator<Item = f32> {
    let half_side = WINDOW_SIDE_IN_CELLS as f32 / 2.0;
    let half_width_in_cells = (window.region_width_in_pixels / 2.0 / cell_size_in_pixels).max(1.0);
    let half_height_in_cells =
        (window.region_height_in_pixels / 2.0 / cell_size_in_pixels).max(1.0);
    let along = |offset: f32, half_extent: f32| {
        let beyond = (offset.abs() / half_extent - 1.0).max(0.0) / TARGET_MASK_TAIL_IN_HALF_SIDES;
        (-beyond * beyond).exp()
    };
    let columns: Vec<f32> = (0..WINDOW_SIDE_IN_CELLS)
        .map(|column| along(column as f32 - half_side, half_width_in_cells))
        .collect();
    let rows: Vec<f32> = (0..WINDOW_SIDE_IN_CELLS)
        .map(|row| along(row as f32 - half_side, half_height_in_cells))
        .collect();
    rows.into_iter().flat_map(move |row_weight| {
        columns.clone().into_iter().map(move |column_weight| {
            BACKGROUND_WEIGHT + (1.0 - BACKGROUND_WEIGHT) * row_weight * column_weight
        })
    })
}

/// The 2D cosine (Hann) window: 1 at the centre cell, fading to 0 at the edges.
fn cosine_window(side: usize) -> Vec<f32> {
    let along_one_side: Vec<f32> = (0..side)
        .map(|index| 0.5 - 0.5 * (std::f32::consts::TAU * index as f32 / side as f32).cos())
        .collect();
    along_one_side
        .iter()
        .flat_map(|row_weight| {
            along_one_side
                .iter()
                .map(move |column_weight| row_weight * column_weight)
        })
        .collect()
}

/// The response the filter is trained to give: a gaussian peak at the centre cell.
fn gaussian_peak(side: usize) -> Vec<f32> {
    let center = (side / 2) as f32;
    let two_sigma_squared = 2.0 * TARGET_PEAK_SIGMA_IN_CELLS * TARGET_PEAK_SIGMA_IN_CELLS;
    (0..side * side)
        .map(|index| {
            let (row, column) = ((index / side) as f32, (index % side) as f32);
            let squared_distance = (column - center).powi(2) + (row - center).powi(2);
            (-squared_distance / two_sigma_squared).exp()
        })
        .collect()
}

/// Finds the highest cell of the response, refines it to a fraction of a cell,
/// and measures how much it stands out.
fn locate_peak(response: &[f32], cell_size_in_pixels: f32) -> CorrelationPeak {
    let side = WINDOW_SIDE_IN_CELLS;
    let (peak_index, peak_value) = response
        .iter()
        .copied()
        .enumerate()
        .max_by(|first, second| first.1.total_cmp(&second.1))
        .unwrap_or((0, 0.0));
    let (peak_row, peak_column) = (peak_index / side, peak_index % side);
    // The response is periodic: neighbours wrap around the edges.
    let value_at = |row: usize, column: usize| response[(row % side) * side + column % side];
    let horizontal_offset = parabolic_peak_offset(
        value_at(peak_row, peak_column + side - 1),
        peak_value,
        value_at(peak_row, peak_column + 1),
    );
    let vertical_offset = parabolic_peak_offset(
        value_at(peak_row + side - 1, peak_column),
        peak_value,
        value_at(peak_row + 1, peak_column),
    );
    let center = (side / 2) as f32;
    CorrelationPeak {
        displacement_x_in_pixels: (peak_column as f32 + horizontal_offset - center)
            * cell_size_in_pixels,
        displacement_y_in_pixels: (peak_row as f32 + vertical_offset - center)
            * cell_size_in_pixels,
        peak_value,
        peak_to_sidelobe_ratio: peak_to_sidelobe_ratio(response, peak_row, peak_column, peak_value),
    }
}

/// Where the top of the parabola through three neighbouring values is, from
/// −0.5 to 0.5 around the middle one.
pub(crate) fn parabolic_peak_offset(before: f32, peak: f32, after: f32) -> f32 {
    let curvature = before - 2.0 * peak + after;
    if curvature.abs() < f32::EPSILON {
        return 0.0;
    }
    (0.5 * (before - after) / curvature).clamp(-0.5, 0.5)
}

/// `(peak − mean) / standard deviation` of the response outside the square
/// around the peak.
fn peak_to_sidelobe_ratio(
    response: &[f32],
    peak_row: usize,
    peak_column: usize,
    peak_value: f32,
) -> f32 {
    let side = WINDOW_SIDE_IN_CELLS;
    let circular_distance = |first: usize, second: usize| {
        let distance = first.abs_diff(second);
        distance.min(side - distance)
    };
    let (mut sum, mut sum_of_squares, mut count) = (0.0_f32, 0.0_f32, 0_usize);
    for (index, &value) in response.iter().enumerate() {
        let (row, column) = (index / side, index % side);
        let is_near_the_peak = circular_distance(row, peak_row)
            <= PEAK_EXCLUSION_HALF_SIDE_IN_CELLS
            && circular_distance(column, peak_column) <= PEAK_EXCLUSION_HALF_SIDE_IN_CELLS;
        if !is_near_the_peak {
            sum += value;
            sum_of_squares += value * value;
            count += 1;
        }
    }
    let mean = sum / count as f32;
    let standard_deviation = (sum_of_squares / count as f32 - mean * mean)
        .max(0.0)
        .sqrt();
    (peak_value - mean) / standard_deviation.max(1e-6)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grey_image::GreyImage;
    use crate::synthetic_scenes::textured_value;

    fn textured_pyramid(seed: u32, shift_x: f32, shift_y: f32) -> ImagePyramid {
        let image = GreyImage::from_fn(240, 200, |column, row| {
            let (x, y) = (column as f32 + 0.5 - shift_x, row as f32 + 0.5 - shift_y);
            40.0 + 180.0 * textured_value(x, y, 6.0, seed)
        });
        ImagePyramid::new(image, 16)
    }

    fn window_at(x: f32, y: f32) -> SquareWindow {
        SquareWindow {
            center: Point { x, y },
            side_in_pixels: 96.0,
            region_width_in_pixels: 48.0,
            region_height_in_pixels: 48.0,
        }
    }

    #[test]
    fn the_peak_shows_how_far_the_content_moved() {
        let mut filter =
            CorrelationFilter::train(&textured_pyramid(3, 0.0, 0.0), window_at(120.0, 100.0))
                .unwrap();
        let peak = filter
            .find_peak(&textured_pyramid(3, 7.3, -4.6), window_at(120.0, 100.0))
            .unwrap();
        assert!(
            (peak.displacement_x_in_pixels - 7.3).abs() < 0.6,
            "{peak:?}"
        );
        assert!(
            (peak.displacement_y_in_pixels + 4.6).abs() < 0.6,
            "{peak:?}"
        );
        assert!(peak.peak_to_sidelobe_ratio > 20.0, "{peak:?}");
    }

    #[test]
    fn an_unrelated_texture_gives_a_low_peak_to_sidelobe_ratio() {
        let mut filter =
            CorrelationFilter::train(&textured_pyramid(3, 0.0, 0.0), window_at(120.0, 100.0))
                .unwrap();
        let same = filter
            .find_peak(&textured_pyramid(3, 0.0, 0.0), window_at(120.0, 100.0))
            .unwrap();
        let other = filter
            .find_peak(&textured_pyramid(4, 0.0, 0.0), window_at(120.0, 100.0))
            .unwrap();
        assert!(
            other.peak_to_sidelobe_ratio < same.peak_to_sidelobe_ratio / 3.0,
            "same {same:?}, other {other:?}"
        );
    }

    #[test]
    fn a_uniform_window_cannot_be_trained_on() {
        let pyramid = ImagePyramid::new(GreyImage::from_fn(100, 100, |_, _| 128.0), 16);
        assert!(CorrelationFilter::train(&pyramid, window_at(50.0, 50.0)).is_none());
    }

    #[test]
    fn parabolic_refinement_finds_the_top_between_cells() {
        assert_eq!(parabolic_peak_offset(1.0, 2.0, 1.0), 0.0);
        assert!(parabolic_peak_offset(1.0, 2.0, 1.8) > 0.3);
        assert_eq!(
            parabolic_peak_offset(2.0, 2.0, 2.0),
            0.0,
            "flat: no refinement"
        );
    }
}
