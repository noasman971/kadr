//! The rules a project must always respect.
//!
//! [`validate_project`] is the single place where every rule is written down.
//! It runs after every command (a change that breaks a rule is undone and
//! refused) and on every loaded file (a corrupted file is rejected), so the rest
//! of Kadr can trust that a project is always valid.

use std::collections::HashSet;

use crate::animation::Interpolation;
use crate::clip::{AnimatableProperty, Clip, ClipContent, CropInsets, MediaSegment, TextContent};
use crate::effect::{
    BlurStyle, ColorAdjustment, ColorLut, Effect, EffectKind, Stabilization, TrackedBlur,
};
use crate::ids::{ClipId, EffectId, MediaAssetId, TrackId};
use crate::media_asset::{MediaAsset, MediaKind};
use crate::project::{Project, ProjectSettings};
use crate::text_animation::TextAnimation;
use crate::time::{TimeSpan, Timestamp};
use crate::timeline::{Track, TrackKind};
use crate::transition::{MAXIMUM_EDGE_ANIMATION_DURATION, Transition};

/// Smallest canvas side, in pixels.
const MINIMUM_CANVAS_SIDE_IN_PIXELS: u32 = 16;
/// Largest canvas side, in pixels (8K).
const MAXIMUM_CANVAS_SIDE_IN_PIXELS: u32 = 8_192;
/// Highest project frame rate.
const MAXIMUM_FRAMES_PER_SECOND: f64 = 240.0;
/// Audio sample rates an export can use.
const SUPPORTED_AUDIO_SAMPLE_RATES_IN_HERTZ: [u32; 12] = [
    8_000, 11_025, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000, 88_200, 96_000, 176_400, 192_000,
];
/// Largest text size, in canvas pixels.
const MAXIMUM_FONT_SIZE_IN_PIXELS: f32 = 2_000.0;

/// A broken rule. The message says which object breaks which rule.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ValidationError {
    /// The project settings are not usable.
    #[error("invalid project settings: {reason}")]
    InvalidSettings {
        /// What is wrong.
        reason: String,
    },

    /// Two objects share the same identifier.
    #[error("identifier {raw_id} is used by more than one object")]
    DuplicateIdentifier {
        /// The shared number.
        raw_id: u64,
    },

    /// An object uses an identifier the allocator has not handed out yet, so a
    /// future object could get the same one.
    #[error("identifier {raw_id} is used but the next identifier to hand out is {next_raw_id}")]
    IdentifierNotYetAllocated {
        /// The number in use.
        raw_id: u64,
        /// The allocator's next number.
        next_raw_id: u64,
    },

    /// A media file description is inconsistent.
    #[error("media {media_asset_id} is invalid: {reason}")]
    InvalidMediaAsset {
        /// The media file.
        media_asset_id: MediaAssetId,
        /// What is wrong.
        reason: String,
    },

    /// The clips of a track are not sorted by start time.
    #[error("the clips of track {track_id} are not sorted by start time")]
    TrackClipsNotSorted {
        /// The track.
        track_id: TrackId,
    },

    /// A clip is on a track that cannot hold it.
    #[error("clip {clip_id} cannot be on {track_kind_name} track {track_id}: {reason}")]
    ClipNotAllowedOnTrack {
        /// The clip.
        clip_id: ClipId,
        /// The track.
        track_id: TrackId,
        /// `"video"` or `"audio"`.
        track_kind_name: &'static str,
        /// Why not.
        reason: String,
    },

    /// A clip shows a media file that is not in the project.
    #[error("clip {clip_id} shows media {media_asset_id}, which is not in the project")]
    MissingMediaAsset {
        /// The clip.
        clip_id: ClipId,
        /// The missing media file.
        media_asset_id: MediaAssetId,
    },

    /// A clip's range of its media file is impossible.
    #[error("clip {clip_id} has an invalid media range: {reason}")]
    InvalidMediaRange {
        /// The clip.
        clip_id: ClipId,
        /// What is wrong.
        reason: String,
    },

    /// A clip lasts no time at all.
    #[error("clip {clip_id} lasts no time at all")]
    EmptyClip {
        /// The clip.
        clip_id: ClipId,
    },

    /// Two clips of the same track overlap.
    #[error(
        "clips {earlier_clip_id} and {later_clip_id} overlap on track {track_id} between {overlap_start} and {overlap_end}"
    )]
    ClipsOverlap {
        /// The track.
        track_id: TrackId,
        /// The clip starting first.
        earlier_clip_id: ClipId,
        /// The clip starting second.
        later_clip_id: ClipId,
        /// Where the overlap starts.
        overlap_start: Timestamp,
        /// Where the overlap ends.
        overlap_end: Timestamp,
    },

    /// A text clip is invalid.
    #[error("text clip {clip_id} is invalid: {reason}")]
    InvalidText {
        /// The clip.
        clip_id: ClipId,
        /// What is wrong.
        reason: String,
    },

    /// A clip property has a value out of its allowed range.
    #[error(
        "the {property} of clip {clip_id} must be between {minimum} and {maximum}, but is {value}"
    )]
    PropertyValueOutOfRange {
        /// The clip.
        clip_id: ClipId,
        /// The property.
        property: AnimatableProperty,
        /// The rejected value.
        value: f32,
        /// Smallest allowed value.
        minimum: f32,
        /// Largest allowed value.
        maximum: f32,
    },

    /// Keyframes are not sorted or two share the same moment.
    #[error(
        "the keyframes of the {property} of clip {clip_id} are not in strictly increasing time order"
    )]
    KeyframesNotSorted {
        /// The clip.
        clip_id: ClipId,
        /// The property.
        property: AnimatableProperty,
    },

    /// A keyframe's Bézier curve has a handle outside the unit square.
    #[error(
        "the curve after the {property} keyframe of clip {clip_id} at {time_offset} from the clip start is invalid: its handles must have coordinates from 0 to 1"
    )]
    InvalidKeyframeCurve {
        /// The clip.
        clip_id: ClipId,
        /// The property.
        property: AnimatableProperty,
        /// The keyframe, from the clip start.
        time_offset: TimeSpan,
    },

    /// A clip's crop is impossible.
    #[error("the crop of clip {clip_id} is invalid: {reason}")]
    InvalidCrop {
        /// The clip.
        clip_id: ClipId,
        /// What is wrong.
        reason: String,
    },

    /// An effect is invalid or applied to a clip that cannot have it.
    #[error("effect {effect_id} of clip {clip_id} is invalid: {reason}")]
    InvalidEffect {
        /// The clip.
        clip_id: ClipId,
        /// The effect.
        effect_id: EffectId,
        /// What is wrong.
        reason: String,
    },

    /// A transition is invalid or on a clip that cannot have one.
    #[error("the transition at the {edge_name} of clip {clip_id} is invalid: {reason}")]
    InvalidTransition {
        /// The clip.
        clip_id: ClipId,
        /// `"start"` or `"end"`.
        edge_name: &'static str,
        /// What is wrong.
        reason: String,
    },

    /// A text animation is invalid.
    #[error("the animation at the {edge_name} of text clip {clip_id} is invalid: {reason}")]
    InvalidTextAnimation {
        /// The clip.
        clip_id: ClipId,
        /// `"start"` or `"end"`.
        edge_name: &'static str,
        /// What is wrong.
        reason: String,
    },
}

