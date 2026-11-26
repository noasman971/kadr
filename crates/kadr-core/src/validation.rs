//! The rules a project must always respect.
//!
//! [`validate_project`] is the single place where every rule is written down.
//! It runs after every command (a change that breaks a rule is undone and
//! refused) and on every loaded file (a corrupted file is rejected), so the rest
//! of Kadr can trust that a project is always valid.

use std::collections::HashSet;

use crate::clip::{AnimatableProperty, Clip, ClipContent, CropInsets, MediaSegment, TextContent};
use crate::effect::{BlurStyle, ColorAdjustment, Effect, EffectKind, TrackedBlur};
use crate::ids::{ClipId, EffectId, MediaAssetId, TrackId};
use crate::media_asset::{MediaAsset, MediaKind};
use crate::project::{Project, ProjectSettings};
use crate::time::Timestamp;
use crate::timeline::{Track, TrackKind};

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
    Ok(())
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
    Ok(())
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
        EffectKind::TrackedBlur(tracked_blur) => {
            if !matches!(clip.content, ClipContent::Media(_)) {
                Some("a tracked blur only applies to a video or an image".to_string())
            } else {
                find_tracked_blur_problem(tracked_blur)
            }
        }
    };
    match problem {
        Some(reason) => invalid(reason),
        None => Ok(()),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::CropInsets;
    use crate::commands::{
        AddClip, AddEffect, AddTrack, ChangeProjectSettings, ClipEdge, CreatedObjectsExt,
        ImportMedia, SetClipCrop, SetClipText, TrackPosition, TrimClip,
    };
    use crate::effect::{PixelRegion, RegionSample, RegionSampleOrigin, TrackingConfidence};
    use crate::error::EditError;
    use crate::history::EditSession;
    use crate::media_asset::test_samples::{logo_image, one_minute_music, ten_second_video};
    use crate::project_file::{ProjectFileError, load_project_from_json, save_project_to_json};
    use crate::time::TimeSpan;

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
