//! Building the picture of the timeline at one moment.
//!
//! For each visible video track, from bottom to top, the composer finds the clip
//! under the playhead, asks that clip's video source for the right source
//! frame, and hands every picture with its placement to the renderer (GPU or CPU).
//!
//! Each clip gets its own video source, opened when the clip first appears and
//! closed once it is over, so two clips of the same file never fight over one
//! decoder position. Text clips are rasterised once and kept while their words
//! and style stay the same.
//!
//! Effects: tracked blurs hide their region in the source picture (at the clip's
//! source time) before it is drawn; colour adjustments are handed to the renderer
//! with the layer.

use std::collections::HashMap;
use std::sync::Arc;

use kadr_core::clip::{Clip, ClipContent, TextContent};
use kadr_core::effect::{ColorAdjustment, EffectKind};
use kadr_core::ids::ClipId;
use kadr_core::media_asset::MediaKind;
use kadr_core::project::Project;
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_core::timeline::{Track, TrackKind};
use kadr_media::{MediaBackend, VideoFrame, VideoFrameSource};
use kadr_render::{
    CanvasDescription, FrameRenderer, HiddenRegion, LayerPlacement, TextRasterizer, VisualLayer,
    hide_regions,
};

use super::ExportError;

/// Renders timeline pictures for one export.
pub(crate) struct FrameComposer<'export> {
    project: &'export Project,
    backend: &'export dyn MediaBackend,
    renderer: Box<dyn FrameRenderer>,
    open_sources: HashMap<ClipId, Box<dyn VideoFrameSource>>,
    /// Created on the first text clip: loading fonts takes a moment.
    text_rasterizer: Option<TextRasterizer>,
    /// The picture of each text clip, with the content it was drawn from.
    text_pictures: HashMap<ClipId, (TextContent, Arc<VideoFrame>)>,
}

impl<'export> FrameComposer<'export> {
    pub(crate) fn new(
        project: &'export Project,
        backend: &'export dyn MediaBackend,
        renderer: Box<dyn FrameRenderer>,
    ) -> Self {
        Self {
            project,
            backend,
            renderer,
            open_sources: HashMap::new(),
            text_rasterizer: None,
            text_pictures: HashMap::new(),
        }
    }

    /// What renders the frames, for the export report.
    pub(crate) fn renderer_description(&self) -> String {
        self.renderer.description()
    }

    /// The finished picture of the timeline at `timeline_time`.
    pub(crate) fn compose_frame_at(
        &mut self,
        timeline_time: Timestamp,
    ) -> Result<VideoFrame, ExportError> {
        self.close_sources_of_finished_clips(timeline_time);

        let mut prepared_layers: Vec<(Arc<VideoFrame>, LayerPlacement, Vec<ColorAdjustment>)> =
            Vec::new();
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
                prepared_layers.push((
                    picture,
                    LayerPlacement::of_clip_at(clip, time_in_clip),
                    enabled_color_adjustments(clip),
                ));
            }
        }

        let layers: Vec<VisualLayer<'_>> = prepared_layers
            .iter()
            .map(|(picture, placement, color_adjustments)| VisualLayer {
                picture: picture.as_ref(),
                placement: *placement,
                color_adjustments,
            })
            .collect();
        let settings = self.project.settings();
        let canvas = CanvasDescription {
            size: settings.canvas_size,
            background: settings.background_color,
        };
        Ok(self.renderer.render_frame(&canvas, &layers)?)
    }

    /// The source picture a clip shows at `timeline_time`, or `None` if the clip
    /// shows no picture (sound only).
    fn source_picture_of(
        &mut self,
        clip: &Clip,
        timeline_time: Timestamp,
    ) -> Result<Option<Arc<VideoFrame>>, ExportError> {
        let segment = match &clip.content {
            ClipContent::Media(segment) => segment,
            ClipContent::Text(text_content) => {
                return Ok(Some(self.text_picture_of(clip.id, text_content)));
            }
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
        let picture = source.frame_at(source_time)?;
        let hidden_regions = hidden_regions_of(clip, source_time);
        if hidden_regions.is_empty() {
            return Ok(Some(picture));
        }
        Ok(Some(Arc::new(hide_regions(&picture, &hidden_regions))))
    }

    /// The picture of a text clip, drawn again only if its words or style changed.
    fn text_picture_of(&mut self, clip_id: ClipId, text_content: &TextContent) -> Arc<VideoFrame> {
        if let Some((drawn_content, picture)) = self.text_pictures.get(&clip_id)
            && drawn_content == text_content
        {
            return Arc::clone(picture);
        }
        let rasterizer = self.text_rasterizer.get_or_insert_with(TextRasterizer::new);
        let picture = Arc::new(VideoFrame::Rgba(
            rasterizer.rasterize(&text_content.text, &text_content.style),
        ));
        self.text_pictures
            .insert(clip_id, (text_content.clone(), Arc::clone(&picture)));
        picture
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

/// The clip's enabled colour adjustments, in order.
fn enabled_color_adjustments(clip: &Clip) -> Vec<ColorAdjustment> {
    clip.effects
        .iter()
        .filter(|effect| effect.is_enabled)
        .filter_map(|effect| match &effect.kind {
            EffectKind::ColorAdjustment(adjustment) => Some(*adjustment),
            EffectKind::TrackedBlur(_) => None,
        })
        .collect()
}

/// The areas the clip's enabled tracked blurs hide at `source_time`.
pub(crate) fn hidden_regions_of(clip: &Clip, source_time: Timestamp) -> Vec<HiddenRegion> {
    clip.effects
        .iter()
        .filter(|effect| effect.is_enabled)
        .filter_map(|effect| match &effect.kind {
            EffectKind::TrackedBlur(tracked_blur) => {
                tracked_blur
                    .hidden_area_at(source_time)
                    .map(|area| HiddenRegion {
                        area,
                        style: tracked_blur.style,
                        shape: tracked_blur.shape,
                        edge_softness_in_pixels: tracked_blur.edge_softness_in_pixels,
                    })
            }
            EffectKind::ColorAdjustment(_) => None,
        })
        .collect()
}
