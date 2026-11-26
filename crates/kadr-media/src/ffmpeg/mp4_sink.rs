//! Writing the rendered video and sound into an MP4 file (H.264 + AAC).
//!
//! The video encoder is chosen at run time: hardware encoders first (NVIDIA
//! NVENC, Intel Quick Sync, AMD AMF, Apple VideoToolbox) because they are much
//! faster, then the software encoder libx264, which works everywhere. An
//! encoder that cannot start on this machine is simply skipped.

use std::path::{Path, PathBuf};

use ffmpeg_next as ffmpeg;
use ffmpeg_next::Rational;
use kadr_core::time::FrameRate;

use crate::audio_buffer::{AudioBuffer, STEREO_CHANNEL_COUNT};
use crate::backend::{EncodingSpeed, ExportSink, ExportSinkSettings, VideoEncoderPreference};
use crate::error::MediaError;
use crate::video_frame::VideoFrame;

/// Hardware encoders, tried in this order.
const HARDWARE_VIDEO_ENCODERS: [&str; 4] =
    ["h264_nvenc", "h264_qsv", "h264_amf", "h264_videotoolbox"];
/// Software encoders, tried after the hardware ones (or alone).
const SOFTWARE_VIDEO_ENCODERS: [&str; 2] = ["libx264", "libopenh264"];

/// Bits per pixel per frame for hardware encoders, which are driven by bit rate.
/// 0.1 gives about 6 Mbit/s for 1080p at 30 fps.
const HARDWARE_BITS_PER_PIXEL: f64 = 0.1;
/// Lowest bit rate given to hardware encoders.
const MINIMUM_VIDEO_BIT_RATE: usize = 1_000_000;
/// AAC bit rate for stereo.
const AUDIO_BIT_RATE: usize = 192_000;
/// One keyframe every this many seconds, so the file can be cut and seeked easily.
const SECONDS_BETWEEN_KEYFRAMES: u32 = 2;
/// Bytes in one RGBA pixel.
const BYTES_PER_RGBA_PIXEL: usize = 4;

/// An MP4 file being written.
pub(crate) struct Mp4ExportSink {
    output_path: PathBuf,
    output: ffmpeg::format::context::Output,
    video: VideoTrackWriter,
    audio: AudioTrackWriter,
}

/// The video half of the file.
struct VideoTrackWriter {
    encoder: ffmpeg::encoder::Video,
    encoder_name: String,
    stream_index: usize,
    encoder_time_base: Rational,
    stream_time_base: Rational,
    pixel_format: ffmpeg::format::Pixel,
    width: u32,
    height: u32,
    rgba_staging_picture: ffmpeg::frame::Video,
    rgba_to_encoder_converter: ffmpeg::software::scaling::Context,
    written_frame_count: i64,
}

/// The sound half of the file.
struct AudioTrackWriter {
    encoder: ffmpeg::encoder::Audio,
    stream_index: usize,
    encoder_time_base: Rational,
    stream_time_base: Rational,
    sample_rate_in_hertz: u32,
    samples_per_encoder_frame: usize,
    pending_interleaved_samples: Vec<f32>,
    written_sample_count: i64,
}

fn encoding_failure(output_path: &Path, step: &str, error: impl std::fmt::Display) -> MediaError {
    MediaError::EncodingFailed {
        file_path: output_path.to_path_buf(),
        reason: format!("{step}: {error}"),
    }
}

impl Mp4ExportSink {
    /// Creates the file, starts the encoders and writes the file header.
    pub(crate) fn create(settings: &ExportSinkSettings) -> Result<Self, MediaError> {
        validate_settings(settings)?;
        let output_path = settings.output_path.clone();
        let mut output = ffmpeg::format::output_as(&output_path, "mp4")
            .map_err(|error| encoding_failure(&output_path, "creating the file", error))?;
        let needs_global_header = output
            .format()
            .flags()
            .contains(ffmpeg::format::Flags::GLOBAL_HEADER);

        let video = VideoTrackWriter::open_first_working_encoder(
            &mut output,
            settings,
            needs_global_header,
        )?;
        let audio = AudioTrackWriter::open(&mut output, settings, needs_global_header)?;

        // "faststart" moves the index to the beginning, so the video can play
        // while it downloads.
        let mut muxer_options = ffmpeg::Dictionary::new();
        muxer_options.set("movflags", "+faststart");
        output
            .write_header_with(muxer_options)
            .map_err(|error| encoding_failure(&output_path, "writing the file header", error))?;

        let mut sink = Self {
            output_path,
            output,
            video,
            audio,
        };
        // The muxer may change the stream time bases while writing the header.
        sink.video.stream_time_base = sink.stream_time_base(sink.video.stream_index)?;
        sink.audio.stream_time_base = sink.stream_time_base(sink.audio.stream_index)?;
        Ok(sink)
    }

