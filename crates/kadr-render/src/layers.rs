//! What to draw: layers, their placement on the canvas, and the geometry shared
//! by the CPU and GPU renderers.

use kadr_core::clip::{AnimatableProperty, Clip, ClipContent, CropInsets};
use kadr_core::color::Color;
use kadr_core::effect::ColorAdjustment;
use kadr_core::media_asset::PictureSize;
use kadr_core::time::TimeSpan;
use kadr_media::VideoFrame;

use crate::color_lookup_table::LayerColorLut;

/// The picture being drawn into.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasDescription {
    /// Size of the result, in pixels.
    pub size: PictureSize,
    /// Colour where no layer covers the canvas.
    pub background: Color,
}

/// How big a picture is drawn before the layer's own scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayerSizing {
    /// As large as possible while fitting entirely inside the canvas (videos, images).
    #[default]
    FitInsideCanvas,
    /// One picture pixel per canvas pixel (texts, whose size is set in canvas pixels).
    NativePixels,
}

/// How the source picture moves inside its layer's window (video
/// stabilisation).
///
/// Source point v is drawn at C + s·R(r)·(v − C) + t, where s is `scale_factor`,
/// r is `rotation_in_degrees`, t is (`shift_x_in_pixels`, `shift_y_in_pixels`),
/// C is the picture's centre and R turns clockwise (y points down), all in the
/// decoded picture's pixels.
/// The layer's crop is a window on that moved picture: the correction does not
/// depend on the crop, and the window, its fit, scale, rotation and position stay
/// where they are. Texts always get [`SourceCorrection::IDENTITY`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceCorrection {
    /// How far the picture moves right (positive) or left, in source pixels.
    pub shift_x_in_pixels: f32,
    /// How far the picture moves down (positive) or up, in source pixels.
    pub shift_y_in_pixels: f32,
    /// Clockwise rotation of the picture around its centre.
    pub rotation_in_degrees: f32,
    /// Enlargement of the picture around its centre (1 = same size; must be
    /// above 0, or the layer shows nothing).
    pub scale_factor: f32,
}

impl SourceCorrection {
    /// The picture as it was decoded: not moved, turned nor enlarged.
    pub const IDENTITY: Self = Self {
        shift_x_in_pixels: 0.0,
        shift_y_in_pixels: 0.0,
        rotation_in_degrees: 0.0,
        scale_factor: 1.0,
    };

    /// Whether the correction leaves every source point where it is.
    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    /// The map from window points to the source points they show (the inverse
    /// of the correction), for a picture of `picture_width` × `picture_height`.
    fn window_to_source_matrix(&self, picture_width: f32, picture_height: f32) -> AffineMatrix {
        if self.is_identity() {
            return IDENTITY_MATRIX;
        }
        let (center_x, center_y) = (
            f64::from(picture_width) / 2.0,
            f64::from(picture_height) / 2.0,
        );
        let (shift_x, shift_y) = (
            f64::from(self.shift_x_in_pixels),
            f64::from(self.shift_y_in_pixels),
        );
        let rotation_in_radians = f64::from(self.rotation_in_degrees).to_radians();
        let scale = f64::from(self.scale_factor);
        // v = C + (1/s)·R(−r)·(w − C − t), written as an affine map of w.
        let (a, b) = (
            rotation_in_radians.cos() / scale,
            -rotation_in_radians.sin() / scale,
        );
        let (c, d) = (-b, a);
        let e = center_x - a * (center_x + shift_x) - c * (center_y + shift_y);
        let f = center_y - b * (center_x + shift_x) - d * (center_y + shift_y);
        [a, b, c, d, e, f].map(|value| value as f32)
    }

    /// The correction itself, from source points to window points, for a
    /// picture of `picture_width` × `picture_height`.
    fn source_to_window_matrix(&self, picture_width: f32, picture_height: f32) -> AffineMatrix {
        if self.is_identity() {
            return IDENTITY_MATRIX;
        }
        let (center_x, center_y) = (
            f64::from(picture_width) / 2.0,
            f64::from(picture_height) / 2.0,
        );
        let rotation_in_radians = f64::from(self.rotation_in_degrees).to_radians();
        let scale = f64::from(self.scale_factor);
        // w = C + s·R(r)·(v − C) + t, written as an affine map of v.
        let (a, b) = (
            scale * rotation_in_radians.cos(),
            scale * rotation_in_radians.sin(),
        );
        let (c, d) = (-b, a);
        let e = center_x + f64::from(self.shift_x_in_pixels) - a * center_x - c * center_y;
        let f = center_y + f64::from(self.shift_y_in_pixels) - b * center_x - d * center_y;
        [a, b, c, d, e, f].map(|value| value as f32)
    }
}

