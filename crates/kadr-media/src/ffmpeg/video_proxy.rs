//! Proxies: small, easy-to-decode copies of a video's pictures for the preview
//! (decision record 0055), and the description of a video stream that tells
//! whether a file is heavy enough to need one.
//!
//! A proxy keeps **every frame at the same time** as its source. Frame times are
//! read exactly as `video_source.rs` reads them (from the start of the stream,
//! negative times counting as zero, and the last of several frames at the same
//! time winning), so asking either file for the frame at a time gives the same
//! frame. The pictures are shrunk with swscale and stored as H.264 without
//! B-frames, with a keyframe at least every few frames and libx264's
//! `fastdecode` tuning, tagged with the colour space the source's pictures were
//! read in. No sound: the preview's sound always comes from the source.

use std::ops::ControlFlow;
use std::path::Path;

use ffmpeg_next as ffmpeg;
use ffmpeg_next::Rational;
use ffmpeg_next::format::Pixel;
use kadr_core::media_asset::PictureSize;
use kadr_core::time::Timestamp;

use super::picture_conversion::{color_space_of, is_full_range_format};
use super::probe::read_frame_rate;
use super::time_conversion::timestamp_from_stream_units;
use super::{
    PictureDecoder, decoding_failure, find_picture_stream, open_input_file, open_picture_decoder,
};
use crate::backend::{
    ChromaSubsampling, VideoProxyOutcome, VideoProxySettings, VideoStreamDetails,
};
use crate::error::MediaError;
use crate::video_frame::{YuvColorSpace, YuvMatrix, YuvRange};

/// Encoders tried in order: libx264 takes the `fastdecode` tuning; OpenH264 is
/// there for FFmpeg builds without it.
const PROXY_VIDEO_ENCODERS: [&str; 2] = ["libx264", "libopenh264"];
/// libx264's speed preset: proxies must be quick to make.
const LIBX264_PRESET: &str = "veryfast";
/// libx264's constant quality (lower is better): plenty for a preview.
const LIBX264_CONSTANT_RATE_FACTOR: &str = "23";
/// No CABAC, no deblocking, no weighted prediction: much less work per frame.
const LIBX264_TUNING: &str = "fastdecode";
/// Bits per pixel per frame given to OpenH264, which is driven by bit rate.
const OPENH264_BITS_PER_PIXEL: f64 = 0.15;
/// Frame rate assumed for the last frame's duration when the file gives none.
const FALLBACK_FRAMES_PER_SECOND: i64 = 30;

/// Describes the picture stream of `file_path`.
pub(crate) fn describe_video_stream(file_path: &Path) -> Result<VideoStreamDetails, MediaError> {
    let input = open_input_file(file_path)?;
    let stream = find_picture_stream(&input).ok_or_else(|| MediaError::NoVideoStream {
        file_path: file_path.to_path_buf(),
    })?;
    let codec_name = stream.parameters().id().name().to_string();
    let stream_bit_rate = stream.parameters().bit_rate();
    let decoder = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
        .and_then(|context| context.decoder().video())
        .map_err(|error| decoding_failure(file_path, "reading the video parameters", error))?;
    let (bits_per_sample, chroma_subsampling) = sample_layout_of(decoder.format());
    let bit_rate_in_bits_per_second = [stream_bit_rate, input.bit_rate()]
        .into_iter()
        .find(|&bit_rate| bit_rate > 0)
        .and_then(|bit_rate| u64::try_from(bit_rate).ok());
    Ok(VideoStreamDetails {
        codec_name,
        picture_size: PictureSize {
            width_in_pixels: decoder.width(),
            height_in_pixels: decoder.height(),
        },
        frame_rate: read_frame_rate(&stream),
        bits_per_sample,
        chroma_subsampling,
        bit_rate_in_bits_per_second,
    })
}

