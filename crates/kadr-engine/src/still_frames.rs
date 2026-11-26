//! Single pictures outside an export: what a file, a clip or the whole timeline
//! shows at a given moment, and contact sheets of thumbnails.
//!
//! These are the pictures Claude looks at through the MCP tools `get_frame` and
//! `get_contact_sheet`, and that an interface can show as a still preview.
//!
//! - [`media_pictures_at`]: the raw pictures of a file, at source times;
//! - [`clip_pictures_at`]: the source picture a clip shows at timeline times, with
//!   its tracked blurs applied but **not** placed on the canvas, so positions in
//!   it are source pixels (the coordinates tracked blurs use);
//! - [`timeline_pictures_at`]: the finished picture of the timeline, exactly as
//!   an export draws it;
//! - [`build_contact_sheet`]: thumbnails in a grid, each labelled with its time.
//!
//! Every picture is returned as 8-bit RGBA at its full size; scaling it down for
//! display is left to the caller.

use std::path::Path;

use image::imageops::{self, FilterType};
use image::{Rgba, RgbaImage};
use kadr_core::clip::{ClipContent, TextAlignment, TextStyle};
use kadr_core::color::Color;
use kadr_core::error::EditError;
use kadr_core::ids::{ClipId, MediaAssetId};
use kadr_core::media_asset::MediaKind;
use kadr_core::project::Project;
use kadr_core::time::Timestamp;
use kadr_media::{MediaBackend, MediaError};
use kadr_render::{
    BUNDLED_FONT_FAMILY, RenderError, RendererPreference, TextRasterizer, create_renderer,
    hide_regions,
};

use crate::export::ExportError;
use crate::export::frame_composer::{FrameComposer, hidden_regions_of};

/// Why a picture could not be made.
#[derive(Debug, thiserror::Error)]
pub enum StillFrameError {
    /// The clip does not exist.
    #[error(transparent)]
    Edit(#[from] EditError),
    /// A file could not be read.
    #[error(transparent)]
    Media(#[from] MediaError),
    /// The picture could not be drawn.
    #[error(transparent)]
    Render(#[from] RenderError),
    /// Composing the timeline picture failed.
    #[error(transparent)]
    Composition(ExportError),
    /// The requested time is not inside the clip.
    #[error(
        "{requested_time} is not inside clip {clip_id}, which runs from {clip_start} to {clip_end} on the timeline"
    )]
    TimeOutsideClip {
        /// The clip.
        clip_id: ClipId,
        /// The rejected timeline time.
        requested_time: Timestamp,
        /// Where the clip starts.
        clip_start: Timestamp,
        /// Where the clip ends (excluded).
        clip_end: Timestamp,
    },
    /// The clip only plays sound.
    #[error("clip {clip_id} shows no picture: it only plays sound")]
    ClipHasNoPicture {
        /// The clip.
        clip_id: ClipId,
    },
    /// The media file of the clip is not in the project.
    #[error("clip {clip_id} shows media {media_asset_id}, which is not in the project")]
    MissingMedia {
        /// The clip.
        clip_id: ClipId,
        /// The missing media.
        media_asset_id: MediaAssetId,
    },
}

impl From<ExportError> for StillFrameError {
    fn from(error: ExportError) -> Self {
        match error {
            ExportError::Media(media_error) => Self::Media(media_error),
            ExportError::Render(render_error) => Self::Render(render_error),
            other => Self::Composition(other),
        }
    }
}

/// The pictures a video or image file shows at each of `source_times`
/// (measured from the start of the file). An image shows the same picture at
/// every time.
///
/// # Errors
///
/// Fails when the file cannot be opened or decoded.
pub fn media_pictures_at(
    backend: &dyn MediaBackend,
    file_path: &Path,
    source_times: &[Timestamp],
) -> Result<Vec<RgbaImage>, StillFrameError> {
    let mut source = backend.open_video_source(file_path)?;
    let mut pictures = Vec::with_capacity(source_times.len());
    for &source_time in source_times {
        pictures.push(source.frame_at(source_time)?.to_rgba().into_owned());
    }
    Ok(pictures)
}

/// The source picture a clip shows at one timeline time.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipPicture {
    /// The picture, at the source's own size, tracked blurs applied.
    pub picture: RgbaImage,
    /// For media clips: the time in the source file this picture comes from.
    pub source_time: Option<Timestamp>,
}