    fn stream_time_base(&self, stream_index: usize) -> Result<Rational, MediaError> {
        self.output
            .stream(stream_index)
            .map(|stream| stream.time_base())
            .ok_or_else(|| {
                encoding_failure(
                    &self.output_path,
                    "reading the stream time base",
                    "missing stream",
                )
            })
    }
}

fn validate_settings(settings: &ExportSinkSettings) -> Result<(), MediaError> {
    let (width, height) = (settings.width_in_pixels, settings.height_in_pixels);
    if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
        return Err(MediaError::InvalidRequest {
            reason: format!("the video size must be even and non-zero, but is {width}×{height}"),
        });
    }
    if settings.audio_sample_rate_in_hertz == 0 {
        return Err(MediaError::InvalidRequest {
            reason: "the audio sample rate is zero".to_string(),
        });
    }
    Ok(())
}

/// The encoders to try, in order.
fn video_encoder_candidates(preference: VideoEncoderPreference) -> Vec<&'static str> {
    match preference {
        VideoEncoderPreference::HardwareFirst => HARDWARE_VIDEO_ENCODERS
            .iter()
            .chain(SOFTWARE_VIDEO_ENCODERS.iter())
            .copied()
            .collect(),
        VideoEncoderPreference::SoftwareOnly => SOFTWARE_VIDEO_ENCODERS.to_vec(),
    }
}

/// The frame duration as an FFmpeg time base (for 30000/1001 fps: 1001/30000 s).
fn frame_duration_time_base(frame_rate: FrameRate) -> Rational {
    Rational::new(
        frame_rate.denominator() as i32,
        frame_rate.numerator() as i32,
    )
}

/// Encoder options for the requested speed / quality trade-off.
fn video_encoder_options(
    encoder_name: &str,
    encoding_speed: EncodingSpeed,
) -> ffmpeg::Dictionary<'static> {
    let mut options = ffmpeg::Dictionary::new();
    let is_fast = encoding_speed == EncodingSpeed::Fast;
    match encoder_name {
        "libx264" => {
            options.set("preset", if is_fast { "veryfast" } else { "medium" });
            options.set("crf", if is_fast { "23" } else { "20" });
        }
        "h264_nvenc" => options.set("preset", if is_fast { "p2" } else { "p5" }),
        "h264_qsv" => options.set("preset", if is_fast { "veryfast" } else { "medium" }),
        "h264_amf" => options.set("quality", if is_fast { "speed" } else { "balanced" }),
        _ => {}
    }
    options
}

/// The bit rate given to encoders driven by bit rate (hardware and openh264).
fn video_bit_rate(settings: &ExportSinkSettings) -> usize {
    let pixels_per_second = f64::from(settings.width_in_pixels)
        * f64::from(settings.height_in_pixels)
        * settings.frame_rate.as_frames_per_second();
    ((pixels_per_second * HARDWARE_BITS_PER_PIXEL) as usize).max(MINIMUM_VIDEO_BIT_RATE)
}

/// YUV 4:2:0 if the encoder accepts it (most compatible), otherwise NV12.
fn choose_pixel_format(codec: &ffmpeg::codec::Video) -> ffmpeg::format::Pixel {
    let supported_formats: Vec<ffmpeg::format::Pixel> =
        codec.formats().map(Iterator::collect).unwrap_or_default();
    [ffmpeg::format::Pixel::YUV420P, ffmpeg::format::Pixel::NV12]
        .into_iter()
        .find(|preferred_format| {
            supported_formats.is_empty() || supported_formats.contains(preferred_format)
        })
        .unwrap_or(ffmpeg::format::Pixel::YUV420P)
}

