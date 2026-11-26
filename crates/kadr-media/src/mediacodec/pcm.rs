//! Raw sound from MediaCodec decoders, turned into Kadr's stereo floats, and
//! Kadr's sound turned into what the AAC encoder accepts.
//!
//! Decoders output interleaved PCM in the encoding announced by `pcm-encoding`
//! (16-bit by default), with as many channels as the file. Kadr works in stereo
//! 32-bit float, so surround sound is folded into two channels with the usual
//! weights (centre and surround channels at −3 dB, the low-frequency channel left
//! out), in Android's channel order: front left, front right, front centre,
//! low frequency, back left, back right, then side left, side right.

use crate::audio_buffer::STEREO_CHANNEL_COUNT;

/// `AudioFormat.ENCODING_PCM_16BIT`: signed 16-bit, the default.
pub(crate) const PCM_ENCODING_SIGNED_16_BIT: i32 = 2;
/// `AudioFormat.ENCODING_PCM_8BIT`: unsigned 8-bit.
const PCM_ENCODING_UNSIGNED_8_BIT: i32 = 3;
/// `AudioFormat.ENCODING_PCM_FLOAT`: 32-bit float.
const PCM_ENCODING_FLOAT: i32 = 4;
/// `AudioFormat.ENCODING_PCM_24BIT_PACKED`: signed 24-bit in three bytes.
const PCM_ENCODING_SIGNED_24_BIT_PACKED: i32 = 21;
/// `AudioFormat.ENCODING_PCM_32BIT`: signed 32-bit.
const PCM_ENCODING_SIGNED_32_BIT: i32 = 22;

/// Weight of a centre or surround channel folded into left or right: −3 dB.
const FOLDED_CHANNEL_WEIGHT: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// How the samples of a decoded buffer are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PcmEncoding {
    /// Unsigned 8-bit.
    Unsigned8Bit,
    /// Signed 16-bit, little endian.
    Signed16Bit,
    /// Signed 24-bit packed in three bytes, little endian.
    Signed24BitPacked,
    /// Signed 32-bit, little endian.
    Signed32Bit,
    /// 32-bit float, little endian.
    Float32,
}

impl PcmEncoding {
    /// The encoding of an Android `pcm-encoding` value; `None` when unknown.
    pub(crate) fn from_android_value(value: i32) -> Option<Self> {
        match value {
            PCM_ENCODING_UNSIGNED_8_BIT => Some(Self::Unsigned8Bit),
            PCM_ENCODING_SIGNED_16_BIT => Some(Self::Signed16Bit),
            PCM_ENCODING_SIGNED_24_BIT_PACKED => Some(Self::Signed24BitPacked),
            PCM_ENCODING_SIGNED_32_BIT => Some(Self::Signed32Bit),
            PCM_ENCODING_FLOAT => Some(Self::Float32),
            _ => None,
        }
    }

    /// Bytes per sample of one channel.
    pub(crate) fn bytes_per_sample(self) -> usize {
        match self {
            Self::Unsigned8Bit => 1,
            Self::Signed16Bit => 2,
            Self::Signed24BitPacked => 3,
            Self::Signed32Bit | Self::Float32 => 4,
        }
    }

    /// One sample as a float, nominally between −1 and 1.
    fn sample_to_float(self, bytes: &[u8]) -> f32 {
        match self {
            Self::Unsigned8Bit => (f32::from(bytes[0]) - 128.0) / 128.0,
            Self::Signed16Bit => f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 32_768.0,
            Self::Signed24BitPacked => {
                // Place the three bytes at the top of an i32 to keep the sign.
                let value = i32::from_le_bytes([0, bytes[0], bytes[1], bytes[2]]);
                value as f32 / 2_147_483_648.0
            }
            Self::Signed32Bit => {
                i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f32
                    / 2_147_483_648.0
            }
            Self::Float32 => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        }
    }
}

/// Converts interleaved PCM bytes with `channel_count` channels into interleaved
/// stereo floats. A trailing incomplete sample is ignored.
pub(crate) fn decode_to_stereo(
    bytes: &[u8],
    encoding: PcmEncoding,
    channel_count: usize,
) -> Vec<f32> {
    let channel_count = channel_count.max(1);
    let frame_size = encoding.bytes_per_sample() * channel_count;
    let mut stereo = Vec::with_capacity(bytes.len() / frame_size * STEREO_CHANNEL_COUNT);
    let mut channels = vec![0.0_f32; channel_count];
    for frame in bytes.chunks_exact(frame_size) {
        for (channel, sample_bytes) in frame.chunks_exact(encoding.bytes_per_sample()).enumerate() {
            channels[channel] = encoding.sample_to_float(sample_bytes);
        }
        let [left, right] = fold_to_stereo(&channels);
        stereo.extend([left, right]);
    }
    stereo
}