/// Bits per sample and colour subsampling of a pixel format. An unknown format
/// counts as 8-bit 4:2:0, the common case, so missing information never makes a
/// file look heavy.
fn sample_layout_of(format: Pixel) -> (u8, ChromaSubsampling) {
    let Some(descriptor) = format.descriptor() else {
        return (8, ChromaSubsampling::Yuv420);
    };
    let component_count = i32::from(descriptor.nb_components()).max(1);
    let bits_per_pixel = descriptor.bits_per_pixel().max(0);
    if component_count != 3 {
        // Grey levels, palettes, or a fourth (alpha) plane.
        let bits_per_sample = bits_per_pixel / component_count;
        return (
            u8::try_from(bits_per_sample).unwrap_or(8),
            ChromaSubsampling::Other,
        );
    }
    let (log2_width, log2_height) = (descriptor.log2_chroma_w(), descriptor.log2_chroma_h());
    let chroma_subsampling = match (log2_width, log2_height) {
        (1, 1) => ChromaSubsampling::Yuv420,
        (1, 0) => ChromaSubsampling::Yuv422,
        (0, 0) => ChromaSubsampling::Yuv444,
        _ => ChromaSubsampling::Other,
    };
    // One full-size plane and two planes `pixels_per_chroma_sample` times smaller:
    // bits per pixel = depth × (1 + 2 / pixels_per_chroma_sample).
    let pixels_per_chroma_sample = 1_i32 << (log2_width + log2_height);
    let bits_per_sample = (bits_per_pixel * pixels_per_chroma_sample
        + (pixels_per_chroma_sample + 2) / 2)
        / (pixels_per_chroma_sample + 2);
    (
        u8::try_from(bits_per_sample).unwrap_or(8),
        chroma_subsampling,
    )
}

/// Writes the proxy `settings` describes.
pub(crate) fn make_video_proxy(
    settings: &VideoProxySettings,
    on_progress: &mut dyn FnMut(Timestamp) -> ControlFlow<()>,
) -> Result<VideoProxyOutcome, MediaError> {
    validate_proxy_settings(settings)?;
    let mut source = ProxySource::open(&settings.source_path)?;
    let mut writer: Option<ProxyWriter> = None;
    // Held back one frame: a later frame at the same time replaces it, as in
    // `video_source.rs`, where the last frame starting at a time is shown.
    let mut held_frame: Option<TimedPicture> = None;
    while let Some(decoded) = source.next_frame()? {
        let writer = match &mut writer {
            Some(writer) => writer,
            None => writer.insert(ProxyWriter::create(settings, &source, &decoded.picture)?),
        };
        let picture = writer.shrink(&decoded.picture)?;
        held_frame = match held_frame.take() {
            Some(previous) if decoded.time_in_stream_units > previous.time_in_stream_units => {
                writer.encode(previous)?;
                Some(TimedPicture {
                    time_in_stream_units: decoded.time_in_stream_units,
                    picture,
                })
            }
            Some(previous) => Some(TimedPicture {
                time_in_stream_units: previous.time_in_stream_units,
                picture,
            }),
            None => Some(TimedPicture {
                time_in_stream_units: decoded.time_in_stream_units,
                picture,
            }),
        };
        let source_time =
            timestamp_from_stream_units(decoded.time_in_stream_units, source.stream_time_base);
        if on_progress(source_time).is_break() {
            return Ok(VideoProxyOutcome::Cancelled);
        }
    }
    let Some(mut writer) = writer else {
        return Err(decoding_failure(
            &settings.source_path,
            "making a proxy",
            "the video has no frame",
        ));
    };
    if let Some(last_frame) = held_frame {
        writer.encode(last_frame)?;
    }
    writer.finish()?;
    Ok(VideoProxyOutcome::Finished)
}

