//! Recognising the region by its appearance: to find it again when it is lost,
//! and to keep the tracker from drifting away from the selection.
//!
//! The tracker keeps small pictures of the region: an [`AppearanceTemplate`] of
//! the user's selection, and one taken recently while tracking was sure. To
//! compare one with a frame, it slides the picture over an area and measures the
//! normalised cross-correlation (NCC) at every offset:
//!
//! `NCC = Σ (T − T̄)(P − P̄) / (‖T − T̄‖ · ‖P − P̄‖)`
//!
//! where `T` is the template and `P` the patch of the frame under it. NCC goes from
//! −1 to 1 and ignores brightness and contrast changes: 1 means the same picture.
//!
//! - **Lost region** ([`AppearanceTemplate::search`]): when the correlation filter
//!   loses the region (a hand in front of the face, a person walking past), both
//!   pictures are searched over a wide area around the last known position. The
//!   best offset is only a candidate: the tracker confirms it with the
//!   correlation filter before saying the region is found.
//! - **Anchor** ([`AppearanceTemplate::best_match_near`]): on every frame the
//!   filter is sure of, the selection's picture is searched a few cells around
//!   the answer, at three sizes, and refined between cells. The filter keeps
//!   learning the region's appearance, so its small errors add up; the
//!   selection's picture never changes, so it cannot drift.
//!
//! Reference: J. P. Lewis, "Fast Normalized Cross-Correlation", Vision Interface 1995.
//!
//! The template is coarse on purpose ([`TEMPLATE_LONGER_SIDE_IN_CELLS`] cells
//! along the longer side): it tolerates small changes of pose, and searching a
//! wide area stays cheap.

use crate::correlation_filter::parabolic_peak_offset;
use crate::geometry::{Point, Region};
use crate::grey_image::{GreyImage, ImagePyramid};

/// Cells along the template's longer side.
const TEMPLATE_LONGER_SIDE_IN_CELLS: usize = 16;

/// Fewest cells along the template's shorter side (for very elongated regions).
const TEMPLATE_MINIMUM_SIDE_IN_CELLS: usize = 4;

/// Largest search radius, in cells: three times the template's longer side.
const MAXIMUM_SEARCH_RADIUS_IN_CELLS: usize = 3 * TEMPLATE_LONGER_SIDE_IN_CELLS;

/// Below this spread of brightness, a patch is uniform and cannot match anything.
const MINIMUM_PATCH_SPREAD: f32 = 1.0;

/// A coarse picture of the region, zero-mean and of unit norm.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AppearanceTemplate {
    width_in_cells: usize,
    height_in_cells: usize,
    values: Vec<f32>,
}

/// The best place found for a template.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AppearanceMatch {
    /// Where the region's centre would be.
    pub(crate) center: Point,
    /// Normalised cross-correlation, from −1 to 1.
    pub(crate) similarity: f32,
}

/// The best place found for a template close to a region.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct NearbyMatch {
    /// The region, moved and resized to where the template matches best.
    pub(crate) region: Region,
    /// Normalised cross-correlation there, from −1 to 1.
    pub(crate) similarity: f32,
}

/// The similarities of a template at every offset of a square around a region.
#[derive(Debug, Clone)]
struct SimilarityGrid {
    /// Offsets along each side (an odd number: the region's centre is the middle).
    side: usize,
    /// Row after row; −1 where the patch is uniform.
    similarities: Vec<f32>,
    best_index: usize,
    best_similarity: f32,
}

impl SimilarityGrid {
    /// The best offset from the middle, in cells, refined between cells.
    fn refined_best_offset_in_cells(&self) -> (f32, f32) {
        let (row, column) = (self.best_index / self.side, self.best_index % self.side);
        let at = |row: usize, column: usize| self.similarities[row * self.side + column];
        let refined = |position: usize, before: Option<f32>, after: Option<f32>| {
            let offset = match (before, after) {
                (Some(before), Some(after)) => {
                    parabolic_peak_offset(before, self.best_similarity, after)
                }
                _ => 0.0,
            };
            position as f32 + offset - (self.side / 2) as f32
        };
        let last = self.side - 1;
        (
            refined(
                column,
                (column > 0).then(|| at(row, column - 1)),
                (column < last).then(|| at(row, column + 1)),
            ),
            refined(
                row,
                (row > 0).then(|| at(row - 1, column)),
                (row < last).then(|| at(row + 1, column)),
            ),
        )
    }
}

