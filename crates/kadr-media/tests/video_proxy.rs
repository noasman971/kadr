//! Proxies made by the FFmpeg backend (decision record 0055): a 4K video written
//! by FFmpeg for the test, its description, and a proxy that shows the same
//! frame at every time, in order and out of order, with the source's colours.

// Test helpers panic with the error on failure, which is how a test should fail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use kadr_core::media_asset::{MediaKind, PictureSize};
use kadr_core::time::{FrameRate, TimeSpan, Timestamp};
use kadr_media::test_patterns::{frame_index_barcode_yuv, read_frame_index_barcode};
use kadr_media::{
    AudioBuffer, ChromaSubsampling, EncodingSpeed, ExportSinkSettings, FfmpegBackend, MediaBackend,
    VideoEncoderPreference, VideoFrame, VideoProxyOutcome, VideoProxySettings, YuvColorSpace,
};

const SAMPLE_RATE: u32 = 48_000;
const FRAMES_PER_KEYFRAME: u32 = 10;

fn size(width_in_pixels: u32, height_in_pixels: u32) -> PictureSize {
    PictureSize {
        width_in_pixels,
        height_in_pixels,
    }
}

/// Writes `frame_count` barcode frames (each shows its number) with silence.
fn write_barcode_video(
    backend: &FfmpegBackend,
    path: &Path,
    picture_size: PictureSize,
    frame_rate: FrameRate,
    frame_count: u32,
) {
    let mut sink = backend
        .create_export_sink(&ExportSinkSettings {
            output_path: path.to_path_buf(),
            width_in_pixels: picture_size.width_in_pixels,
            height_in_pixels: picture_size.height_in_pixels,
            frame_rate,
            audio_sample_rate_in_hertz: SAMPLE_RATE,
            video_encoder_preference: VideoEncoderPreference::SoftwareOnly,
            encoding_speed: EncodingSpeed::Fast,
        })
        .unwrap();
    for frame_index in 0..frame_count {
        sink.write_video_frame(&frame_index_barcode_yuv(
            picture_size.width_in_pixels,
            picture_size.height_in_pixels,
            frame_index,
        ))
        .unwrap();
    }
    let duration = frame_rate.timestamp_of_frame(u64::from(frame_count));
    let samples = (duration.as_seconds() * f64::from(SAMPLE_RATE)) as usize;
    sink.write_audio(&AudioBuffer::silence(SAMPLE_RATE, samples))
        .unwrap();
    sink.finish().unwrap();
}

fn make_proxy(backend: &FfmpegBackend, source: &Path, proxy_size: PictureSize) -> PathBuf {
    let output_path = source.with_extension("proxy.mp4");
    let outcome = backend
        .make_video_proxy(
            &VideoProxySettings {
                source_path: source.to_path_buf(),
                output_path: output_path.clone(),
                picture_size: proxy_size,
                frames_per_keyframe: FRAMES_PER_KEYFRAME,
            },
            &mut |_| ControlFlow::Continue(()),
        )
        .unwrap();
    assert_eq!(outcome, VideoProxyOutcome::Finished);
    output_path
}

/// The frame number shown at `time`.
fn frame_number_at(source: &mut dyn kadr_media::VideoFrameSource, time: Timestamp) -> u32 {
    read_frame_index_barcode(&source.frame_at(time).unwrap())
}

/// Times inside each frame: its start, and just before the next one starts.
fn times_inside_frames(frame_rate: FrameRate, frame_count: u32) -> Vec<(u32, Timestamp)> {
    (0..frame_count)
        .flat_map(|frame_index| {
            let start = frame_rate.timestamp_of_frame(u64::from(frame_index));
            let next = frame_rate.timestamp_of_frame(u64::from(frame_index) + 1);
            let last_moment = next.checked_sub(TimeSpan::from_ticks(1)).unwrap();
            [(frame_index, start), (frame_index, last_moment)]
        })
        .collect()
}

