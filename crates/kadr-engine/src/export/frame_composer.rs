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
//!
//! Transitions and text animations move layers (see [`super::clip_layers`]); a
//! clip shown under its neighbour's transition reads its source past its cut,
//! holding the file's first or last frame where the file ends.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use kadr_core::clip::{Clip, ClipContent, TextContent};
use kadr_core::effect::{ColorAdjustment, EffectKind};
use kadr_core::ids::ClipId;
use kadr_core::media_asset::MediaKind;
use kadr_core::project::Project;
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_core::timeline::TrackKind;
use kadr_media::{MediaBackend, VideoFrame, VideoFrameSource};
use kadr_render::{
    CanvasDescription, FrameRenderer, HiddenRegion, LayerPlacement, TextRasterizer, TextReveal,
    VisualLayer, hide_regions,
};

use super::ExportError;
use super::clip_layers::plan_track_layers;

/// Renders timeline pictures for one export.
pub(super) struct FrameComposer<'export> {
    project: &'export Project,
    backend: &'export dyn MediaBackend,
    renderer: Box<dyn FrameRenderer>,
    open_sources: HashMap<ClipId, Box<dyn VideoFrameSource>>,
    /// Created on the first text clip: loading fonts takes a moment.
    text_rasterizer: Option<TextRasterizer>,
    /// The picture of each text clip, with the content and the part of it it
    /// was drawn from.
    text_pictures: HashMap<ClipId, (TextContent, TextReveal, Arc<VideoFrame>)>,
}

impl<'export> FrameComposer<'export> {
    pub(super) fn new(
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
    pub(super) fn renderer_description(&self) -> String {
        self.renderer.description()
    }

    /// The finished picture of the timeline at `timeline_time`.
    pub(super) fn compose_frame_at(
        &mut self,
        timeline_time: Timestamp,
    ) -> Result<VideoFrame, ExportError> {
        let settings = self.project.settings();
        let mut prepared_layers: Vec<(Arc<VideoFrame>, LayerPlacement, Vec<ColorAdjustment>)> =
            Vec::new();
        let mut clips_shown = HashSet::new();
        let visible_tracks = self
            .project
            .timeline()
            .tracks()
            .iter()
            .filter(|track| track.kind == TrackKind::Video && !track.is_hidden);
        for track in visible_tracks {
            for planned_layer in plan_track_layers(track, timeline_time, settings.canvas_size) {
                let clip = planned_layer.clip;
                clips_shown.insert(clip.id);
                let picture =
                    self.source_picture_of(clip, timeline_time, planned_layer.text_reveal)?;
                if let Some(picture) = picture {
                    let placement = LayerPlacement::of_clip_at(clip, planned_layer.time_in_clip)
                        .moved_by(planned_layer.motion);
                    prepared_layers.push((picture, placement, enabled_color_adjustments(clip)));
                }
            }
        }
        // A clip leaves the picture for good once it is over (its transitions
        // included): free its decoder.
        self.open_sources
            .retain(|clip_id, _| clips_shown.contains(clip_id));

        let layers: Vec<VisualLayer<'_>> = prepared_layers
            .iter()
            .map(|(picture, placement, color_adjustments)| VisualLayer {
                picture: picture.as_ref(),
                placement: *placement,
                color_adjustments,
            })
            .collect();
        let canvas = CanvasDescription {
            size: settings.canvas_size,
            background: settings.background_color,
        };
        Ok(self.renderer.render_frame(&canvas, &layers)?)
    }

    /// The source picture a clip shows at `timeline_time`, or `None` if the clip
    /// shows no picture (sound only). Outside the clip (under a transition of
    /// the clip touching it), the source continues past the clip's cut.
    fn source_picture_of(
        &mut self,
        clip: &Clip,
        timeline_time: Timestamp,
        text_reveal: TextReveal,
    ) -> Result<Option<Arc<VideoFrame>>, ExportError> {
        let segment = match &clip.content {
            ClipContent::Media(segment) => segment,
            ClipContent::Text(text_content) => {
                return Ok(Some(self.text_picture_of(
                    clip.id,
                    text_content,
                    text_reveal,
                )));
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
            match clip.source_time_continued_at(timeline_time) {
                Some(source_time) => {
                    held_inside_file(source_time, media_asset.description.duration)
                }
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

    /// The picture of a text clip, drawn again only if its words, its style or
    /// the part of it shown (type-on animations) changed.
    fn text_picture_of(
        &mut self,
        clip_id: ClipId,
        text_content: &TextContent,
        text_reveal: TextReveal,
    ) -> Arc<VideoFrame> {
        if let Some((drawn_content, drawn_reveal, picture)) = self.text_pictures.get(&clip_id)
            && drawn_content == text_content
            && *drawn_reveal == text_reveal
        {
            return Arc::clone(picture);
        }
        let rasterizer = self.text_rasterizer.get_or_insert_with(TextRasterizer::new);
        let picture = Arc::new(VideoFrame::Rgba(rasterizer.rasterize_revealed(
            &text_content.text,
            &text_content.style,
            text_reveal,
        )));
        self.text_pictures.insert(
            clip_id,
            (text_content.clone(), text_reveal, Arc::clone(&picture)),
        );
        picture
    }
}

/// `source_time`, moved back inside a file lasting `file_duration` if it is past
/// its end, so the last frame is held. Files without a duration (still images)
/// are left alone.
fn held_inside_file(source_time: Timestamp, file_duration: Option<TimeSpan>) -> Timestamp {
    let Some(file_duration) = file_duration else {
        return source_time;
    };
    let last_moment = (Timestamp::ZERO + file_duration)
        .checked_sub(TimeSpan::from_ticks(1))
        .unwrap_or(Timestamp::ZERO);
    source_time.min(last_moment)
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
fn hidden_regions_of(clip: &Clip, source_time: Timestamp) -> Vec<HiddenRegion> {
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
