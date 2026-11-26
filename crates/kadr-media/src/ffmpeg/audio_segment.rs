//! Decoding a range of a file's sound, at any speed, to stereo.
//!
//! The decoded sound goes through an FFmpeg filter graph that does the whole
//! job in one place:
//!
//! ```text
//! decoder → atrim (keep the range) → asetpts (restart at 0)
//!         → atempo (change speed, keep pitch) → aresample (sample rate)
//!         → aformat (32-bit float, stereo) → result
//! ```
//!
//! The result is then padded or cut to the exact length requested, so a clip's
//! sound always lasts exactly as long as the clip.

use std::path::Path;

use ffmpeg_next as ffmpeg;
use ffmpeg_next::Rational;
use kadr_core::clip::PlaybackSpeed;
use kadr_core::time::{TimeSpan, Timestamp};

use super::time_conversion::{
    microseconds_from_timestamp, seconds_for_filter_argument, timestamp_from_stream_units,
};
use super::{decoding_failure, open_input_file};
use crate::audio_buffer::{AudioBuffer, STEREO_CHANNEL_COUNT};
use crate::backend::AudioSegmentRequest;
use crate::error::MediaError;

/// Decoding continues this long past the end of the range, so the trim filter
/// receives every sample it needs even if packets are slightly out of order.
const DECODING_MARGIN_AFTER_RANGE: TimeSpan = TimeSpan::from_milliseconds(500);

/// One `atempo` filter only accepts speeds from 0.5 to 100; slower speeds are
/// made of several filters in a row.
const SLOWEST_SINGLE_TEMPO: f64 = 0.5;

/// Bytes in one 32-bit float sample.
const BYTES_PER_FLOAT_SAMPLE: usize = 4;

/// Decodes the requested range of a file's sound.
pub(crate) fn decode_audio_segment(
    request: &AudioSegmentRequest,
) -> Result<AudioBuffer, MediaError> {
    validate_request(request)?;
    let file_path = request.file_path.as_path();
    let mut input = open_input_file(file_path)?;
    let stream = input
        .streams()
        .best(ffmpeg::media::Type::Audio)
        .ok_or_else(|| MediaError::NoAudioStream {
            file_path: file_path.to_path_buf(),
        })?;
    let stream_index = stream.index();
    let stream_time_base = stream.time_base();
    let stream_start_in_stream_units = stream.start_time().max(0);
    let mut decoder = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
        .and_then(|context| context.decoder().audio())
        .map_err(|error| decoding_failure(file_path, "opening the audio decoder", error))?;
    decoder.set_packet_time_base(stream_time_base);

    if request.source_start > Timestamp::ZERO {
        let stream_start =
            timestamp_from_stream_units(stream_start_in_stream_units, stream_time_base);
        let target = microseconds_from_timestamp(
            stream_start + TimeSpan::from_ticks(request.source_start.ticks()),
        );
        input.seek(target, ..target).map_err(|error| {
            decoding_failure(file_path, "jumping to the start of the range", error)
        })?;
    }

    let mut filter_graph = build_filter_graph(&decoder, stream_time_base, request, file_path)?;
    let mut segment_decoder = SegmentDecoder {
        file_path,
        decoder,
        stream_time_base,
        stream_start_in_stream_units,
        stop_after: request.source_end + DECODING_MARGIN_AFTER_RANGE,
        interleaved_samples: Vec::with_capacity(
            request.output_samples_per_channel * STEREO_CHANNEL_COUNT,
        ),
        filters_have_ended: false,
    };

    for (stream, packet) in input.packets() {
        if stream.index() != stream_index {
            continue;
        }
        segment_decoder
            .decoder
            .send_packet(&packet)
            .map_err(|error| decoding_failure(file_path, "decoding an audio packet", error))?;
        if segment_decoder.move_decoded_frames_through(&mut filter_graph)? {
            break;
        }
    }
    segment_decoder.finish(&mut filter_graph)?;

    let mut result = AudioBuffer::from_interleaved_stereo(
        request.output_sample_rate_in_hertz,
        segment_decoder.interleaved_samples,
    );
    result.resize(request.output_samples_per_channel);
    Ok(result)
}

