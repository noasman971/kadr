//! Following a selected region through a clip: the analysis behind the tracked blur.
//!
//! 1. [`RegionTrackingRequest::for_clip`] finds the clip's video file, the part
//!    of it the clip shows, and the moment of the file shown at the chosen
//!    timeline time.
//! 2. [`track_region`] decodes that part of the file and follows the region
//!    with `kadr-vision`'s `RegionTracker`, **forwards** to the end and
//!    **backwards** to the start from the selected frame, on frames shrunk to
//!    about 640 pixels wide. The track is smoothed, then turned into the
//!    [`RegionSample`]s of a [`TrackedBlur`]: one sample per frame where the
//!    motion is not a straight line, fewer where it is.
//! 3. [`export_tracking_debug_video`] draws the tracked box and its confidence on
//!    every frame, to check an analysis by eye.
//!
//! Backwards tracking decodes the file in chunks of [`BACKWARD_CHUNK_IN_FRAMES`]
//! frames, each chunk read forwards then tracked in reverse: decoders are fast
//! forwards and slow backwards.
//!
//! See `docs/tracking-and-blur.md`.

mod correction;
mod debug_video;
mod frame_reading;
mod track_samples;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use kadr_core::clip::ClipContent;
use kadr_core::effect::{
    PixelRegion, RegionSample, RegionSampleOrigin, TrackedBlur, TrackingConfidence, UnsureRange,
};
use kadr_core::error::EditError;
use kadr_core::ids::{ClipId, EffectId};
use kadr_core::media_asset::PictureSize;
use kadr_core::project::Project;
use kadr_core::time::{FrameRate, TimeSpan, Timestamp};
use kadr_core::timeline::TrackKind;
use kadr_media::{MediaBackend, MediaError};
use kadr_vision::{
    GreyImage, MINIMUM_REGION_SIDE_IN_PIXELS, Region, RegionTracker, TrackedRegion, TrackingError,
    TrackingState, shrink_factor_for_tracking, smooth_track,
};

pub use correction::{corrected_tracked_blur, find_tracked_blur};
pub use debug_video::{DebugVideoRequest, export_tracking_debug_video};
use frame_reading::FrameReader;

/// Frames decoded at once when tracking backwards (about one second of video,
/// 30 small grey images in memory).
pub const BACKWARD_CHUNK_IN_FRAMES: u64 = 30;

/// What to track.
#[derive(Debug, Clone, PartialEq)]
pub struct RegionTrackingRequest {
    /// The video file.
    pub video_path: PathBuf,
    /// Its frame rate.
    pub frame_rate: FrameRate,
    /// The size of its pictures.
    pub picture_size: PictureSize,
    /// The first moment to analyse, in the file's time.
    pub source_start: Timestamp,
    /// The end of the part to analyse (excluded), in the file's time.
    pub source_end: Timestamp,
    /// The moment the region was selected, in the file's time.
    pub selection_time: Timestamp,
    /// The selected region, in the file's pixels (origin at the top-left corner).
    pub selection: PixelRegion,
    /// Who drew the selection: the first selection of a blur, or a correction.
    pub selection_origin: RegionSampleOrigin,
}

impl RegionTrackingRequest {
    /// The request for `selection`, drawn on clip `clip_id` as shown at
    /// `timeline_time`. The whole part of the file the clip shows is analysed.
    ///
    /// # Errors
    ///
    /// The clip does not exist, does not show a video on a video track, or is not
    /// on screen at `timeline_time`.
    pub fn for_clip(
        project: &Project,
        clip_id: ClipId,
        timeline_time: Timestamp,
        selection: PixelRegion,
    ) -> Result<Self, RegionTrackingError> {
        let (track, clip) = project.timeline().find_clip(clip_id).ok_or_else(|| {
            RegionTrackingError::Edit(EditError::ClipNotFound {
                clip_id,
                existing_clip_ids: project.timeline().clip_ids(),
            })
        })?;
        // A video placed on an audio track only gives its sound: it is never drawn.
        let ClipContent::Media(segment) = &clip.content else {
            return Err(RegionTrackingError::NotAVideoClip { clip_id });
        };
        if track.kind != TrackKind::Video {
            return Err(RegionTrackingError::NotAVideoClip { clip_id });
        }
        let description = project
            .media_asset(segment.media_asset_id)
            .map(|asset| &asset.description)
            .ok_or(RegionTrackingError::NotAVideoClip { clip_id })?;
        let (Some(picture_size), Some(frame_rate)) =
            (description.picture_size, description.frame_rate)
        else {
            return Err(RegionTrackingError::NotAVideoClip { clip_id });
        };
        let selection_time =
            clip.source_time_at(timeline_time)
                .ok_or(RegionTrackingError::TimeOutsideClip {
                    clip_id,
                    timeline_time,
                })?;
        Ok(Self {
            video_path: description.file_path.clone(),
            frame_rate,
            picture_size,
            source_start: segment.source_start,
            source_end: segment.source_end,
            selection_time,
            selection,
            selection_origin: RegionSampleOrigin::UserSelection,
        })
    }

