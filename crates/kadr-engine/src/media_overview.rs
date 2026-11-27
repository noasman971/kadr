//! Small summaries of media files for an interface: thumbnails and waveforms.
//!
//! - [`ThumbnailMaker`] gives the picture of a video or image file at any time,
//!   shrunk to a thumbnail. It keeps a few decoders open, so a filmstrip (many
//!   thumbnails of the same file, in time order) does not reopen the file each time.
//!   Given [`ReadyProxies`], it reads heavy videos from their proxies, which give
//!   the same pictures, smaller and quicker (decision record 0055).
//! - [`compute_waveform_peaks`] decodes a file's sound once, at a low sample rate,
//!   and keeps the loudest value of each short slice: enough to draw a waveform at
//!   any zoom, for a few kilobytes per minute.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::RgbaImage;
use kadr_core::clip::PlaybackSpeed;
use kadr_core::media_asset::{MediaAssetDescription, PictureSize};
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_media::audio_buffer::{sample_index_at, span_of_sample_count};
use kadr_media::{AudioSegmentRequest, MediaBackend, MediaError, VideoFrameSource};

use crate::picture_scaling::shrink_frame_to_rgba;
use crate::proxies::ReadyProxies;

/// Files whose decoder a [`ThumbnailMaker`] keeps open at once.
///
/// A timeline rarely shows more than a handful of different files at a time;
/// each open decoder holds a few frames of memory.
const MAXIMUM_OPEN_THUMBNAIL_SOURCES: usize = 6;

/// Makes thumbnails of video and image files, keeping recent decoders open.
///
/// Like video sources, it stays on the thread that created it.
#[derive(Default)]
pub struct ThumbnailMaker {
    /// Most recently used last.
    open_sources: Vec<(PathBuf, Box<dyn VideoFrameSource>)>,
    /// Proxies to read instead of their originals.
    proxies: Option<Arc<ReadyProxies>>,
}

impl ThumbnailMaker {
    /// A thumbnail maker with no file open yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads the videos listed in `proxies` from their proxies from now on.
    pub fn use_proxies(&mut self, proxies: Arc<ReadyProxies>) {
        self.proxies = Some(proxies);
    }

    /// The picture of `file_path` at `source_time` (from the start of the file),
    /// shrunk to fit inside `maximum_size`. Still images ignore the time.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be opened or decoded at that point.
    pub fn thumbnail_at(
        &mut self,
        backend: &dyn MediaBackend,
        file_path: &Path,
        source_time: Timestamp,
        maximum_size: PictureSize,
    ) -> Result<RgbaImage, MediaError> {
        let proxy = self
            .proxies
            .as_ref()
            .and_then(|proxies| proxies.proxy_for(file_path));
        if let Some(proxy) = proxy {
            let from_proxy = self
                .source_for(backend, &proxy.proxy_path, true)
                .and_then(|source| source.frame_at(source_time));
            if let Ok(frame) = from_proxy {
                return Ok(shrink_frame_to_rgba(&frame, maximum_size));
            }
            // A proxy that cannot be read is dropped: the original is read instead.
            if let Some(proxies) = &self.proxies {
                proxies.remove(file_path);
            }
        }
        let source = self.source_for(backend, file_path, false)?;
        let frame = source.frame_at(source_time)?;
        Ok(shrink_frame_to_rgba(&frame, maximum_size))
    }

    /// Closes every open decoder, so their files can be deleted (proxies).
    pub fn close_open_files(&mut self) {
        self.open_sources.clear();
    }

    /// The open decoder of `file_path`, opened if needed (a proxy for quick
    /// jumps), marked most recently used.
    fn source_for(
        &mut self,
        backend: &dyn MediaBackend,
        file_path: &Path,
        is_proxy: bool,
    ) -> Result<&mut Box<dyn VideoFrameSource>, MediaError> {
        let position = self
            .open_sources
            .iter()
            .position(|(open_path, _)| open_path == file_path);
        let entry = match position {
            Some(position) => self.open_sources.remove(position),
            None if is_proxy => (
                file_path.to_path_buf(),
                backend.open_video_proxy(file_path)?,
            ),
            None => (
                file_path.to_path_buf(),
                backend.open_video_source(file_path)?,
            ),
        };
        if self.open_sources.len() >= MAXIMUM_OPEN_THUMBNAIL_SOURCES {
            self.open_sources.remove(0);
        }
        self.open_sources.push(entry);
        let (_, source) =
            self.open_sources
                .last_mut()
                .ok_or_else(|| MediaError::InvalidRequest {
                    reason: "the thumbnail decoder could not be kept open".to_string(),
                })?;
        Ok(source)
    }
}

