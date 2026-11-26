//! A safe wrapper around Android's `AMediaMuxer`, which writes compressed video
//! and sound into an MP4 file.
//!
//! Like the extractor, the muxer is not wrapped by the `ndk` crate, so this
//! module calls `libmediandk` itself and opts out of the workspace's `unsafe`
//! ban for those calls only (see `extractor.rs` and decision record 0042). The
//! rules every `unsafe` block relies on:
//!
//! - the muxer pointer comes from `AMediaMuxer_new`, is checked non-null, is owned
//!   by one [`Mp4Muxer`] and deleted once, in `Drop`;
//! - the file descriptor belongs to a [`File`] kept alive in the same struct;
//! - formats and sample data are borrowed for the duration of the call only, and
//!   the muxer copies what it keeps.

#![allow(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::ptr::NonNull;

use ndk::media::media_format::MediaFormat;
use ndk_sys as ffi;

use super::extractor::check_status;

/// Writes an MP4 file from compressed samples.
pub(crate) struct Mp4Muxer {
    muxer: NonNull<ffi::AMediaMuxer>,
    /// Kept open while the muxer writes to its descriptor.
    file: File,
    is_started: bool,
}

impl Mp4Muxer {
    /// Creates (or empties) `output_path` and prepares to write an MP4 file in it.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be created.
    pub(crate) fn create(output_path: &Path) -> Result<Self, String> {
        // The muxer reads back what it wrote (to move the index to the front), so
        // the file is opened for reading too.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(output_path)
            .map_err(|error| format!("creating the file failed: {error}"))?;
        // SAFETY: the descriptor is open and stays open as long as the muxer, since
        // both live in the returned struct; null (failure) is checked just below.
        let muxer = unsafe {
            ffi::AMediaMuxer_new(
                file.as_raw_fd(),
                ffi::OutputFormat::AMEDIAMUXER_OUTPUT_FORMAT_MPEG_4,
            )
        };
        let muxer = NonNull::new(muxer).ok_or("Android could not create an MP4 writer")?;
        Ok(Self {
            muxer,
            file,
            is_started: false,
        })
    }

    /// Adds a track described by an encoder's output format (which carries the
    /// codec's set-up data), and returns its index. Only before [`Self::start`].
    ///
    /// # Errors
    ///
    /// Fails when the muxer refuses the format.
    pub(crate) fn add_track(&mut self, format: &MediaFormat) -> Result<usize, String> {
        // SAFETY: the muxer and the format are valid for the duration of the call;
        // the muxer copies the format.
        let track_index =
            unsafe { ffi::AMediaMuxer_addTrack(self.muxer.as_ptr(), format.as_ptr()) };
        usize::try_from(track_index)
            .map_err(|_| format!("adding a track failed (status {track_index})"))
    }

    /// Writes the file header. Tracks can no longer be added.
    ///
    /// # Errors
    ///
    /// Fails when the header cannot be written.
    pub(crate) fn start(&mut self) -> Result<(), String> {
        // SAFETY: the muxer is valid.
        let status = unsafe { ffi::AMediaMuxer_start(self.muxer.as_ptr()) };
        check_status(status, "starting the MP4 writer")?;
        self.is_started = true;
        Ok(())
    }

    /// Whether [`Self::start`] succeeded.
    pub(crate) fn is_started(&self) -> bool {
        self.is_started
    }

    /// Writes one compressed sample of track `track_index`.
    ///
    /// # Errors
    ///
    /// Fails when the muxer is not started or cannot write.
    pub(crate) fn write_sample(
        &mut self,
        track_index: usize,
        data: &[u8],
        presentation_time_in_microseconds: i64,
        flags: u32,
    ) -> Result<(), String> {
        let size = i32::try_from(data.len())
            .map_err(|_| "a compressed sample is too large".to_string())?;
        let information = ffi::AMediaCodecBufferInfo {
            offset: 0,
            size,
            presentationTimeUs: presentation_time_in_microseconds,
            flags,
        };
        // SAFETY: the muxer is valid; `data` holds exactly `size` bytes from
        // offset 0 and is only read during the call.
        let status = unsafe {
            ffi::AMediaMuxer_writeSampleData(
                self.muxer.as_ptr(),
                track_index,
                data.as_ptr(),
                &information,
            )
        };
        check_status(status, "writing a sample")
    }

    /// Writes the end of the file (its index). Nothing can be written afterwards.
    ///
    /// # Errors
    ///
    /// Fails when the end of the file cannot be written.
    pub(crate) fn stop(mut self) -> Result<(), String> {
        // SAFETY: the muxer is valid.
        let status = unsafe { ffi::AMediaMuxer_stop(self.muxer.as_ptr()) };
        self.is_started = false;
        check_status(status, "finishing the MP4 file")?;
        self.file
            .sync_all()
            .map_err(|error| format!("saving the file failed: {error}"))
    }
}

impl Drop for Mp4Muxer {
    fn drop(&mut self) {
        // SAFETY: the muxer was created by `AMediaMuxer_new`, is owned by `self`
        // only, and is deleted exactly once, here (deleting a started muxer stops
        // it first). The status is ignored: nothing is left to do on failure.
        unsafe {
            ffi::AMediaMuxer_delete(self.muxer.as_ptr());
        }
    }
}