/// The scale factor between the best one and its neighbours, from a parabola
/// through their best similarities (on a logarithmic scale, since scale steps
/// multiply).
fn refined_scale_factor(
    per_scale: &[Option<SimilarityGrid>],
    scale_factors: &[f32],
    best_index: usize,
) -> f32 {
    let best = scale_factors[best_index];
    let similarity_of = |index: usize| {
        per_scale
            .get(index)
            .and_then(Option::as_ref)
            .map(|grid| grid.best_similarity)
    };
    let Some(before_index) = best_index.checked_sub(1) else {
        return best;
    };
    let after_index = best_index + 1;
    let (Some(before), Some(peak), Some(after)) = (
        similarity_of(before_index),
        similarity_of(best_index),
        similarity_of(after_index),
    ) else {
        return best;
    };
    // Equal steps on a logarithmic scale: interpolate the exponent.
    let step = (scale_factors[after_index] / best).ln();
    best * (parabolic_peak_offset(before, peak, after) * step).exp()
}

/// The cells of a template laid over a region: their number and size.
#[derive(Debug, Clone, Copy)]
struct CellGrid {
    width_in_cells: usize,
    height_in_cells: usize,
    cell_size_in_pixels: f32,
}

impl CellGrid {
    fn covering(region: &Region) -> Self {
        let cell_size_in_pixels = region.longer_side() / TEMPLATE_LONGER_SIDE_IN_CELLS as f32;
        let cells_along = |side: f32| {
            ((side / cell_size_in_pixels).round() as usize).max(TEMPLATE_MINIMUM_SIDE_IN_CELLS)
        };
        Self {
            width_in_cells: cells_along(region.width),
            height_in_cells: cells_along(region.height),
            cell_size_in_pixels,
        }
    }
}

impl AppearanceTemplate {
    /// Takes a picture of `region`.
    ///
    /// Returns `None` if the region is uniform.
    pub(crate) fn capture(pyramid: &ImagePyramid, region: &Region) -> Option<Self> {
        let grid = CellGrid::covering(region);
        let mut values = sample_cells(
            pyramid,
            grid.cell_size_in_pixels,
            Point {
                x: region.center_x - grid.width_in_cells as f32 * grid.cell_size_in_pixels / 2.0,
                y: region.center_y - grid.height_in_cells as f32 * grid.cell_size_in_pixels / 2.0,
            },
            grid.width_in_cells,
            grid.height_in_cells,
        );
        let count = values.len() as f32;
        let mean = values.iter().sum::<f32>() / count;
        values.iter_mut().for_each(|value| *value -= mean);
        let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
        if norm < MINIMUM_PATCH_SPREAD {
            return None;
        }
        values.iter_mut().for_each(|value| *value /= norm);
        Some(Self {
            width_in_cells: grid.width_in_cells,
            height_in_cells: grid.height_in_cells,
            values,
        })
    }

    /// Looks for the template within `search_radius_in_pixels` of `region`'s
    /// centre, assuming the region still has `region`'s size.
    ///
    /// Returns `None` if every place searched is uniform.
    pub(crate) fn search(
        &self,
        pyramid: &ImagePyramid,
        region: &Region,
        search_radius_in_pixels: f32,
    ) -> Option<AppearanceMatch> {
        let cell_size_in_pixels = region.longer_side() / TEMPLATE_LONGER_SIDE_IN_CELLS as f32;
        let radius_in_cells = ((search_radius_in_pixels / cell_size_in_pixels).ceil() as usize)
            .min(MAXIMUM_SEARCH_RADIUS_IN_CELLS);
        let area_width = 2 * radius_in_cells + self.width_in_cells;
        let area_height = 2 * radius_in_cells + self.height_in_cells;
        let area = sample_cells(
            pyramid,
            cell_size_in_pixels,
            Point {
                x: region.center_x
                    - (radius_in_cells as f32 + self.width_in_cells as f32 / 2.0)
                        * cell_size_in_pixels,
                y: region.center_y
                    - (radius_in_cells as f32 + self.height_in_cells as f32 / 2.0)
                        * cell_size_in_pixels,
            },
            area_width,
            area_height,
        );
        let mut best: Option<AppearanceMatch> = None;
        for offset_row in 0..=2 * radius_in_cells {
            for offset_column in 0..=2 * radius_in_cells {
                let Some(similarity) =
                    self.similarity_at(&area, area_width, offset_column, offset_row)
                else {
                    continue;
                };
                if best.is_none_or(|best| similarity > best.similarity) {
                    best = Some(AppearanceMatch {
                        center: Point {
                            x: region.center_x
                                + (offset_column as f32 - radius_in_cells as f32)
                                    * cell_size_in_pixels,
                            y: region.center_y
                                + (offset_row as f32 - radius_in_cells as f32)
                                    * cell_size_in_pixels,
                        },
                        similarity,
                    });
                }
            }
        }
        best
    }