    /// Checks, without decoding anything, that the selection can be tracked:
    /// inside the picture and large enough. [`track_region`] checks it again,
    /// and also that the region has some detail.
    ///
    /// # Errors
    ///
    /// [`RegionTrackingError::SelectionOutsidePicture`] or
    /// [`RegionTrackingError::SelectionTooSmall`].
    pub fn check_selection(&self) -> Result<(), RegionTrackingError> {
        check_selection(
            self,
            shrink_factor_for_tracking(self.picture_size.width_in_pixels as usize),
        )
    }
}

/// How far the analysis has gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackingProgress {
    /// Frames analysed so far.
    pub analysed_frame_count: u64,
    /// Frames to analyse in all.
    pub total_frame_count: u64,
}

impl TrackingProgress {
    /// Progress from 0 to 1.
    pub fn fraction(&self) -> f64 {
        if self.total_frame_count == 0 {
            return 1.0;
        }
        self.analysed_frame_count as f64 / self.total_frame_count as f64
    }
}

/// Lets another thread (the interface) stop an analysis. Cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct TrackingCancellation {
    is_cancelled: Arc<AtomicBool>,
}

impl TrackingCancellation {
    /// A cancellation switch that is off.
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks the analysis to stop as soon as possible.
    pub fn cancel(&self) {
        self.is_cancelled.store(true, Ordering::Relaxed);
    }

    /// Whether [`TrackingCancellation::cancel`] was called.
    pub fn is_cancelled(&self) -> bool {
        self.is_cancelled.load(Ordering::Relaxed)
    }
}

/// The tracker's answer for one frame, in the file's pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackedFrame {
    /// The moment the frame starts, in the file's time.
    pub source_time: Timestamp,
    /// Where the region is (smoothed), or was last seen when lost.
    pub region: PixelRegion,
    /// How sure the tracker was.
    pub confidence: TrackingConfidence,
    /// The user's selection, or the tracker's answer.
    pub origin: RegionSampleOrigin,
    /// How clearly the correlation filter found the region (`None` for the
    /// selected frame).
    pub peak_to_sidelobe_ratio: Option<f32>,
}

/// The result of an analysis.
#[derive(Debug, Clone, PartialEq)]
pub struct RegionTrackingReport {
    /// Every analysed frame, in time order.
    pub frames: Vec<TrackedFrame>,
    /// The samples to store in a [`TrackedBlur`]: the frames, without those a
    /// straight line between their neighbours already gives within a pixel.
    pub region_samples: Vec<RegionSample>,
    /// Where the tracker was not confident.
    pub unsure_ranges: Vec<UnsureRange>,
    /// How much the frames were shrunk for tracking (1 = not at all).
    pub shrink_factor: usize,
}

impl RegionTrackingReport {
    /// How many frames had each confidence: (confident, uncertain, lost).
    pub fn confidence_counts(&self) -> (usize, usize, usize) {
        let count = |confidence| {
            self.frames
                .iter()
                .filter(|frame| frame.confidence == confidence)
                .count()
        };
        (
            count(TrackingConfidence::Confident),
            count(TrackingConfidence::Uncertain),
            count(TrackingConfidence::Lost),
        )
    }

    /// A tracked blur in `style` (shape, margin, softness) holding these samples.
    pub fn tracked_blur_like(&self, style: &TrackedBlur) -> TrackedBlur {
        TrackedBlur {
            region_samples: self.region_samples.clone(),
            ..style.clone()
        }
    }
}

