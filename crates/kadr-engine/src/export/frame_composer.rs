//! Building the picture of the timeline at one moment.
//!
//! For each visible video track, from bottom to top, the composer finds the clip
//! under the playhead, asks that clip's video source for the right source
//! frame, and hands every picture with its placement to the renderer (GPU or CPU).
//!
//! Each clip gets its own video source, opened when the clip first appears and
//! closed once it is over, so two clips of the same file never fight over one
//! decoder position. Text clips are rasterised once and kept while their words,
//! style and revealed part stay the same.
//!
//! The composer does not hold the project: the export hands it the same project
//! for every frame, the preview ([`crate::preview`]) the latest version after each
//! edit. Its caches check that what they hold still matches the clip.
//!
//! Effects: tracked blurs hide their region in the source picture (at the clip's
//! source time) before it is drawn; colour adjustments and the colour LUT are
//! handed to the renderer with the layer, which applies them in that order. LUT
//! files are read once and kept ([`crate::color_grading`]); one that cannot be read
//! is skipped (the export report says so), so the clip is drawn without its look.
//!
//! Transitions and text animations move layers (see [`super::clip_layers`]); a
//! clip shown under its neighbour's transition reads its source past its cut,
//! holding the file's first or last frame where the file ends.
//!
//! Subtitle tracks show their cue under the playhead, drawn with the track's
//! style and placed by it ([`TextRasterizer::rasterize_subtitle`]). Only the cues
//! on screen keep their picture, so a film with a thousand cues does not keep a
//! thousand pictures.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kadr_core::clip::{Clip, ClipContent, TextContent};
use kadr_core::effect::{ColorAdjustment, EffectKind};
use kadr_core::ids::ClipId;
use kadr_core::media_asset::MediaKind;
use kadr_core::media_asset::PictureSize;
use kadr_core::project::Project;
use kadr_core::subtitle::SubtitleStyle;
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_core::timeline::{Track, TrackKind};
use kadr_media::{MediaBackend, MediaError, VideoFrame, VideoFrameSource};
use kadr_render::{
    CanvasDescription, ColorLookupTable, FrameRenderer, HiddenRegion, LayerColorLut,
    LayerPlacement, RenderError, TextRasterizer, TextReveal, VisualLayer, hide_regions,
};

use super::clip_layers::{clip_under_playhead, plan_track_layers};
use crate::color_grading::{ColorTableError, ColorTableLibrary, applied_color_lut_of};

/// The decoder of one clip, with the file it reads.
struct OpenVideoSource {
    /// Checked on reuse: after an undo, a clip number can come back for another file.
    file_path: PathBuf,
    source: Box<dyn VideoFrameSource>,
}

/// A layer ready to hand to the renderer, with what it owns.
struct PreparedLayer {
    picture: Arc<VideoFrame>,
    placement: LayerPlacement,
    color_adjustments: Vec<ColorAdjustment>,
    /// The table and its strength.
    color_lut: Option<(Arc<ColorLookupTable>, f32)>,
}

/// Renders timeline pictures, keeping decoders, text pictures and LUTs between frames.
pub(crate) struct FrameComposer {
    renderer: Box<dyn FrameRenderer>,
    open_sources: HashMap<ClipId, OpenVideoSource>,
    /// Created on the first text clip: loading fonts takes a moment.
    text_rasterizer: Option<TextRasterizer>,
    /// The picture of each text clip, with the content and the part of it it
    /// was drawn from.
    text_pictures: HashMap<ClipId, (TextContent, TextReveal, Arc<VideoFrame>)>,
    /// The picture of each subtitle cue on screen, with what it was drawn from.
    subtitle_pictures: HashMap<ClipId, (SubtitleDrawing, Arc<VideoFrame>, LayerPlacement)>,
    /// The LUT files read so far.
    color_tables: ColorTableLibrary,
}

impl FrameComposer {
    pub(crate) fn new(renderer: Box<dyn FrameRenderer>) -> Self {
        Self {
            renderer,
            open_sources: HashMap::new(),
            text_rasterizer: None,
            text_pictures: HashMap::new(),
            subtitle_pictures: HashMap::new(),
            color_tables: ColorTableLibrary::default(),
        }
    }