    /// Finds where the template matches best close to `region`: within
    /// `radius_in_cells` template cells of its centre, at its size multiplied by
    /// each of `scale_factors`. The position and the size are refined to a
    /// fraction of a cell and of a scale step (a parabola through the best answer
    /// and its neighbours), so that small corrections are not rounded away.
    ///
    /// Returns `None` if every place searched is uniform.
    pub(crate) fn best_match_near(
        &self,
        pyramid: &ImagePyramid,
        region: &Region,
        radius_in_cells: usize,
        scale_factors: &[f32],
    ) -> Option<NearbyMatch> {
        let per_scale: Vec<Option<SimilarityGrid>> = scale_factors
            .iter()
            .map(|&scale_factor| {
                self.similarity_grid(pyramid, &region.scaled_by(scale_factor), radius_in_cells)
            })
            .collect();
        let (best_scale_index, best_grid) = per_scale
            .iter()
            .enumerate()
            .filter_map(|(index, grid)| grid.as_ref().map(|grid| (index, grid)))
            .max_by(|first, second| first.1.best_similarity.total_cmp(&second.1.best_similarity))?;
        let (offset_column, offset_row) = best_grid.refined_best_offset_in_cells();
        let cell_size_in_pixels = region
            .scaled_by(scale_factors[best_scale_index])
            .longer_side()
            / TEMPLATE_LONGER_SIDE_IN_CELLS as f32;
        let scale_factor = refined_scale_factor(&per_scale, scale_factors, best_scale_index);
        Some(NearbyMatch {
            region: region
                .moved_by(
                    offset_column * cell_size_in_pixels,
                    offset_row * cell_size_in_pixels,
                )
                .scaled_by(scale_factor),
            similarity: best_grid.best_similarity,
        })
    }

    /// The similarity at every offset of up to `radius_in_cells` cells around
    /// `region`, at `region`'s size.
    fn similarity_grid(
        &self,
        pyramid: &ImagePyramid,
        region: &Region,
        radius_in_cells: usize,
    ) -> Option<SimilarityGrid> {
        let cell_size_in_pixels = region.longer_side() / TEMPLATE_LONGER_SIDE_IN_CELLS as f32;
        let area_width = 2 * radius_in_cells + self.width_in_cells;
        let area_height = 2 * radius_in_cells + self.height_in_cells;
        let area = sample_cells(
            pyramid,
            cell_size_in_pixels,
            Point {
                x: region.center_x
                    - (radius_in_cells as f32 + self.width_in_cells as f32 / 2.0)
                        * cell_size_in_pixels,
                y: region.center_y
                    - (radius_in_cells as f32 + self.height_in_cells as f32 / 2.0)
                        * cell_size_in_pixels,
            },
            area_width,
            area_height,
        );
        let side = 2 * radius_in_cells + 1;
        let similarities: Vec<f32> = (0..side * side)
            .map(|index| {
                self.similarity_at(&area, area_width, index % side, index / side)
                    .unwrap_or(-1.0)
            })
            .collect();
        let (best_index, best_similarity) = similarities
            .iter()
            .copied()
            .enumerate()
            .max_by(|first, second| first.1.total_cmp(&second.1))?;
        (best_similarity > -1.0).then_some(SimilarityGrid {
            side,
            similarities,
            best_index,
            best_similarity,
        })
    }

    /// NCC between the template and the patch of `area` whose top-left cell is
    /// at (`offset_column`, `offset_row`). `None` if the patch is uniform.
    fn similarity_at(
        &self,
        area: &[f32],
        area_width: usize,
        offset_column: usize,
        offset_row: usize,
    ) -> Option<f32> {
        let (mut dot_product, mut sum, mut sum_of_squares) = (0.0_f32, 0.0_f32, 0.0_f32);
        for row in 0..self.height_in_cells {
            let template_row =
                &self.values[row * self.width_in_cells..(row + 1) * self.width_in_cells];
            let area_start = (offset_row + row) * area_width + offset_column;
            let area_row = &area[area_start..area_start + self.width_in_cells];
            for (&template_value, &area_value) in template_row.iter().zip(area_row) {
                dot_product += template_value * area_value;
                sum += area_value;
                sum_of_squares += area_value * area_value;
            }
        }
        // The template has a mean of 0, so Σ T·(P − P̄) = Σ T·P.
        let spread = (sum_of_squares - sum * sum / self.values.len() as f32)
            .max(0.0)
            .sqrt();
        (spread >= MINIMUM_PATCH_SPREAD).then(|| dot_product / spread)
    }
}