fn validate_request(request: &AudioSegmentRequest) -> Result<(), MediaError> {
    if request.source_start >= request.source_end {
        return Err(MediaError::InvalidRequest {
            reason: format!(
                "the sound range starts at {} and ends at {}",
                request.source_start, request.source_end
            ),
        });
    }
    if request.output_sample_rate_in_hertz == 0 {
        return Err(MediaError::InvalidRequest {
            reason: "the output sample rate is zero".to_string(),
        });
    }
    Ok(())
}

/// The decoding state of one segment.
struct SegmentDecoder<'path> {
    file_path: &'path Path,
    decoder: ffmpeg::decoder::Audio,
    stream_time_base: Rational,
    stream_start_in_stream_units: i64,
    stop_after: Timestamp,
    interleaved_samples: Vec<f32>,
    /// Set once the trim filter has passed the end of the range: the filters
    /// then refuse any more sound, which is expected.
    filters_have_ended: bool,
}

impl SegmentDecoder<'_> {
    /// Sends every decoded frame through the filters. Returns `true` once the
    /// decoding has gone past the end of the range or the filters have ended.
    fn move_decoded_frames_through(
        &mut self,
        filter_graph: &mut ffmpeg::filter::Graph,
    ) -> Result<bool, MediaError> {
        let mut decoded_frame = ffmpeg::frame::Audio::empty();
        let mut is_past_the_range = false;
        while self.decoder.receive_frame(&mut decoded_frame).is_ok() {
            // Timestamps relative to the start of the stream, as the trim filter expects.
            let presentation_time = decoded_frame
                .timestamp()
                .unwrap_or(self.stream_start_in_stream_units);
            let relative_time = presentation_time - self.stream_start_in_stream_units;
            decoded_frame.set_pts(Some(relative_time));
            is_past_the_range |=
                timestamp_from_stream_units(relative_time, self.stream_time_base) > self.stop_after;

            if self.filters_have_ended {
                continue;
            }
            match filter_input(filter_graph, self.file_path)?
                .source()
                .add(&decoded_frame)
            {
                Ok(()) => {}
                Err(ffmpeg::Error::Eof) => self.filters_have_ended = true,
                Err(error) => {
                    return Err(decoding_failure(
                        self.file_path,
                        "filtering the sound",
                        error,
                    ));
                }
            }
            self.collect_filtered_samples(filter_graph)?;
        }
        Ok(is_past_the_range || self.filters_have_ended)
    }

    /// Drains the decoder and the filters.
    fn finish(&mut self, filter_graph: &mut ffmpeg::filter::Graph) -> Result<(), MediaError> {
        // The decoder may refuse the end-of-file signal if it already received it.
        let _ = self.decoder.send_eof();
        self.move_decoded_frames_through(filter_graph)?;
        if !self.filters_have_ended {
            match filter_input(filter_graph, self.file_path)?.source().flush() {
                Ok(()) | Err(ffmpeg::Error::Eof) => {}
                Err(error) => {
                    return Err(decoding_failure(
                        self.file_path,
                        "finishing the sound filters",
                        error,
                    ));
                }
            }
        }
        self.collect_filtered_samples(filter_graph)
    }

    /// Appends every frame the filters have ready.
    fn collect_filtered_samples(
        &mut self,
        filter_graph: &mut ffmpeg::filter::Graph,
    ) -> Result<(), MediaError> {
        let mut filtered_frame = ffmpeg::frame::Audio::empty();
        let mut output = filter_graph.get("out").ok_or_else(|| {
            decoding_failure(self.file_path, "filtering the sound", "missing output")
        })?;
        while output.sink().frame(&mut filtered_frame).is_ok() {
            let interleaved_value_count = filtered_frame.samples() * STEREO_CHANNEL_COUNT;
            let bytes = &filtered_frame.data(0)[..interleaved_value_count * BYTES_PER_FLOAT_SAMPLE];
            let (sample_bytes, _) = bytes.as_chunks::<BYTES_PER_FLOAT_SAMPLE>();
            self.interleaved_samples
                .extend(sample_bytes.iter().map(|bytes| f32::from_ne_bytes(*bytes)));
        }
        Ok(())
    }
}

