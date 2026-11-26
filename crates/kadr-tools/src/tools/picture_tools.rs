//! Picture tools: let Claude see a frame, or thumbnails across a whole video.

use image::RgbaImage;
use kadr_core::clip::ClipContent;
use kadr_core::ids::{ClipId, MediaAssetId};
use kadr_core::media_asset::{MediaAsset, MediaKind};
use kadr_core::project::Project;
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_engine::RendererPreference;
use kadr_engine::still_frames::{
    ContactSheetCell, build_contact_sheet, clip_pictures_at, media_pictures_at,
    timeline_pictures_at,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{ToolEffect, ToolEntry, declare_tool};
use crate::output::{ToolError, ToolOutput};
use crate::pictures_for_claude::{MAXIMUM_PICTURE_SIDE_IN_PIXELS, PictureForClaude};
use crate::time_input::{seconds_in, seconds_of, timestamp_from_seconds};
use crate::toolbox::ToolCall;

/// The picture tools.
pub(crate) fn picture_tools() -> Vec<ToolEntry> {
    vec![
        declare_tool(
            "get_frame",
            "Look at one frame",
            GET_FRAME_DESCRIPTION,
            ToolEffect::ReadsOnly,
            get_frame,
        ),
        declare_tool(
            "get_contact_sheet",
            "Look at thumbnails across a video",
            GET_CONTACT_SHEET_DESCRIPTION,
            ToolEffect::ReadsOnly,
            get_contact_sheet,
        ),
    ]
}

/// Single frames are drawn on the CPU: starting the GPU takes longer than one frame.
const STILL_FRAME_RENDERER: RendererPreference = RendererPreference::CpuOnly;

// ----- get_frame -----

const GET_FRAME_DESCRIPTION: &str = "\
Returns a picture so you can see the video. Three uses:
- without clip_id and media_id: the finished picture of the timeline at time_in_seconds, \
exactly as the export will look (texts, placement, effects). Use it to check an edit.
- with clip_id: the source picture that clip shows at time_in_seconds ON THE TIMELINE, before it \
is placed on the canvas (tracked blurs applied). Positions on it are pixels of the source video, \
origin at the top-left corner, x to the right, y down: the coordinates a blur rectangle uses.
- with media_id: the picture of an imported file at time_in_seconds INTO THE FILE.
Pictures larger than 1024 pixels are scaled down; the answer gives the original size and the \
factor to convert what you measure back to original pixels.

Examples: {\"time_in_seconds\": 2.5}; {\"clip_id\": 4, \"time_in_seconds\": 10}; {\"media_id\": \
1, \"time_in_seconds\": 0}";

/// Parameters of `get_frame`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetFrameParameters {
    /// The moment, in seconds: on the timeline, or into the file with media_id.
    time_in_seconds: f64,
    /// Show this clip's source picture (time on the timeline, inside the clip).
    clip_id: Option<u64>,
    /// Show this imported file's picture (time into the file).
    media_id: Option<u64>,
}

fn get_frame(
    toolbox: &mut ToolCall<'_>,
    parameters: GetFrameParameters,
) -> Result<ToolOutput, ToolError> {
    let time = timestamp_from_seconds("time_in_seconds", parameters.time_in_seconds)?;
    let project = toolbox.require_project()?;
    let (picture, description) = match (parameters.clip_id, parameters.media_id) {
        (Some(_), Some(_)) => {
            return Err(ToolError::new(
                "give clip_id or media_id, not both (or neither for the finished timeline picture)",
            ));
        }
        (Some(raw_clip_id), None) => {
            clip_frame(toolbox, project, ClipId::from_raw(raw_clip_id), time)?
        }
        (None, Some(raw_media_id)) => {
            media_frame(toolbox, project, MediaAssetId::from_raw(raw_media_id), time)?
        }
        (None, None) => timeline_frame(toolbox, project, time)?,
    };
    let prepared = PictureForClaude::prepare(&picture)?;
    let text = format!("{description} {}", prepared.describe_size());
    Ok(ToolOutput::text(text).with_jpeg_image(prepared.jpeg_data))
}

fn timeline_frame(
    toolbox: &ToolCall<'_>,
    project: &Project,
    time: Timestamp,
) -> Result<(RgbaImage, String), ToolError> {
    let timeline_end = Timestamp::ZERO + project.timeline().duration();
    if time >= timeline_end {
        return Err(ToolError::new(format!(
            "the timeline lasts {} s: ask for a time from 0 to below that",
            seconds_of(timeline_end)
        )));
    }
    ensure_project_media_readable(toolbox, project)?;
    let pictures = timeline_pictures_at(project, toolbox.backend(), STILL_FRAME_RENDERER, &[time])?;
    let picture = pictures.into_iter().next().ok_or_else(no_picture)?;
    let description = format!(
        "The finished picture of the timeline at {} s, as the export will show it. Positions are \
         canvas pixels, origin at the top-left corner.",
        seconds_of(time)
    );
    Ok((picture, description))
}

fn clip_frame(
    toolbox: &ToolCall<'_>,
    project: &Project,
    clip_id: ClipId,
    time: Timestamp,
) -> Result<(RgbaImage, String), ToolError> {
    let (_, clip) = project.require_clip(clip_id)?;
    if let ClipContent::Media(segment) = &clip.content {
        ensure_media_readable(
            toolbox,
            project.require_media_asset(segment.media_asset_id)?,
        )?;
    }
    let clip_picture = clip_pictures_at(project, toolbox.backend(), clip_id, &[time])?
        .into_iter()
        .next()
        .ok_or_else(no_picture)?;
    let source = match (&clip.content, clip_picture.source_time) {
        (ClipContent::Media(segment), Some(source_time)) => format!(
            " ({} s into media {})",
            seconds_of(source_time),
            segment.media_asset_id
        ),
        _ => String::new(),
    };
    let description = format!(
        "The source picture clip {clip_id} shows at {} s on the timeline{source}, with its blurs, \
         before it is placed on the canvas. Positions on it are source pixels, origin at the \
         top-left corner (the coordinates of a blur rectangle).",
        seconds_of(time)
    );
    Ok((clip_picture.picture, description))
}

fn media_frame(
    toolbox: &ToolCall<'_>,
    project: &Project,
    media_id: MediaAssetId,
    time: Timestamp,
) -> Result<(RgbaImage, String), ToolError> {
    let media_asset = project.require_media_asset(media_id)?;
    let description = &media_asset.description;
    if !description.has_picture() {
        return Err(ToolError::new(format!(
            "media {media_id} (\"{}\") is a sound file: it has no picture",
            description.display_name
        )));
    }
    if let Some(duration) = description.duration
        && description.kind == MediaKind::Video
        && time >= Timestamp::ZERO + duration
    {
        return Err(ToolError::new(format!(
            "media {media_id} lasts {} s: ask for a time from 0 to below that",
            seconds_in(duration)
        )));
    }
    ensure_media_readable(toolbox, media_asset)?;
    let picture = media_pictures_at(toolbox.backend(), &description.file_path, &[time])?
        .into_iter()
        .next()
        .ok_or_else(no_picture)?;
    let text = format!(
        "The picture of media {media_id} (\"{}\") at {} s into the file. Positions are pixels of \
         the file's picture, origin at the top-left corner.",
        description.display_name,
        seconds_of(time)
    );
    Ok((picture, text))
}

fn no_picture() -> ToolError {
    ToolError::new("no picture was produced")
}

/// Refuses to read a media file outside the allowed folders (a project file can
/// name any path).
fn ensure_media_readable(
    toolbox: &ToolCall<'_>,
    media_asset: &MediaAsset,
) -> Result<(), ToolError> {
    toolbox
        .allowed_folders()
        .check_existing_file(&media_asset.description.file_path)
        .map(|_| ())
        .map_err(|error| error.in_context(format!("media {}", media_asset.id)))
}

/// [`ensure_media_readable`] for every media file the timeline uses.
pub(crate) fn ensure_project_media_readable(
    toolbox: &ToolCall<'_>,
    project: &Project,
) -> Result<(), ToolError> {
    for media_asset in project.media_assets() {
        if !project.clips_using_media_asset(media_asset.id).is_empty() {
            ensure_media_readable(toolbox, media_asset)?;
        }
    }
    Ok(())
}

// ----- get_contact_sheet -----

const GET_CONTACT_SHEET_DESCRIPTION: &str = "\
Returns one picture made of evenly spaced thumbnails, each labelled with its time in seconds, \
to see a whole video at a glance (find scenes, people, where something happens) before looking \
closer with get_frame. With media_id: across an imported file (times into the file). With \
clip_id: across that clip's source pictures (times on the timeline). With neither: across the \
finished timeline. The thumbnails read left to right, then top to bottom.

Example: {\"media_id\": 1, \"count\": 16}";

/// Parameters of `get_contact_sheet`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetContactSheetParameters {
    /// Thumbnails across this imported file.
    media_id: Option<u64>,
    /// Thumbnails across this clip.
    clip_id: Option<u64>,
    /// Number of thumbnails, from 2 to 48. Default 12.
    count: Option<u32>,
}

const DEFAULT_THUMBNAIL_COUNT: u32 = 12;
const THUMBNAIL_COUNT_RANGE: (u32, u32) = (2, 48);

#[derive(Debug, Serialize)]
struct ContactSheetView {
    columns: u32,
    rows: u32,
    thumbnail_times_in_seconds: Vec<f64>,
    original_width_in_pixels: u32,
    original_height_in_pixels: u32,
    thumbnail_width_in_pixels: u32,
    thumbnail_height_in_pixels: u32,
}

fn get_contact_sheet(
    toolbox: &mut ToolCall<'_>,
    parameters: GetContactSheetParameters,
) -> Result<ToolOutput, ToolError> {
    let (smallest, largest) = THUMBNAIL_COUNT_RANGE;
    let count = parameters.count.unwrap_or(DEFAULT_THUMBNAIL_COUNT);
    if !(smallest..=largest).contains(&count) {
        return Err(ToolError::new(format!(
            "count must be from {smallest} to {largest}"
        )));
    }
    let project = toolbox.require_project()?;
    let (pictures, times, subject) = match (parameters.clip_id, parameters.media_id) {
        (Some(_), Some(_)) => return Err(ToolError::new("give clip_id or media_id, not both")),
        (Some(raw_clip_id), None) => {
            clip_thumbnails(toolbox, project, ClipId::from_raw(raw_clip_id), count)?
        }
        (None, Some(raw_media_id)) => media_thumbnails(
            toolbox,
            project,
            MediaAssetId::from_raw(raw_media_id),
            count,
        )?,
        (None, None) => timeline_thumbnails(toolbox, project, count)?,
    };
    let (original_width, original_height) = pictures.first().map_or((0, 0), RgbaImage::dimensions);
    let cells: Vec<ContactSheetCell> = pictures
        .into_iter()
        .zip(&times)
        .map(|(picture, time)| ContactSheetCell {
            picture,
            label: format!("{:.2} s", seconds_of(*time)),
        })
        .collect();
    let (sheet, layout) = build_contact_sheet(&cells, MAXIMUM_PICTURE_SIDE_IN_PIXELS);
    let prepared = PictureForClaude::prepare(&sheet)?;
    let view = ContactSheetView {
        columns: layout.column_count,
        rows: layout.row_count,
        thumbnail_times_in_seconds: times.iter().map(|time| seconds_of(*time)).collect(),
        original_width_in_pixels: original_width,
        original_height_in_pixels: original_height,
        thumbnail_width_in_pixels: layout.cell_width_in_pixels,
        thumbnail_height_in_pixels: layout.cell_height_in_pixels,
    };
    let message = format!(
        "Contact sheet of {subject}: {} thumbnails in {} columns × {} rows, left to right then top \
         to bottom, each labelled with its time. The pictures are {original_width}×{original_height} \
         pixels, shown {}×{}: use get_frame to see one at full detail.",
        times.len(),
        layout.column_count,
        layout.row_count,
        layout.cell_width_in_pixels,
        layout.cell_height_in_pixels
    );
    Ok(ToolOutput::message_with_details(message, &view).with_jpeg_image(prepared.jpeg_data))
}

/// `count` moments spread evenly over `start`..`start + length`, each in the
/// middle of its share (so neither the very first nor the very last frame,
/// which are often black).
fn evenly_spaced_times(start: Timestamp, length: TimeSpan, count: u32) -> Vec<Timestamp> {
    let length_ticks = u128::from(length.ticks());
    let count = u128::from(count.max(1));
    (0..count)
        .map(|index| {
            let offset = length_ticks * (2 * index + 1) / (2 * count);
            Timestamp::from_ticks(start.ticks() + u64::try_from(offset).unwrap_or(u64::MAX))
        })
        .collect()
}

type Thumbnails = (Vec<RgbaImage>, Vec<Timestamp>, String);

fn timeline_thumbnails(
    toolbox: &ToolCall<'_>,
    project: &Project,
    count: u32,
) -> Result<Thumbnails, ToolError> {
    let duration = project.timeline().duration();
    if duration.is_zero() {
        return Err(ToolError::new("the timeline is empty: add clips first"));
    }
    ensure_project_media_readable(toolbox, project)?;
    let times = evenly_spaced_times(Timestamp::ZERO, duration, count);
    let pictures = timeline_pictures_at(project, toolbox.backend(), STILL_FRAME_RENDERER, &times)?;
    Ok((
        pictures,
        times,
        "the finished timeline (times on the timeline)".to_string(),
    ))
}

fn clip_thumbnails(
    toolbox: &ToolCall<'_>,
    project: &Project,
    clip_id: ClipId,
    count: u32,
) -> Result<Thumbnails, ToolError> {
    let (_, clip) = project.require_clip(clip_id)?;
    if let ClipContent::Media(segment) = &clip.content {
        ensure_media_readable(
            toolbox,
            project.require_media_asset(segment.media_asset_id)?,
        )?;
    }
    let times = evenly_spaced_times(clip.timeline_start, clip.timeline_duration(), count);
    let pictures = clip_pictures_at(project, toolbox.backend(), clip_id, &times)?
        .into_iter()
        .map(|clip_picture| clip_picture.picture)
        .collect();
    Ok((
        pictures,
        times,
        format!("clip {clip_id} (times on the timeline)"),
    ))
}

fn media_thumbnails(
    toolbox: &ToolCall<'_>,
    project: &Project,
    media_id: MediaAssetId,
    count: u32,
) -> Result<Thumbnails, ToolError> {
    let media_asset = project.require_media_asset(media_id)?;
    let description = &media_asset.description;
    let Some(duration) = description
        .duration
        .filter(|_| description.kind == MediaKind::Video)
    else {
        return Err(ToolError::new(format!(
            "media {media_id} (\"{}\") is not a video: use get_frame for an image",
            description.display_name
        )));
    };
    ensure_media_readable(toolbox, media_asset)?;
    let times = evenly_spaced_times(Timestamp::ZERO, duration, count);
    let pictures = media_pictures_at(toolbox.backend(), &description.file_path, &times)?;
    Ok((
        pictures,
        times,
        format!(
            "media {media_id} \"{}\" (times into the file)",
            description.display_name
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_times_are_centred_in_equal_shares() {
        let times = evenly_spaced_times(Timestamp::ZERO, TimeSpan::from_whole_seconds(4), 4);
        let seconds: Vec<f64> = times.iter().map(|time| seconds_of(*time)).collect();
        assert_eq!(seconds, vec![0.5, 1.5, 2.5, 3.5]);

        let offset = evenly_spaced_times(
            Timestamp::from_whole_seconds(10),
            TimeSpan::from_whole_seconds(2),
            2,
        );
        assert_eq!(seconds_of(offset[0]), 10.5);
    }
}