/// Why an analysis failed.
#[derive(Debug, thiserror::Error)]
pub enum RegionTrackingError {
    /// The clip does not exist.
    #[error(transparent)]
    Edit(#[from] EditError),
    /// The clip shows a text, a sound or a still image, or is on an audio track.
    #[error(
        "clip {clip_id} does not show a video: only video clips on a video track can be tracked"
    )]
    NotAVideoClip {
        /// The clip asked for.
        clip_id: ClipId,
    },
    /// The effect to correct is not a tracked blur.
    #[error("effect {effect_id} of clip {clip_id} is not a tracked blur")]
    NotATrackedBlur {
        /// The clip asked for.
        clip_id: ClipId,
        /// The effect asked for.
        effect_id: EffectId,
    },
    /// The clip is not on screen at the requested time.
    #[error("clip {clip_id} is not on screen at {timeline_time}")]
    TimeOutsideClip {
        /// The clip asked for.
        clip_id: ClipId,
        /// The requested timeline time.
        timeline_time: Timestamp,
    },
    /// The selected region is outside the picture, or not a proper rectangle.
    #[error(
        "the region (left {}, top {}, {} × {} pixels) is not inside the {} × {} picture",
        selection.left_in_pixels,
        selection.top_in_pixels,
        selection.width_in_pixels,
        selection.height_in_pixels,
        picture_size.width_in_pixels,
        picture_size.height_in_pixels
    )]
    SelectionOutsidePicture {
        /// The selected region.
        selection: PixelRegion,
        /// The size of the video's pictures.
        picture_size: PictureSize,
    },
    /// The selected region is too small to follow.
    #[error(
        "the region is {width_in_pixels} × {height_in_pixels} pixels: each side must be at least \
         {minimum_side_in_pixels} pixels to be tracked in this video"
    )]
    SelectionTooSmall {
        /// The region's width.
        width_in_pixels: f32,
        /// The region's height.
        height_in_pixels: f32,
        /// The shortest side accepted for this video.
        minimum_side_in_pixels: f32,
    },
    /// The selected region and its surroundings are uniform.
    #[error("the region and its surroundings are uniform: there is nothing to follow")]
    SelectionWithoutTexture,
    /// The selected region itself is uniform, even if its surroundings are not.
    #[error("the region itself is uniform: it has no detail to recognise it by when it is lost")]
    SelectionWithoutDetail,
    /// The debug video was cancelled.
    #[error("the debug video was cancelled")]
    DebugVideoCancelled,
    /// The finished debug video could not be put in place.
    #[error("could not create \"{}\": {reason}", output_path.display())]
    CannotPlaceDebugVideo {
        /// The requested output file.
        output_path: PathBuf,
        /// What the system said.
        reason: String,
    },
    /// The video's pictures changed size in the middle of the clip.
    #[error(
        "the video's pictures change size in the middle of the clip, which tracking does not support"
    )]
    VideoSizeChanged,
    /// The analysis was cancelled.
    #[error("the analysis was cancelled")]
    Cancelled,
    /// Reading the video failed.
    #[error(transparent)]
    Media(#[from] MediaError),
}

/// The frames of the request, by index.
#[derive(Debug, Clone, Copy)]
struct FrameRange {
    first: u64,
    selected: u64,
    last: u64,
}

impl FrameRange {
    fn of(request: &RegionTrackingRequest) -> Self {
        let frame_rate = request.frame_rate;
        let first = frame_rate.frame_index_at(request.source_start);
        let frames_before_end =
            frame_rate.frame_count_covering(TimeSpan::from_ticks(request.source_end.ticks()));
        let last = frames_before_end.saturating_sub(1).max(first);
        let selected = frame_rate
            .frame_index_at(request.selection_time)
            .clamp(first, last);
        Self {
            first,
            selected,
            last,
        }
    }

    fn count(&self) -> u64 {
        self.last - self.first + 1
    }
}

/// Counts analysed frames, reports progress and checks for cancellation.
struct ProgressCounter<'a> {
    progress: TrackingProgress,
    on_progress: &'a mut dyn FnMut(TrackingProgress),
    cancellation: &'a TrackingCancellation,
}

impl ProgressCounter<'_> {
    fn check_cancellation(&self) -> Result<(), RegionTrackingError> {
        if self.cancellation.is_cancelled() {
            return Err(RegionTrackingError::Cancelled);
        }
        Ok(())
    }

    fn count_one_frame(&mut self) {
        self.progress.analysed_frame_count += 1;
        (self.on_progress)(self.progress);
    }
}