/// Checks every rule. Returns the first broken rule found.
///
/// # Errors
///
/// Returns the first [`ValidationError`] found.
pub fn validate_project(project: &Project) -> Result<(), ValidationError> {
    validate_settings(project.settings())?;
    validate_identifiers(project)?;
    for media_asset in project.media_assets() {
        validate_media_asset(media_asset)?;
    }
    for track in project.timeline().tracks() {
        validate_track(project, track)?;
    }
    Ok(())
}

// ----- Settings and identifiers -----

fn validate_settings(settings: &ProjectSettings) -> Result<(), ValidationError> {
    let invalid = |reason: String| Err(ValidationError::InvalidSettings { reason });
    let canvas_size = settings.canvas_size;
    for (side_name, side_in_pixels) in [
        ("width", canvas_size.width_in_pixels),
        ("height", canvas_size.height_in_pixels),
    ] {
        if !(MINIMUM_CANVAS_SIDE_IN_PIXELS..=MAXIMUM_CANVAS_SIDE_IN_PIXELS)
            .contains(&side_in_pixels)
        {
            return invalid(format!(
                "the canvas {side_name} must be between {MINIMUM_CANVAS_SIDE_IN_PIXELS} and {MAXIMUM_CANVAS_SIDE_IN_PIXELS} pixels, but is {side_in_pixels}"
            ));
        }
        // H.264 and most codecs store colour at half resolution: sides must be even.
        if side_in_pixels % 2 != 0 {
            return invalid(format!(
                "the canvas {side_name} must be an even number of pixels, but is {side_in_pixels}"
            ));
        }
    }
    if settings.frame_rate.as_frames_per_second() > MAXIMUM_FRAMES_PER_SECOND {
        return invalid(format!(
            "the frame rate must be at most {MAXIMUM_FRAMES_PER_SECOND} fps, but is {}",
            settings.frame_rate
        ));
    }
    if !SUPPORTED_AUDIO_SAMPLE_RATES_IN_HERTZ.contains(&settings.audio_sample_rate_in_hertz) {
        return invalid(format!(
            "{} Hz is not a supported audio sample rate (use 44100 or 48000)",
            settings.audio_sample_rate_in_hertz
        ));
    }
    Ok(())
}

/// Every identifier in use, of every kind.
fn all_raw_identifiers(project: &Project) -> Vec<u64> {
    let media_ids = project.media_assets().iter().map(|asset| asset.id.raw());
    let track_ids = project
        .timeline()
        .tracks()
        .iter()
        .map(|track| track.id.raw());
    let clip_and_effect_ids = project.timeline().all_clips().flat_map(|(_, clip)| {
        std::iter::once(clip.id.raw()).chain(clip.effects.iter().map(|effect| effect.id.raw()))
    });
    media_ids
        .chain(track_ids)
        .chain(clip_and_effect_ids)
        .collect()
}

fn validate_identifiers(project: &Project) -> Result<(), ValidationError> {
    let next_raw_id = project.id_allocator().next_raw_id();
    let mut seen_identifiers = HashSet::new();
    for raw_id in all_raw_identifiers(project) {
        if !seen_identifiers.insert(raw_id) {
            return Err(ValidationError::DuplicateIdentifier { raw_id });
        }
        if raw_id >= next_raw_id {
            return Err(ValidationError::IdentifierNotYetAllocated {
                raw_id,
                next_raw_id,
            });
        }
    }
    Ok(())
}

// ----- Media -----

fn validate_media_asset(media_asset: &MediaAsset) -> Result<(), ValidationError> {
    let description = &media_asset.description;
    let invalid = |reason: &str| {
        Err(ValidationError::InvalidMediaAsset {
            media_asset_id: media_asset.id,
            reason: reason.to_string(),
        })
    };
    if description.display_name.trim().is_empty() {
        return invalid("its display name is empty");
    }
    if description
        .duration
        .is_some_and(|duration| duration.is_zero())
    {
        return invalid("its duration is zero");
    }
    match description.kind {
        MediaKind::Video => {
            if description.duration.is_none() {
                return invalid("a video must have a duration");
            }
            if !description.has_picture() || description.frame_rate.is_none() {
                return invalid("a video must have a picture size and a frame rate");
            }
        }
        MediaKind::Audio => {
            if description.duration.is_none() || !description.has_audio() {
                return invalid("an audio file must have a duration and an audio format");
            }
            if description.has_picture() {
                return invalid("an audio file cannot have a picture size");
            }
        }
        MediaKind::Image => {
            if !description.has_picture() {
                return invalid("an image must have a picture size");
            }
            if description.duration.is_some() || description.has_audio() {
                return invalid("an image has neither a duration nor sound");
            }
        }
    }
    Ok(())
}

// ----- Tracks and clips -----

fn validate_track(project: &Project, track: &Track) -> Result<(), ValidationError> {
    if !track.has_clips_sorted_by_start() {
        return Err(ValidationError::TrackClipsNotSorted { track_id: track.id });
    }
    for clip in track.clips() {
        validate_clip(project, track, clip)?;
    }
    validate_no_overlap(track)
}

fn validate_no_overlap(track: &Track) -> Result<(), ValidationError> {
    for pair in track.clips().windows(2) {
        let (earlier_clip, later_clip) = (&pair[0], &pair[1]);
        let earlier_clip_end = earlier_clip.timeline_end();
        if earlier_clip_end > later_clip.timeline_start {
            return Err(ValidationError::ClipsOverlap {
                track_id: track.id,
                earlier_clip_id: earlier_clip.id,
                later_clip_id: later_clip.id,
                overlap_start: later_clip.timeline_start,
                overlap_end: earlier_clip_end.min(later_clip.timeline_end()),
            });
        }
    }
    Ok(())
}