/// The source pictures clip `clip_id` shows at each of `timeline_times`.
///
/// Media clips give their source picture (tracked blurs applied, crop, position
/// and colour adjustments not applied); text clips give their rasterised text.
///
/// # Errors
///
/// Fails when the clip does not exist, a time is outside the clip, the clip
/// shows no picture, or the file cannot be decoded.
pub fn clip_pictures_at(
    project: &Project,
    backend: &dyn MediaBackend,
    clip_id: ClipId,
    timeline_times: &[Timestamp],
) -> Result<Vec<ClipPicture>, StillFrameError> {
    let (_, clip) =
        project
            .timeline()
            .find_clip(clip_id)
            .ok_or_else(|| EditError::ClipNotFound {
                clip_id,
                existing_clip_ids: project.timeline().clip_ids(),
            })?;
    for &timeline_time in timeline_times {
        if !clip.contains_timeline_time(timeline_time) {
            return Err(StillFrameError::TimeOutsideClip {
                clip_id,
                requested_time: timeline_time,
                clip_start: clip.timeline_start,
                clip_end: clip.timeline_end(),
            });
        }
    }
    let segment = match &clip.content {
        ClipContent::Text(text_content) => {
            let picture = TextRasterizer::new().rasterize(&text_content.text, &text_content.style);
            let pictures = timeline_times.iter().map(|_| ClipPicture {
                picture: picture.clone(),
                source_time: None,
            });
            return Ok(pictures.collect());
        }
        ClipContent::Media(segment) => segment,
    };
    let media_asset =
        project
            .media_asset(segment.media_asset_id)
            .ok_or(StillFrameError::MissingMedia {
                clip_id,
                media_asset_id: segment.media_asset_id,
            })?;
    if !media_asset.description.has_picture() {
        return Err(StillFrameError::ClipHasNoPicture { clip_id });
    }
    let is_still_image = media_asset.description.kind == MediaKind::Image;
    let mut source = backend.open_video_source(&media_asset.description.file_path)?;
    let mut pictures = Vec::with_capacity(timeline_times.len());
    for &timeline_time in timeline_times {
        let source_time = match (is_still_image, clip.source_time_at(timeline_time)) {
            (true, _) | (false, None) => Timestamp::ZERO,
            (false, Some(source_time)) => source_time,
        };
        let frame = source.frame_at(source_time)?;
        let hidden_regions = hidden_regions_of(clip, source_time);
        let picture = if hidden_regions.is_empty() {
            frame.to_rgba().into_owned()
        } else {
            hide_regions(&frame, &hidden_regions).to_rgba().into_owned()
        };
        pictures.push(ClipPicture {
            picture,
            source_time: (!is_still_image).then_some(source_time),
        });
    }
    Ok(pictures)
}