fn validate_proxy_settings(settings: &VideoProxySettings) -> Result<(), MediaError> {
    let PictureSize {
        width_in_pixels: width,
        height_in_pixels: height,
    } = settings.picture_size;
    if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
        return Err(MediaError::InvalidRequest {
            reason: format!("a proxy's size must be even and non-zero, but is {width}×{height}"),
        });
    }
    if settings.frames_per_keyframe == 0 {
        return Err(MediaError::InvalidRequest {
            reason: "a proxy needs at least one frame per keyframe".to_string(),
        });
    }
    Ok(())
}

/// A picture with its time, in the source stream's units, from the stream's start.
struct TimedPicture {
    time_in_stream_units: i64,
    picture: ffmpeg::frame::Video,
}

/// The source's pictures, in presentation order, with their times.
struct ProxySource {
    input: ffmpeg::format::context::Input,
    file_path: std::path::PathBuf,
    stream_index: usize,
    stream_time_base: Rational,
    stream_start_in_stream_units: i64,
    frame_rate: Option<Rational>,
    decoder: ffmpeg::decoder::Video,
    end_of_file_reached: bool,
}

impl ProxySource {
    fn open(file_path: &Path) -> Result<Self, MediaError> {
        let input = open_input_file(file_path)?;
        let frame_rate = find_picture_stream(&input)
            .and_then(|stream| read_frame_rate(&stream))
            .map(|rate| Rational::new(rate.numerator() as i32, rate.denominator() as i32));
        let PictureDecoder {
            stream_index,
            stream_time_base,
            stream_start_in_stream_units,
            decoder,
        } = open_picture_decoder(&input, file_path)?;
        Ok(Self {
            input,
            file_path: file_path.to_path_buf(),
            stream_index,
            stream_time_base,
            stream_start_in_stream_units,
            frame_rate,
            decoder,
            end_of_file_reached: false,
        })
    }

    /// The next decoded picture, or `None` at the end of the file.
    fn next_frame(&mut self) -> Result<Option<TimedPicture>, MediaError> {
        loop {
            let mut picture = ffmpeg::frame::Video::empty();
            if self.decoder.receive_frame(&mut picture).is_ok() {
                let presentation_time = picture
                    .timestamp()
                    .unwrap_or(self.stream_start_in_stream_units);
                let time_in_stream_units =
                    (presentation_time - self.stream_start_in_stream_units).max(0);
                return Ok(Some(TimedPicture {
                    time_in_stream_units,
                    picture,
                }));
            }
            if self.end_of_file_reached {
                return Ok(None);
            }
            self.feed_decoder_with_next_packet()?;
        }
    }

    fn feed_decoder_with_next_packet(&mut self) -> Result<(), MediaError> {
        for (stream, packet) in self.input.packets() {
            if stream.index() != self.stream_index {
                continue;
            }
            return self.decoder.send_packet(&packet).map_err(|error| {
                decoding_failure(&self.file_path, "decoding a video packet", error)
            });
        }
        self.end_of_file_reached = true;
        self.decoder.send_eof().map_err(|error| {
            decoding_failure(&self.file_path, "finishing the video decoding", error)
        })
    }
}

/// The proxy file being written.
struct ProxyWriter {
    output_path: std::path::PathBuf,
    output: ffmpeg::format::context::Output,
    encoder: ffmpeg::encoder::Video,
    stream_index: usize,
    /// The source stream's time base: frame times are copied as they are.
    encoder_time_base: Rational,
    stream_time_base: Rational,
    /// One frame period in `encoder_time_base` units, for the last frame's duration.
    frame_duration_in_time_base_units: i64,
    picture_size: PictureSize,
    shrinker: Option<PictureShrinker>,
}

/// A swscale context for one source format and size.
struct PictureShrinker {
    source_format: Pixel,
    source_width: u32,
    source_height: u32,
    scaler: ffmpeg::software::scaling::Context,
}

fn encoding_failure(output_path: &Path, step: &str, error: impl std::fmt::Display) -> MediaError {
    MediaError::EncodingFailed {
        file_path: output_path.to_path_buf(),
        reason: format!("{step}: {error}"),
    }
}