fn validate_clip(project: &Project, track: &Track, clip: &Clip) -> Result<(), ValidationError> {
    match &clip.content {
        ClipContent::Media(segment) => validate_media_segment(project, track, clip.id, segment)?,
        ClipContent::Text(text) => validate_text(track, clip.id, text)?,
    }
    if clip.timeline_duration().is_zero() {
        return Err(ValidationError::EmptyClip { clip_id: clip.id });
    }
    validate_animated_properties(clip)?;
    validate_crop(clip.id, &clip.transform.crop)?;
    let clip_has_picture = clip_shows_picture(project, clip);
    for effect in &clip.effects {
        validate_effect(clip, clip_has_picture, effect)?;
    }
    validate_at_most_one_stabilization(clip)?;
    validate_transitions(clip, clip_has_picture)
}

/// Whether the clip shows pictures (a text, a video or an image).
fn clip_shows_picture(project: &Project, clip: &Clip) -> bool {
    match &clip.content {
        ClipContent::Text(_) => true,
        ClipContent::Media(segment) => project
            .media_asset(segment.media_asset_id)
            .is_some_and(|asset| asset.description.has_picture()),
    }
}

fn validate_media_segment(
    project: &Project,
    track: &Track,
    clip_id: ClipId,
    segment: &MediaSegment,
) -> Result<(), ValidationError> {
    let media_asset =
        project
            .media_asset(segment.media_asset_id)
            .ok_or(ValidationError::MissingMediaAsset {
                clip_id,
                media_asset_id: segment.media_asset_id,
            })?;
    let description = &media_asset.description;
    let not_allowed = |reason: &str| {
        Err(ValidationError::ClipNotAllowedOnTrack {
            clip_id,
            track_id: track.id,
            track_kind_name: track.kind.display_name(),
            reason: reason.to_string(),
        })
    };
    match track.kind {
        TrackKind::Video if !description.has_picture() => {
            return not_allowed("its media has no picture; put it on an audio track");
        }
        TrackKind::Audio if !description.has_audio() => {
            return not_allowed("its media has no sound; put it on a video track");
        }
        _ => {}
    }

    let invalid_range =
        |reason: String| Err(ValidationError::InvalidMediaRange { clip_id, reason });
    if segment.source_start >= segment.source_end {
        return invalid_range(format!(
            "it starts at {} and ends at {} in its media file",
            segment.source_start, segment.source_end
        ));
    }
    if let Some(media_duration) = description.duration {
        let media_end = Timestamp::ZERO + media_duration;
        if segment.source_end > media_end {
            return invalid_range(format!(
                "it ends at {} but media {} only lasts {}",
                segment.source_end, segment.media_asset_id, media_duration
            ));
        }
    }
    Ok(())
}

fn validate_text(
    track: &Track,
    clip_id: ClipId,
    text: &TextContent,
) -> Result<(), ValidationError> {
    if track.kind != TrackKind::Video {
        return Err(ValidationError::ClipNotAllowedOnTrack {
            clip_id,
            track_id: track.id,
            track_kind_name: track.kind.display_name(),
            reason: "texts go on video tracks".to_string(),
        });
    }
    let invalid = |reason: String| Err(ValidationError::InvalidText { clip_id, reason });
    if text.text.trim().is_empty() {
        return invalid("the text is empty".to_string());
    }
    if text.style.font_family.trim().is_empty() {
        return invalid("the font name is empty".to_string());
    }
    let font_size = text.style.font_size_in_pixels;
    if !font_size.is_finite() || !(1.0..=MAXIMUM_FONT_SIZE_IN_PIXELS).contains(&font_size) {
        return invalid(format!(
            "the font size must be between 1 and {MAXIMUM_FONT_SIZE_IN_PIXELS} pixels, but is {font_size}"
        ));
    }
    validate_text_animations(clip_id, text)
}

fn validate_text_animations(clip_id: ClipId, text: &TextContent) -> Result<(), ValidationError> {
    let edges = [
        ("start", text.entry_animation),
        ("end", text.exit_animation),
    ];
    for (edge_name, animation) in edges {
        let Some(TextAnimation { duration, .. }) = animation else {
            continue;
        };
        if let Some(reason) = find_edge_duration_problem(duration) {
            return Err(ValidationError::InvalidTextAnimation {
                clip_id,
                edge_name,
                reason,
            });
        }
    }
    Ok(())
}

// ----- Transitions -----

fn validate_transitions(clip: &Clip, clip_has_picture: bool) -> Result<(), ValidationError> {
    let edges = [
        ("start", clip.entry_transition),
        ("end", clip.exit_transition),
    ];
    for (edge_name, transition) in edges {
        let Some(Transition { duration, .. }) = transition else {
            continue;
        };
        let problem = match &clip.content {
            ClipContent::Text(_) => Some(
                "texts take text animations (fade, slide, pop, type-on…) instead of transitions"
                    .to_string(),
            ),
            ClipContent::Media(_) if !clip_has_picture => {
                Some("a transition needs a picture, and this clip only has sound".to_string())
            }
            ClipContent::Media(_) => find_edge_duration_problem(duration),
        };
        if let Some(reason) = problem {
            return Err(ValidationError::InvalidTransition {
                clip_id: clip.id,
                edge_name,
                reason,
            });
        }
    }
    Ok(())
}

/// Describes what is wrong with the duration of a transition or text animation.
/// Durations longer than the clip are allowed: they are shortened to fit when
/// drawn, so trimming or splitting a clip is never refused because of them.
fn find_edge_duration_problem(duration: TimeSpan) -> Option<String> {
    if duration.is_zero() {
        return Some("it lasts no time at all".to_string());
    }
    (duration > MAXIMUM_EDGE_ANIMATION_DURATION).then(|| {
        format!("it lasts {duration}, longer than the maximum of {MAXIMUM_EDGE_ANIMATION_DURATION}")
    })
}

fn validate_animated_properties(clip: &Clip) -> Result<(), ValidationError> {
    for property in AnimatableProperty::ALL {
        let animated_value = clip.animated_property(property);
        if !animated_value.has_strictly_increasing_keyframe_times() {
            return Err(ValidationError::KeyframesNotSorted {
                clip_id: clip.id,
                property,
            });
        }
        if let Some(rejected_value) = animated_value
            .all_values()
            .find(|value| !property.accepts_value(*value))
        {
            let (minimum, maximum) = property.allowed_range();
            return Err(ValidationError::PropertyValueOutOfRange {
                clip_id: clip.id,
                property,
                value: rejected_value,
                minimum,
                maximum,
            });
        }
        // Handles inside the unit square keep every value between the two
        // keyframes' values, so the range check above covers every moment.
        if let Some(keyframe) = animated_value.keyframes().iter().find(|keyframe| {
            matches!(keyframe.interpolation_to_next,
                Interpolation::CubicBezier(curve) if !curve.has_handles_inside_the_unit_square())
        }) {
            return Err(ValidationError::InvalidKeyframeCurve {
                clip_id: clip.id,
                property,
                time_offset: keyframe.time_offset,
            });
        }
    }
    Ok(())
}

