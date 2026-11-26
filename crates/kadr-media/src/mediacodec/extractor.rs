//! A safe wrapper around Android's `AMediaExtractor`, which reads the compressed
//! samples of each track of a video or audio file.
//!
//! The `ndk` crate wraps `AMediaCodec` and `AMediaFormat` safely, but not the
//! extractor, so this module calls the C functions of `libmediandk` itself.
//!
//! # Why this module uses `unsafe`
//!
//! `unsafe` is denied in the whole workspace. Calling C functions is `unsafe` by
//! nature, so this module (and `muxer.rs`) opts out with `allow(unsafe_code)`,
//! and only for these calls. Every `unsafe` block states why it is sound; the
//! rules they rely on are:
//!
//! - the extractor pointer comes from `AMediaExtractor_new`, is checked non-null,
//!   is owned by one [`MediaExtractor`] and deleted once, in `Drop`;
//! - the file descriptor given to the extractor belongs to a [`File`] kept alive
//!   in the same struct, so it stays open as long as the extractor reads it;
//! - buffers are passed with their exact length, and the extractor never writes
//!   past the capacity it is given.
//!
//! See decision record 0042.

#![allow(unsafe_code)]

use std::fs::File;
use std::mem::MaybeUninit;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::ptr::NonNull;

use ndk::media::media_format::MediaFormat;
use ndk_sys as ffi;

use crate::error::MediaError;

/// `AMEDIA_OK`, the status of a successful call.
const MEDIA_STATUS_OK: i32 = 0;

/// Reads the compressed samples of a file, track by track.
pub(crate) struct MediaExtractor {
    extractor: NonNull<ffi::AMediaExtractor>,
    /// Kept open while the extractor reads from its descriptor.
    file: File,
}

/// One compressed sample, as read from the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SampleInformation {
    /// Bytes written into the buffer.
    pub(crate) size_in_bytes: usize,
    /// Presentation time, in microseconds.
    pub(crate) presentation_time_in_microseconds: i64,
}

impl MediaExtractor {
    /// Opens `file_path` for reading.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be opened or is not a media file Android reads.
    pub(crate) fn open(file_path: &Path) -> Result<Self, MediaError> {
        let cannot_open = |reason: String| MediaError::CannotOpenFile {
            file_path: file_path.to_path_buf(),
            reason,
        };
        let file = File::open(file_path).map_err(|error| cannot_open(error.to_string()))?;
        let file_length = file
            .metadata()
            .map_err(|error| cannot_open(error.to_string()))?
            .len();
        let file_length = i64::try_from(file_length)
            .map_err(|_| cannot_open("the file is too large".to_string()))?;
        // SAFETY: `AMediaExtractor_new` takes no argument and returns an owned
        // extractor, or null on failure (checked just below).
        let extractor = NonNull::new(unsafe { ffi::AMediaExtractor_new() })
            .ok_or_else(|| cannot_open("Android could not create a media extractor".to_string()))?;
        let media_extractor = Self { extractor, file };
        // SAFETY: the extractor is valid (owned above); the descriptor is open and
        // stays open as long as `media_extractor` lives, since it owns the file.
        let status = unsafe {
            ffi::AMediaExtractor_setDataSourceFd(
                media_extractor.extractor.as_ptr(),
                media_extractor.file.as_raw_fd(),
                0,
                file_length,
            )
        };
        if status.0 != MEDIA_STATUS_OK {
            return Err(cannot_open(format!(
                "Android cannot read this file (media status {})",
                status.0
            )));
        }
        Ok(media_extractor)
    }

    /// Number of tracks (video, audio, subtitles…) in the file.
    pub(crate) fn track_count(&self) -> usize {
        // SAFETY: the extractor is valid for the lifetime of `self`.
        unsafe { ffi::AMediaExtractor_getTrackCount(self.extractor.as_ptr()) }
    }

    /// The format of track `track_index`: codec, size, rate, duration…
    pub(crate) fn track_format(&self, track_index: usize) -> Option<MediaFormat> {
        // SAFETY: the extractor is valid; Android checks the index and returns
        // null when it is out of range.
        let format =
            unsafe { ffi::AMediaExtractor_getTrackFormat(self.extractor.as_ptr(), track_index) };
        // SAFETY: a non-null result is a new format the caller owns: `MediaFormat`
        // takes ownership and deletes it when dropped.
        NonNull::new(format).map(|format| unsafe { MediaFormat::from_ptr(format) })
    }