/// An affine map in the order of SVG's `matrix(a, b, c, d, e, f)`:
/// `x′ = a·x + c·y + e` and `y′ = b·x + d·y + f`.
pub(crate) type AffineMatrix = [f32; 6];

/// The affine map leaving every point where it is.
const IDENTITY_MATRIX: AffineMatrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// Where `matrix` sends point (x, y).
fn apply_affine(matrix: &AffineMatrix, x: f32, y: f32) -> (f32, f32) {
    let [a, b, c, d, e, f] = *matrix;
    (a * x + c * y + e, b * x + d * y + f)
}

/// The affine map applying `inner` first, then `outer`.
fn compose_affine(outer: &AffineMatrix, inner: &AffineMatrix) -> AffineMatrix {
    let [a1, b1, c1, d1, e1, f1] = *outer;
    let [a2, b2, c2, d2, e2, f2] = *inner;
    [
        a1 * a2 + c1 * b2,
        b1 * a2 + d1 * b2,
        a1 * c2 + c1 * d2,
        b1 * c2 + d1 * d2,
        a1 * e2 + c1 * f2 + e1,
        b1 * e2 + d1 * f2 + f1,
    ]
}

/// Where and how one picture is drawn on the canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerPlacement {
    /// Horizontal offset of the picture's centre from the canvas centre; right is positive.
    pub offset_x_in_pixels: f32,
    /// Vertical offset of the picture's centre from the canvas centre; down is positive.
    pub offset_y_in_pixels: f32,
    /// How big the picture is at scale 1.
    pub sizing: LayerSizing,
    /// Size multiplier, applied on top of `sizing`.
    pub scale_factor: f32,
    /// Clockwise rotation around the picture's centre.
    pub rotation_in_degrees: f32,
    /// Edges removed from the source picture before fitting: the layer's window.
    pub crop: CropInsets,
    /// 0 invisible to 1 opaque.
    pub opacity: f32,
    /// How the source picture moves inside the window, before the window is
    /// fitted, scaled, rotated and moved (video stabilisation).
    pub source_correction: SourceCorrection,
}

impl LayerPlacement {
    /// Centred, fitted, opaque, not rotated nor cropped nor corrected.
    pub const FITTED: Self = Self {
        offset_x_in_pixels: 0.0,
        offset_y_in_pixels: 0.0,
        sizing: LayerSizing::FitInsideCanvas,
        scale_factor: 1.0,
        rotation_in_degrees: 0.0,
        crop: CropInsets {
            left_fraction: 0.0,
            top_fraction: 0.0,
            right_fraction: 0.0,
            bottom_fraction: 0.0,
        },
        opacity: 1.0,
        source_correction: SourceCorrection::IDENTITY,
    };

    /// The placement of a clip at `time_offset` from its start, with its
    /// keyframe animations evaluated at that moment. Text clips are drawn at
    /// their native size, everything else is fitted inside the canvas.
    ///
    /// The source correction is left at [`SourceCorrection::IDENTITY`]: it
    /// depends on the source time, which the caller sets.
    pub fn of_clip_at(clip: &Clip, time_offset: TimeSpan) -> Self {
        let value_of = |property| clip.animated_property(property).value_at(time_offset);
        let sizing = match clip.content {
            ClipContent::Text(_) => LayerSizing::NativePixels,
            ClipContent::Media(_) => LayerSizing::FitInsideCanvas,
        };
        Self {
            offset_x_in_pixels: value_of(AnimatableProperty::PositionX),
            offset_y_in_pixels: value_of(AnimatableProperty::PositionY),
            sizing,
            scale_factor: value_of(AnimatableProperty::Scale),
            rotation_in_degrees: value_of(AnimatableProperty::Rotation),
            crop: clip.transform.crop,
            opacity: value_of(AnimatableProperty::Opacity),
            source_correction: SourceCorrection::IDENTITY,
        }
    }

    /// The opacity, clamped between 0 and 1.
    pub fn clamped_opacity(&self) -> f32 {
        self.opacity.clamp(0.0, 1.0)
    }
}