    /// The table of a LUT file, read if it was not yet or if it changed.
    pub(crate) fn color_table_at(
        &mut self,
        file_path: &Path,
    ) -> Result<Arc<ColorLookupTable>, ColorTableError> {
        self.color_tables.table_at(file_path)
    }

    /// What renders the frames, for reports.
    pub(crate) fn renderer_description(&self) -> String {
        self.renderer.description()
    }

    /// The finished picture of `project`'s timeline at `timeline_time`.
    pub(crate) fn compose_frame_at<Error: From<MediaError> + From<RenderError>>(
        &mut self,
        project: &Project,
        backend: &dyn MediaBackend,
        timeline_time: Timestamp,
    ) -> Result<VideoFrame, Error> {
        let settings = project.settings();
        let mut prepared_layers: Vec<PreparedLayer> = Vec::new();
        let mut clips_shown = HashSet::new();
        let visible_tracks = project
            .timeline()
            .tracks()
            .iter()
            .filter(|track| track.kind != TrackKind::Audio && !track.is_hidden);
        for track in visible_tracks {
            if track.kind == TrackKind::Subtitle {
                if let Some(layer) =
                    self.subtitle_layer_of(track, timeline_time, settings.canvas_size)
                {
                    prepared_layers.push(layer);
                }
                continue;
            }
            for planned_layer in plan_track_layers(track, timeline_time, settings.canvas_size) {
                let clip = planned_layer.clip;
                clips_shown.insert(clip.id);
                let picture = self.source_picture_of(
                    project,
                    backend,
                    clip,
                    timeline_time,
                    planned_layer.text_reveal,
                )?;
                if let Some(picture) = picture {
                    let placement = LayerPlacement::of_clip_at(clip, planned_layer.time_in_clip)
                        .moved_by(planned_layer.motion);
                    prepared_layers.push(PreparedLayer {
                        picture,
                        placement,
                        color_adjustments: enabled_color_adjustments(clip),
                        color_lut: self.color_lut_of(clip),
                    });
                }
            }
        }
        // A clip that is not in the picture (over, transitions included, or
        // deleted) frees its decoder.
        self.open_sources
            .retain(|clip_id, _| clips_shown.contains(clip_id));
        let subtitle_track_count = project.timeline().subtitle_tracks().count();
        if self.subtitle_pictures.len() > subtitle_track_count {
            self.forget_subtitle_pictures_not_shown(project, timeline_time);
        }
        self.color_tables.forget_files_unused_by(project);

        let layers: Vec<VisualLayer<'_>> = prepared_layers
            .iter()
            .map(|layer| VisualLayer {
                picture: layer.picture.as_ref(),
                placement: layer.placement,
                color_adjustments: &layer.color_adjustments,
                color_lut: layer.color_lut.as_ref().map(|(table, strength_fraction)| {
                    LayerColorLut {
                        table: table.as_ref(),
                        strength_fraction: *strength_fraction,
                    }
                }),
            })
            .collect();
        let canvas = CanvasDescription {
            size: settings.canvas_size,
            background: settings.background_color,
        };
        Ok(self.renderer.render_frame(&canvas, &layers)?)
    }

    /// The layer of the cue `track` shows at `timeline_time`, if any, drawn
    /// again only if its words, the track's style or the canvas changed.
    fn subtitle_layer_of(
        &mut self,
        track: &Track,
        timeline_time: Timestamp,
        canvas_size: PictureSize,
    ) -> Option<PreparedLayer> {
        let clip = clip_under_playhead(track, timeline_time)?;
        let (ClipContent::Subtitle(cue), Some(style)) = (&clip.content, &track.subtitle_style)
        else {
            return None;
        };
        let drawing = SubtitleDrawing {
            text: cue.text.clone(),
            style: style.clone(),
            canvas_size,
        };
        let (picture, placement) = match self.subtitle_pictures.get(&clip.id) {
            Some((drawn, picture, placement)) if *drawn == drawing => {
                (Arc::clone(picture), *placement)
            }
            _ => {
                let rasterizer = self.text_rasterizer.get_or_insert_with(TextRasterizer::new);
                let drawn = rasterizer.rasterize_subtitle(&cue.text, style, canvas_size);
                let picture = Arc::new(VideoFrame::Rgba(drawn.picture));
                self.subtitle_pictures
                    .insert(clip.id, (drawing, Arc::clone(&picture), drawn.placement));
                (picture, drawn.placement)
            }
        };
        Some(PreparedLayer {
            picture,
            placement,
            color_adjustments: Vec::new(),
            color_lut: None,
        })
    }

