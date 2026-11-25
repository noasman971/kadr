//! # kadr-media — reading and writing video and audio files
//!
//! This crate turns media files into decoded frames and audio samples, and turns
//! rendered frames back into a video file:
//!
//! - probing a file (duration, streams, resolution, frame rate, codecs);
//! - decoding video and audio, with frame-accurate seeking;
//! - encoding the final export (H.264 / AAC in MP4 first);
//! - extracting thumbnails and audio waveforms for the timeline.
//!
//! ## Rules for this crate
//!
//! - Decoders and encoders are hidden behind traits. The desktop implementation
//!   uses FFmpeg; the Android implementation will use `MediaCodec` (phase 9).
//! - Hardware acceleration is used when available, with an automatic software
//!   fallback.
//!
//! Implemented in phase 2. See `docs/architecture.md`.