fn validate_crop(clip_id: ClipId, crop: &CropInsets) -> Result<(), ValidationError> {
    let invalid = |reason: &str| {
        Err(ValidationError::InvalidCrop {
            clip_id,
            reason: reason.to_string(),
        })
    };
    let all_fractions = [
        crop.left_fraction,
        crop.top_fraction,
        crop.right_fraction,
        crop.bottom_fraction,
    ];
    if all_fractions
        .iter()
        .any(|fraction| !fraction.is_finite() || !(0.0..1.0).contains(fraction))
    {
        return invalid("each edge must be a fraction from 0 (nothing removed) to below 1");
    }
    if crop.left_fraction + crop.right_fraction >= 1.0
        || crop.top_fraction + crop.bottom_fraction >= 1.0
    {
        return invalid("opposite edges together remove the whole picture");
    }
    Ok(())
}

// ----- Effects -----

fn validate_effect(
    clip: &Clip,
    clip_has_picture: bool,
    effect: &Effect,
) -> Result<(), ValidationError> {
    let invalid = |reason: String| {
        Err(ValidationError::InvalidEffect {
            clip_id: clip.id,
            effect_id: effect.id,
            reason,
        })
    };
    if effect.kind.needs_picture() && !clip_has_picture {
        return invalid(format!(
            "a {} needs a picture, and this clip only has sound",
            effect.kind.display_name()
        ));
    }
    let problem = match &effect.kind {
        EffectKind::ColorAdjustment(adjustment) => find_color_adjustment_problem(adjustment),
        EffectKind::ColorLut(color_lut) => find_color_lut_problem(color_lut),
        EffectKind::TrackedBlur(tracked_blur) => {
            if !matches!(clip.content, ClipContent::Media(_)) {
                Some("a tracked blur only applies to a video or an image".to_string())
            } else {
                find_tracked_blur_problem(tracked_blur)
            }
        }
        EffectKind::Stabilization(stabilization) => {
            if !matches!(clip.content, ClipContent::Media(_)) {
                Some("a stabilisation only applies to a video".to_string())
            } else {
                find_stabilization_problem(stabilization)
            }
        }
    };
    match problem {
        Some(reason) => invalid(reason),
        None => Ok(()),
    }
}

/// Two stabilisations of one clip would correct the shake twice: a clip has at
/// most one, and the second is refused (this also settles a race between the
/// user and Claude stabilising the same clip).
fn validate_at_most_one_stabilization(clip: &Clip) -> Result<(), ValidationError> {
    let mut stabilizations = clip
        .effects
        .iter()
        .filter(|effect| matches!(effect.kind, EffectKind::Stabilization(_)));
    let (Some(first), Some(second)) = (stabilizations.next(), stabilizations.next()) else {
        return Ok(());
    };
    Err(ValidationError::InvalidEffect {
        clip_id: clip.id,
        effect_id: second.id,
        reason: format!(
            "a clip has at most one stabilisation, and effect {} already steadies this one",
            first.id
        ),
    })
}

/// Describes the first value out of range, if any.
fn first_out_of_range(checks: &[(&str, f32, f32, f32)]) -> Option<String> {
    checks.iter().find_map(|&(name, value, minimum, maximum)| {
        let is_valid = value.is_finite() && (minimum..=maximum).contains(&value);
        (!is_valid)
            .then(|| format!("the {name} must be between {minimum} and {maximum}, but is {value}"))
    })
}

fn find_color_adjustment_problem(adjustment: &ColorAdjustment) -> Option<String> {
    first_out_of_range(&[
        ("exposure", adjustment.exposure_in_stops, -10.0, 10.0),
        ("contrast", adjustment.contrast_multiplier, 0.0, 4.0),
        ("saturation", adjustment.saturation_multiplier, 0.0, 4.0),
        ("temperature", adjustment.temperature_shift, -1.0, 1.0),
    ])
}

/// Only the path's form: whether the file exists and can be read is checked by
/// the engine, never here (validation runs after every change and on load, and
/// a moved LUT must not make the project impossible to open).
fn find_color_lut_problem(color_lut: &ColorLut) -> Option<String> {
    if color_lut.file_path.as_os_str().is_empty() {
        return Some("it names no .cube file".to_string());
    }
    let is_cube_file = color_lut
        .file_path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(ColorLut::FILE_EXTENSION));
    if !is_cube_file {
        return Some(format!(
            "\"{}\" is not a .cube file: colour LUTs are read in the .cube format",
            color_lut.file_path.display()
        ));
    }
    first_out_of_range(&[("strength", color_lut.strength_fraction, 0.0, 1.0)])
}

fn find_tracked_blur_problem(tracked_blur: &TrackedBlur) -> Option<String> {
    let style_check = match tracked_blur.style {
        BlurStyle::GaussianBlur { radius_in_pixels } => {
            ("blur radius", radius_in_pixels, 1.0, 500.0)
        }
        BlurStyle::Pixelate {
            block_size_in_pixels,
        } => ("pixel block size", block_size_in_pixels as f32, 2.0, 512.0),
        BlurStyle::SolidFill { .. } => ("fill", 0.0, 0.0, 0.0),
    };
    if let Some(problem) = first_out_of_range(&[
        style_check,
        ("margin", tracked_blur.margin_in_percent, 0.0, 200.0),
        (
            "edge softness",
            tracked_blur.edge_softness_in_pixels,
            0.0,
            500.0,
        ),
    ]) {
        return Some(problem);
    }
    let samples_are_sorted = tracked_blur
        .region_samples
        .windows(2)
        .all(|pair| pair[0].source_time < pair[1].source_time);
    if !samples_are_sorted {
        return Some("its region samples are not in strictly increasing time order".to_string());
    }
    tracked_blur
        .region_samples
        .iter()
        .find(|sample| !sample.region.is_well_formed())
        .map(|sample| {
            format!(
                "its region at {} has a non-positive size or a non-finite coordinate",
                sample.source_time
            )
        })
}