/// The finished pictures of the timeline at each of `timeline_times`, exactly as
/// an export draws them, at the project's canvas size.
///
/// Single pictures are usually drawn fastest by the CPU renderer
/// ([`RendererPreference::CpuOnly`]): starting a GPU device takes longer than
/// drawing a few frames.
///
/// # Errors
///
/// Fails when a source file cannot be decoded or a picture cannot be drawn.
pub fn timeline_pictures_at(
    project: &Project,
    backend: &dyn MediaBackend,
    renderer_preference: RendererPreference,
    timeline_times: &[Timestamp],
) -> Result<Vec<RgbaImage>, StillFrameError> {
    let mut composer = FrameComposer::new(project, backend, create_renderer(renderer_preference)?);
    let mut sorted_times: Vec<(usize, Timestamp)> =
        timeline_times.iter().copied().enumerate().collect();
    // The composer reads sources forward fastest; give the times in order.
    sorted_times.sort_by_key(|(_, time)| *time);
    let mut pictures: Vec<Option<RgbaImage>> = vec![None; timeline_times.len()];
    for (original_index, timeline_time) in sorted_times {
        let frame = composer.compose_frame_at(timeline_time)?;
        pictures[original_index] = Some(frame.to_rgba().into_owned());
    }
    Ok(pictures.into_iter().flatten().collect())
}

/// One thumbnail of a contact sheet.
#[derive(Debug, Clone, PartialEq)]
pub struct ContactSheetCell {
    /// The picture, at any size: it is scaled to fit its cell.
    pub picture: RgbaImage,
    /// The text written in the corner of the thumbnail, for example `"12.5 s"`.
    pub label: String,
}

/// Empty space between thumbnails and around the sheet, in pixels.
const CONTACT_SHEET_GAP_IN_PIXELS: u32 = 4;
/// Colour behind and between the thumbnails.
const CONTACT_SHEET_BACKGROUND: Rgba<u8> = Rgba([24, 24, 24, 255]);
/// Colour of the box behind each label, so it stays readable on any picture.
const LABEL_BACKGROUND: Rgba<u8> = Rgba([0, 0, 0, 190]);
/// Label font size as a fraction of the thumbnail height.
const LABEL_SIZE_FRACTION_OF_CELL_HEIGHT: f32 = 0.11;
/// Smallest and largest label font size, in pixels.
const LABEL_FONT_SIZE_RANGE_IN_PIXELS: (f32, f32) = (11.0, 22.0);

/// The thumbnail grid chosen for a contact sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContactSheetLayout {
    /// Thumbnails per row.
    pub column_count: u32,
    /// Rows of thumbnails.
    pub row_count: u32,
    /// Width of one thumbnail, in pixels.
    pub cell_width_in_pixels: u32,
    /// Height of one thumbnail, in pixels.
    pub cell_height_in_pixels: u32,
}

impl ContactSheetLayout {
    /// The grid giving the largest thumbnails for `cell_count` pictures of
    /// `picture_width` × `picture_height` on a sheet of at most
    /// `maximum_sheet_side_in_pixels` on each side.
    pub fn choose(
        cell_count: u32,
        picture_width: u32,
        picture_height: u32,
        maximum_sheet_side_in_pixels: u32,
    ) -> Self {
        let cell_count = cell_count.max(1);
        let aspect_ratio = f64::from(picture_width.max(1)) / f64::from(picture_height.max(1));
        let mut best =
            Self::with_columns(1, cell_count, aspect_ratio, maximum_sheet_side_in_pixels);
        for column_count in 2..=cell_count {
            let candidate = Self::with_columns(
                column_count,
                cell_count,
                aspect_ratio,
                maximum_sheet_side_in_pixels,
            );
            if candidate.cell_area() > best.cell_area() {
                best = candidate;
            }
        }
        best
    }

    fn with_columns(
        column_count: u32,
        cell_count: u32,
        aspect_ratio: f64,
        maximum_sheet_side_in_pixels: u32,
    ) -> Self {
        let row_count = cell_count.div_ceil(column_count);
        let usable_width = available_length(maximum_sheet_side_in_pixels, column_count);
        let usable_height = available_length(maximum_sheet_side_in_pixels, row_count);
        let width_from_columns = usable_width / f64::from(column_count);
        let width_from_rows = usable_height / f64::from(row_count) * aspect_ratio;
        let cell_width = width_from_columns.min(width_from_rows).floor().max(1.0);
        let cell_height = (cell_width / aspect_ratio).floor().max(1.0);
        Self {
            column_count,
            row_count,
            cell_width_in_pixels: cell_width as u32,
            cell_height_in_pixels: cell_height as u32,
        }
    }