impl ProxyWriter {
    /// Creates the file and its encoder; `first_picture` gives the colour space.
    fn create(
        settings: &VideoProxySettings,
        source: &ProxySource,
        first_picture: &ffmpeg::frame::Video,
    ) -> Result<Self, MediaError> {
        let output_path = settings.output_path.clone();
        let mut output = ffmpeg::format::output_as(&output_path, "mp4")
            .map_err(|error| encoding_failure(&output_path, "creating the file", error))?;
        let needs_global_header = output
            .format()
            .flags()
            .contains(ffmpeg::format::Flags::GLOBAL_HEADER);
        let color_space = proxy_color_space_of(first_picture);
        let request = EncoderRequest {
            settings,
            time_base: source.stream_time_base,
            frame_rate: source.frame_rate,
            color_space,
            needs_global_header,
        };
        let mut tried_encoder_names = Vec::new();
        let mut opened = None;
        for encoder_name in PROXY_VIDEO_ENCODERS {
            let Some(codec) = ffmpeg::encoder::find_by_name(encoder_name) else {
                continue;
            };
            tried_encoder_names.push(encoder_name.to_string());
            if let Some(encoder) = open_proxy_encoder(codec, encoder_name, &request) {
                opened = Some((codec, encoder));
                break;
            }
        }
        let Some((codec, encoder)) = opened else {
            return Err(MediaError::NoUsableVideoEncoder {
                tried_encoder_names,
            });
        };
        let mut stream = output
            .add_stream(codec)
            .map_err(|error| encoding_failure(&output_path, "adding the video stream", error))?;
        stream.set_parameters(&encoder);
        stream.set_time_base(source.stream_time_base);
        let stream_index = stream.index();
        output
            .write_header()
            .map_err(|error| encoding_failure(&output_path, "writing the file header", error))?;
        // The muxer may change the stream time base while writing the header.
        let stream_time_base = output
            .stream(stream_index)
            .map(|stream| stream.time_base())
            .ok_or_else(|| {
                encoding_failure(
                    &output_path,
                    "reading the stream time base",
                    "missing stream",
                )
            })?;
        Ok(Self {
            output_path,
            output,
            encoder,
            stream_index,
            encoder_time_base: source.stream_time_base,
            stream_time_base,
            frame_duration_in_time_base_units: frame_duration_in_units(
                source.stream_time_base,
                source.frame_rate,
            ),
            picture_size: settings.picture_size,
            shrinker: None,
        })
    }

    /// `picture` at the proxy's size, as 8-bit YUV 4:2:0.
    fn shrink(
        &mut self,
        picture: &ffmpeg::frame::Video,
    ) -> Result<ffmpeg::frame::Video, MediaError> {
        let matches_picture = self.shrinker.as_ref().is_some_and(|shrinker| {
            shrinker.source_format == picture.format()
                && shrinker.source_width == picture.width()
                && shrinker.source_height == picture.height()
        });
        if !matches_picture {
            let scaler = ffmpeg::software::scaling::Context::get(
                picture.format(),
                picture.width(),
                picture.height(),
                Pixel::YUV420P,
                self.picture_size.width_in_pixels,
                self.picture_size.height_in_pixels,
                ffmpeg::software::scaling::Flags::BICUBIC,
            )
            .map_err(|error| {
                encoding_failure(&self.output_path, "preparing the picture scaler", error)
            })?;
            self.shrinker = Some(PictureShrinker {
                source_format: picture.format(),
                source_width: picture.width(),
                source_height: picture.height(),
                scaler,
            });
        }
        let shrinker = self.shrinker.as_mut().ok_or_else(|| {
            encoding_failure(
                &self.output_path,
                "preparing the picture scaler",
                "no scaler",
            )
        })?;
        let mut shrunk = ffmpeg::frame::Video::empty();
        shrinker
            .scaler
            .run(picture, &mut shrunk)
            .map_err(|error| encoding_failure(&self.output_path, "shrinking a picture", error))?;
        Ok(shrunk)
    }