#[test]
fn a_4k_video_is_described_and_its_proxy_shows_the_same_frame_at_every_time() {
    let folder = tempfile::tempdir().unwrap();
    let backend = FfmpegBackend::new().unwrap();
    let source = folder.path().join("barcode_4k.mp4");
    let frame_count = 36;
    write_barcode_video(
        &backend,
        &source,
        size(3840, 2160),
        FrameRate::FPS_30,
        frame_count,
    );

    let details = backend.describe_video_stream(&source).unwrap();
    assert_eq!(details.codec_name, "h264");
    assert_eq!(details.picture_size, size(3840, 2160));
    assert_eq!(details.frame_rate, Some(FrameRate::FPS_30));
    assert_eq!(details.bits_per_sample, 8);
    assert_eq!(details.chroma_subsampling, ChromaSubsampling::Yuv420);
    assert!(details.is_eight_bit_yuv420());
    assert!(details.bit_rate_in_bits_per_second.is_some());

    let proxy = make_proxy(&backend, &source, size(1920, 1080));
    let proxy_description = backend.probe(&proxy).unwrap();
    assert_eq!(proxy_description.kind, MediaKind::Video);
    assert_eq!(proxy_description.picture_size, Some(size(1920, 1080)));
    assert_eq!(proxy_description.frame_rate, Some(FrameRate::FPS_30));
    assert!(!proxy_description.has_audio(), "proxies carry no sound");

    let mut original_frames = backend.open_video_source(&source).unwrap();
    let mut proxy_frames = backend.open_video_source(&proxy).unwrap();
    for (frame_index, time) in times_inside_frames(FrameRate::FPS_30, frame_count) {
        assert_eq!(
            frame_number_at(proxy_frames.as_mut(), time),
            frame_index,
            "at {time}"
        );
    }
    // Out of order: backwards, far ahead, then back again.
    for frame_index in [30, 3, 17, 16, 35, 0, 11, 29, 2] {
        let time = FrameRate::FPS_30.timestamp_of_frame(frame_index);
        let shown_by_proxy = frame_number_at(proxy_frames.as_mut(), time);
        assert_eq!(u64::from(shown_by_proxy), frame_index, "at {time}");
        assert_eq!(
            frame_number_at(original_frames.as_mut(), time),
            shown_by_proxy
        );
    }
    // Past the end, both hold their last frame.
    let after_the_end = FrameRate::FPS_30.timestamp_of_frame(u64::from(frame_count) + 5);
    assert_eq!(
        frame_number_at(proxy_frames.as_mut(), after_the_end),
        frame_count - 1
    );
}

#[test]
fn ntsc_frame_times_are_kept() {
    let folder = tempfile::tempdir().unwrap();
    let backend = FfmpegBackend::new().unwrap();
    let source = folder.path().join("barcode_ntsc.mp4");
    let frame_count = 40;
    write_barcode_video(
        &backend,
        &source,
        size(640, 360),
        FrameRate::FPS_29_97,
        frame_count,
    );

    let proxy = make_proxy(&backend, &source, size(320, 180));

    let mut proxy_frames = backend.open_video_source(&proxy).unwrap();
    for (frame_index, time) in times_inside_frames(FrameRate::FPS_29_97, frame_count) {
        assert_eq!(
            frame_number_at(proxy_frames.as_mut(), time),
            frame_index,
            "at {time}"
        );
    }
}

#[test]
fn small_proxies_keep_the_colour_matrix_of_their_source() {
    // A 720p source is read as BT.709; an untagged 360-line file would be read
    // as BT.601, so the proxy must say BT.709 itself.
    let folder = tempfile::tempdir().unwrap();
    let backend = FfmpegBackend::new().unwrap();
    let source = folder.path().join("barcode_720p.mp4");
    write_barcode_video(&backend, &source, size(1280, 720), FrameRate::FPS_30, 5);

    let proxy = make_proxy(&backend, &source, size(640, 360));

    let frame = backend
        .open_video_source(&proxy)
        .unwrap()
        .frame_at(Timestamp::ZERO)
        .unwrap();
    let VideoFrame::Yuv420(picture) = frame.as_ref() else {
        panic!("a proxy's pictures are YUV 4:2:0");
    };
    assert_eq!(picture.color_space(), YuvColorSpace::BT709_LIMITED);
}

#[test]
fn making_a_proxy_stops_when_asked() {
    let folder = tempfile::tempdir().unwrap();
    let backend = FfmpegBackend::new().unwrap();
    let source = folder.path().join("barcode.mp4");
    write_barcode_video(&backend, &source, size(640, 360), FrameRate::FPS_30, 30);
    let mut frames_seen = 0;

    let outcome = backend
        .make_video_proxy(
            &VideoProxySettings {
                source_path: source.clone(),
                output_path: folder.path().join("proxy.mp4"),
                picture_size: size(320, 180),
                frames_per_keyframe: FRAMES_PER_KEYFRAME,
            },
            &mut |_| {
                frames_seen += 1;
                if frames_seen == 3 {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            },
        )
        .unwrap();

    assert_eq!(outcome, VideoProxyOutcome::Cancelled);
    assert_eq!(frames_seen, 3);
}

#[test]
fn odd_or_empty_proxy_sizes_are_refused() {
    let folder = tempfile::tempdir().unwrap();
    let backend = FfmpegBackend::new().unwrap();
    let source = folder.path().join("barcode.mp4");
    write_barcode_video(&backend, &source, size(640, 360), FrameRate::FPS_30, 3);
    for picture_size in [size(321, 180), size(0, 180)] {
        let result = backend.make_video_proxy(
            &VideoProxySettings {
                source_path: source.clone(),
                output_path: folder.path().join("proxy.mp4"),
                picture_size,
                frames_per_keyframe: FRAMES_PER_KEYFRAME,
            },
            &mut |_| ControlFlow::Continue(()),
        );
        assert!(result.is_err(), "{picture_size:?}");
    }
}