impl LayerPlacement {
    /// Where the source picture (of `picture_size`) lands on a canvas of
    /// `canvas_size` with this placement, for interfaces that draw on top of a
    /// clip (a blur's box on the preview). `None` when the layer shows nothing.
    pub fn source_to_canvas_mapping(
        &self,
        canvas_size: PictureSize,
        picture_size: PictureSize,
    ) -> Option<SourceToCanvasMapping> {
        let geometry = LayerGeometry::new(
            canvas_size,
            picture_size.width_in_pixels,
            picture_size.height_in_pixels,
            self,
        )?;
        let scale = 1.0 / geometry.source_pixels_per_canvas_pixel;
        let (a, b) = (
            scale * geometry.rotation_cosine,
            scale * geometry.rotation_sine,
        );
        let (c, d) = (-b, a);
        let e = geometry.canvas_center_x
            - (a * geometry.source_center_x + c * geometry.source_center_y);
        let f = geometry.canvas_center_y
            - (b * geometry.source_center_x + d * geometry.source_center_y);
        let window_to_canvas = [a, b, c, d, e, f];
        let source_to_window = self
            .source_correction
            .source_to_window_matrix(geometry.picture_width, geometry.picture_height);
        Some(SourceToCanvasMapping {
            matrix: compose_affine(&window_to_canvas, &source_to_window),
            visible_source: geometry.visible_source(),
        })
    }
}

/// How a layer's source pixels map onto the canvas: the source correction moves
/// the picture inside the layer's window (its crop), then the window is fitted,
/// scaled, rotated clockwise around its centre, and moved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceToCanvasMapping {
    /// The affine map in the order of SVG's `matrix(a, b, c, d, e, f)`:
    /// `canvas_x = a·x + c·y + e` and `canvas_y = b·x + d·y + f`, for a source
    /// point (x, y) in pixels, origin at the top-left corner.
    pub matrix: [f32; 6],
    /// The part of the source picture that can show: left, top, right and
    /// bottom edges, in source pixels. Without a source correction, the crop;
    /// with one, the bounding box of the source area seen through the window
    /// (turned when the correction turns), kept inside the picture.
    pub visible_source: [f32; 4],
}

impl SourceToCanvasMapping {
    /// The canvas point showing source point (x, y).
    pub fn canvas_point_of(&self, source_x: f32, source_y: f32) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.matrix;
        (
            a * source_x + c * source_y + e,
            b * source_x + d * source_y + f,
        )
    }
}

/// A picture to draw, and how.
#[derive(Debug, Clone, Copy)]
pub struct VisualLayer<'picture> {
    /// The source picture (YUV or RGBA).
    pub picture: &'picture VideoFrame,
    /// Where and how to draw it.
    pub placement: LayerPlacement,
    /// Colour adjustments applied in order (at most
    /// [`MAXIMUM_COLOR_ADJUSTMENTS_PER_LAYER`](crate::color_adjustment::MAXIMUM_COLOR_ADJUSTMENTS_PER_LAYER)).
    pub color_adjustments: &'picture [ColorAdjustment],
    /// A colour LUT applied after the colour adjustments.
    pub color_lut: Option<LayerColorLut<'picture>>,
}

impl VisualLayer<'_> {
    /// Whether drawing the layer changes its colours (colour adjustments, or a
    /// LUT with some strength). A layer that changes none may be copied as it is:
    /// every shortcut that copies pictures must ask this.
    pub fn changes_colors(&self) -> bool {
        !self.color_adjustments.is_empty() || self.color_lut.is_some_and(|lut| lut.is_visible())
    }
}

/// The mapping between canvas pixels and source pixels for one layer.
///
/// The source correction moves the picture inside the layer's window (the
/// crop); the window is fitted inside the canvas, scaled, rotated clockwise
/// around its centre, then moved. Without a correction, window points are
/// source points.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LayerGeometry {
    /// The layer's window (its crop), in source pixels of the corrected picture.
    pub(crate) crop_left: f32,
    pub(crate) crop_top: f32,
    pub(crate) crop_right: f32,
    pub(crate) crop_bottom: f32,
    /// Centre of the window, in the same pixels.
    pub(crate) source_center_x: f32,
    pub(crate) source_center_y: f32,
    /// Where that centre lands on the canvas.
    pub(crate) canvas_center_x: f32,
    pub(crate) canvas_center_y: f32,
    /// Source pixels per canvas pixel.
    pub(crate) source_pixels_per_canvas_pixel: f32,
    /// Displayed size of the window, in canvas pixels.
    pub(crate) displayed_width: f32,
    pub(crate) displayed_height: f32,
    pub(crate) rotation_cosine: f32,
    pub(crate) rotation_sine: f32,
    /// The map from window points to the source points they show: the inverse
    /// of the source correction (the identity without one).
    pub(crate) window_to_source: AffineMatrix,
    /// Size of the source picture, in pixels.
    pub(crate) picture_width: f32,
    pub(crate) picture_height: f32,
    /// Whether the source correction moves the picture at all. A corrected
    /// layer may show any part of the picture, not only its crop.
    pub(crate) has_source_correction: bool,
}

