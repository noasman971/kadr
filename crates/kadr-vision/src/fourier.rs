//! Two-dimensional Fourier transforms of square windows.
//!
//! Correlation filters work in the frequency domain: correlating two `N × N`
//! windows costs `N²` multiplications there instead of `N⁴` directly. A 2D
//! transform is a 1D transform of every row, then of every column.

use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

/// Forward and inverse 2D transforms for one window size.
pub(crate) struct Fourier2d {
    size: usize,
    forward: Arc<dyn Fft<f32>>,
    inverse: Arc<dyn Fft<f32>>,
    transposed: Vec<Complex<f32>>,
}

impl std::fmt::Debug for Fourier2d {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Fourier2d({}×{})", self.size, self.size)
    }
}

impl Fourier2d {
    /// Prepares transforms of `size × size` windows.
    pub(crate) fn new(size: usize) -> Self {
        let mut planner = FftPlanner::new();
        Self {
            size,
            forward: planner.plan_fft_forward(size),
            inverse: planner.plan_fft_inverse(size),
            transposed: vec![Complex::new(0.0, 0.0); size * size],
        }
    }

    /// Transforms real values (row after row) into the frequency domain.
    pub(crate) fn forward_real(&mut self, values: &[f32]) -> Vec<Complex<f32>> {
        let mut data: Vec<Complex<f32>> = values
            .iter()
            .map(|&value| Complex::new(value, 0.0))
            .collect();
        self.transform(&mut data, true);
        data
    }

    /// Transforms back to the spatial domain and keeps the real part.
    pub(crate) fn inverse_real(&mut self, mut data: Vec<Complex<f32>>) -> Vec<f32> {
        self.transform(&mut data, false);
        let normalization = 1.0 / (self.size * self.size) as f32;
        data.iter().map(|value| value.re * normalization).collect()
    }

    fn transform(&mut self, data: &mut [Complex<f32>], is_forward: bool) {
        let transform = if is_forward {
            Arc::clone(&self.forward)
        } else {
            Arc::clone(&self.inverse)
        };
        // Every row (rustfft processes the buffer in chunks of `size`).
        transform.process(data);
        transpose_square(data, &mut self.transposed, self.size);
        // Every column, as rows of the transposed window.
        transform.process(&mut self.transposed);
        transpose_square(&self.transposed, data, self.size);
    }
}

fn transpose_square(source: &[Complex<f32>], destination: &mut [Complex<f32>], size: usize) {
    for row in 0..size {
        for column in 0..size {
            destination[column * size + row] = source[row * size + column];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_then_inverse_gives_the_values_back() {
        let mut fourier = Fourier2d::new(8);
        let values: Vec<f32> = (0..64).map(|index| (index as f32 * 0.37).sin()).collect();
        let spectrum = fourier.forward_real(&values);
        let back = fourier.inverse_real(spectrum);
        assert!(values.iter().zip(&back).all(|(a, b)| (a - b).abs() < 1e-4));
    }

    #[test]
    fn a_constant_window_has_only_a_zero_frequency() {
        let mut fourier = Fourier2d::new(4);
        let spectrum = fourier.forward_real(&[1.0; 16]);
        assert!((spectrum[0].re - 16.0).abs() < 1e-5);
        assert!(spectrum[1..].iter().all(|value| value.norm() < 1e-5));
    }
}