/// The entry point of the filter graph.
fn filter_input(
    filter_graph: &mut ffmpeg::filter::Graph,
    file_path: &Path,
) -> Result<ffmpeg::filter::Context, MediaError> {
    filter_graph
        .get("in")
        .ok_or_else(|| decoding_failure(file_path, "filtering the sound", "missing input"))
}

/// Builds `in → atrim → asetpts → atempo… → aresample → aformat → out`.
fn build_filter_graph(
    decoder: &ffmpeg::decoder::Audio,
    stream_time_base: Rational,
    request: &AudioSegmentRequest,
    file_path: &Path,
) -> Result<ffmpeg::filter::Graph, MediaError> {
    let filter_failure =
        |error: ffmpeg::Error| decoding_failure(file_path, "preparing the sound filters", error);
    let mut filter_graph = ffmpeg::filter::Graph::new();

    let channel_layout = decoder.channel_layout();
    let channel_layout_bits = if channel_layout.bits() == 0 {
        ffmpeg::ChannelLayout::default(channel_layout.channels().max(1)).bits()
    } else {
        channel_layout.bits()
    };
    let source_arguments = format!(
        "time_base={}/{}:sample_rate={}:sample_fmt={}:channel_layout=0x{:x}",
        stream_time_base.numerator(),
        stream_time_base.denominator(),
        decoder.rate(),
        decoder.format().name(),
        channel_layout_bits
    );
    let source_filter = ffmpeg::filter::find("abuffer")
        .ok_or_else(|| filter_failure(ffmpeg::Error::FilterNotFound))?;
    let sink_filter = ffmpeg::filter::find("abuffersink")
        .ok_or_else(|| filter_failure(ffmpeg::Error::FilterNotFound))?;
    filter_graph
        .add(&source_filter, "in", &source_arguments)
        .map_err(filter_failure)?;
    filter_graph
        .add(&sink_filter, "out", "")
        .map_err(filter_failure)?;

    let filter_chain = describe_filter_chain(request);
    filter_graph
        .output("in", 0)
        .and_then(|parser| parser.input("out", 0))
        .and_then(|parser| parser.parse(&filter_chain))
        .map_err(filter_failure)?;
    filter_graph.validate().map_err(filter_failure)?;
    Ok(filter_graph)
}

/// The text description of the filters between the decoder and the result.
fn describe_filter_chain(request: &AudioSegmentRequest) -> String {
    format!(
        "atrim=start={}:end={},asetpts=PTS-STARTPTS{},aresample={},aformat=sample_fmts=flt:channel_layouts=stereo",
        seconds_for_filter_argument(request.source_start),
        seconds_for_filter_argument(request.source_end),
        describe_tempo_filters(request.speed),
        request.output_sample_rate_in_hertz
    )
}

/// `,atempo=…` filters giving `speed` (nothing at normal speed).
fn describe_tempo_filters(speed: PlaybackSpeed) -> String {
    let mut remaining_speed = speed.as_multiplier();
    if (remaining_speed - 1.0).abs() < f64::EPSILON {
        return String::new();
    }
    let mut tempo_filters = String::new();
    while remaining_speed < SLOWEST_SINGLE_TEMPO {
        tempo_filters.push_str(&format!(",atempo={SLOWEST_SINGLE_TEMPO}"));
        remaining_speed /= SLOWEST_SINGLE_TEMPO;
    }
    tempo_filters.push_str(&format!(",atempo={remaining_speed:.6}"));
    tempo_filters
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_speed_needs_no_tempo_filter() {
        assert_eq!(describe_tempo_filters(PlaybackSpeed::NORMAL), "");
    }

    #[test]
    fn slow_speeds_chain_several_tempo_filters() {
        let tenth_speed = PlaybackSpeed::from_multiplier(0.1).unwrap();
        // 0.5 × 0.5 × 0.5 × 0.8 = 0.1, every factor within atempo's 0.5–100 range.
        assert_eq!(
            describe_tempo_filters(tenth_speed),
            ",atempo=0.5,atempo=0.5,atempo=0.5,atempo=0.800000"
        );
        let faster = PlaybackSpeed::from_multiplier(1.5).unwrap();
        assert_eq!(describe_tempo_filters(faster), ",atempo=1.500000");
    }
}