    /// Encodes one picture at its time, then writes the packets that are ready.
    fn encode(&mut self, timed_picture: TimedPicture) -> Result<(), MediaError> {
        let TimedPicture {
            time_in_stream_units,
            mut picture,
        } = timed_picture;
        picture.set_pts(Some(time_in_stream_units));
        self.encoder
            .send_frame(&picture)
            .map_err(|error| encoding_failure(&self.output_path, "encoding a frame", error))?;
        self.write_ready_packets()
    }

    fn write_ready_packets(&mut self) -> Result<(), MediaError> {
        let mut packet = ffmpeg::Packet::empty();
        while self.encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(self.stream_index);
            // Without a duration, the last frame gets none and players drop it.
            if packet.duration() <= 0 {
                packet.set_duration(self.frame_duration_in_time_base_units);
            }
            packet.rescale_ts(self.encoder_time_base, self.stream_time_base);
            packet
                .write_interleaved(&mut self.output)
                .map_err(|error| {
                    encoding_failure(&self.output_path, "writing video data", error)
                })?;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<(), MediaError> {
        self.encoder
            .send_eof()
            .map_err(|error| encoding_failure(&self.output_path, "finishing the video", error))?;
        self.write_ready_packets()?;
        self.output.write_trailer().map_err(|error| {
            encoding_failure(&self.output_path, "writing the end of the file", error)
        })
    }
}

/// What an encoder is opened with.
struct EncoderRequest<'settings> {
    settings: &'settings VideoProxySettings,
    time_base: Rational,
    frame_rate: Option<Rational>,
    color_space: YuvColorSpace,
    needs_global_header: bool,
}

/// Configures and opens one encoder; `None` if it cannot start here.
fn open_proxy_encoder(
    codec: ffmpeg::Codec,
    encoder_name: &str,
    request: &EncoderRequest<'_>,
) -> Option<ffmpeg::encoder::Video> {
    let mut encoder = ffmpeg::codec::context::Context::new_with_codec(codec)
        .encoder()
        .video()
        .ok()?;
    let PictureSize {
        width_in_pixels: width,
        height_in_pixels: height,
    } = request.settings.picture_size;
    encoder.set_width(width);
    encoder.set_height(height);
    encoder.set_format(Pixel::YUV420P);
    encoder.set_time_base(request.time_base);
    encoder.set_frame_rate(request.frame_rate);
    encoder.set_gop(request.settings.frames_per_keyframe);
    // Without B-frames, frames come out in order and a jump decodes fewer of them.
    encoder.set_max_b_frames(0);
    // Every core: FFmpeg's default for encoders is a single thread.
    encoder.set_threading(ffmpeg::codec::threading::Config::kind(
        ffmpeg::codec::threading::Type::Frame,
    ));
    let (matrix, range) = color_tags_of(request.color_space);
    encoder.set_colorspace(matrix);
    encoder.set_color_range(range);
    if request.needs_global_header {
        encoder.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
    }
    let mut options = ffmpeg::Dictionary::new();
    if encoder_name == "libx264" {
        options.set("preset", LIBX264_PRESET);
        options.set("tune", LIBX264_TUNING);
        options.set("crf", LIBX264_CONSTANT_RATE_FACTOR);
    } else {
        let frames_per_second = request
            .frame_rate
            .map_or(FALLBACK_FRAMES_PER_SECOND as f64, f64::from);
        let bits_per_second =
            f64::from(width) * f64::from(height) * frames_per_second * OPENH264_BITS_PER_PIXEL;
        encoder.set_bit_rate(bits_per_second as usize);
    }
    encoder.open_with(options).ok()
}

/// The colour space the source's pictures are read in (`picture_conversion.rs`),
/// which the proxy is tagged with. The scaler keeps the values as they are,
/// except the old full-range `yuvj` formats, which it turns into limited range.
fn proxy_color_space_of(picture: &ffmpeg::frame::Video) -> YuvColorSpace {
    let mut color_space = color_space_of(picture);
    if is_full_range_format(picture.format()) {
        color_space.range = YuvRange::Limited;
    }
    color_space
}