/// Follows `request.selection` through the requested part of the video.
///
/// `on_progress` is called after every frame; `cancellation` is checked before
/// every frame.
///
/// # Errors
///
/// See [`RegionTrackingError`].
pub fn track_region(
    backend: &dyn MediaBackend,
    request: &RegionTrackingRequest,
    on_progress: &mut dyn FnMut(TrackingProgress),
    cancellation: &TrackingCancellation,
) -> Result<RegionTrackingReport, RegionTrackingError> {
    let frames = FrameRange::of(request);
    let shrink_factor = shrink_factor_for_tracking(request.picture_size.width_in_pixels as usize);
    check_selection(request, shrink_factor)?;
    let selection = region_from_pixels(&request.selection).in_shrunk_image(shrink_factor);
    let mut reader = FrameReader::open(backend, request, shrink_factor)?;
    let mut counter = ProgressCounter {
        progress: TrackingProgress {
            analysed_frame_count: 0,
            total_frame_count: frames.count(),
        },
        on_progress,
        cancellation,
    };

    counter.check_cancellation()?;
    let selected_image = reader.grey_frame(frames.selected)?;
    counter.count_one_frame();
    let start = TrackingStart {
        request,
        image: selected_image,
        selection,
        frames,
    };
    let forward = track_forwards(&mut reader, &start, &mut counter)?;
    let backward = track_backwards(&mut reader, &start, &mut counter)?;

    let selection_answer = TrackedRegion {
        region: selection,
        state: TrackingState::Confident,
        peak_to_sidelobe_ratio: f32::INFINITY,
    };
    let track: Vec<TrackedRegion> = backward
        .into_iter()
        .rev()
        .chain(std::iter::once(selection_answer))
        .chain(forward)
        .collect();
    let smoothed = smooth_track(&track, request.frame_rate.as_frames_per_second() as f32);
    let selected_offset = (frames.selected - frames.first) as usize;
    let tracked_frames: Vec<TrackedFrame> = smoothed
        .iter()
        .zip(&track)
        .enumerate()
        .map(|(offset, (smoothed, raw))| {
            let source_time = request
                .frame_rate
                .timestamp_of_frame(frames.first + offset as u64);
            if offset == selected_offset {
                return TrackedFrame {
                    source_time,
                    region: request.selection,
                    confidence: TrackingConfidence::Confident,
                    origin: request.selection_origin,
                    peak_to_sidelobe_ratio: None,
                };
            }
            TrackedFrame {
                source_time,
                region: pixels_from_region(&smoothed.region.in_full_size_image(shrink_factor)),
                confidence: confidence_of(raw.state),
                origin: RegionSampleOrigin::Tracker,
                peak_to_sidelobe_ratio: Some(raw.peak_to_sidelobe_ratio),
            }
        })
        .collect();
    let region_samples = track_samples::simplified_samples(&tracked_frames);
    let unsure_ranges = TrackedBlur {
        region_samples: region_samples.clone(),
        ..TrackedBlur::with_default_style()
    }
    .unsure_ranges();
    Ok(RegionTrackingReport {
        frames: tracked_frames,
        region_samples,
        unsure_ranges,
        shrink_factor,
    })
}

/// Where both passes start: the selected frame and region.
struct TrackingStart<'a> {
    request: &'a RegionTrackingRequest,
    image: GreyImage,
    selection: Region,
    frames: FrameRange,
}

impl TrackingStart<'_> {
    fn start_tracker(&self) -> Result<RegionTracker, RegionTrackingError> {
        RegionTracker::start(self.image.clone(), self.selection)
            .map_err(|error| from_tracking_error(error, self.request))
    }
}

/// Tracks the frames after the selected one, in order.
fn track_forwards(
    reader: &mut FrameReader,
    start: &TrackingStart<'_>,
    counter: &mut ProgressCounter<'_>,
) -> Result<Vec<TrackedRegion>, RegionTrackingError> {
    let (request, frames) = (start.request, start.frames);
    let mut tracker = start.start_tracker()?;
    let mut answers = Vec::new();
    for frame_index in frames.selected + 1..=frames.last {
        counter.check_cancellation()?;
        let image = reader.grey_frame(frame_index)?;
        answers.push(
            tracker
                .track(image)
                .map_err(|error| from_tracking_error(error, request))?,
        );
        counter.count_one_frame();
    }
    Ok(answers)
}