/// Samples `width × height` cell centres, row after row, starting from the
/// top-left corner `top_left`, from the pyramid level that suits the cell size.
fn sample_cells(
    pyramid: &ImagePyramid,
    cell_size_in_pixels: f32,
    top_left: Point,
    width_in_cells: usize,
    height_in_cells: usize,
) -> Vec<f32> {
    let level = pyramid.level_for_step(cell_size_in_pixels);
    let (image, level_scale): (&GreyImage, f32) =
        (pyramid.level(level), pyramid.scale_of_level(level));
    let mut values = Vec::with_capacity(width_in_cells * height_in_cells);
    for row in 0..height_in_cells {
        let y = top_left.y + (row as f32 + 0.5) * cell_size_in_pixels;
        for column in 0..width_in_cells {
            let x = top_left.x + (column as f32 + 0.5) * cell_size_in_pixels;
            values.push(image.sample(x / level_scale, y / level_scale));
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic_scenes::{SceneFrame, SceneObject};

    fn scene_pyramid(object_left: f32, object_top: f32) -> ImagePyramid {
        pyramid_with_object(Region::from_top_left(object_left, object_top, 40.0, 48.0))
    }

    fn pyramid_with_object(object: Region) -> ImagePyramid {
        let frame = SceneFrame {
            width: 320,
            height: 240,
            background_seed: 11,
            background_offset_in_pixels: 0.0,
            object: Some(SceneObject {
                region: object,
                texture_seed: 5,
            }),
            obstacle: None,
        };
        ImagePyramid::new(frame.render(), 16)
    }

    #[test]
    fn a_small_shift_and_change_of_size_are_measured_between_cells() {
        let selection = Region::from_top_left(100.0, 80.0, 40.0, 48.0);
        let template =
            AppearanceTemplate::capture(&pyramid_with_object(selection), &selection).unwrap();
        // A cell is 3 pixels here: the object moved by a fraction of a cell and grew by 3 %.
        let moved = Region {
            center_x: selection.center_x + 4.4,
            center_y: selection.center_y - 2.3,
            ..selection.scaled_by(1.03)
        };
        let found = template
            .best_match_near(
                &pyramid_with_object(moved),
                &selection,
                3,
                &[1.0 / 1.04, 1.0, 1.04],
            )
            .unwrap();
        // The texture's details are about two cells wide: cells that do not line
        // up with them lower the similarity, not the accuracy.
        assert!(found.similarity > 0.75, "{found:?}");
        assert!(found.region.center_distance_to(&moved) < 0.5, "{found:?}");
        assert!(
            (found.region.width / moved.width - 1.0).abs() < 0.015,
            "{found:?}"
        );
    }

    #[test]
    fn where_nothing_moved_the_region_is_kept() {
        let selection = Region::from_top_left(100.0, 80.0, 40.0, 48.0);
        let pyramid = pyramid_with_object(selection);
        let template = AppearanceTemplate::capture(&pyramid, &selection).unwrap();
        let found = template
            .best_match_near(&pyramid, &selection, 3, &[1.0 / 1.04, 1.0, 1.04])
            .unwrap();
        assert!(found.similarity > 0.99, "{found:?}");
        assert!(
            found.region.center_distance_to(&selection) < 0.3,
            "{found:?}"
        );
        assert!(
            (found.region.width / selection.width - 1.0).abs() < 0.01,
            "{found:?}"
        );
    }

    #[test]
    fn the_template_is_found_where_the_object_went() {
        let region = Region::from_top_left(100.0, 80.0, 40.0, 48.0);
        let template = AppearanceTemplate::capture(&scene_pyramid(100.0, 80.0), &region).unwrap();
        let found = template
            .search(&scene_pyramid(160.0, 60.0), &region, 100.0)
            .unwrap();
        let expected = Region::from_top_left(160.0, 60.0, 40.0, 48.0);
        // The template's corners show the background, which changed: not a perfect 1.
        assert!(found.similarity > 0.75, "{found:?}");
        assert!(
            (found.center.x - expected.center_x).hypot(found.center.y - expected.center_y) < 3.0,
            "{found:?}"
        );
    }

    #[test]
    fn a_uniform_region_has_no_template() {
        let pyramid = ImagePyramid::new(GreyImage::from_fn(100, 100, |_, _| 90.0), 16);
        let region = Region::from_top_left(30.0, 30.0, 20.0, 20.0);
        assert!(AppearanceTemplate::capture(&pyramid, &region).is_none());
    }
}
