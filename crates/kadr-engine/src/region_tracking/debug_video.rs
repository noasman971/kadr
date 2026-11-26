//! A video showing what the tracker saw, to check an analysis by eye.
//!
//! Every analysed frame is drawn with the tracked box and a label: the state
//! and the peak-to-sidelobe ratio. Colours: blue for the user's selection,
//! green when confident, amber when uncertain, red when lost.

use std::fs;
use std::path::{Path, PathBuf};

use image::{Rgba, RgbaImage};
use kadr_core::clip::{TextAlignment, TextStyle};
use kadr_core::color::Color;
use kadr_core::effect::{PixelRegion, RegionSampleOrigin, TrackingConfidence};
use kadr_media::{
    EncodingSpeed, ExportSinkSettings, MediaBackend, VideoEncoderPreference, VideoFrame,
};
use kadr_render::TextRasterizer;

use super::{
    ProgressCounter, RegionTrackingError, RegionTrackingReport, RegionTrackingRequest,
    TrackedFrame, TrackingCancellation, TrackingProgress,
};

/// Sound sample rate declared for the debug video (it has no sound).
const DEBUG_VIDEO_AUDIO_SAMPLE_RATE_IN_HERTZ: u32 = 48_000;

const SELECTION_COLOR: Rgba<u8> = Rgba([59, 130, 246, 255]);
const CONFIDENT_COLOR: Rgba<u8> = Rgba([34, 197, 94, 255]);
const UNCERTAIN_COLOR: Rgba<u8> = Rgba([245, 158, 11, 255]);
const LOST_COLOR: Rgba<u8> = Rgba([239, 68, 68, 255]);

/// The label's font size, as a fraction of the picture's height (30 pixels in 1080p).
const LABEL_FONT_SIZE_FRACTION: f32 = 1.0 / 36.0;

/// The box outline's thickness, as a fraction of the picture's height (3 pixels in 1080p).
const OUTLINE_THICKNESS_FRACTION: f32 = 1.0 / 360.0;

/// Where to write the debug video, and how.
#[derive(Debug, Clone, PartialEq)]
pub struct DebugVideoRequest {
    /// The MP4 file to create. Replaced only if the debug video is complete.
    pub output_path: PathBuf,
    /// Hardware encoder when available, or always software.
    pub video_encoder_preference: VideoEncoderPreference,
    /// Speed / quality trade-off.
    pub encoding_speed: EncodingSpeed,
}

/// Writes the analysed part of the video with the tracked box drawn on every
/// frame. The video is written to a temporary file next to the requested one,
/// which is only replaced when the debug video is complete.
///
/// # Errors
///
/// Reading the video or writing the debug video failed, or it was cancelled
/// ([`RegionTrackingError::DebugVideoCancelled`]).
pub fn export_tracking_debug_video(
    backend: &dyn MediaBackend,
    tracking: &RegionTrackingRequest,
    report: &RegionTrackingReport,
    request: &DebugVideoRequest,
    on_progress: &mut dyn FnMut(TrackingProgress),
    cancellation: &TrackingCancellation,
) -> Result<(), RegionTrackingError> {
    let mut temporary_name = request.output_path.as_os_str().to_owned();
    temporary_name.push(".drawing");
    let temporary_path = PathBuf::from(temporary_name);
    let written = write_debug_video(
        backend,
        tracking,
        report,
        request,
        &temporary_path,
        on_progress,
        cancellation,
    );
    let placed = written.and_then(|()| {
        fs::rename(&temporary_path, &request.output_path).map_err(|error| {
            RegionTrackingError::CannotPlaceDebugVideo {
                output_path: request.output_path.clone(),
                reason: error.to_string(),
            }
        })
    });
    if placed.is_err() {
        // Best effort: the partial file is useless.
        let _ = fs::remove_file(&temporary_path);
    }
    placed.map_err(|error| match error {
        RegionTrackingError::Cancelled => RegionTrackingError::DebugVideoCancelled,
        other => other,
    })
}