impl LayerGeometry {
    /// `None` when the layer cannot show anything (empty picture, zero scale, crop
    /// removing everything, a correction shrinking the picture to nothing).
    pub(crate) fn new(
        canvas_size: PictureSize,
        picture_width: u32,
        picture_height: u32,
        placement: &LayerPlacement,
    ) -> Option<Self> {
        let (picture_width, picture_height) = (picture_width as f32, picture_height as f32);
        let crop = placement.crop;
        let crop_left = crop.left_fraction * picture_width;
        let crop_right = picture_width - crop.right_fraction * picture_width;
        let crop_top = crop.top_fraction * picture_height;
        let crop_bottom = picture_height - crop.bottom_fraction * picture_height;
        let (visible_width, visible_height) = (crop_right - crop_left, crop_bottom - crop_top);
        let correction = placement.source_correction;
        if visible_width < 1.0
            || visible_height < 1.0
            || placement.scale_factor <= 0.0
            || correction.scale_factor <= 0.0
        {
            return None;
        }
        let (canvas_width, canvas_height) = (
            canvas_size.width_in_pixels as f32,
            canvas_size.height_in_pixels as f32,
        );
        let sizing_scale = match placement.sizing {
            LayerSizing::FitInsideCanvas => {
                (canvas_width / visible_width).min(canvas_height / visible_height)
            }
            LayerSizing::NativePixels => 1.0,
        };
        let total_scale = sizing_scale * placement.scale_factor;
        let rotation_in_radians = placement.rotation_in_degrees.to_radians();
        Some(Self {
            crop_left,
            crop_top,
            crop_right,
            crop_bottom,
            source_center_x: crop_left + visible_width / 2.0,
            source_center_y: crop_top + visible_height / 2.0,
            canvas_center_x: canvas_width / 2.0 + placement.offset_x_in_pixels,
            canvas_center_y: canvas_height / 2.0 + placement.offset_y_in_pixels,
            source_pixels_per_canvas_pixel: 1.0 / total_scale,
            displayed_width: visible_width * total_scale,
            displayed_height: visible_height * total_scale,
            rotation_cosine: rotation_in_radians.cos(),
            rotation_sine: rotation_in_radians.sin(),
            window_to_source: correction.window_to_source_matrix(picture_width, picture_height),
            picture_width,
            picture_height,
            has_source_correction: !correction.is_identity(),
        })
    }

    /// Whether canvas pixel (x, y) shows source pixel (x, y) exactly: same size,
    /// no crop, no move, no rotation, no scaling, no source correction.
    pub(crate) fn maps_pixels_one_to_one(
        &self,
        canvas_size: PictureSize,
        picture_width: u32,
        picture_height: u32,
        placement: &LayerPlacement,
    ) -> bool {
        (canvas_size.width_in_pixels, canvas_size.height_in_pixels)
            == (picture_width, picture_height)
            && placement.crop == CropInsets::default()
            && placement.offset_x_in_pixels == 0.0
            && placement.offset_y_in_pixels == 0.0
            && placement.rotation_in_degrees == 0.0
            && self.source_pixels_per_canvas_pixel == 1.0
            && placement.source_correction.is_identity()
    }

    /// The window's corners, in the order top-left, top-right, bottom-left,
    /// bottom-right, in window points.
    fn window_corners(&self) -> [(f32, f32); 4] {
        [
            (self.crop_left, self.crop_top),
            (self.crop_right, self.crop_top),
            (self.crop_left, self.crop_bottom),
            (self.crop_right, self.crop_bottom),
        ]
    }

    /// The source point shown at window point (x, y).
    fn source_point_of_window_point(&self, window_x: f32, window_y: f32) -> (f32, f32) {
        apply_affine(&self.window_to_source, window_x, window_y)
    }

