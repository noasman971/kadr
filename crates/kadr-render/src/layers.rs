//! What to draw: layers, their placement on the canvas, and the geometry shared
//! by the CPU and GPU renderers.

use kadr_core::clip::{AnimatableProperty, Clip, ClipContent, CropInsets};
use kadr_core::color::Color;
use kadr_core::media_asset::PictureSize;
use kadr_core::time::TimeSpan;
use kadr_media::VideoFrame;

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
    /// Edges removed from the source picture before fitting.
    pub crop: CropInsets,
    /// 0 invisible to 1 opaque.
    pub opacity: f32,
}

impl LayerPlacement {
    /// Centred, fitted, opaque, not rotated nor cropped.
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
    };

    /// The placement of a clip at `time_offset` from its start, with its
    /// keyframe animations evaluated at that moment. Text clips are drawn at
    /// their native size, everything else is fitted inside the canvas.
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
        }
    }

    /// The opacity, clamped between 0 and 1.
    pub fn clamped_opacity(&self) -> f32 {
        self.opacity.clamp(0.0, 1.0)
    }
}

/// A picture to draw, and how.
#[derive(Debug, Clone, Copy)]
pub struct VisualLayer<'picture> {
    /// The source picture (YUV or RGBA).
    pub picture: &'picture VideoFrame,
    /// Where and how to draw it.
    pub placement: LayerPlacement,
}

/// The mapping between canvas pixels and source pixels for one layer.
///
/// The visible (cropped) part of the picture is fitted inside the canvas, scaled,
/// rotated clockwise around its centre, then moved.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LayerGeometry {
    /// Visible part of the source picture, in source pixels.
    pub(crate) crop_left: f32,
    pub(crate) crop_top: f32,
    pub(crate) crop_right: f32,
    pub(crate) crop_bottom: f32,
    /// Centre of the visible part, in source pixels.
    pub(crate) source_center_x: f32,
    pub(crate) source_center_y: f32,
    /// Where that centre lands on the canvas.
    pub(crate) canvas_center_x: f32,
    pub(crate) canvas_center_y: f32,
    /// Source pixels per canvas pixel.
    pub(crate) source_pixels_per_canvas_pixel: f32,
    /// Displayed size of the visible part, in canvas pixels.
    pub(crate) displayed_width: f32,
    pub(crate) displayed_height: f32,
    pub(crate) rotation_cosine: f32,
    pub(crate) rotation_sine: f32,
}

impl LayerGeometry {
    /// `None` when the layer cannot show anything (empty picture, zero scale, crop
    /// removing everything).
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
        if visible_width < 1.0 || visible_height < 1.0 || placement.scale_factor <= 0.0 {
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
        })
    }

    /// Whether canvas pixel (x, y) shows source pixel (x, y) exactly: same size,
    /// no crop, no move, no rotation, no scaling.
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

    /// Positions in the source picture (0–1) of the same corners.
    pub(crate) fn texture_corners(
        &self,
        picture_width: u32,
        picture_height: u32,
    ) -> [(f32, f32); 4] {
        let (width, height) = (picture_width as f32, picture_height as f32);
        let (left, right) = (self.crop_left / width, self.crop_right / width);
        let (top, bottom) = (self.crop_top / height, self.crop_bottom / height);
        [(left, top), (right, top), (left, bottom), (right, bottom)]
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
    /// it falls outside the visible part of the picture.
    pub(crate) fn source_point_for(&self, canvas_x: u32, canvas_y: u32) -> Option<(f32, f32)> {
        let from_center_x = canvas_x as f32 + 0.5 - self.canvas_center_x;
        let from_center_y = canvas_y as f32 + 0.5 - self.canvas_center_y;
        // Undo the clockwise rotation (y points down), then the scaling.
        let unrotated_x = from_center_x * self.rotation_cosine + from_center_y * self.rotation_sine;
        let unrotated_y =
            -from_center_x * self.rotation_sine + from_center_y * self.rotation_cosine;
        let source_x = self.source_center_x + unrotated_x * self.source_pixels_per_canvas_pixel;
        let source_y = self.source_center_y + unrotated_y * self.source_pixels_per_canvas_pixel;
        let is_inside = source_x >= self.crop_left
            && source_x < self.crop_right
            && source_y >= self.crop_top
            && source_y < self.crop_bottom;
        is_inside.then_some((source_x, source_y))
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
            geometry.texture_corners(400, 200),
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