/// FFmpeg's tags for a colour space. Both matrices are tagged explicitly: an
/// untagged proxy smaller than 720 lines would otherwise be read as BT.601.
fn color_tags_of(color_space: YuvColorSpace) -> (ffmpeg::color::Space, ffmpeg::color::Range) {
    let matrix = match color_space.matrix {
        YuvMatrix::Bt709 => ffmpeg::color::Space::BT709,
        YuvMatrix::Bt601 => ffmpeg::color::Space::SMPTE170M,
    };
    let range = match color_space.range {
        YuvRange::Limited => ffmpeg::color::Range::MPEG,
        YuvRange::Full => ffmpeg::color::Range::JPEG,
    };
    (matrix, range)
}

/// One frame period in `time_base` units (at least one unit).
fn frame_duration_in_units(time_base: Rational, frame_rate: Option<Rational>) -> i64 {
    let (frames, seconds) = frame_rate
        .filter(|rate| rate.numerator() > 0 && rate.denominator() > 0)
        .map_or((FALLBACK_FRAMES_PER_SECOND, 1), |rate| {
            (i64::from(rate.numerator()), i64::from(rate.denominator()))
        });
    let (units_numerator, units_denominator) = (
        i64::from(time_base.numerator()).max(1),
        i64::from(time_base.denominator()).max(1),
    );
    // seconds per frame / seconds per unit = (seconds / frames) × (denominator / numerator).
    ((seconds * units_denominator) / (frames * units_numerator)).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_layouts_are_read_from_pixel_formats() {
        assert_eq!(
            sample_layout_of(Pixel::YUV420P),
            (8, ChromaSubsampling::Yuv420)
        );
        assert_eq!(
            sample_layout_of(Pixel::NV12),
            (8, ChromaSubsampling::Yuv420)
        );
        assert_eq!(
            sample_layout_of(Pixel::YUV420P10LE),
            (10, ChromaSubsampling::Yuv420)
        );
        assert_eq!(
            sample_layout_of(Pixel::YUV422P10LE),
            (10, ChromaSubsampling::Yuv422)
        );
        assert_eq!(
            sample_layout_of(Pixel::YUV444P12LE),
            (12, ChromaSubsampling::Yuv444)
        );
        assert_eq!(
            sample_layout_of(Pixel::P010LE),
            (10, ChromaSubsampling::Yuv420)
        );
        assert_eq!(
            sample_layout_of(Pixel::GRAY8),
            (8, ChromaSubsampling::Other)
        );
        assert_eq!(sample_layout_of(Pixel::RGBA), (8, ChromaSubsampling::Other));
        assert_eq!(
            sample_layout_of(Pixel::None),
            (8, ChromaSubsampling::Yuv420)
        );
    }

    #[test]
    fn frame_durations_follow_the_time_base() {
        let thirty = Some(Rational::new(30, 1));
        assert_eq!(
            frame_duration_in_units(Rational::new(1, 15_360), thirty),
            512
        );
        assert_eq!(
            frame_duration_in_units(Rational::new(1, 90_000), thirty),
            3_000
        );
        let ntsc = Some(Rational::new(30_000, 1_001));
        assert_eq!(
            frame_duration_in_units(Rational::new(1, 30_000), ntsc),
            1_001
        );
        assert_eq!(frame_duration_in_units(Rational::new(1, 600), None), 20);
    }

    #[test]
    fn proxies_are_always_tagged_with_their_matrix() {
        let (matrix, range) = color_tags_of(YuvColorSpace {
            matrix: YuvMatrix::Bt601,
            range: YuvRange::Full,
        });
        assert_eq!(matrix, ffmpeg::color::Space::SMPTE170M);
        assert_eq!(range, ffmpeg::color::Range::JPEG);
    }
}
