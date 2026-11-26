//! Describing a media file on Android: kind, duration, picture size, frame
//! rate, sound format.

use std::path::Path;

use kadr_core::media_asset::{AudioFormat, MediaAssetDescription, MediaKind, PictureSize};
use kadr_core::time::{FrameRate, Timestamp};
use ndk::media::media_format::MediaFormat;

use super::codec_support::{
    KEY_CHANNEL_COUNT, KEY_DURATION, KEY_FRAME_RATE, KEY_HEIGHT, KEY_SAMPLE_RATE, KEY_WIDTH,
    mime_type_of,
};
use super::extractor::MediaExtractor;
use super::frame_rate::{FRAMES_MEASURED_FOR_FRAME_RATE, measure_frame_rate};
use super::media_time::timestamp_from_microseconds;
use super::{display_name_of, still_image};
use crate::error::MediaError;

/// The kind of a track, from the start of its codec type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrackKind {
    /// `video/…`.
    Video,
    /// `audio/…`.
    Audio,
}

/// The first track of `kind` in the file, with its index and format.
pub(crate) fn find_track(
    extractor: &MediaExtractor,
    kind: TrackKind,
) -> Option<(usize, MediaFormat)> {
    let prefix = match kind {
        TrackKind::Video => "video/",
        TrackKind::Audio => "audio/",
    };
    (0..extractor.track_count()).find_map(|track_index| {
        let mut format = extractor.track_format(track_index)?;
        let is_wanted_kind =
            mime_type_of(&mut format).is_some_and(|mime_type| mime_type.starts_with(prefix));
        is_wanted_kind.then_some((track_index, format))
    })
}

/// Describes the file at `file_path`.
pub(crate) fn probe_media_file(file_path: &Path) -> Result<MediaAssetDescription, MediaError> {
    if still_image::picture_format_of(file_path).is_some() {
        return still_image::probe_picture(file_path);
    }
    let extractor = MediaExtractor::open(file_path)?;
    let video_track = find_track(&extractor, TrackKind::Video);
    let audio_track = find_track(&extractor, TrackKind::Audio);
    let kind = match (&video_track, &audio_track) {
        (Some(_), _) => MediaKind::Video,
        (None, Some(_)) => MediaKind::Audio,
        (None, None) => {
            return Err(MediaError::NoUsableStream {
                file_path: file_path.to_path_buf(),
            });
        }
    };

    let picture_size = video_track
        .as_ref()
        .map(|(_, format)| read_picture_size(format));
    let frame_rate = match &video_track {
        Some((track_index, format)) => read_frame_rate(file_path, *track_index, format)?,
        None => None,
    };
    let audio_format = audio_track
        .as_ref()
        .map(|(_, format)| read_audio_format(format));
    let duration = [&video_track, &audio_track]
        .into_iter()
        .flatten()
        .filter_map(|(_, format)| format.i64(KEY_DURATION))
        .max()
        .map(timestamp_from_microseconds)
        .and_then(|end| end.span_since(Timestamp::ZERO))
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| MediaError::DecodingFailed {
            file_path: file_path.to_path_buf(),
            reason: "reading the duration: the file reports no duration".to_string(),
        })?;

    Ok(MediaAssetDescription {
        file_path: file_path.to_path_buf(),
        display_name: display_name_of(file_path),
        kind,
        duration: Some(duration),
        picture_size,
        frame_rate,
        audio_format,
    })
}

/// The picture size the file announces (the displayed size, without padding).
fn read_picture_size(format: &MediaFormat) -> PictureSize {
    let dimension = |key: &str| {
        format
            .i32(key)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0)
    };
    PictureSize {
        width_in_pixels: dimension(KEY_WIDTH),
        height_in_pixels: dimension(KEY_HEIGHT),
    }
}

/// The frame rate, measured from the first frames' times (Android only announces
/// a whole number).
fn read_frame_rate(
    file_path: &Path,
    track_index: usize,
    format: &MediaFormat,
) -> Result<Option<FrameRate>, MediaError> {
    let announced = format
        .i32(KEY_FRAME_RATE)
        .map(f64::from)
        .or_else(|| format.f32(KEY_FRAME_RATE).map(f64::from));
    let mut extractor = MediaExtractor::open(file_path)?;
    extractor
        .select_track(track_index)
        .map_err(|reason| MediaError::DecodingFailed {
            file_path: file_path.to_path_buf(),
            reason,
        })?;
    let mut frame_starts = Vec::with_capacity(FRAMES_MEASURED_FOR_FRAME_RATE);
    while frame_starts.len() < FRAMES_MEASURED_FOR_FRAME_RATE {
        let Some(frame_start) = extractor.sample_time_in_microseconds() else {
            break;
        };
        frame_starts.push(frame_start);
        if !extractor.advance() {
            break;
        }
    }
    Ok(measure_frame_rate(&frame_starts, announced))
}

/// The sound's sample rate and channel count.
fn read_audio_format(format: &MediaFormat) -> AudioFormat {
    AudioFormat {
        sample_rate_in_hertz: format
            .i32(KEY_SAMPLE_RATE)
            .and_then(|rate| u32::try_from(rate).ok())
            .unwrap_or(0),
        channel_count: format
            .i32(KEY_CHANNEL_COUNT)
            .and_then(|count| u16::try_from(count).ok())
            .unwrap_or(2)
            .max(1),
    }
}