    /// Makes the extractor return the samples of track `track_index`.
    ///
    /// # Errors
    ///
    /// Fails when the track does not exist.
    pub(crate) fn select_track(&mut self, track_index: usize) -> Result<(), String> {
        // SAFETY: the extractor is valid; Android checks the index.
        let status =
            unsafe { ffi::AMediaExtractor_selectTrack(self.extractor.as_ptr(), track_index) };
        check_status(status, "selecting the track")
    }

    /// Jumps to the keyframe at or before `time_in_microseconds`.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read there.
    pub(crate) fn seek_to_keyframe_before(
        &mut self,
        time_in_microseconds: i64,
    ) -> Result<(), String> {
        // SAFETY: the extractor is valid; any time is accepted.
        let status = unsafe {
            ffi::AMediaExtractor_seekTo(
                self.extractor.as_ptr(),
                time_in_microseconds.max(0),
                ffi::SeekMode::AMEDIAEXTRACTOR_SEEK_PREVIOUS_SYNC,
            )
        };
        check_status(status, "jumping to a keyframe")
    }

    /// The presentation time of the current sample, or `None` at the end.
    pub(crate) fn sample_time_in_microseconds(&self) -> Option<i64> {
        // SAFETY: the extractor is valid; −1 means "no more samples".
        let time = unsafe { ffi::AMediaExtractor_getSampleTime(self.extractor.as_ptr()) };
        (time >= 0).then_some(time)
    }

    /// Copies the current sample into `buffer`. `Ok(None)` at the end of the file.
    ///
    /// # Errors
    ///
    /// Fails when the sample does not fit in the buffer.
    pub(crate) fn read_sample(
        &mut self,
        buffer: &mut [MaybeUninit<u8>],
    ) -> Result<Option<SampleInformation>, String> {
        let Some(presentation_time_in_microseconds) = self.sample_time_in_microseconds() else {
            return Ok(None);
        };
        // SAFETY: the extractor is valid.
        let sample_size = unsafe { ffi::AMediaExtractor_getSampleSize(self.extractor.as_ptr()) };
        let Ok(sample_size) = usize::try_from(sample_size) else {
            return Ok(None);
        };
        if sample_size > buffer.len() {
            return Err(format!(
                "a compressed sample of {sample_size} bytes does not fit in the decoder's {}-byte buffer",
                buffer.len()
            ));
        }
        // SAFETY: the extractor is valid; the pointer and capacity describe
        // `buffer` exactly, and the extractor writes at most `capacity` bytes.
        let written = unsafe {
            ffi::AMediaExtractor_readSampleData(
                self.extractor.as_ptr(),
                buffer.as_mut_ptr().cast::<u8>(),
                buffer.len(),
            )
        };
        let size_in_bytes = usize::try_from(written)
            .map_err(|_| format!("reading a compressed sample failed (status {written})"))?;
        Ok(Some(SampleInformation {
            size_in_bytes,
            presentation_time_in_microseconds,
        }))
    }

    /// Moves to the next sample. Returns `false` at the end of the file.
    pub(crate) fn advance(&mut self) -> bool {
        // SAFETY: the extractor is valid.
        unsafe { ffi::AMediaExtractor_advance(self.extractor.as_ptr()) }
    }
}

impl Drop for MediaExtractor {
    fn drop(&mut self) {
        // SAFETY: the extractor was created by `AMediaExtractor_new`, is owned by
        // `self` only, and is deleted exactly once, here. Its status is ignored:
        // there is nothing left to do if deleting fails.
        unsafe {
            ffi::AMediaExtractor_delete(self.extractor.as_ptr());
        }
    }
}

/// Turns a media status into an error naming the step that failed.
pub(crate) fn check_status(status: ffi::media_status_t, step: &str) -> Result<(), String> {
    if status.0 == MEDIA_STATUS_OK {
        Ok(())
    } else {
        Err(format!("{step} failed (media status {})", status.0))
    }
}
