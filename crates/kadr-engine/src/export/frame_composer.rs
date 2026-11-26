//! Building the picture of the timeline at one moment.
//!
//! For each visible video track, from bottom to top, the composer finds the clip
//! under the playhead, asks that clip's video source for the right source
//! frame, and hands every picture with its placement to the compositor.
//!
//! Each clip gets its own video source, opened when the clip first appears and
//! closed once it is over, so two clips of the same file never fight over one
//! decoder position.

use std::collections::HashMap;
use std::sync::Arc;

use kadr_core::clip::{Clip, ClipContent};
use kadr_core::ids::ClipId;
use kadr_core::media_asset::MediaKind;
use kadr_core::project::Project;
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_core::timeline::{Track, TrackKind};
use kadr_media::{MediaBackend, VideoFrame, VideoFrameSource};
use kadr_render::{LayerPlacement, VisualLayer, composite_layers};

use super::ExportError;

/// Renders timeline pictures for one export.
pub(super) struct FrameComposer<'export> {
    project: &'export Project,
    backend: &'export dyn MediaBackend,
    open_sources: HashMap<ClipId, Box<dyn VideoFrameSource>>,
}

impl<'export> FrameComposer<'export> {
    pub(super) fn new(project: &'export Project, backend: &'export dyn MediaBackend) -> Self {
        Self {
            project,
            backend,
            open_sources: HashMap::new(),
        }
    }

    /// The finished picture of the timeline at `timeline_time`.
    pub(super) fn compose_frame_at(
        &mut self,
        timeline_time: Timestamp,
    ) -> Result<VideoFrame, ExportError> {
        self.close_sources_of_finished_clips(timeline_time);

        let mut pictures_with_placements: Vec<(Arc<VideoFrame>, LayerPlacement)> = Vec::new();
        let visible_tracks = self
            .project
            .timeline()
            .tracks()
            .iter()
            .filter(|track| track.kind == TrackKind::Video && !track.is_hidden);
        for track in visible_tracks {
            let Some(clip) = clip_under_playhead(track, timeline_time) else {
                continue;
            };
            if let Some(picture) = self.source_picture_of(clip, timeline_time)? {
                let time_in_clip = timeline_time
                    .span_since(clip.timeline_start)
                    .unwrap_or(TimeSpan::ZERO);
                pictures_with_placements
                    .push((picture, LayerPlacement::of_clip_at(clip, time_in_clip)));
            }
        }

        let layers: Vec<VisualLayer<'_>> = pictures_with_placements
            .iter()
            .map(|(picture, placement)| VisualLayer {
                picture: picture.as_ref(),
                placement: *placement,
            })
            .collect();
        let settings = self.project.settings();
        Ok(composite_layers(
            settings.canvas_size,
            settings.background_color,
            &layers,
        ))
    }

    /// The source picture a clip shows at `timeline_time`, or `None` if the clip
    /// shows no picture (text clips are rendered from phase 3; sound has none).
    fn source_picture_of(
        &mut self,
        clip: &Clip,
        timeline_time: Timestamp,
    ) -> Result<Option<Arc<VideoFrame>>, ExportError> {
        let ClipContent::Media(segment) = &clip.content else {
            return Ok(None);
        };
        let Some(media_asset) = self.project.media_asset(segment.media_asset_id) else {
            return Ok(None);
        };
        if !media_asset.description.has_picture() {
            return Ok(None);
        }
        let source_time = if media_asset.description.kind == MediaKind::Image {
            Timestamp::ZERO
        } else {
            match clip.source_time_at(timeline_time) {
                Some(source_time) => source_time,
                None => return Ok(None),
            }
        };
        let source = match self.open_sources.entry(clip.id) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => entry.insert(
                self.backend
                    .open_video_source(&media_asset.description.file_path)?,
            ),
        };
        Ok(Some(source.frame_at(source_time)?))
    }

    /// Frees the decoders of clips that ended before `timeline_time`.
    fn close_sources_of_finished_clips(&mut self, timeline_time: Timestamp) {
        let timeline = self.project.timeline();
        self.open_sources.retain(|clip_id, _| {
            timeline
                .find_clip(*clip_id)
                .is_some_and(|(_, clip)| clip.timeline_end() > timeline_time)
        });
    }
}

/// The clip of `track` containing `timeline_time`, if any.
fn clip_under_playhead(track: &Track, timeline_time: Timestamp) -> Option<&Clip> {
    let clips = track.clips();
    // Clips are sorted by start and never overlap: the candidate is the last
    // clip starting at or before the playhead.
    let clips_started = clips.partition_point(|clip| clip.timeline_start <= timeline_time);
    let candidate = clips.get(clips_started.checked_sub(1)?)?;
    candidate
        .contains_timeline_time(timeline_time)
        .then_some(candidate)
}