    /// Forgets the pictures of cues that are not under the playhead.
    fn forget_subtitle_pictures_not_shown(&mut self, project: &Project, timeline_time: Timestamp) {
        let cues_shown: HashSet<ClipId> = project
            .timeline()
            .subtitle_tracks()
            .filter_map(|track| clip_under_playhead(track, timeline_time))
            .map(|clip| clip.id)
            .collect();
        self.subtitle_pictures
            .retain(|clip_id, _| cues_shown.contains(clip_id));
    }

    /// The table and strength of the LUT `clip` applies, or `None` if it applies
    /// none or its file cannot be used (the export reports it).
    fn color_lut_of(&mut self, clip: &Clip) -> Option<(Arc<ColorLookupTable>, f32)> {
        let color_lut = applied_color_lut_of(&clip.effects)?;
        let table = self.color_tables.table_at(&color_lut.file_path).ok()?;
        Some((table, color_lut.strength_fraction))
    }

    /// The source picture a clip shows at `timeline_time`, or `None` if the clip
    /// shows no picture (sound only). Outside the clip (under a transition of
    /// the clip touching it), the source continues past the clip's cut.
    fn source_picture_of(
        &mut self,
        project: &Project,
        backend: &dyn MediaBackend,
        clip: &Clip,
        timeline_time: Timestamp,
        text_reveal: TextReveal,
    ) -> Result<Option<Arc<VideoFrame>>, MediaError> {
        let segment = match &clip.content {
            ClipContent::Media(segment) => segment,
            ClipContent::Text(text_content) => {
                return Ok(Some(self.text_picture_of(
                    clip.id,
                    text_content,
                    text_reveal,
                )));
            }
            // Drawn by `subtitle_layer_of`, with their track's style.
            ClipContent::Subtitle(_) => return Ok(None),
        };
        let Some(media_asset) = project.media_asset(segment.media_asset_id) else {
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
        let picture = self
            .video_source_of(backend, clip.id, &media_asset.description.file_path)?
            .frame_at(source_time)?;
        let hidden_regions = hidden_regions_of(clip, source_time);
        if hidden_regions.is_empty() {
            return Ok(Some(picture));
        }
        Ok(Some(Arc::new(hide_regions(&picture, &hidden_regions))))
    }

    /// The decoder of `clip_id`, opened (or opened again) if it does not read `file_path`.
    fn video_source_of(
        &mut self,
        backend: &dyn MediaBackend,
        clip_id: ClipId,
        file_path: &Path,
    ) -> Result<&mut Box<dyn VideoFrameSource>, MediaError> {
        let has_matching_source = self
            .open_sources
            .get(&clip_id)
            .is_some_and(|open_source| open_source.file_path == file_path);
        if !has_matching_source {
            self.open_sources.remove(&clip_id);
        }
        let open_source = match self.open_sources.entry(clip_id) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => entry.insert(OpenVideoSource {
                file_path: file_path.to_path_buf(),
                source: backend.open_video_source(file_path)?,
            }),
        };
        Ok(&mut open_source.source)
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

/// What a subtitle picture was drawn from.
#[derive(Debug, Clone, PartialEq)]
struct SubtitleDrawing {
    text: String,
    style: SubtitleStyle,
    canvas_size: PictureSize,
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
            EffectKind::ColorLut(_) | EffectKind::TrackedBlur(_) => None,
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
            EffectKind::ColorAdjustment(_) | EffectKind::ColorLut(_) => None,
        })
        .collect()
}