impl VideoTrackWriter {
    /// Tries each candidate encoder until one starts, then adds the video stream.
    fn open_first_working_encoder(
        output: &mut ffmpeg::format::context::Output,
        settings: &ExportSinkSettings,
        needs_global_header: bool,
    ) -> Result<Self, MediaError> {
        let mut tried_encoder_names = Vec::new();
        // An encoder whose hardware is missing (no NVIDIA card, for example)
        // complains on stderr. That is expected while probing, so FFmpeg is
        // kept quiet meanwhile.
        ffmpeg::log::set_level(ffmpeg::log::Level::Fatal);
        let mut working_encoder = None;
        for encoder_name in video_encoder_candidates(settings.video_encoder_preference) {
            let Some(codec) = ffmpeg::encoder::find_by_name(encoder_name) else {
                continue;
            };
            tried_encoder_names.push(encoder_name.to_string());
            if let Some(opened) =
                open_video_encoder(codec, encoder_name, settings, needs_global_header)
            {
                working_encoder = Some((codec, encoder_name, opened));
                break;
            }
        }
        ffmpeg::log::set_level(ffmpeg::log::Level::Error);
        match working_encoder {
            Some((codec, encoder_name, opened)) => {
                Self::add_stream(output, settings, codec, encoder_name, opened)
            }
            None => Err(MediaError::NoUsableVideoEncoder {
                tried_encoder_names,
            }),
        }
    }

    fn add_stream(
        output: &mut ffmpeg::format::context::Output,
        settings: &ExportSinkSettings,
        codec: ffmpeg::Codec,
        encoder_name: &str,
        (encoder, pixel_format): (ffmpeg::encoder::Video, ffmpeg::format::Pixel),
    ) -> Result<Self, MediaError> {
        let output_path = settings.output_path.as_path();
        let encoder_time_base = frame_duration_time_base(settings.frame_rate);
        let mut stream = output
            .add_stream(codec)
            .map_err(|error| encoding_failure(output_path, "adding the video stream", error))?;
        stream.set_parameters(&encoder);
        stream.set_time_base(encoder_time_base);
        let stream_index = stream.index();

        let (width, height) = (settings.width_in_pixels, settings.height_in_pixels);
        let rgba_to_encoder_converter = ffmpeg::software::scaling::Context::get(
            ffmpeg::format::Pixel::RGBA,
            width,
            height,
            pixel_format,
            width,
            height,
            ffmpeg::software::scaling::Flags::BILINEAR,
        )
        .map_err(|error| encoding_failure(output_path, "preparing the colour conversion", error))?;

        Ok(Self {
            encoder,
            encoder_name: encoder_name.to_string(),
            stream_index,
            encoder_time_base,
            stream_time_base: encoder_time_base,
            pixel_format,
            width,
            height,
            rgba_staging_picture: ffmpeg::frame::Video::new(
                ffmpeg::format::Pixel::RGBA,
                width,
                height,
            ),
            rgba_to_encoder_converter,
            written_frame_count: 0,
        })
    }

    /// Converts and encodes one frame, then writes the packets that are ready.
    fn write_frame(
        &mut self,
        frame: &VideoFrame,
        output: &mut ffmpeg::format::context::Output,
        output_path: &Path,
    ) -> Result<(), MediaError> {
        if frame.width() != self.width || frame.height() != self.height {
            return Err(MediaError::InvalidRequest {
                reason: format!(
                    "the frame is {}×{} but the video is {}×{}",
                    frame.width(),
                    frame.height(),
                    self.width,
                    self.height
                ),
            });
        }
        copy_image_into_rgba_picture(frame, &mut self.rgba_staging_picture);
        // A new picture each time: the encoder may still hold the previous one.
        let mut encoder_picture =
            ffmpeg::frame::Video::new(self.pixel_format, self.width, self.height);
        self.rgba_to_encoder_converter
            .run(&self.rgba_staging_picture, &mut encoder_picture)
            .map_err(|error| encoding_failure(output_path, "converting colours", error))?;
        encoder_picture.set_pts(Some(self.written_frame_count));
        self.written_frame_count += 1;
        self.encoder
            .send_frame(&encoder_picture)
            .map_err(|error| encoding_failure(output_path, "encoding a video frame", error))?;
        self.write_ready_packets(output, output_path)
    }

    fn write_ready_packets(
        &mut self,
        output: &mut ffmpeg::format::context::Output,
        output_path: &Path,
    ) -> Result<(), MediaError> {
        let mut packet = ffmpeg::Packet::empty();
        while self.encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(self.stream_index);
            // Every frame lasts one frame period (one unit of the encoder time
            // base). Without it, the last frame gets a zero duration and MP4
            // players drop it.
            if packet.duration() <= 0 {
                packet.set_duration(1);
            }
            packet.rescale_ts(self.encoder_time_base, self.stream_time_base);
            packet
                .write_interleaved(output)
                .map_err(|error| encoding_failure(output_path, "writing video data", error))?;
        }
        Ok(())
    }

    fn finish(
        &mut self,
        output: &mut ffmpeg::format::context::Output,
        output_path: &Path,
    ) -> Result<(), MediaError> {
        self.encoder
            .send_eof()
            .map_err(|error| encoding_failure(output_path, "finishing the video", error))?;
        self.write_ready_packets(output, output_path)
    }
}