fn write_debug_video(
    backend: &dyn MediaBackend,
    tracking: &RegionTrackingRequest,
    report: &RegionTrackingReport,
    request: &DebugVideoRequest,
    temporary_path: &Path,
    on_progress: &mut dyn FnMut(TrackingProgress),
    cancellation: &TrackingCancellation,
) -> Result<(), RegionTrackingError> {
    // Encoders need even sizes: an odd last column or row is dropped.
    let width = tracking.picture_size.width_in_pixels & !1;
    let height = tracking.picture_size.height_in_pixels & !1;
    let mut source = backend.open_video_source(&tracking.video_path)?;
    let mut sink = backend.create_export_sink(&ExportSinkSettings {
        output_path: temporary_path.to_path_buf(),
        width_in_pixels: width,
        height_in_pixels: height,
        frame_rate: tracking.frame_rate,
        audio_sample_rate_in_hertz: DEBUG_VIDEO_AUDIO_SAMPLE_RATE_IN_HERTZ,
        video_encoder_preference: request.video_encoder_preference,
        encoding_speed: request.encoding_speed,
    })?;
    let mut text_rasterizer = TextRasterizer::new();
    let mut counter = ProgressCounter {
        progress: TrackingProgress {
            analysed_frame_count: 0,
            total_frame_count: report.frames.len() as u64,
        },
        on_progress,
        cancellation,
    };
    for frame in &report.frames {
        counter.check_cancellation()?;
        let picture = source.frame_at(frame.source_time)?;
        let mut image = picture.to_rgba().into_owned();
        if (image.width(), image.height()) != (width, height) {
            image = image::imageops::crop_imm(&image, 0, 0, width, height).to_image();
        }
        draw_tracked_frame(&mut image, frame, &mut text_rasterizer);
        sink.write_video_frame(&VideoFrame::Rgba(image))?;
        counter.count_one_frame();
    }
    sink.finish()?;
    Ok(())
}

/// Draws the box and the label of one frame.
fn draw_tracked_frame(
    image: &mut RgbaImage,
    frame: &TrackedFrame,
    text_rasterizer: &mut TextRasterizer,
) {
    let color = color_of(frame);
    let picture_height = image.height() as f32;
    let thickness = (picture_height * OUTLINE_THICKNESS_FRACTION)
        .round()
        .max(2.0) as u32;
    draw_outline(image, &frame.region, thickness, color);
    let label = text_rasterizer.rasterize(
        &label_of(frame),
        &TextStyle {
            font_family: "Inter".to_string(),
            font_size_in_pixels: (picture_height * LABEL_FONT_SIZE_FRACTION).max(12.0),
            color: Color::WHITE,
            alignment: TextAlignment::Left,
        },
    );
    // Above the box, or inside it when the box touches the top of the picture.
    // Kept inside the picture horizontally (a label wider than the picture
    // starts at its left edge).
    let label_width = i64::from(label.width());
    let left = (frame.region.left_in_pixels.max(0.0) as i64)
        .min(i64::from(image.width()) - label_width)
        .max(0);
    let above = frame.region.top_in_pixels as i64 - i64::from(label.height());
    let top = if above >= 0 {
        above
    } else {
        frame.region.top_in_pixels.max(0.0) as i64
    };
    fill_rectangle(
        image,
        left,
        top,
        i64::from(label.width()),
        i64::from(label.height()),
        color,
    );
    blend_over(image, &label, left, top);
}

fn color_of(frame: &TrackedFrame) -> Rgba<u8> {
    if frame.origin == RegionSampleOrigin::UserSelection {
        return SELECTION_COLOR;
    }
    match frame.confidence {
        TrackingConfidence::Confident => CONFIDENT_COLOR,
        TrackingConfidence::Uncertain => UNCERTAIN_COLOR,
        TrackingConfidence::Lost => LOST_COLOR,
    }
}

fn label_of(frame: &TrackedFrame) -> String {
    let state = match (frame.origin, frame.confidence) {
        (RegionSampleOrigin::UserSelection, _) => return "selection".to_string(),
        (_, TrackingConfidence::Confident) => "confident",
        (_, TrackingConfidence::Uncertain) => "uncertain",
        (_, TrackingConfidence::Lost) => "lost",
    };
    match frame.peak_to_sidelobe_ratio {
        Some(ratio) => format!("{state} · PSR {ratio:.1}"),
        None => state.to_string(),
    }
}

