//! Colour adjustment: exposure, contrast, saturation and temperature.
//!
//! The same formula runs on the CPU ([`adjust_color`]) and on the GPU
//! (`adjust_color` in `gpu/layers.wgsl`), on colours from 0 to 1, after the
//! picture is sampled and before it is blended onto the canvas:
//!
//! 1. exposure: multiply by 2^stops;
//! 2. contrast: stretch around mid-grey (0.5);
//! 3. saturation: move away from (or towards) the pixel's brightness;
//! 4. temperature: add red and remove blue (warmer), or the reverse (cooler);
//! 5. clamp to 0–1.

use kadr_core::effect::ColorAdjustment;

/// How many colour adjustments one layer can have; further ones are ignored
/// (and reported by the export).
pub const MAXIMUM_COLOR_ADJUSTMENTS_PER_LAYER: usize = 4;

/// Contribution of red, green and blue to brightness (BT.709).
const BRIGHTNESS_WEIGHTS: [f32; 3] = [0.2126, 0.7152, 0.0722];
/// Red added and blue removed at temperature +1.
const TEMPERATURE_STRENGTH: f32 = 0.1;
/// The grey contrast pivots around.
const MID_GREY: f32 = 0.5;

/// Applies one adjustment to a colour (each channel from 0 to 1).
pub fn adjust_color(rgb: [f32; 3], adjustment: &ColorAdjustment) -> [f32; 3] {
    let exposure_multiplier = adjustment.exposure_in_stops.exp2();
    let exposed = rgb.map(|channel| channel * exposure_multiplier);
    let contrasted =
        exposed.map(|channel| (channel - MID_GREY) * adjustment.contrast_multiplier + MID_GREY);
    let brightness: f32 = contrasted
        .iter()
        .zip(BRIGHTNESS_WEIGHTS)
        .map(|(channel, weight)| channel * weight)
        .sum();
    let saturated = contrasted
        .map(|channel| brightness + (channel - brightness) * adjustment.saturation_multiplier);
    let temperature_shift = TEMPERATURE_STRENGTH * adjustment.temperature_shift;
    [
        saturated[0] + temperature_shift,
        saturated[1],
        saturated[2] - temperature_shift,
    ]
    .map(|channel| channel.clamp(0.0, 1.0))
}

/// Applies several adjustments in order.
pub fn adjust_color_in_sequence(rgb: [f32; 3], adjustments: &[ColorAdjustment]) -> [f32; 3] {
    adjustments
        .iter()
        .take(MAXIMUM_COLOR_ADJUSTMENTS_PER_LAYER)
        .fold(rgb, adjust_color)
}

/// The four numbers the GPU shader reads for one adjustment:
/// exposure multiplier, contrast, saturation, temperature.
pub(crate) fn shader_values(adjustment: &ColorAdjustment) -> [f32; 4] {
    [
        adjustment.exposure_in_stops.exp2(),
        adjustment.contrast_multiplier,
        adjustment.saturation_multiplier,
        adjustment.temperature_shift,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(first: [f32; 3], second: [f32; 3]) -> bool {
        first.iter().zip(second).all(|(a, b)| (a - b).abs() < 1e-5)
    }

    #[test]
    fn the_neutral_adjustment_changes_nothing() {
        let colour = [0.2, 0.5, 0.9];
        assert!(close(
            adjust_color(colour, &ColorAdjustment::NEUTRAL),
            colour
        ));
    }

    #[test]
    fn one_stop_of_exposure_doubles_the_light() {
        let brighter = ColorAdjustment {
            exposure_in_stops: 1.0,
            ..ColorAdjustment::NEUTRAL
        };
        assert!(close(
            adjust_color([0.1, 0.2, 0.3], &brighter),
            [0.2, 0.4, 0.6]
        ));
        assert!(
            close(adjust_color([0.8, 0.8, 0.8], &brighter), [1.0, 1.0, 1.0]),
            "clamped"
        );
    }

    #[test]
    fn zero_saturation_gives_grey() {
        let black_and_white = ColorAdjustment {
            saturation_multiplier: 0.0,
            ..ColorAdjustment::NEUTRAL
        };
        let [red, green, blue] = adjust_color([1.0, 0.0, 0.0], &black_and_white);
        assert!((red - green).abs() < 1e-6 && (green - blue).abs() < 1e-6);
        assert!((red - 0.2126).abs() < 1e-5);
    }

    #[test]
    fn contrast_pivots_around_mid_grey_and_warmth_shifts_red_and_blue() {
        let flat = ColorAdjustment {
            contrast_multiplier: 0.0,
            ..ColorAdjustment::NEUTRAL
        };
        assert!(close(adjust_color([0.1, 0.9, 0.3], &flat), [0.5, 0.5, 0.5]));
        let warm = ColorAdjustment {
            temperature_shift: 1.0,
            ..ColorAdjustment::NEUTRAL
        };
        assert!(close(adjust_color([0.5, 0.5, 0.5], &warm), [0.6, 0.5, 0.4]));
    }
}