/// Configures and opens one video encoder; `None` if it cannot start here.
fn open_video_encoder(
    codec: ffmpeg::Codec,
    encoder_name: &str,
    settings: &ExportSinkSettings,
    needs_global_header: bool,
) -> Option<(ffmpeg::encoder::Video, ffmpeg::format::Pixel)> {
    let pixel_format = choose_pixel_format(&codec.video().ok()?);
    let mut encoder = ffmpeg::codec::context::Context::new_with_codec(codec)
        .encoder()
        .video()
        .ok()?;
    encoder.set_width(settings.width_in_pixels);
    encoder.set_height(settings.height_in_pixels);
    encoder.set_format(pixel_format);
    encoder.set_time_base(frame_duration_time_base(settings.frame_rate));
    encoder.set_frame_rate(Some(Rational::new(
        settings.frame_rate.numerator() as i32,
        settings.frame_rate.denominator() as i32,
    )));
    let frames_per_keyframe = (settings.frame_rate.as_frames_per_second().round() as u32).max(1)
        * SECONDS_BETWEEN_KEYFRAMES;
    encoder.set_gop(frames_per_keyframe);
    if encoder_name != "libx264" {
        encoder.set_bit_rate(video_bit_rate(settings));
    }
    if needs_global_header {
        encoder.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
    }
    let opened_encoder = encoder
        .open_with(video_encoder_options(encoder_name, settings.encoding_speed))
        .ok()?;
    Some((opened_encoder, pixel_format))
}

/// Copies a tightly packed RGBA image into an FFmpeg picture whose rows may be padded.
fn copy_image_into_rgba_picture(image: &VideoFrame, picture: &mut ffmpeg::frame::Video) {
    let row_length_in_bytes = image.width() as usize * BYTES_PER_RGBA_PIXEL;
    let stride_in_bytes = picture.stride(0);
    let plane = picture.data_mut(0);
    for (row_index, image_row) in image.as_raw().chunks_exact(row_length_in_bytes).enumerate() {
        let row_start = row_index * stride_in_bytes;
        plane[row_start..row_start + row_length_in_bytes].copy_from_slice(image_row);
    }
}

impl AudioTrackWriter {
    fn open(
        output: &mut ffmpeg::format::context::Output,
        settings: &ExportSinkSettings,
        needs_global_header: bool,
    ) -> Result<Self, MediaError> {
        let output_path = settings.output_path.as_path();
        let failure = |step: &str, error: ffmpeg::Error| encoding_failure(output_path, step, error);
        let codec = ffmpeg::encoder::find(ffmpeg::codec::Id::AAC)
            .ok_or_else(|| failure("finding the AAC encoder", ffmpeg::Error::EncoderNotFound))?;
        let mut encoder = ffmpeg::codec::context::Context::new_with_codec(codec)
            .encoder()
            .audio()
            .map_err(|error| failure("preparing the AAC encoder", error))?;
        let sample_rate = settings.audio_sample_rate_in_hertz;
        let encoder_time_base = Rational::new(1, sample_rate as i32);
        encoder.set_rate(sample_rate as i32);
        encoder.set_channel_layout(ffmpeg::ChannelLayout::STEREO);
        encoder.set_format(ffmpeg::format::Sample::F32(
            ffmpeg::format::sample::Type::Planar,
        ));
        encoder.set_bit_rate(AUDIO_BIT_RATE);
        encoder.set_time_base(encoder_time_base);
        if needs_global_header {
            encoder.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
        }
        let encoder = encoder
            .open_as(codec)
            .map_err(|error| failure("starting the AAC encoder", error))?;
        let samples_per_encoder_frame = (encoder.frame_size() as usize).max(1);

        let mut stream = output
            .add_stream(codec)
            .map_err(|error| failure("adding the audio stream", error))?;
        stream.set_parameters(&encoder);
        stream.set_time_base(encoder_time_base);
        let stream_index = stream.index();

        Ok(Self {
            encoder,
            stream_index,
            encoder_time_base,
            stream_time_base: encoder_time_base,
            sample_rate_in_hertz: sample_rate,
            samples_per_encoder_frame,
            pending_interleaved_samples: Vec::new(),
            written_sample_count: 0,
        })
    }