/// Folds one sample of every channel into left and right, keeping full-scale
/// sound within full scale.
fn fold_to_stereo(channels: &[f32]) -> [f32; 2] {
    // Index of each channel in Android's order, for the layouts phones produce.
    const FRONT_LEFT: usize = 0;
    const FRONT_RIGHT: usize = 1;
    match channels.len() {
        1 => [channels[0], channels[0]],
        2 => [channels[FRONT_LEFT], channels[FRONT_RIGHT]],
        channel_count => {
            let (centre, left_surround, right_surround) = match channel_count {
                // Front left, front right, centre.
                3 => (Some(2), None, None),
                // Quadraphonic: front left, front right, back left, back right.
                4 => (None, Some(2), Some(3)),
                // 5.0: front left, front right, centre, back left, back right.
                5 => (Some(2), Some(3), Some(4)),
                // 5.1 and up: …, centre, low frequency, back left, back right, …
                _ => (Some(2), Some(4), Some(5)),
            };
            let channel_value = |index: Option<usize>| index.map_or(0.0, |index| channels[index]);
            let centre_value = channel_value(centre) * FOLDED_CHANNEL_WEIGHT;
            let total_weight = 1.0
                + centre.map_or(0.0, |_| FOLDED_CHANNEL_WEIGHT)
                + left_surround.map_or(0.0, |_| FOLDED_CHANNEL_WEIGHT);
            let left = channels[FRONT_LEFT]
                + centre_value
                + channel_value(left_surround) * FOLDED_CHANNEL_WEIGHT;
            let right = channels[FRONT_RIGHT]
                + centre_value
                + channel_value(right_surround) * FOLDED_CHANNEL_WEIGHT;
            [left / total_weight, right / total_weight]
        }
    }
}

/// Converts float samples into signed 16-bit little-endian bytes, the input the
/// AAC encoder accepts on every Android version. Values are clamped to full scale.
pub(crate) fn encode_signed_16_bit(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|sample| {
            let value = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
            value.to_le_bytes()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sixteen_bit(values: &[i16]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    #[test]
    fn sixteen_bit_stereo_becomes_floats() {
        let bytes = sixteen_bit(&[16_384, -32_768, 0, 32_767]);

        let stereo = decode_to_stereo(&bytes, PcmEncoding::Signed16Bit, 2);

        assert_eq!(stereo, [0.5, -1.0, 0.0, 32_767.0 / 32_768.0]);
    }

    #[test]
    fn mono_is_copied_to_both_sides() {
        let bytes = sixteen_bit(&[16_384, -16_384]);

        assert_eq!(
            decode_to_stereo(&bytes, PcmEncoding::Signed16Bit, 1),
            [0.5, 0.5, -0.5, -0.5]
        );
    }

    #[test]
    fn surround_folds_centre_into_both_sides_and_drops_low_frequencies() {
        // 5.1: front left, front right, centre, low frequency, back left, back right.
        let only_centre = [0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        let only_low_frequency = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let only_back_left = [0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

        let [centre_left, centre_right] = fold_to_stereo(&only_centre);
        let [bass_left, bass_right] = fold_to_stereo(&only_low_frequency);
        let [back_left, back_right] = fold_to_stereo(&only_back_left);

        assert!((centre_left - centre_right).abs() < 1e-6 && centre_left > 0.2);
        assert_eq!((bass_left, bass_right), (0.0, 0.0));
        assert!(back_left > 0.2 && back_right == 0.0);
    }

    #[test]
    fn full_scale_surround_stays_within_full_scale() {
        let [left, right] = fold_to_stereo(&[1.0; 6]);
        assert!(left <= 1.0 + 1e-6 && right <= 1.0 + 1e-6);
    }

    #[test]
    fn other_encodings_are_scaled_to_full_scale() {
        let float_bytes: Vec<u8> = [0.25_f32, -0.75]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let packed_24_bit = [0x00, 0x00, 0x40, 0x00, 0x00, 0xC0]; // +0.5, −0.5
        let unsigned_8_bit = [192, 64]; // +0.5, −0.5

        assert_eq!(
            decode_to_stereo(&float_bytes, PcmEncoding::Float32, 2),
            [0.25, -0.75]
        );
        assert_eq!(
            decode_to_stereo(&packed_24_bit, PcmEncoding::Signed24BitPacked, 2),
            [0.5, -0.5]
        );
        assert_eq!(
            decode_to_stereo(&unsigned_8_bit, PcmEncoding::Unsigned8Bit, 2),
            [0.5, -0.5]
        );
    }

    #[test]
    fn android_encoding_values_are_recognised() {
        assert_eq!(
            PcmEncoding::from_android_value(PCM_ENCODING_SIGNED_16_BIT),
            Some(PcmEncoding::Signed16Bit)
        );
        assert_eq!(
            PcmEncoding::from_android_value(4),
            Some(PcmEncoding::Float32)
        );
        assert_eq!(PcmEncoding::from_android_value(99), None);
    }

    #[test]
    fn incomplete_trailing_samples_are_ignored() {
        let mut bytes = sixteen_bit(&[100, 200]);
        bytes.push(7);
        assert_eq!(
            decode_to_stereo(&bytes, PcmEncoding::Signed16Bit, 2).len(),
            2
        );
    }

    #[test]
    fn encoding_to_16_bit_clamps_and_rounds() {
        let bytes = encode_signed_16_bit(&[0.5, -2.0, 1.0]);
        let values: Vec<i16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| i16::from_le_bytes(*pair))
            .collect();
        assert_eq!(values, [16_384, -32_767, 32_767]);
    }
}
