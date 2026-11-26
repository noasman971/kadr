//! Reading a video's frames as the small grey images the tracker works on.

use kadr_media::{MediaBackend, VideoFrame, VideoFrameSource};
use kadr_vision::GreyImage;

use super::{RegionTrackingError, RegionTrackingRequest};

/// BT.709 weights of red, green and blue in brightness, for RGBA frames.
const LUMA_WEIGHTS: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// Decodes frames by index and shrinks their brightness for tracking.
pub(super) struct FrameReader {
    source: Box<dyn VideoFrameSource>,
    frame_rate: kadr_core::time::FrameRate,
    shrink_factor: usize,
}

impl FrameReader {
    /// Opens the request's video.
    pub(super) fn open(
        backend: &dyn MediaBackend,
        request: &RegionTrackingRequest,
        shrink_factor: usize,
    ) -> Result<Self, RegionTrackingError> {
        Ok(Self {
            source: backend.open_video_source(&request.video_path)?,
            frame_rate: request.frame_rate,
            shrink_factor,
        })
    }

    /// Frame number `frame_index` of the file, as a shrunk grey image.
    pub(super) fn grey_frame(
        &mut self,
        frame_index: u64,
    ) -> Result<GreyImage, RegionTrackingError> {
        let frame = self
            .source
            .frame_at(self.frame_rate.timestamp_of_frame(frame_index))?;
        grey_image_of(&frame, self.shrink_factor).ok_or(RegionTrackingError::VideoSizeChanged)
    }
}

/// The brightness of `frame`, shrunk by `shrink_factor`. YUV frames give their Y
/// plane as is; RGBA frames are converted with the BT.709 weights.
pub(super) fn grey_image_of(frame: &VideoFrame, shrink_factor: usize) -> Option<GreyImage> {
    let (width, height) = (frame.width() as usize, frame.height() as usize);
    match frame {
        VideoFrame::Yuv420(picture) => {
            GreyImage::from_luma_shrunk(width, height, width, picture.y_plane(), shrink_factor)
        }
        VideoFrame::Rgba(image) => {
            let luma: Vec<u8> = image
                .pixels()
                .map(|pixel| {
                    let [red, green, blue, _] = pixel.0;
                    let brightness = LUMA_WEIGHTS[0] * f32::from(red)
                        + LUMA_WEIGHTS[1] * f32::from(green)
                        + LUMA_WEIGHTS[2] * f32::from(blue);
                    brightness.round().clamp(0.0, 255.0) as u8
                })
                .collect();
            GreyImage::from_luma_shrunk(width, height, width, &luma, shrink_factor)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};
    use kadr_media::{YuvColorSpace, YuvPicture};

    #[test]
    fn rgba_and_yuv_frames_give_their_brightness() {
        let white = VideoFrame::Rgba(RgbaImage::from_pixel(8, 4, Rgba([255, 255, 255, 255])));
        let grey = grey_image_of(&white, 2).unwrap();
        assert_eq!((grey.width(), grey.height()), (4, 2));
        assert_eq!(grey.value(1, 1), 255.0);

        let black = RgbaImage::from_pixel(8, 4, Rgba([0, 0, 0, 255]));
        let yuv = VideoFrame::Yuv420(YuvPicture::from_rgba(&black, YuvColorSpace::BT709_LIMITED));
        // Limited range: black is 16.
        assert_eq!(grey_image_of(&yuv, 1).unwrap().value(3, 2), 16.0);
    }
}