    fn cell_area(&self) -> u64 {
        u64::from(self.cell_width_in_pixels) * u64::from(self.cell_height_in_pixels)
    }

    /// Width of the whole sheet, gaps included.
    pub fn sheet_width_in_pixels(&self) -> u32 {
        self.column_count * self.cell_width_in_pixels
            + (self.column_count + 1) * CONTACT_SHEET_GAP_IN_PIXELS
    }

    /// Height of the whole sheet, gaps included.
    pub fn sheet_height_in_pixels(&self) -> u32 {
        self.row_count * self.cell_height_in_pixels
            + (self.row_count + 1) * CONTACT_SHEET_GAP_IN_PIXELS
    }
}

/// Room left for `cell_count` cells along a side once the gaps are taken out.
fn available_length(maximum_side_in_pixels: u32, cell_count: u32) -> f64 {
    let gaps = (cell_count + 1) * CONTACT_SHEET_GAP_IN_PIXELS;
    f64::from(maximum_side_in_pixels.saturating_sub(gaps).max(cell_count))
}

/// Draws `cells` in a grid, left to right then top to bottom, each scaled to
/// the same thumbnail size (from the first picture's proportions) and labelled
/// in its bottom-left corner. The sheet is at most `maximum_sheet_side_in_pixels`
/// on each side.
///
/// Returns the sheet and its layout. An empty list gives a sheet with one empty cell.
pub fn build_contact_sheet(
    cells: &[ContactSheetCell],
    maximum_sheet_side_in_pixels: u32,
) -> (RgbaImage, ContactSheetLayout) {
    let (picture_width, picture_height) = cells
        .first()
        .map(|cell| (cell.picture.width(), cell.picture.height()))
        .unwrap_or((16, 9));
    let layout = ContactSheetLayout::choose(
        u32::try_from(cells.len()).unwrap_or(u32::MAX),
        picture_width,
        picture_height,
        maximum_sheet_side_in_pixels,
    );
    let mut sheet = RgbaImage::from_pixel(
        layout.sheet_width_in_pixels(),
        layout.sheet_height_in_pixels(),
        CONTACT_SHEET_BACKGROUND,
    );
    let mut rasterizer = TextRasterizer::new();
    let label_style = label_style_for(layout.cell_height_in_pixels);
    for (index, cell) in cells.iter().enumerate() {
        let index = u32::try_from(index).unwrap_or(u32::MAX);
        let column = index % layout.column_count;
        let row = index / layout.column_count;
        let left = CONTACT_SHEET_GAP_IN_PIXELS
            + column * (layout.cell_width_in_pixels + CONTACT_SHEET_GAP_IN_PIXELS);
        let top = CONTACT_SHEET_GAP_IN_PIXELS
            + row * (layout.cell_height_in_pixels + CONTACT_SHEET_GAP_IN_PIXELS);
        let thumbnail = imageops::resize(
            &cell.picture,
            layout.cell_width_in_pixels,
            layout.cell_height_in_pixels,
            FilterType::Triangle,
        );
        imageops::overlay(&mut sheet, &thumbnail, i64::from(left), i64::from(top));
        let label_picture = rasterizer.rasterize(&cell.label, &label_style);
        let label_top = (top + layout.cell_height_in_pixels).saturating_sub(label_picture.height());
        draw_label(&mut sheet, &label_picture, left, label_top);
    }
    (sheet, layout)
}

/// A small white label, sized for thumbnails `cell_height_in_pixels` high.
fn label_style_for(cell_height_in_pixels: u32) -> TextStyle {
    let (smallest, largest) = LABEL_FONT_SIZE_RANGE_IN_PIXELS;
    let font_size = (cell_height_in_pixels as f32 * LABEL_SIZE_FRACTION_OF_CELL_HEIGHT)
        .clamp(smallest, largest);
    TextStyle {
        font_family: BUNDLED_FONT_FAMILY.to_string(),
        font_size_in_pixels: font_size,
        color: Color::WHITE,
        alignment: TextAlignment::Left,
    }
}