/// Draws the four sides of `region`, `thickness` pixels wide, inside it.
fn draw_outline(image: &mut RgbaImage, region: &PixelRegion, thickness: u32, color: Rgba<u8>) {
    let left = region.left_in_pixels.round() as i64;
    let top = region.top_in_pixels.round() as i64;
    let width = region.width_in_pixels.round() as i64;
    let height = region.height_in_pixels.round() as i64;
    let thickness = i64::from(thickness);
    fill_rectangle(image, left, top, width, thickness, color);
    fill_rectangle(
        image,
        left,
        top + height - thickness,
        width,
        thickness,
        color,
    );
    fill_rectangle(image, left, top, thickness, height, color);
    fill_rectangle(
        image,
        left + width - thickness,
        top,
        thickness,
        height,
        color,
    );
}

/// Fills a rectangle, clipped to the picture.
fn fill_rectangle(
    image: &mut RgbaImage,
    left: i64,
    top: i64,
    width: i64,
    height: i64,
    color: Rgba<u8>,
) {
    let (picture_width, picture_height) = (i64::from(image.width()), i64::from(image.height()));
    for y in top.max(0)..(top + height).min(picture_height) {
        for x in left.max(0)..(left + width).min(picture_width) {
            image.put_pixel(x as u32, y as u32, color);
        }
    }
}

/// Draws `overlay` over `image` with its transparency, its top-left corner at
/// (`left`, `top`), clipped to the picture.
fn blend_over(image: &mut RgbaImage, overlay: &RgbaImage, left: i64, top: i64) {
    for (x, y, pixel) in overlay.enumerate_pixels() {
        let (target_x, target_y) = (left + i64::from(x), top + i64::from(y));
        if target_x < 0
            || target_y < 0
            || target_x >= i64::from(image.width())
            || target_y >= i64::from(image.height())
        {
            continue;
        }
        let alpha = f32::from(pixel.0[3]) / 255.0;
        let target = image.get_pixel_mut(target_x as u32, target_y as u32);
        for channel in 0..3 {
            let mixed =
                f32::from(pixel.0[channel]) * alpha + f32::from(target.0[channel]) * (1.0 - alpha);
            target.0[channel] = mixed.round() as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kadr_core::time::Timestamp;

    fn tracked_frame(confidence: TrackingConfidence, origin: RegionSampleOrigin) -> TrackedFrame {
        TrackedFrame {
            source_time: Timestamp::ZERO,
            region: PixelRegion {
                left_in_pixels: 20.0,
                top_in_pixels: 30.0,
                width_in_pixels: 40.0,
                height_in_pixels: 50.0,
            },
            confidence,
            origin,
            peak_to_sidelobe_ratio: Some(23.42),
        }
    }

    #[test]
    fn labels_say_the_state_and_the_ratio() {
        let confident = tracked_frame(TrackingConfidence::Confident, RegionSampleOrigin::Tracker);
        assert_eq!(label_of(&confident), "confident · PSR 23.4");
        let selection = tracked_frame(
            TrackingConfidence::Confident,
            RegionSampleOrigin::UserSelection,
        );
        assert_eq!(label_of(&selection), "selection");
        assert_eq!(color_of(&selection), SELECTION_COLOR);
        assert_eq!(
            color_of(&tracked_frame(
                TrackingConfidence::Lost,
                RegionSampleOrigin::Tracker
            )),
            LOST_COLOR
        );
    }

    #[test]
    fn the_outline_is_drawn_inside_the_box_and_clipped() {
        let mut image = RgbaImage::from_pixel(100, 100, Rgba([0, 0, 0, 255]));
        let frame = tracked_frame(TrackingConfidence::Uncertain, RegionSampleOrigin::Tracker);
        draw_outline(&mut image, &frame.region, 2, UNCERTAIN_COLOR);
        assert_eq!(*image.get_pixel(20, 30), UNCERTAIN_COLOR, "top-left corner");
        assert_eq!(
            *image.get_pixel(59, 79),
            UNCERTAIN_COLOR,
            "bottom-right corner"
        );
        assert_eq!(
            *image.get_pixel(40, 55),
            Rgba([0, 0, 0, 255]),
            "inside stays untouched"
        );
        // Partly outside the picture: no panic.
        fill_rectangle(&mut image, 90, -5, 30, 10, LOST_COLOR);
        assert_eq!(*image.get_pixel(99, 0), LOST_COLOR);
    }
}