/// Tracks the frames before the selected one, from the nearest to the first.
/// The answers come in that order (reverse time).
fn track_backwards(
    reader: &mut FrameReader,
    start: &TrackingStart<'_>,
    counter: &mut ProgressCounter<'_>,
) -> Result<Vec<TrackedRegion>, RegionTrackingError> {
    let (request, frames) = (start.request, start.frames);
    let mut tracker = start.start_tracker()?;
    let mut answers = Vec::new();
    let mut chunk_end = frames.selected;
    while chunk_end > frames.first {
        let chunk_start = chunk_end
            .saturating_sub(BACKWARD_CHUNK_IN_FRAMES)
            .max(frames.first);
        let mut chunk = Vec::new();
        for frame_index in chunk_start..chunk_end {
            counter.check_cancellation()?;
            chunk.push(reader.grey_frame(frame_index)?);
        }
        for image in chunk.into_iter().rev() {
            counter.check_cancellation()?;
            answers.push(
                tracker
                    .track(image)
                    .map_err(|error| from_tracking_error(error, request))?,
            );
            counter.count_one_frame();
        }
        chunk_end = chunk_start;
    }
    Ok(answers)
}

/// Refuses selections the tracker could not follow, with sizes in the file's
/// pixels (the tracker itself works on shrunk frames).
fn check_selection(
    request: &RegionTrackingRequest,
    shrink_factor: usize,
) -> Result<(), RegionTrackingError> {
    let selection = &request.selection;
    let picture = request.picture_size;
    let center_x = selection.left_in_pixels + selection.width_in_pixels / 2.0;
    let center_y = selection.top_in_pixels + selection.height_in_pixels / 2.0;
    // Shrinking drops the columns and rows left over at the right and bottom
    // edges: the tracker only sees the part of the picture the blocks cover.
    let covered = |side: u32| (side as usize / shrink_factor * shrink_factor) as f32;
    let is_inside = selection.is_well_formed()
        && (0.0..=covered(picture.width_in_pixels)).contains(&center_x)
        && (0.0..=covered(picture.height_in_pixels)).contains(&center_y);
    if !is_inside {
        return Err(RegionTrackingError::SelectionOutsidePicture {
            selection: *selection,
            picture_size: picture,
        });
    }
    let minimum_side_in_pixels = MINIMUM_REGION_SIDE_IN_PIXELS * shrink_factor as f32;
    if selection.width_in_pixels < minimum_side_in_pixels
        || selection.height_in_pixels < minimum_side_in_pixels
    {
        return Err(RegionTrackingError::SelectionTooSmall {
            width_in_pixels: selection.width_in_pixels,
            height_in_pixels: selection.height_in_pixels,
            minimum_side_in_pixels,
        });
    }
    Ok(())
}

/// The tracker's error, told with the request's own numbers (the tracker works
/// on shrunk frames, whose sizes would confuse whoever made the selection).
fn from_tracking_error(
    error: TrackingError,
    request: &RegionTrackingRequest,
) -> RegionTrackingError {
    let selection = request.selection;
    match error {
        // Both are checked beforehand by `check_selection`; kept for completeness.
        TrackingError::RegionTooSmall {
            minimum_side_in_pixels,
            ..
        } => RegionTrackingError::SelectionTooSmall {
            width_in_pixels: selection.width_in_pixels,
            height_in_pixels: selection.height_in_pixels,
            minimum_side_in_pixels: minimum_side_in_pixels
                * shrink_factor_for_tracking(request.picture_size.width_in_pixels as usize) as f32,
        },
        TrackingError::RegionOutsideImage { .. } => RegionTrackingError::SelectionOutsidePicture {
            selection,
            picture_size: request.picture_size,
        },
        TrackingError::RegionWithoutTexture => RegionTrackingError::SelectionWithoutTexture,
        TrackingError::RegionWithoutDetail => RegionTrackingError::SelectionWithoutDetail,
        TrackingError::FrameSizeChanged { .. } => RegionTrackingError::VideoSizeChanged,
    }
}