/// Draws the label on a dark box, at `left`, `top` on the sheet.
fn draw_label(sheet: &mut RgbaImage, label_picture: &RgbaImage, left: u32, top: u32) {
    let label_box = RgbaImage::from_pixel(
        label_picture.width(),
        label_picture.height(),
        LABEL_BACKGROUND,
    );
    imageops::overlay(sheet, &label_box, i64::from(left), i64::from(top));
    imageops::overlay(sheet, label_picture, i64::from(left), i64::from(top));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_giving_the_largest_landscape_thumbnails_is_chosen() {
        // In a 1024 × 1024 box, 12 thumbnails of 16:9 are largest in 3 columns of
        // 4 rows (336 × 189 each), larger than 4 columns of 3 rows (251 × 141).
        let layout = ContactSheetLayout::choose(12, 1920, 1080, 1024);
        assert_eq!((layout.column_count, layout.row_count), (3, 4));
        assert_eq!(
            (layout.cell_width_in_pixels, layout.cell_height_in_pixels),
            (336, 189)
        );
        assert!(layout.sheet_width_in_pixels() <= 1024);
        assert!(layout.sheet_height_in_pixels() <= 1024);
    }

    #[test]
    fn vertical_thumbnails_go_in_more_columns_than_rows() {
        let layout = ContactSheetLayout::choose(12, 1080, 1920, 1024);
        assert_eq!((layout.column_count, layout.row_count), (4, 3));
        assert!(layout.sheet_width_in_pixels() <= 1024);
        assert!(layout.sheet_height_in_pixels() <= 1024);
        // The thumbnails keep the picture's proportions (to the pixel).
        let ratio =
            f64::from(layout.cell_width_in_pixels) / f64::from(layout.cell_height_in_pixels);
        assert!((ratio - 1080.0 / 1920.0).abs() < 0.02);
    }

    #[test]
    fn a_contact_sheet_places_every_thumbnail_and_its_label() {
        let red = RgbaImage::from_pixel(320, 180, Rgba([255, 0, 0, 255]));
        let cells: Vec<ContactSheetCell> = (0..6)
            .map(|index| ContactSheetCell {
                picture: red.clone(),
                label: format!("{index}.0 s"),
            })
            .collect();
        let (sheet, layout) = build_contact_sheet(&cells, 800);
        assert_eq!(sheet.width(), layout.sheet_width_in_pixels());
        assert!(sheet.width() <= 800 && sheet.height() <= 800);

        // The top right corner of the last thumbnail is red (labels sit bottom left).
        let last_column = 5 % layout.column_count;
        let last_row = 5 / layout.column_count;
        let x = CONTACT_SHEET_GAP_IN_PIXELS
            + last_column * (layout.cell_width_in_pixels + CONTACT_SHEET_GAP_IN_PIXELS)
            + layout.cell_width_in_pixels
            - 2;
        let y = CONTACT_SHEET_GAP_IN_PIXELS
            + last_row * (layout.cell_height_in_pixels + CONTACT_SHEET_GAP_IN_PIXELS)
            + 2;
        assert_eq!(sheet.get_pixel(x, y), &Rgba([255, 0, 0, 255]));

        // The bottom left corner of the first thumbnail is darkened by its label box.
        let label_pixel = sheet.get_pixel(
            CONTACT_SHEET_GAP_IN_PIXELS + 1,
            CONTACT_SHEET_GAP_IN_PIXELS + layout.cell_height_in_pixels - 2,
        );
        assert!(label_pixel.0[0] < 200, "label box missing: {label_pixel:?}");
    }
}