    /// The part of the source picture that can show, as left, top, right and
    /// bottom edges in source pixels: the crop without a correction, the
    /// bounding box of the window's source corners (inside the picture) with one.
    pub(crate) fn visible_source(&self) -> [f32; 4] {
        if !self.has_source_correction {
            return [
                self.crop_left,
                self.crop_top,
                self.crop_right,
                self.crop_bottom,
            ];
        }
        let corners = self
            .window_corners()
            .map(|(window_x, window_y)| self.source_point_of_window_point(window_x, window_y));
        let smallest = |values: [f32; 4]| values.into_iter().fold(f32::INFINITY, f32::min);
        let largest = |values: [f32; 4]| values.into_iter().fold(f32::NEG_INFINITY, f32::max);
        let xs = corners.map(|(x, _)| x);
        let ys = corners.map(|(_, y)| y);
        [
            smallest(xs).clamp(0.0, self.picture_width),
            smallest(ys).clamp(0.0, self.picture_height),
            largest(xs).clamp(0.0, self.picture_width),
            largest(ys).clamp(0.0, self.picture_height),
        ]
    }

    /// Canvas positions of the corners of the visible part, in the order
    /// top-left, top-right, bottom-left, bottom-right (of the unrotated picture).
    pub(crate) fn canvas_corners(&self) -> [(f32, f32); 4] {
        let (half_width, half_height) = (self.displayed_width / 2.0, self.displayed_height / 2.0);
        [
            (-half_width, -half_height),
            (half_width, -half_height),
            (-half_width, half_height),
            (half_width, half_height),
        ]
        .map(|(local_x, local_y)| {
            // Clockwise rotation, y pointing down.
            (
                self.canvas_center_x + local_x * self.rotation_cosine
                    - local_y * self.rotation_sine,
                self.canvas_center_y
                    + local_x * self.rotation_sine
                    + local_y * self.rotation_cosine,
            )
        })
    }

    /// Positions in the source picture (0–1) of the same corners: the crop,
    /// moved through the inverse of the source correction. The map from canvas
    /// to source is affine, so interpolating these across the quad's two
    /// triangles reproduces it exactly (texture coordinates may then leave 0–1,
    /// where the sampler repeats the picture's edge pixels).
    pub(crate) fn texture_corners(&self) -> [(f32, f32); 4] {
        self.window_corners().map(|(window_x, window_y)| {
            let (source_x, source_y) = self.source_point_of_window_point(window_x, window_y);
            (
                source_x / self.picture_width,
                source_y / self.picture_height,
            )
        })
    }

    /// The canvas rows and columns the layer may cover (rotated bounding box),
    /// as `(first_column, end_column, first_row, end_row)`.
    pub(crate) fn covered_canvas_area(&self, canvas_size: PictureSize) -> (u32, u32, u32, u32) {
        let (half_width, half_height) = (self.displayed_width / 2.0, self.displayed_height / 2.0);
        let half_extent_x =
            half_width * self.rotation_cosine.abs() + half_height * self.rotation_sine.abs();
        let half_extent_y =
            half_width * self.rotation_sine.abs() + half_height * self.rotation_cosine.abs();
        let clamp_column = |value: f32| value.clamp(0.0, canvas_size.width_in_pixels as f32) as u32;
        let clamp_row = |value: f32| value.clamp(0.0, canvas_size.height_in_pixels as f32) as u32;
        (
            clamp_column((self.canvas_center_x - half_extent_x).floor()),
            clamp_column((self.canvas_center_x + half_extent_x).ceil()),
            clamp_row((self.canvas_center_y - half_extent_y).floor()),
            clamp_row((self.canvas_center_y + half_extent_y).ceil()),
        )
    }