/// The sample rate sound is decoded at for waveforms: plenty to find peaks
/// 100 times per second, and six times less work than 48 kHz.
const WAVEFORM_ANALYSIS_SAMPLE_RATE_IN_HERTZ: u32 = 8_000;

/// The loudness outline of a file's sound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaveformPeaks {
    /// How many peaks describe one second of the file.
    pub peaks_per_second: u32,
    /// For each slice of `1 / peaks_per_second` seconds, from the start of the
    /// file: the loudest sample of either channel, from 0 (silence) to 255 (full scale).
    pub peaks: Vec<u8>,
}

/// Computes the waveform of the file described by `description`, with
/// `peaks_per_second` peaks per second (at most 1 000).
///
/// # Errors
///
/// Fails when the file has no sound or cannot be decoded.
pub fn compute_waveform_peaks(
    backend: &dyn MediaBackend,
    description: &MediaAssetDescription,
    peaks_per_second: u32,
) -> Result<WaveformPeaks, MediaError> {
    let peaks_per_second = peaks_per_second.clamp(1, 1_000);
    let duration = match description.duration {
        Some(duration) if description.has_audio() && !duration.is_zero() => duration,
        _ => {
            return Err(MediaError::NoAudioStream {
                file_path: description.file_path.clone(),
            });
        }
    };
    let file_end = Timestamp::ZERO + duration;
    let sound = backend.decode_audio_segment(&AudioSegmentRequest {
        file_path: description.file_path.clone(),
        source_start: Timestamp::ZERO,
        source_end: file_end,
        speed: PlaybackSpeed::NORMAL,
        output_sample_rate_in_hertz: WAVEFORM_ANALYSIS_SAMPLE_RATE_IN_HERTZ,
        output_samples_per_channel: sample_index_at(
            file_end,
            WAVEFORM_ANALYSIS_SAMPLE_RATE_IN_HERTZ,
        ),
    })?;
    let peak_count = peak_count_for(duration, peaks_per_second);
    Ok(WaveformPeaks {
        peaks_per_second,
        peaks: loudest_values_per_slice(sound.interleaved_samples(), peak_count),
    })
}

/// Number of peaks covering `duration` (a partial last slice counts).
fn peak_count_for(duration: TimeSpan, peaks_per_second: u32) -> usize {
    let whole_peaks = sample_index_at(Timestamp::ZERO + duration, peaks_per_second);
    let covers_exactly = span_of_sample_count(whole_peaks, peaks_per_second) == duration;
    if covers_exactly {
        whole_peaks
    } else {
        whole_peaks + 1
    }
}

/// Splits interleaved stereo samples into `slice_count` equal slices and keeps the
/// loudest absolute value of each, as 0–255.
fn loudest_values_per_slice(interleaved_samples: &[f32], slice_count: usize) -> Vec<u8> {
    let frame_count = interleaved_samples.len() / 2;
    (0..slice_count)
        .map(|slice| {
            let first_frame = slice * frame_count / slice_count.max(1);
            let end_frame = ((slice + 1) * frame_count / slice_count.max(1)).min(frame_count);
            let loudest = interleaved_samples[first_frame * 2..end_frame * 2]
                .iter()
                .fold(0.0_f32, |loudest, sample| loudest.max(sample.abs()));
            (loudest.clamp(0.0, 1.0) * 255.0).round() as u8
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_keep_the_loudest_value_of_each_slice() {
        // Four stereo frames, two slices.
        let samples = [0.1, -0.2, 0.05, 0.0, -1.0, 0.5, 0.25, 0.25];
        assert_eq!(loudest_values_per_slice(&samples, 2), vec![51, 255]);
    }

    #[test]
    fn a_partial_last_slice_counts() {
        let duration = TimeSpan::from_milliseconds(2_005);
        assert_eq!(peak_count_for(duration, 100), 201);
        assert_eq!(peak_count_for(TimeSpan::from_whole_seconds(2), 100), 200);
    }

    #[test]
    fn silence_gives_flat_peaks() {
        assert_eq!(loudest_values_per_slice(&[0.0; 20], 5), vec![0; 5]);
    }
}
