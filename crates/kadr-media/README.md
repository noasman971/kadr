# kadr-media

Media input and output, behind the `MediaBackend` traits:

- `probe`: kind, duration, picture size, frame rate, sound format of a file;
- `open_video_source`: the exact picture shown at any time (frame-accurate, in order
  or with jumps);
- `decode_audio_segment`: a range of sound at any speed (pitch kept), as stereo, at
  the exact length requested;
- `create_export_sink`: an MP4 file (H.264 + AAC), with the first hardware encoder
  that works (NVENC, Quick Sync, AMF, VideoToolbox) or libx264.

`test_patterns` creates videos whose frames show their own number as a barcode and
whose sound beeps at known times, to check exports exactly.

- **Depends on:** `kadr-core`, `ffmpeg-next` (feature `ffmpeg`, on by default), `image`.
- **Used by:** `kadr-engine`, `kadr-cli`.
- **Backends:** FFmpeg 7+ on desktop (phase 2); Android `MediaCodec` (phase 9) behind
  the same traits.
- **Status:** implemented (phase 2). Hardware *decoding* arrives with GPU rendering
  (phase 3).

See [docs/media-and-export.md](../../docs/media-and-export.md).
