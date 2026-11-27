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
//! Proxies (decision record 0055): only a composer given [`ReadyProxies`] (the
//! preview's) reads them, instead of the originals they stand for. Tracked blur
//! regions, given in the original's pixels, are then scaled to the proxy's.
//! Crops are fractions and positions are canvas pixels, so the rest of the
//! placement is the same. The export's composer has no proxies.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kadr_core::clip::{Clip, ClipContent, TextContent};
use kadr_core::effect::{ColorAdjustment, EffectKind};
use kadr_core::ids::ClipId;
use kadr_core::media_asset::{MediaKind, PictureSize};
use kadr_core::project::Project;
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_core::timeline::TrackKind;
use kadr_media::{MediaBackend, MediaError, VideoFrame, VideoFrameSource};
use kadr_render::{
    CanvasDescription, ColorLookupTable, FrameRenderer, HiddenRegion, LayerColorLut,
    LayerPlacement, LayerSizing, RenderError, TextRasterizer, TextReveal, VisualLayer,
    hide_regions,
};

use super::clip_layers::plan_track_layers;
use crate::color_grading::{ColorTableError, ColorTableLibrary, applied_color_lut_of};
use crate::proxies::{ReadyProxies, hidden_region_for_picture_size};

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
    /// The LUT files read so far.
    color_tables: ColorTableLibrary,
    /// Proxies to read instead of their originals: the preview's only.
    proxies: Option<Arc<ReadyProxies>>,
}

impl FrameComposer {
    pub(crate) fn new(renderer: Box<dyn FrameRenderer>) -> Self {
        Self {
            renderer,
            open_sources: HashMap::new(),
            text_rasterizer: None,
            text_pictures: HashMap::new(),
            color_tables: ColorTableLibrary::default(),
            proxies: None,
        }
    }

    /// Reads the proxies listed in `proxies` instead of their originals.
    pub(crate) fn use_proxies(&mut self, proxies: Arc<ReadyProxies>) {
        self.proxies = Some(proxies);
    }

    /// Closes every decoder (they reopen when needed), so their files can be
    /// deleted.
    pub(crate) fn close_open_sources(&mut self) {
        self.open_sources.clear();
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
        let canvas_size = project.settings().canvas_size;
        self.compose_frame_at_size(project, backend, timeline_time, canvas_size)
    }

    /// The picture of `project`'s timeline at `timeline_time`, composed on a
    /// canvas of `drawn_size` (the canvas's shape, smaller: the preview's): what
    /// is measured in canvas pixels (positions, text sizes) is scaled to it,
    /// while videos fit it as they fit the canvas. At the canvas's own size, the
    /// export's picture exactly.
    pub(crate) fn compose_frame_at_size<Error: From<MediaError> + From<RenderError>>(
        &mut self,
        project: &Project,
        backend: &dyn MediaBackend,
        timeline_time: Timestamp,
        drawn_size: PictureSize,
    ) -> Result<VideoFrame, Error> {
        let settings = project.settings();
        let canvas_scale = CanvasScale::between(settings.canvas_size, drawn_size);
        let mut prepared_layers: Vec<PreparedLayer> = Vec::new();
        let mut clips_shown = HashSet::new();
        let visible_tracks = project
            .timeline()
            .tracks()
            .iter()
            .filter(|track| track.kind == TrackKind::Video && !track.is_hidden);
        for track in visible_tracks {
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
                    let placement = canvas_scale.applied_to(
                        LayerPlacement::of_clip_at(clip, planned_layer.time_in_clip)
                            .moved_by(planned_layer.motion),
                    );
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
            size: drawn_size,
            background: settings.background_color,
        };
        Ok(self.renderer.render_frame(&canvas, &layers)?)
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
        let original_path = media_asset.description.file_path.as_path();
        let (picture, is_from_proxy) =
            self.picture_preferably_from_proxy(backend, clip.id, original_path, source_time)?;
        let mut hidden_regions = hidden_regions_of(clip, source_time);
        if hidden_regions.is_empty() {
            return Ok(Some(picture));
        }
        if is_from_proxy && let Some(original_size) = media_asset.description.picture_size {
            let shown_size = PictureSize {
                width_in_pixels: picture.width(),
                height_in_pixels: picture.height(),
            };
            for region in &mut hidden_regions {
                *region = hidden_region_for_picture_size(region, original_size, shown_size);
            }
        }
        Ok(Some(Arc::new(hide_regions(&picture, &hidden_regions))))
    }

    /// The picture of `original_path` at `source_time`, read from its proxy when
    /// one is listed (and `true`), or from the file itself. A proxy that cannot
    /// be read is dropped from the list, and the original is read instead.
    fn picture_preferably_from_proxy(
        &mut self,
        backend: &dyn MediaBackend,
        clip_id: ClipId,
        original_path: &Path,
        source_time: Timestamp,
    ) -> Result<(Arc<VideoFrame>, bool), MediaError> {
        let proxy = self
            .proxies
            .as_ref()
            .and_then(|proxies| proxies.proxy_for(original_path));
        if let Some(proxy) = proxy {
            let from_proxy = self
                .video_source_of(backend, clip_id, &proxy.proxy_path, true)
                .and_then(|source| source.frame_at(source_time));
            match from_proxy {
                Ok(picture) => return Ok((picture, true)),
                Err(_) => {
                    if let Some(proxies) = &self.proxies {
                        proxies.remove(original_path);
                    }
                    self.open_sources.remove(&clip_id);
                }
            }
        }
        let picture = self
            .video_source_of(backend, clip_id, original_path, false)?
            .frame_at(source_time)?;
        Ok((picture, false))
    }

    /// The decoder of `clip_id`, opened (or opened again) if it does not read
    /// `file_path`; a proxy (`is_proxy`) is opened for quick jumps.
    fn video_source_of(
        &mut self,
        backend: &dyn MediaBackend,
        clip_id: ClipId,
        file_path: &Path,
        is_proxy: bool,
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
                source: if is_proxy {
                    backend.open_video_proxy(file_path)?
                } else {
                    backend.open_video_source(file_path)?
                },
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

/// How much smaller than the project's canvas a picture is composed.
#[derive(Debug, Clone, Copy, PartialEq)]
struct CanvasScale {
    horizontal: f32,
    vertical: f32,
}

impl CanvasScale {
    fn between(canvas_size: PictureSize, drawn_size: PictureSize) -> Self {
        let ratio = |drawn: u32, canvas: u32| {
            if drawn == canvas || canvas == 0 {
                1.0
            } else {
                drawn as f32 / canvas as f32
            }
        };
        Self {
            horizontal: ratio(drawn_size.width_in_pixels, canvas_size.width_in_pixels),
            vertical: ratio(drawn_size.height_in_pixels, canvas_size.height_in_pixels),
        }
    }

    /// `placement`, in the canvas's pixels, in the smaller picture's pixels.
    /// At the canvas's own size it is returned untouched.
    fn applied_to(self, placement: LayerPlacement) -> LayerPlacement {
        if self.horizontal == 1.0 && self.vertical == 1.0 {
            return placement;
        }
        let scale_factor = match placement.sizing {
            // Fitted pictures follow the canvas by themselves.
            LayerSizing::FitInsideCanvas => placement.scale_factor,
            LayerSizing::NativePixels => {
                placement.scale_factor * self.horizontal.min(self.vertical)
            }
        };
        LayerPlacement {
            offset_x_in_pixels: placement.offset_x_in_pixels * self.horizontal,
            offset_y_in_pixels: placement.offset_y_in_pixels * self.vertical,
            scale_factor,
            ..placement
        }
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