    /// The source point shown at the centre of canvas pixel (x, y), or `None` if
    /// it falls outside the layer's window.
    pub(crate) fn source_point_for(&self, canvas_x: u32, canvas_y: u32) -> Option<(f32, f32)> {
        let from_center_x = canvas_x as f32 + 0.5 - self.canvas_center_x;
        let from_center_y = canvas_y as f32 + 0.5 - self.canvas_center_y;
        // Undo the clockwise rotation (y points down), then the scaling.
        let unrotated_x = from_center_x * self.rotation_cosine + from_center_y * self.rotation_sine;
        let unrotated_y =
            -from_center_x * self.rotation_sine + from_center_y * self.rotation_cosine;
        let window_x = self.source_center_x + unrotated_x * self.source_pixels_per_canvas_pixel;
        let window_y = self.source_center_y + unrotated_y * self.source_pixels_per_canvas_pixel;
        let is_inside = window_x >= self.crop_left
            && window_x < self.crop_right
            && window_y >= self.crop_top
            && window_y < self.crop_bottom;
        // Then undo the source correction.
        is_inside.then(|| self.source_point_of_window_point(window_x, window_y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_of_a_fitted_picture_touch_the_canvas_edges() {
        let canvas = PictureSize {
            width_in_pixels: 200,
            height_in_pixels: 100,
        };
        let geometry = LayerGeometry::new(canvas, 400, 200, &LayerPlacement::FITTED).unwrap();
        assert_eq!(
            geometry.canvas_corners(),
            [(0.0, 0.0), (200.0, 0.0), (0.0, 100.0), (200.0, 100.0)]
        );
        assert_eq!(
            geometry.texture_corners(),
            [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
        );
    }

    #[test]
    fn native_pixels_keep_the_picture_size() {
        let canvas = PictureSize {
            width_in_pixels: 1920,
            height_in_pixels: 1080,
        };
        let native = LayerPlacement {
            sizing: LayerSizing::NativePixels,
            ..LayerPlacement::FITTED
        };
        let geometry = LayerGeometry::new(canvas, 200, 80, &native).unwrap();
        assert_eq!(geometry.canvas_corners()[0], (860.0, 500.0));
        assert_eq!(geometry.canvas_corners()[3], (1060.0, 580.0));
    }

    /// A vertical canvas.
    fn portrait_canvas() -> PictureSize {
        PictureSize {
            width_in_pixels: 1080,
            height_in_pixels: 1920,
        }
    }

    /// A full HD picture.
    fn landscape_picture() -> PictureSize {
        PictureSize {
            width_in_pixels: 1920,
            height_in_pixels: 1080,
        }
    }

    /// Moved, enlarged, turned and cropped; not corrected.
    fn moved_and_cropped() -> LayerPlacement {
        LayerPlacement {
            offset_x_in_pixels: 120.0,
            offset_y_in_pixels: -40.0,
            scale_factor: 1.2,
            rotation_in_degrees: 12.0,
            crop: CropInsets {
                left_fraction: 0.1,
                top_fraction: 0.0,
                right_fraction: 0.05,
                bottom_fraction: 0.2,
            },
            ..LayerPlacement::FITTED
        }
    }

    /// A stabilisation's correction: a shift, a slight turn and an enlargement.
    const SHAKE_CORRECTION: SourceCorrection = SourceCorrection {
        shift_x_in_pixels: 7.3,
        shift_y_in_pixels: -4.1,
        rotation_in_degrees: 0.8,
        scale_factor: 1.06,
    };

    /// The source point that `correction` draws at window point (x, y),
    /// straight from the definition of [`SourceCorrection`].
    fn source_point_drawn_at(
        correction: SourceCorrection,
        picture: PictureSize,
        (window_x, window_y): (f32, f32),
    ) -> (f64, f64) {
        let center_x = f64::from(picture.width_in_pixels) / 2.0;
        let center_y = f64::from(picture.height_in_pixels) / 2.0;
        let rotation = f64::from(correction.rotation_in_degrees).to_radians();
        let scale = f64::from(correction.scale_factor);
        let x = f64::from(window_x) - center_x - f64::from(correction.shift_x_in_pixels);
        let y = f64::from(window_y) - center_y - f64::from(correction.shift_y_in_pixels);
        // Undo the clockwise turn (y points down), then the enlargement.
        (
            center_x + (x * rotation.cos() + y * rotation.sin()) / scale,
            center_y + (-x * rotation.sin() + y * rotation.cos()) / scale,
        )
    }

    #[test]
    fn the_public_mapping_agrees_with_the_renderers_geometry() {
        let (canvas, picture) = (portrait_canvas(), landscape_picture());
        let uncorrected = moved_and_cropped();
        let corrected = LayerPlacement {
            source_correction: SHAKE_CORRECTION,
            ..uncorrected
        };
        for placement in [uncorrected, corrected] {
            let mapping = placement.source_to_canvas_mapping(canvas, picture).unwrap();
            let geometry = LayerGeometry::new(canvas, 1920, 1080, &placement).unwrap();
            // The canvas pixel showing a source point (all on the canvas) maps
            // back near that point.
            for (source_x, source_y) in [(400.0, 300.0), (960.0, 540.0), (1500.0, 800.0)] {
                let (canvas_x, canvas_y) = mapping.canvas_point_of(source_x, source_y);
                let pixel = (canvas_x.floor() as u32, canvas_y.floor() as u32);
                let (back_x, back_y) = geometry.source_point_for(pixel.0, pixel.1).unwrap();
                let pixel_in_source = geometry.source_pixels_per_canvas_pixel;
                assert!(
                    (back_x - source_x).abs() <= pixel_in_source * 1.5,
                    "{back_x} vs {source_x}"
                );
                assert!(
                    (back_y - source_y).abs() <= pixel_in_source * 1.5,
                    "{back_y} vs {source_y}"
                );
            }
        }
        let visible_source_of = |placement: LayerPlacement| {
            placement
                .source_to_canvas_mapping(canvas, picture)
                .unwrap()
                .visible_source
        };
        assert_eq!(visible_source_of(uncorrected), [192.0, 0.0, 1824.0, 864.0]);
        // Corrected: the bounding box of the source points seen at the crop's
        // corners.
        let seen_at_corners = [(192.0, 0.0), (1824.0, 0.0), (192.0, 864.0), (1824.0, 864.0)]
            .map(|corner| source_point_drawn_at(SHAKE_CORRECTION, picture, corner));
        let xs = seen_at_corners.map(|(x, _)| x);
        let ys = seen_at_corners.map(|(_, y)| y);
        let smallest = |values: [f64; 4]| values.into_iter().fold(f64::INFINITY, f64::min);
        let largest = |values: [f64; 4]| values.into_iter().fold(f64::NEG_INFINITY, f64::max);
        let expected = [smallest(xs), smallest(ys), largest(xs), largest(ys)];
        for (edge, expected_edge) in visible_source_of(corrected).into_iter().zip(expected) {
            assert!(
                (f64::from(edge) - expected_edge).abs() < 1e-2,
                "{edge} vs {expected_edge}"
            );
        }
        // A box reaching past the picture is kept inside it.
        let far_right = LayerPlacement {
            source_correction: SourceCorrection {
                shift_x_in_pixels: 500.0,
                ..SourceCorrection::IDENTITY
            },
            ..uncorrected
        };
        assert_eq!(visible_source_of(far_right), [0.0, 0.0, 1324.0, 864.0]);
    }

    #[test]
    fn a_correction_turns_clockwise_and_enlarges_around_the_picture_centre() {
        let picture = landscape_picture();
        let turned_and_doubled = LayerPlacement {
            source_correction: SourceCorrection {
                rotation_in_degrees: 90.0,
                scale_factor: 2.0,
                shift_x_in_pixels: 3.0,
                shift_y_in_pixels: -1.0,
            },
            ..LayerPlacement::FITTED
        };
        let mapping = turned_and_doubled
            .source_to_canvas_mapping(picture, picture)
            .unwrap();
        // 10 pixels right of the centre lands 20 pixels below it, then shifted.
        let (x, y) = mapping.canvas_point_of(970.0, 540.0);
        assert!(
            (x - 963.0).abs() < 1e-3 && (y - 559.0).abs() < 1e-3,
            "({x}, {y})"
        );
    }

    #[test]
    fn an_identity_correction_changes_no_geometry() {
        let (canvas, picture) = (portrait_canvas(), landscape_picture());
        assert!(
            SourceCorrection {
                shift_x_in_pixels: -0.0,
                ..SourceCorrection::IDENTITY
            }
            .is_identity()
        );
        let placement = moved_and_cropped();
        let geometry = LayerGeometry::new(canvas, 1920, 1080, &placement).unwrap();
        assert!(!geometry.has_source_correction);
        assert_eq!(geometry.window_to_source, IDENTITY_MATRIX);
        assert_eq!(
            geometry.texture_corners(),
            [(0.1, 0.0), (0.95, 0.0), (0.1, 0.8), (0.95, 0.8)]
        );
        assert_eq!(geometry.visible_source(), [192.0, 0.0, 1824.0, 864.0]);
        // The public mapping sends the crop's corners to the quad's corners.
        let mapping = placement.source_to_canvas_mapping(canvas, picture).unwrap();
        for ((source_x, source_y), (canvas_x, canvas_y)) in geometry
            .window_corners()
            .into_iter()
            .zip(geometry.canvas_corners())
        {
            let (x, y) = mapping.canvas_point_of(source_x, source_y);
            assert!(
                (x - canvas_x).abs() < 1e-3 && (y - canvas_y).abs() < 1e-3,
                "({x}, {y}) vs ({canvas_x}, {canvas_y})"
            );
        }
    }

    #[test]
    fn interpolated_texture_corners_agree_with_source_point_for() {
        let canvas = portrait_canvas();
        let placement = LayerPlacement {
            source_correction: SHAKE_CORRECTION,
            ..moved_and_cropped()
        };
        let geometry = LayerGeometry::new(canvas, 1920, 1080, &placement).unwrap();
        let [top_left, top_right, bottom_left, _] = geometry
            .canvas_corners()
            .map(|(x, y)| (f64::from(x), f64::from(y)));
        let [
            texture_top_left,
            texture_top_right,
            texture_bottom_left,
            texture_bottom_right,
        ] = geometry
            .texture_corners()
            .map(|(x, y)| (f64::from(x) * 1920.0, f64::from(y) * 1080.0));
        // A canvas point is top_left + u·along_top + v·along_left.
        let along_top = (top_right.0 - top_left.0, top_right.1 - top_left.1);
        let along_left = (bottom_left.0 - top_left.0, bottom_left.1 - top_left.1);
        let determinant = along_top.0 * along_left.1 - along_left.0 * along_top.1;
        let mut checked_pixels = 0;
        for canvas_y in (0..canvas.height_in_pixels).step_by(37) {
            for canvas_x in (0..canvas.width_in_pixels).step_by(37) {
                let Some((source_x, source_y)) = geometry.source_point_for(canvas_x, canvas_y)
                else {
                    continue;
                };
                let from_corner = (
                    f64::from(canvas_x) + 0.5 - top_left.0,
                    f64::from(canvas_y) + 0.5 - top_left.1,
                );
                let u = (from_corner.0 * along_left.1 - along_left.0 * from_corner.1) / determinant;
                let v = (along_top.0 * from_corner.1 - from_corner.0 * along_top.1) / determinant;
                // What the GPU interpolates between the corners.
                let interpolate = |pick: fn((f64, f64)) -> f64| {
                    (1.0 - u) * (1.0 - v) * pick(texture_top_left)
                        + u * (1.0 - v) * pick(texture_top_right)
                        + (1.0 - u) * v * pick(texture_bottom_left)
                        + u * v * pick(texture_bottom_right)
                };
                let (interpolated_x, interpolated_y) =
                    (interpolate(|(x, _)| x), interpolate(|(_, y)| y));
                assert!(
                    (interpolated_x - f64::from(source_x)).abs() < 1e-3
                        && (interpolated_y - f64::from(source_y)).abs() < 1e-3,
                    "pixel ({canvas_x}, {canvas_y}): ({interpolated_x}, {interpolated_y}) vs ({source_x}, {source_y})"
                );
                checked_pixels += 1;
            }
        }
        assert!(checked_pixels > 500, "only {checked_pixels} pixels checked");
    }

    #[test]
    fn a_correction_stops_the_one_to_one_shortcut() {
        let picture = landscape_picture();
        let corrected = |source_correction| LayerPlacement {
            source_correction,
            ..LayerPlacement::FITTED
        };
        let cases = [
            (LayerPlacement::FITTED, true),
            (corrected(SHAKE_CORRECTION), false),
            (
                corrected(SourceCorrection {
                    shift_y_in_pixels: 0.25,
                    ..SourceCorrection::IDENTITY
                }),
                false,
            ),
            (
                corrected(SourceCorrection {
                    rotation_in_degrees: 0.01,
                    ..SourceCorrection::IDENTITY
                }),
                false,
            ),
            (
                corrected(SourceCorrection {
                    scale_factor: 1.001,
                    ..SourceCorrection::IDENTITY
                }),
                false,
            ),
        ];
        for (placement, is_one_to_one) in cases {
            let geometry = LayerGeometry::new(picture, 1920, 1080, &placement).unwrap();
            assert_eq!(
                geometry.maps_pixels_one_to_one(picture, 1920, 1080, &placement),
                is_one_to_one,
                "{:?}",
                placement.source_correction
            );
        }
    }

    #[test]
    fn a_correction_shrinking_the_picture_to_nothing_shows_nothing() {
        let collapsed = LayerPlacement {
            source_correction: SourceCorrection {
                scale_factor: 0.0,
                ..SourceCorrection::IDENTITY
            },
            ..LayerPlacement::FITTED
        };
        let picture = landscape_picture();
        assert!(LayerGeometry::new(picture, 1920, 1080, &collapsed).is_none());
        assert!(
            collapsed
                .source_to_canvas_mapping(picture, picture)
                .is_none()
        );
    }

    #[test]
    fn a_quarter_turn_moves_the_top_left_corner_to_the_top_right() {
        let canvas = PictureSize {
            width_in_pixels: 100,
            height_in_pixels: 100,
        };
        let quarter_turn = LayerPlacement {
            rotation_in_degrees: 90.0,
            ..LayerPlacement::FITTED
        };
        let geometry = LayerGeometry::new(canvas, 100, 100, &quarter_turn).unwrap();
        let (x, y) = geometry.canvas_corners()[0];
        assert!((x - 100.0).abs() < 1e-3 && y.abs() < 1e-3, "({x}, {y})");
    }
}
