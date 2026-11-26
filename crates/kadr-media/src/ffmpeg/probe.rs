//! Describing a media file: kind, duration, picture size, frame rate, sound.

use std::path::Path;

use ffmpeg_next as ffmpeg;
use kadr_core::media_asset::{AudioFormat, MediaAssetDescription, MediaKind, PictureSize};
use kadr_core::time::{FrameRate, TimeSpan, Timestamp};

use super::time_conversion::timestamp_from_stream_units;
use super::{decoding_failure, find_picture_stream, is_still_image_format, open_input_file};
use crate::error::MediaError;

/// FFmpeg's container durations are in microseconds.
const CONTAINER_TIME_BASE: (i32, i32) = (1, 1_000_000);

/// Describes the file at `file_path`.
pub(crate) fn probe_media_file(file_path: &Path) -> Result<MediaAssetDescription, MediaError> {
    let input = open_input_file(file_path)?;
    let picture_stream = find_picture_stream(&input);
    let audio_stream = input.streams().best(ffmpeg::media::Type::Audio);

    let kind = match (&picture_stream, &audio_stream) {
        (Some(_), _) if is_still_image_format(&input) => MediaKind::Image,
        (Some(_), _) => MediaKind::Video,
        (None, Some(_)) => MediaKind::Audio,
        (None, None) => {
            return Err(MediaError::NoUsableStream {
                file_path: file_path.to_path_buf(),
            });
        }
    };

    let picture_size = match &picture_stream {
        Some(stream) => Some(read_picture_size(file_path, stream)?),
        None => None,
    };
    let frame_rate = match (&picture_stream, kind) {
        (Some(stream), MediaKind::Video) => read_frame_rate(stream),
        _ => None,
    };
    let audio_format = match (&audio_stream, kind) {
        (Some(stream), MediaKind::Video | MediaKind::Audio) => {
            Some(read_audio_format(file_path, stream)?)
        }
        _ => None,
    };
    let duration = match kind {
        MediaKind::Image => None,
        MediaKind::Video | MediaKind::Audio => Some(read_duration(&input, file_path)?),
    };

    Ok(MediaAssetDescription {
        file_path: file_path.to_path_buf(),
        display_name: display_name_of(file_path),
        kind,
        duration,
        picture_size,
        frame_rate,
        audio_format,
    })
}

/// The file name, used as the name shown in the media library.
fn display_name_of(file_path: &Path) -> String {
    file_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| file_path.to_string_lossy().into_owned())
}

fn read_picture_size(
    file_path: &Path,
    stream: &ffmpeg::format::stream::Stream<'_>,
) -> Result<PictureSize, MediaError> {
    let decoder = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
        .and_then(|context| context.decoder().video())
        .map_err(|error| decoding_failure(file_path, "reading the picture size", error))?;
    Ok(PictureSize {
        width_in_pixels: decoder.width(),
        height_in_pixels: decoder.height(),
    })
}

/// Highest believable nominal frame rate. Some files report a nominal rate equal
/// to their clock (for example 90000 fps); the average rate is used instead.
const MAXIMUM_PLAUSIBLE_FRAMES_PER_SECOND: f64 = 240.0;

/// The nominal frame rate, or the average one; `None` if the file gives neither.
///
/// The nominal rate comes first: the average is computed from the duration and
/// is slightly off for short files (3600/119 instead of 30/1 for 4 seconds).
fn read_frame_rate(stream: &ffmpeg::format::stream::Stream<'_>) -> Option<FrameRate> {
    [stream.rate(), stream.avg_frame_rate()]
        .into_iter()
        .filter_map(|rate| {
            let numerator = u32::try_from(rate.numerator()).ok()?;
            let denominator = u32::try_from(rate.denominator()).ok()?;
            FrameRate::new(numerator, denominator).ok()
        })
        .find(|frame_rate| frame_rate.as_frames_per_second() <= MAXIMUM_PLAUSIBLE_FRAMES_PER_SECOND)
}

fn read_audio_format(
    file_path: &Path,
    stream: &ffmpeg::format::stream::Stream<'_>,
) -> Result<AudioFormat, MediaError> {
    let decoder = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
        .and_then(|context| context.decoder().audio())
        .map_err(|error| decoding_failure(file_path, "reading the audio format", error))?;
    let channel_count = u16::try_from(decoder.channel_layout().channels())
        .unwrap_or(2)
        .max(1);
    Ok(AudioFormat {
        sample_rate_in_hertz: decoder.rate(),
        channel_count,
    })
}

/// The duration of the whole file, from the container or, failing that, from
/// the longest stream.
fn read_duration(
    input: &ffmpeg::format::context::Input,
    file_path: &Path,
) -> Result<TimeSpan, MediaError> {
    let container_duration =
        timestamp_from_stream_units(input.duration(), CONTAINER_TIME_BASE.into());
    let longest_stream_duration = input
        .streams()
        .map(|stream| timestamp_from_stream_units(stream.duration(), stream.time_base()))
        .max()
        .unwrap_or(Timestamp::ZERO);
    let duration = container_duration.max(longest_stream_duration);
    duration
        .span_since(Timestamp::ZERO)
        .filter(|span| !span.is_zero())
        .ok_or_else(|| {
            decoding_failure(
                file_path,
                "reading the duration",
                "the file reports no duration",
            )
        })
}