/// Only the form: the zoom is derived from the path by the engine, never here.
fn find_stabilization_problem(stabilization: &Stabilization) -> Option<String> {
    if let Some(problem) = first_out_of_range(&[
        (
            "smoothing",
            stabilization.smoothing_in_seconds,
            Stabilization::MINIMUM_SMOOTHING_IN_SECONDS,
            Stabilization::MAXIMUM_SMOOTHING_IN_SECONDS,
        ),
        (
            "zoom",
            stabilization.zoom_factor,
            Stabilization::MINIMUM_ZOOM_FACTOR,
            Stabilization::MAXIMUM_ZOOM_FACTOR,
        ),
    ]) {
        return Some(problem);
    }
    let picture_size = stabilization.analysed_picture_size;
    if picture_size.width_in_pixels == 0 || picture_size.height_in_pixels == 0 {
        return Some(format!(
            "its analysed picture size is {} × {} pixels: neither side may be zero",
            picture_size.width_in_pixels, picture_size.height_in_pixels
        ));
    }
    find_camera_path_problem(stabilization)
}

fn find_camera_path_problem(stabilization: &Stabilization) -> Option<String> {
    let camera_path = &stabilization.camera_path;
    if camera_path.is_empty() {
        return Some("its camera path is empty".to_string());
    }
    let samples_are_sorted = camera_path
        .windows(2)
        .all(|pair| pair[0].source_time < pair[1].source_time);
    if !samples_are_sorted {
        return Some("its camera path is not in strictly increasing time order".to_string());
    }
    camera_path.iter().find_map(|sample| {
        let all_finite = [
            sample.shift_x_in_pixels,
            sample.shift_y_in_pixels,
            sample.rotation_in_degrees,
            sample.scale_factor,
        ]
        .iter()
        .all(|value| value.is_finite());
        if !all_finite {
            return Some(format!(
                "its camera path at {} has a non-finite value",
                sample.source_time
            ));
        }
        let scale_range =
            Stabilization::MINIMUM_PATH_SCALE_FACTOR..=Stabilization::MAXIMUM_PATH_SCALE_FACTOR;
        (!scale_range.contains(&sample.scale_factor)).then(|| {
            format!(
                "its camera path at {} has a scale of {}, outside {} to {}",
                sample.source_time,
                sample.scale_factor,
                Stabilization::MINIMUM_PATH_SCALE_FACTOR,
                Stabilization::MAXIMUM_PATH_SCALE_FACTOR
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::CropInsets;
    use crate::commands::{
        AddClip, AddEffect, AddTrack, ChangeProjectSettings, ClipEdge, CreatedObjectsExt,
        ImportMedia, RemoveEffect, SetClipCrop, SetClipText, SetClipTransition, SetTextAnimation,
        SplitClip, TrackPosition, TrimClip,
    };
    use crate::effect::test_samples::stabilization_of_frames;
    use crate::effect::{
        CameraPathSample, PixelRegion, RegionSample, RegionSampleOrigin, TrackingConfidence,
    };
    use crate::error::EditError;
    use crate::history::EditSession;
    use crate::media_asset::PictureSize;
    use crate::media_asset::test_samples::{logo_image, one_minute_music, ten_second_video};
    use crate::project_file::{ProjectFileError, load_project_from_json, save_project_to_json};
    use crate::text_animation::TextAnimationStyle;
    use crate::time::TimeSpan;
    use crate::transition::TransitionStyle;

    fn seconds(value: u64) -> Timestamp {
        Timestamp::from_whole_seconds(value)
    }

    /// A session with a video track, an audio track, and three imported media.
    struct Fixture {
        session: EditSession,
        video_track_id: TrackId,
        audio_track_id: TrackId,
        video_id: MediaAssetId,
        music_id: MediaAssetId,
        image_id: MediaAssetId,
    }

    fn fixture() -> Fixture {
        let mut session =
            EditSession::new(Project::new(ProjectSettings::landscape_1080p())).unwrap();
        let mut import = |description| {
            session
                .execute(ImportMedia::new(description))
                .unwrap()
                .first_media_asset_id()
                .unwrap()
        };
        let (video_id, music_id, image_id) = (
            import(ten_second_video()),
            import(one_minute_music()),
            import(logo_image()),
        );
        let mut add_track = |kind| {
            session
                .execute(AddTrack::new(kind, TrackPosition::Top))
                .unwrap()
                .first_track_id()
                .unwrap()
        };
        let (video_track_id, audio_track_id) =
            (add_track(TrackKind::Video), add_track(TrackKind::Audio));
        Fixture {
            session,
            video_track_id,
            audio_track_id,
            video_id,
            music_id,
            image_id,
        }
    }

    fn refused_rule(
        result: Result<Vec<crate::commands::CreatedObject>, EditError>,
    ) -> ValidationError {
        match result {
            Err(EditError::WouldMakeProjectInvalid(validation_error)) => validation_error,
            other => panic!("expected a refused change, got {other:?}"),
        }
    }

    #[test]
    fn media_must_match_the_kind_of_track() {
        let mut fixture = fixture();
        let music_on_video_track = fixture.session.execute(AddClip::media(
            fixture.video_track_id,
            fixture.music_id,
            seconds(0),
            seconds(5),
            seconds(0),
        ));
        assert!(matches!(
            refused_rule(music_on_video_track),
            ValidationError::ClipNotAllowedOnTrack { .. }
        ));

        let image_on_audio_track = fixture.session.execute(AddClip::media(
            fixture.audio_track_id,
            fixture.image_id,
            seconds(0),
            seconds(5),
            seconds(0),
        ));
        assert!(matches!(
            refused_rule(image_on_audio_track),
            ValidationError::ClipNotAllowedOnTrack { .. }
        ));

        let text_on_audio_track = fixture.session.execute(AddClip::text(
            fixture.audio_track_id,
            "Hi",
            TimeSpan::from_whole_seconds(1),
            seconds(0),
        ));
        assert!(matches!(
            refused_rule(text_on_audio_track),
            ValidationError::ClipNotAllowedOnTrack { .. }
        ));

        // A video with sound may go on an audio track (only its sound is used).
        fixture
            .session
            .execute(AddClip::media(
                fixture.audio_track_id,
                fixture.video_id,
                seconds(0),
                seconds(5),
                seconds(0),
            ))
            .unwrap();
    }

    #[test]
    fn a_clip_cannot_go_past_the_end_of_its_media() {
        let mut fixture = fixture();
        let clip_id = fixture
            .session
            .execute(AddClip::media(
                fixture.video_track_id,
                fixture.video_id,
                seconds(5),
                seconds(10),
                seconds(0),
            ))
            .unwrap()
            .first_clip_id()
            .unwrap();

        let error = refused_rule(fixture.session.execute(TrimClip::new(
            clip_id,
            ClipEdge::End,
            seconds(6),
        )));
        assert_eq!(
            error.to_string(),
            format!(
                "clip {clip_id} has an invalid media range: it ends at 11.000 s but media {} only lasts 10.000 s",
                fixture.video_id
            )
        );
    }

    #[test]
    fn a_still_image_can_last_any_time() {
        let mut fixture = fixture();
        fixture
            .session
            .execute(AddClip::media(
                fixture.video_track_id,
                fixture.image_id,
                seconds(0),
                seconds(3_600),
                seconds(0),
            ))
            .unwrap();
    }

    #[test]
    fn texts_must_not_be_empty() {
        let mut fixture = fixture();
        let clip_id = fixture
            .session
            .execute(AddClip::text(
                fixture.video_track_id,
                "Title",
                TimeSpan::from_whole_seconds(2),
                seconds(0),
            ))
            .unwrap()
            .first_clip_id()
            .unwrap();
        let error = refused_rule(fixture.session.execute(SetClipText::new(clip_id, "   ")));
        assert!(matches!(error, ValidationError::InvalidText { .. }));
    }

    #[test]
    fn bezier_curve_handles_must_stay_inside_the_unit_square() {
        use crate::animation::{BezierCurve, BezierHandle, Keyframe};
        use crate::commands::SetKeyframe;

        let mut fixture = fixture();
        let clip_id = fixture
            .session
            .execute(AddClip::text(
                fixture.video_track_id,
                "Title",
                TimeSpan::from_whole_seconds(2),
                seconds(0),
            ))
            .unwrap()
            .first_clip_id()
            .unwrap();
        let keyframe_with_end_handle = |progress_fraction| Keyframe {
            time_offset: TimeSpan::from_milliseconds(500),
            value: 0.0,
            interpolation_to_next: Interpolation::CubicBezier(BezierCurve {
                start_handle: BezierHandle {
                    time_fraction: 0.25,
                    progress_fraction: 0.0,
                },
                end_handle: BezierHandle {
                    time_fraction: 0.75,
                    progress_fraction,
                },
            }),
        };
        // An overshoot would take the opacity above 1 between the keyframes.
        let error = refused_rule(fixture.session.execute(SetKeyframe::new(
            clip_id,
            AnimatableProperty::Opacity,
            keyframe_with_end_handle(1.4),
        )));
        assert_eq!(
            error.to_string(),
            format!(
                "the curve after the opacity keyframe of clip {clip_id} at 0.500 s from the clip start is invalid: its handles must have coordinates from 0 to 1"
            )
        );
        fixture
            .session
            .execute(SetKeyframe::new(
                clip_id,
                AnimatableProperty::Opacity,
                keyframe_with_end_handle(1.0),
            ))
            .unwrap();
    }

    #[test]
    fn crops_cannot_remove_the_whole_picture() {
        let mut fixture = fixture();
        let clip_id = fixture
            .session
            .execute(AddClip::media(
                fixture.video_track_id,
                fixture.video_id,
                seconds(0),
                seconds(2),
                seconds(0),
            ))
            .unwrap()
            .first_clip_id()
            .unwrap();
        let too_much = CropInsets {
            left_fraction: 0.6,
            right_fraction: 0.5,
            ..CropInsets::default()
        };
        let error = refused_rule(fixture.session.execute(SetClipCrop::new(clip_id, too_much)));
        assert!(matches!(error, ValidationError::InvalidCrop { .. }));
    }

    #[test]
    fn picture_effects_need_a_picture_and_valid_settings() {
        let mut fixture = fixture();
        let music_clip_id = fixture
            .session
            .execute(AddClip::media(
                fixture.audio_track_id,
                fixture.music_id,
                seconds(0),
                seconds(5),
                seconds(0),
            ))
            .unwrap()
            .first_clip_id()
            .unwrap();
        let blur_on_music = fixture.session.execute(AddEffect::new(
            music_clip_id,
            EffectKind::TrackedBlur(TrackedBlur::with_default_style()),
        ));
        assert!(matches!(
            refused_rule(blur_on_music),
            ValidationError::InvalidEffect { .. }
        ));

        let video_clip_id = fixture
            .session
            .execute(AddClip::media(
                fixture.video_track_id,
                fixture.video_id,
                seconds(0),
                seconds(5),
                seconds(0),
            ))
            .unwrap()
            .first_clip_id()
            .unwrap();
        let too_bright = ColorAdjustment {
            exposure_in_stops: 50.0,
            ..ColorAdjustment::NEUTRAL
        };
        let error = refused_rule(fixture.session.execute(AddEffect::new(
            video_clip_id,
            EffectKind::ColorAdjustment(too_bright),
        )));
        assert!(
            error
                .to_string()
                .contains("the exposure must be between -10 and 10, but is 50")
        );

        let mut unsorted_blur = TrackedBlur::with_default_style();
        let sample_at = |second| RegionSample {
            source_time: seconds(second),
            region: PixelRegion {
                left_in_pixels: 10.0,
                top_in_pixels: 10.0,
                width_in_pixels: 50.0,
                height_in_pixels: 60.0,
            },
            confidence: TrackingConfidence::Confident,
            origin: RegionSampleOrigin::Tracker,
        };
        unsorted_blur.region_samples = vec![sample_at(2), sample_at(1)];
        let error = refused_rule(fixture.session.execute(AddEffect::new(
            video_clip_id,
            EffectKind::TrackedBlur(unsorted_blur),
        )));
        assert!(
            error
                .to_string()
                .contains("not in strictly increasing time order")
        );
    }

    #[test]
    fn colour_luts_name_a_cube_file_and_have_a_strength_between_0_and_1() {
        let mut fixture = fixture();
        let music_clip_id = fixture
            .session
            .execute(AddClip::media(
                fixture.audio_track_id,
                fixture.music_id,
                seconds(0),
                seconds(5),
                seconds(0),
            ))
            .unwrap()
            .first_clip_id()
            .unwrap();
        let video_clip_id = fixture
            .session
            .execute(AddClip::media(
                fixture.video_track_id,
                fixture.video_id,
                seconds(0),
                seconds(5),
                seconds(0),
            ))
            .unwrap()
            .first_clip_id()
            .unwrap();
        let lut = |path: &str, strength_fraction: f32| {
            EffectKind::ColorLut(ColorLut {
                file_path: std::path::PathBuf::from(path),
                strength_fraction,
            })
        };
        let refusal = |fixture: &mut Fixture, clip_id, kind| {
            refused_rule(fixture.session.execute(AddEffect::new(clip_id, kind))).to_string()
        };

        assert!(
            refusal(&mut fixture, music_clip_id, lut("looks/warm.cube", 1.0))
                .contains("needs a picture")
        );
        assert!(refusal(&mut fixture, video_clip_id, lut("", 1.0)).contains("names no .cube file"));
        assert!(
            refusal(&mut fixture, video_clip_id, lut("looks/warm.png", 1.0))
                .contains("is not a .cube file")
        );
        assert!(
            refusal(&mut fixture, video_clip_id, lut("looks/warm.cube", 1.5))
                .contains("the strength must be between 0 and 1, but is 1.5")
        );
        assert!(
            refusal(
                &mut fixture,
                video_clip_id,
                lut("looks/warm.cube", f32::NAN)
            )
            .contains("strength")
        );

        // The extension is compared without case, and the file's existence is
        // the engine's business: this path exists nowhere.
        fixture
            .session
            .execute(AddEffect::new(
                video_clip_id,
                lut("looks/missing/Warm.CUBE", 0.5),
            ))
            .unwrap();
    }

    /// A fixture with a five-second video clip, and the clip's identifier.
    fn fixture_with_video_clip() -> (Fixture, ClipId) {
        let mut fixture = fixture();
        let video_clip_id = fixture
            .session
            .execute(AddClip::media(
                fixture.video_track_id,
                fixture.video_id,
                seconds(0),
                seconds(5),
                seconds(0),
            ))
            .unwrap()
            .first_clip_id()
            .unwrap();
        (fixture, video_clip_id)
    }

    /// The reason a stabilisation was refused on `clip_id`.
    fn stabilization_refusal(
        fixture: &mut Fixture,
        clip_id: ClipId,
        stabilization: Stabilization,
    ) -> String {
        let result = fixture.session.execute(AddEffect::new(
            clip_id,
            EffectKind::Stabilization(stabilization),
        ));
        refused_rule(result).to_string()
    }

    /// A stabilisation of the first two seconds whose path is rebuilt by `change`.
    fn stabilization_with_changed_path(
        change: impl FnOnce(&mut Vec<CameraPathSample>),
    ) -> Stabilization {
        let stabilization = stabilization_of_frames(0, 60);
        let mut camera_path = stabilization.camera_path.to_vec();
        change(&mut camera_path);
        Stabilization {
            camera_path: camera_path.into(),
            ..stabilization
        }
    }

    #[test]
    fn a_stabilization_on_a_title_is_refused() {
        let mut fixture = fixture();
        let title_clip_id = fixture
            .session
            .execute(AddClip::text(
                fixture.video_track_id,
                "Title",
                TimeSpan::from_whole_seconds(2),
                seconds(0),
            ))
            .unwrap()
            .first_clip_id()
            .unwrap();
        let music_clip_id = fixture
            .session
            .execute(AddClip::media(
                fixture.audio_track_id,
                fixture.music_id,
                seconds(0),
                seconds(5),
                seconds(0),
            ))
            .unwrap()
            .first_clip_id()
            .unwrap();
        let on_title =
            stabilization_refusal(&mut fixture, title_clip_id, stabilization_of_frames(0, 60));
        assert!(
            on_title.contains("a stabilisation only applies to a video"),
            "{on_title}"
        );
        let on_music =
            stabilization_refusal(&mut fixture, music_clip_id, stabilization_of_frames(0, 60));
        assert!(
            on_music.contains("a stabilisation needs a picture"),
            "{on_music}"
        );
    }

    #[test]
    fn a_second_stabilization_is_refused() {
        let (mut fixture, video_clip_id) = fixture_with_video_clip();
        let first_effect_id = fixture
            .session
            .execute(AddEffect::new(
                video_clip_id,
                EffectKind::Stabilization(stabilization_of_frames(0, 60)),
            ))
            .unwrap()
            .first_effect_id()
            .unwrap();
        let error = refused_rule(fixture.session.execute(AddEffect::new(
            video_clip_id,
            EffectKind::Stabilization(stabilization_of_frames(0, 90)),
        )));
        let ValidationError::InvalidEffect {
            effect_id, reason, ..
        } = error
        else {
            panic!("expected an invalid effect, got {error:?}");
        };
        assert_ne!(effect_id, first_effect_id);
        assert_eq!(
            reason,
            format!(
                "a clip has at most one stabilisation, and effect {first_effect_id} already steadies this one"
            )
        );
        // Other effects beside the stabilisation are fine.
        fixture
            .session
            .execute(AddEffect::new(
                video_clip_id,
                EffectKind::ColorAdjustment(ColorAdjustment::NEUTRAL),
            ))
            .unwrap();
    }

    #[test]
    fn stabilization_settings_out_of_range_are_refused() {
        let (mut fixture, video_clip_id) = fixture_with_video_clip();
        let with_settings = |smoothing_in_seconds, zoom_factor| Stabilization {
            smoothing_in_seconds,
            zoom_factor,
            ..stabilization_of_frames(0, 60)
        };
        for (stabilization, expected) in [
            (
                with_settings(0.05, 1.0),
                "the smoothing must be between 0.1 and 10, but is 0.05",
            ),
            (
                with_settings(f32::NAN, 1.0),
                "the smoothing must be between",
            ),
            (
                with_settings(12.0, 1.0),
                "the smoothing must be between 0.1 and 10, but is 12",
            ),
            (
                with_settings(0.5, 0.9),
                "the zoom must be between 1 and 1.2, but is 0.9",
            ),
            (
                with_settings(0.5, 1.3),
                "the zoom must be between 1 and 1.2, but is 1.3",
            ),
            (
                Stabilization {
                    analysed_picture_size: PictureSize {
                        width_in_pixels: 1920,
                        height_in_pixels: 0,
                    },
                    ..stabilization_of_frames(0, 60)
                },
                "neither side may be zero",
            ),
        ] {
            let refusal = stabilization_refusal(&mut fixture, video_clip_id, stabilization);
            assert!(refusal.contains(expected), "{refusal}");
        }
        for accepted in [with_settings(0.1, 1.0), with_settings(10.0, 1.2)] {
            let effect_id = fixture
                .session
                .execute(AddEffect::new(
                    video_clip_id,
                    EffectKind::Stabilization(accepted),
                ))
                .unwrap()
                .first_effect_id()
                .unwrap();
            fixture
                .session
                .execute(RemoveEffect::new(video_clip_id, effect_id))
                .unwrap();
        }
    }

    #[test]
    fn an_empty_or_unsorted_camera_path_is_refused() {
        let (mut fixture, video_clip_id) = fixture_with_video_clip();
        let empty = stabilization_with_changed_path(Vec::clear);
        let refusal = stabilization_refusal(&mut fixture, video_clip_id, empty);
        assert!(refusal.contains("its camera path is empty"), "{refusal}");

        let repeated_time = stabilization_with_changed_path(|path| {
            path[3].source_time = path[2].source_time;
        });
        let unsorted = stabilization_with_changed_path(|path| path.swap(4, 5));
        for stabilization in [repeated_time, unsorted] {
            let refusal = stabilization_refusal(&mut fixture, video_clip_id, stabilization);
            assert!(
                refusal.contains("not in strictly increasing time order"),
                "{refusal}"
            );
        }
    }

    #[test]
    fn non_finite_path_values_are_refused() {
        let (mut fixture, video_clip_id) = fixture_with_video_clip();
        let changes: [fn(&mut CameraPathSample); 5] = [
            |sample| sample.shift_x_in_pixels = f32::NAN,
            |sample| sample.shift_y_in_pixels = f32::INFINITY,
            |sample| sample.rotation_in_degrees = f32::NEG_INFINITY,
            |sample| sample.scale_factor = f32::NAN,
            |sample| sample.scale_factor = f32::INFINITY,
        ];
        for change in changes {
            let stabilization = stabilization_with_changed_path(|path| change(&mut path[7]));
            let refusal = stabilization_refusal(&mut fixture, video_clip_id, stabilization);
            assert!(refusal.contains("has a non-finite value"), "{refusal}");
        }
        for impossible_scale in [0.2, 4.5, 0.0, -1.0] {
            let stabilization = stabilization_with_changed_path(|path| {
                path[7].scale_factor = impossible_scale;
            });
            let refusal = stabilization_refusal(&mut fixture, video_clip_id, stabilization);
            assert!(
                refusal.contains(&format!(
                    "has a scale of {impossible_scale}, outside 0.25 to 4"
                )),
                "{refusal}"
            );
        }
    }

    #[test]
    fn transitions_go_on_pictures_and_last_a_sensible_time() {
        let mut fixture = fixture();
        let add_clip = |session: &mut EditSession, track_id, media_id| {
            session
                .execute(AddClip::media(
                    track_id,
                    media_id,
                    seconds(0),
                    seconds(4),
                    seconds(0),
                ))
                .unwrap()
                .first_clip_id()
                .unwrap()
        };
        let video_clip_id = add_clip(
            &mut fixture.session,
            fixture.video_track_id,
            fixture.video_id,
        );
        let music_clip_id = add_clip(
            &mut fixture.session,
            fixture.audio_track_id,
            fixture.music_id,
        );
        let fade_lasting = |duration| {
            Some(Transition {
                style: TransitionStyle::Fade,
                duration,
            })
        };

        let on_music = fixture.session.execute(SetClipTransition::new(
            music_clip_id,
            ClipEdge::Start,
            fade_lasting(TimeSpan::from_whole_seconds(1)),
        ));
        assert!(
            refused_rule(on_music)
                .to_string()
                .contains("needs a picture")
        );
        for refused_duration in [TimeSpan::ZERO, TimeSpan::from_whole_seconds(61)] {
            let error = refused_rule(fixture.session.execute(SetClipTransition::new(
                video_clip_id,
                ClipEdge::End,
                fade_lasting(refused_duration),
            )));
            assert!(
                matches!(error, ValidationError::InvalidTransition { .. }),
                "{error}"
            );
        }

        // Longer than the clip is fine (it is shortened when drawn), and the clip
        // can still be split in the middle of it.
        fixture
            .session
            .execute(SetClipTransition::new(
                video_clip_id,
                ClipEdge::Start,
                fade_lasting(TimeSpan::from_whole_seconds(10)),
            ))
            .unwrap();
        fixture
            .session
            .execute(SplitClip::new(video_clip_id, seconds(1)))
            .unwrap();
    }

    #[test]
    fn texts_take_animations_not_transitions() {
        let mut fixture = fixture();
        let text_clip_id = fixture
            .session
            .execute(AddClip::text(
                fixture.video_track_id,
                "Title",
                TimeSpan::from_whole_seconds(2),
                seconds(0),
            ))
            .unwrap()
            .first_clip_id()
            .unwrap();
        let error = refused_rule(fixture.session.execute(SetClipTransition::new(
            text_clip_id,
            ClipEdge::Start,
            Some(Transition {
                style: TransitionStyle::Fade,
                duration: TimeSpan::from_whole_seconds(1),
            }),
        )));
        assert!(error.to_string().contains("text animations"), "{error}");

        let instant_type_on = TextAnimation {
            style: TextAnimationStyle::TypeOn,
            duration: TimeSpan::ZERO,
        };
        let error = refused_rule(fixture.session.execute(SetTextAnimation::new(
            text_clip_id,
            ClipEdge::End,
            Some(instant_type_on),
        )));
        assert_eq!(
            error.to_string(),
            format!(
                "the animation at the end of text clip {text_clip_id} is invalid: it lasts no time at all"
            )
        );
    }

    #[test]
    fn settings_must_be_usable_for_export() {
        let mut fixture = fixture();
        let mut odd_settings = ProjectSettings::landscape_1080p();
        odd_settings.audio_sample_rate_in_hertz = 12_345;
        let error = refused_rule(
            fixture
                .session
                .execute(ChangeProjectSettings::new(odd_settings)),
        );
        assert!(
            error
                .to_string()
                .contains("12345 Hz is not a supported audio sample rate")
        );
    }

    #[test]
    fn damaged_files_with_reused_or_future_identifiers_are_refused() {
        let fixture = fixture();
        let json_text = save_project_to_json(fixture.session.project()).unwrap();

        let mut document: serde_json::Value = serde_json::from_str(&json_text).unwrap();
        document["project"]["id_allocator"]["next_raw_id"] = serde_json::Value::from(2);
        let error = load_project_from_json(&document.to_string()).unwrap_err();
        assert!(matches!(
            error,
            ProjectFileError::InvalidProject(ValidationError::IdentifierNotYetAllocated { .. })
        ));

        let mut document: serde_json::Value = serde_json::from_str(&json_text).unwrap();
        document["project"]["media_assets"][1]["id"] = serde_json::Value::from(1);
        let error = load_project_from_json(&document.to_string()).unwrap_err();
        assert!(matches!(
            error,
            ProjectFileError::InvalidProject(ValidationError::DuplicateIdentifier { raw_id: 1 })
        ));
    }
}