fn confidence_of(state: TrackingState) -> TrackingConfidence {
    match state {
        TrackingState::Confident => TrackingConfidence::Confident,
        TrackingState::Uncertain => TrackingConfidence::Uncertain,
        TrackingState::Lost => TrackingConfidence::Lost,
    }
}

fn region_from_pixels(region: &PixelRegion) -> Region {
    Region::from_top_left(
        region.left_in_pixels,
        region.top_in_pixels,
        region.width_in_pixels,
        region.height_in_pixels,
    )
}

fn pixels_from_region(region: &Region) -> PixelRegion {
    PixelRegion {
        left_in_pixels: region.left(),
        top_in_pixels: region.top(),
        width_in_pixels: region.width,
        height_in_pixels: region.height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_with(width: u32, selection: PixelRegion) -> RegionTrackingRequest {
        RegionTrackingRequest {
            video_path: PathBuf::from("video.mp4"),
            frame_rate: FrameRate::FPS_30,
            picture_size: PictureSize {
                width_in_pixels: width,
                height_in_pixels: width * 9 / 16,
            },
            source_start: Timestamp::from_whole_seconds(1),
            source_end: Timestamp::from_whole_seconds(3),
            selection_time: Timestamp::from_milliseconds(2_500),
            selection,
            selection_origin: RegionSampleOrigin::UserSelection,
        }
    }

    fn rectangle(left: f32, top: f32, width: f32, height: f32) -> PixelRegion {
        PixelRegion {
            left_in_pixels: left,
            top_in_pixels: top,
            width_in_pixels: width,
            height_in_pixels: height,
        }
    }

    #[test]
    fn the_frame_range_covers_the_clip_and_holds_the_selection() {
        let frames = FrameRange::of(&request_with(1920, rectangle(0.0, 0.0, 50.0, 50.0)));
        assert_eq!((frames.first, frames.selected, frames.last), (30, 75, 89));
        assert_eq!(frames.count(), 60);
    }

    #[test]
    fn selections_are_checked_in_the_files_pixels() {
        // 1920 pixels wide: frames are shrunk 3 times, so sides need 24 pixels.
        let too_small = request_with(1920, rectangle(100.0, 100.0, 20.0, 80.0));
        assert!(matches!(
            check_selection(&too_small, 3),
            Err(RegionTrackingError::SelectionTooSmall {
                minimum_side_in_pixels: 24.0,
                ..
            })
        ));
        let outside = request_with(1920, rectangle(1950.0, 100.0, 50.0, 50.0));
        assert!(matches!(
            check_selection(&outside, 3),
            Err(RegionTrackingError::SelectionOutsidePicture { .. })
        ));
        let fine = request_with(1920, rectangle(100.0, 100.0, 30.0, 40.0));
        assert!(check_selection(&fine, 3).is_ok());
    }

    #[test]
    fn the_columns_dropped_by_shrinking_are_outside_the_tracked_picture() {
        // 1366 pixels wide, shrunk 3 times: 455 blocks cover 1365 columns.
        let mut request = request_with(1366, rectangle(1340.0, 300.0, 52.0, 60.0));
        request.picture_size.height_in_pixels = 768;
        assert!(matches!(
            check_selection(&request, 3),
            Err(RegionTrackingError::SelectionOutsidePicture { picture_size, .. })
                if picture_size.width_in_pixels == 1366
        ));
    }

    #[test]
    fn tracker_errors_keep_the_selection_and_the_files_size() {
        let request = request_with(1920, rectangle(100.0, 100.0, 30.0, 40.0));
        let outside = TrackingError::RegionOutsideImage {
            center_x: 1.0,
            center_y: 2.0,
            image_width: 640,
            image_height: 360,
        };
        let RegionTrackingError::SelectionOutsidePicture {
            selection,
            picture_size,
        } = from_tracking_error(outside, &request)
        else {
            panic!("expected SelectionOutsidePicture");
        };
        assert_eq!(selection, request.selection);
        assert_eq!(picture_size.width_in_pixels, 1920);
        assert!(matches!(
            from_tracking_error(TrackingError::RegionWithoutDetail, &request),
            RegionTrackingError::SelectionWithoutDetail
        ));
    }

    #[test]
    fn regions_convert_both_ways() {
        let pixels = rectangle(420.0, 180.0, 160.0, 200.0);
        assert_eq!(pixels_from_region(&region_from_pixels(&pixels)), pixels);
    }
}
