//! The binary formats of pictures and sound sent to the front end.
//!
//! Large data never goes through JSON: commands answer raw bytes
//! (`tauri::ipc::Response`), which the front end reads as an `ArrayBuffer`.
//! Every number is little-endian. Decoders live in
//! `apps/desktop/src/lib/api/binaryMessages.ts` and must match.
//!
//! | Message | Layout |
//! |---|---|
//! | Preview frame | `u32` width, `u32` height, Y plane, U plane, V plane (4:2:0, BT.709 limited range) |
//! | Thumbnail | `u32` width, `u32` height, RGBA pixels (straight alpha) |
//! | Waveform | `u32` peaks per second, then one byte per peak (0 silence – 255 full scale) |
//! | Sound | Interleaved stereo `f32` samples at the project's sample rate |

use kadr_engine::media_overview::WaveformPeaks;
use kadr_media::{AudioBuffer, YuvPicture};

/// Bytes of the width and height header.
const SIZE_HEADER_LENGTH: usize = 8;

/// A preview frame: its size, then its three planes.
pub fn encode_yuv_picture(picture: &YuvPicture) -> Vec<u8> {
    let planes = [picture.y_plane(), picture.u_plane(), picture.v_plane()];
    let mut bytes = Vec::with_capacity(
        SIZE_HEADER_LENGTH + planes.iter().map(|plane| plane.len()).sum::<usize>(),
    );
    bytes.extend_from_slice(&picture.width().to_le_bytes());
    bytes.extend_from_slice(&picture.height().to_le_bytes());
    for plane in planes {
        bytes.extend_from_slice(plane);
    }
    bytes
}

/// A thumbnail: its size, then its RGBA pixels.
pub fn encode_rgba_picture(width: u32, height: u32, rgba_pixels: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(SIZE_HEADER_LENGTH + rgba_pixels.len());
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes.extend_from_slice(rgba_pixels);
    bytes
}

/// A waveform: its peaks per second, then its peaks.
pub fn encode_waveform(waveform: &WaveformPeaks) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(4 + waveform.peaks.len());
    bytes.extend_from_slice(&waveform.peaks_per_second.to_le_bytes());
    bytes.extend_from_slice(&waveform.peaks);
    bytes
}

/// Sound: its interleaved samples.
pub fn encode_sound(sound: &AudioBuffer) -> Vec<u8> {
    let samples = sound.interleaved_samples();
    let mut bytes = Vec::with_capacity(size_of_val(samples));
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use kadr_media::YuvColorSpace;

    #[test]
    fn a_yuv_picture_starts_with_its_size_then_its_planes() {
        let picture = YuvPicture::from_planes(
            4,
            2,
            vec![1; 8],
            vec![2; 2],
            vec![3; 2],
            YuvColorSpace::BT709_LIMITED,
        )
        .unwrap();
        let bytes = encode_yuv_picture(&picture);
        assert_eq!(&bytes[..8], &[4, 0, 0, 0, 2, 0, 0, 0]);
        assert_eq!(&bytes[8..], &[1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 3, 3]);
    }

    #[test]
    fn sound_is_little_endian_floats() {
        let sound = AudioBuffer::from_interleaved_stereo(48_000, vec![0.5, -1.0]);
        let bytes = encode_sound(&sound);
        assert_eq!(bytes.len(), 8);
        assert_eq!(f32::from_le_bytes(bytes[4..8].try_into().unwrap()), -1.0);
    }

    #[test]
    fn a_waveform_starts_with_its_rate() {
        let bytes = encode_waveform(&WaveformPeaks {
            peaks_per_second: 100,
            peaks: vec![0, 255],
        });
        assert_eq!(bytes, vec![100, 0, 0, 0, 0, 255]);
    }
}