    fn write(
        &mut self,
        audio: &AudioBuffer,
        output: &mut ffmpeg::format::context::Output,
        output_path: &Path,
    ) -> Result<(), MediaError> {
        if audio.sample_rate_in_hertz() != self.sample_rate_in_hertz {
            return Err(MediaError::InvalidRequest {
                reason: format!(
                    "the sound is at {} Hz but the file expects {} Hz",
                    audio.sample_rate_in_hertz(),
                    self.sample_rate_in_hertz
                ),
            });
        }
        self.pending_interleaved_samples
            .extend_from_slice(audio.interleaved_samples());
        let values_per_encoder_frame = self.samples_per_encoder_frame * STEREO_CHANNEL_COUNT;
        while self.pending_interleaved_samples.len() >= values_per_encoder_frame {
            self.encode_pending_samples(self.samples_per_encoder_frame, output, output_path)?;
        }
        Ok(())
    }

    /// Encodes the first `sample_count` pending samples (per channel).
    fn encode_pending_samples(
        &mut self,
        sample_count: usize,
        output: &mut ffmpeg::format::context::Output,
        output_path: &Path,
    ) -> Result<(), MediaError> {
        let mut planar_frame = ffmpeg::frame::Audio::new(
            ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar),
            sample_count,
            ffmpeg::ChannelLayout::STEREO,
        );
        planar_frame.set_rate(self.sample_rate_in_hertz);
        for channel in 0..STEREO_CHANNEL_COUNT {
            let plane = planar_frame.plane_mut::<f32>(channel);
            for (sample_index, sample) in plane.iter_mut().take(sample_count).enumerate() {
                *sample =
                    self.pending_interleaved_samples[sample_index * STEREO_CHANNEL_COUNT + channel];
            }
        }
        self.pending_interleaved_samples
            .drain(..sample_count * STEREO_CHANNEL_COUNT);
        planar_frame.set_pts(Some(self.written_sample_count));
        self.written_sample_count += sample_count as i64;
        self.encoder
            .send_frame(&planar_frame)
            .map_err(|error| encoding_failure(output_path, "encoding sound", error))?;
        self.write_ready_packets(output, output_path)
    }

    fn write_ready_packets(
        &mut self,
        output: &mut ffmpeg::format::context::Output,
        output_path: &Path,
    ) -> Result<(), MediaError> {
        let mut packet = ffmpeg::Packet::empty();
        while self.encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(self.stream_index);
            packet.rescale_ts(self.encoder_time_base, self.stream_time_base);
            packet
                .write_interleaved(output)
                .map_err(|error| encoding_failure(output_path, "writing sound data", error))?;
        }
        Ok(())
    }

    fn finish(
        &mut self,
        output: &mut ffmpeg::format::context::Output,
        output_path: &Path,
    ) -> Result<(), MediaError> {
        let remaining_sample_count = self.pending_interleaved_samples.len() / STEREO_CHANNEL_COUNT;
        if remaining_sample_count > 0 {
            self.encode_pending_samples(remaining_sample_count, output, output_path)?;
        }
        self.encoder
            .send_eof()
            .map_err(|error| encoding_failure(output_path, "finishing the sound", error))?;
        self.write_ready_packets(output, output_path)
    }
}

impl ExportSink for Mp4ExportSink {
    fn video_encoder_name(&self) -> &str {
        &self.video.encoder_name
    }

    fn write_video_frame(&mut self, frame: &VideoFrame) -> Result<(), MediaError> {
        self.video
            .write_frame(frame, &mut self.output, &self.output_path)
    }

    fn write_audio(&mut self, audio: &AudioBuffer) -> Result<(), MediaError> {
        self.audio.write(audio, &mut self.output, &self.output_path)
    }

    fn finish(mut self: Box<Self>) -> Result<(), MediaError> {
        let output_path = self.output_path.clone();
        self.video.finish(&mut self.output, &output_path)?;
        self.audio.finish(&mut self.output, &output_path)?;
        self.output
            .write_trailer()
            .map_err(|error| encoding_failure(&output_path, "writing the end of the file", error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn software_only_never_tries_hardware_encoders() {
        assert_eq!(
            video_encoder_candidates(VideoEncoderPreference::SoftwareOnly),
            vec!["libx264", "libopenh264"]
        );
        assert_eq!(
            video_encoder_candidates(VideoEncoderPreference::HardwareFirst)[0],
            "h264_nvenc"
        );
    }

    #[test]
    fn ntsc_frame_rate_gives_an_exact_time_base() {
        let time_base = frame_duration_time_base(FrameRate::FPS_29_97);
        assert_eq!(
            (time_base.numerator(), time_base.denominator()),
            (1_001, 30_000)
        );
    }
}
