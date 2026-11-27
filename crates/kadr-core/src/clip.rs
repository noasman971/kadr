//! Clips: pieces of media or text placed on a track of the timeline.
//!
//! A clip shows either a range of a media file ([`MediaSegment`]) or a text
//! ([`TextContent`]). It also carries its volume, opacity, position, scale,
//! rotation and crop (most of them animatable by keyframes) and its effects.
//!
//! The pure operations on a clip (splitting it, trimming its start or end) live
//! here so they can be tested without a project; the commands call them.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::animation::AnimatedValue;
use crate::color::Color;
use crate::effect::Effect;
use crate::error::EditError;
use crate::ids::{ClipId, EffectId, IdAllocator, MediaAssetId};
use crate::text_animation::{TextAnimation, TextAnimationStyle};
use crate::time::{TimeRange, TimeSpan, Timestamp};
use crate::transition::{EdgePhase, Transition, TransitionStyle, active_edge_style};

/// Error raised when a playback speed is out of range.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error(
    "{requested_multiplier} is not a valid speed: it must be between {} and {} (1 is normal speed)",
    PlaybackSpeed::SLOWEST_MULTIPLIER,
    PlaybackSpeed::FASTEST_MULTIPLIER
)]
pub struct PlaybackSpeedError {
    /// The rejected speed, as a multiplier.
    pub requested_multiplier: f64,
}

/// How fast a media clip plays, as an exact fraction: 2/1 plays twice as fast
/// (the clip lasts half as long), 1/2 plays at half speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "PlaybackSpeedFields", into = "PlaybackSpeedFields")]
pub struct PlaybackSpeed {
    numerator: u32,
    denominator: u32,
}

/// The serialized form of [`PlaybackSpeed`], checked on load.
#[derive(Serialize, Deserialize)]
struct PlaybackSpeedFields {
    numerator: u32,
    denominator: u32,
}

impl TryFrom<PlaybackSpeedFields> for PlaybackSpeed {
    type Error = PlaybackSpeedError;

    fn try_from(fields: PlaybackSpeedFields) -> Result<Self, Self::Error> {
        PlaybackSpeed::from_fraction(fields.numerator, fields.denominator)
    }
}

impl From<PlaybackSpeed> for PlaybackSpeedFields {
    fn from(speed: PlaybackSpeed) -> Self {
        Self {
            numerator: speed.numerator,
            denominator: speed.denominator,
        }
    }
}

/// Speeds given as decimals are rounded to this precision (1/1000).
const SPEED_MULTIPLIER_PRECISION: u32 = 1_000;

impl PlaybackSpeed {
    /// Normal speed.
    pub const NORMAL: Self = Self {
        numerator: 1,
        denominator: 1,
    };
    /// The slowest allowed speed (10 times slower).
    pub const SLOWEST_MULTIPLIER: f64 = 0.1;
    /// The fastest allowed speed (16 times faster).
    pub const FASTEST_MULTIPLIER: f64 = 16.0;

    /// A speed of `numerator / denominator`, stored in lowest terms.
    ///
    /// # Errors
    ///
    /// Fails when the speed is outside [`Self::SLOWEST_MULTIPLIER`] to
    /// [`Self::FASTEST_MULTIPLIER`], or when the denominator is zero.
    pub fn from_fraction(numerator: u32, denominator: u32) -> Result<Self, PlaybackSpeedError> {
        let requested_multiplier = if denominator == 0 {
            f64::INFINITY
        } else {
            f64::from(numerator) / f64::from(denominator)
        };
        if !(Self::SLOWEST_MULTIPLIER..=Self::FASTEST_MULTIPLIER).contains(&requested_multiplier) {
            return Err(PlaybackSpeedError {
                requested_multiplier,
            });
        }
        let common_divisor = greatest_common_divisor(numerator, denominator);
        Ok(Self {
            numerator: numerator / common_divisor,
            denominator: denominator / common_divisor,
        })
    }

    /// A speed given as a decimal multiplier (1.5 = one and a half times faster),
    /// rounded to the nearest thousandth.
    ///
    /// # Errors
    ///
    /// Fails when the speed is not finite or out of range.
    pub fn from_multiplier(multiplier: f64) -> Result<Self, PlaybackSpeedError> {
        let out_of_range = !multiplier.is_finite()
            || !(Self::SLOWEST_MULTIPLIER..=Self::FASTEST_MULTIPLIER).contains(&multiplier);
        if out_of_range {
            return Err(PlaybackSpeedError {
                requested_multiplier: multiplier,
            });
        }
        let thousandths = (multiplier * f64::from(SPEED_MULTIPLIER_PRECISION)).round() as u32;
        Self::from_fraction(thousandths, SPEED_MULTIPLIER_PRECISION)
    }

    /// The speed as a decimal multiplier, **for display only**.
    pub fn as_multiplier(self) -> f64 {
        f64::from(self.numerator) / f64::from(self.denominator)
    }

    /// How long a range of the source file lasts on the timeline at this speed.
    pub fn timeline_span_for_source_span(self, source_span: TimeSpan) -> TimeSpan {
        source_span.scaled_by_ratio(u64::from(self.denominator), u64::from(self.numerator))
    }

    /// How much of the source file plays during a span of the timeline at this speed.
    pub fn source_span_for_timeline_span(self, timeline_span: TimeSpan) -> TimeSpan {
        timeline_span.scaled_by_ratio(u64::from(self.numerator), u64::from(self.denominator))
    }
}

impl fmt::Display for PlaybackSpeed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "×{}", self.as_multiplier())
    }
}

/// Euclid's algorithm. Returns 1 when both numbers are zero.
fn greatest_common_divisor(mut first: u32, mut second: u32) -> u32 {
    while second != 0 {
        (first, second) = (second, first % second);
    }
    first.max(1)
}

/// A range of a media file, played at some speed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaSegment {
    /// The media file.
    pub media_asset_id: MediaAssetId,
    /// Where the range starts in the source file.
    pub source_start: Timestamp,
    /// Where the range ends in the source file (excluded).
    pub source_end: Timestamp,
    /// How fast it plays.
    pub speed: PlaybackSpeed,
}

impl MediaSegment {
    /// Length of the range in the source file (zero if the range is inverted).
    pub fn source_span(&self) -> TimeSpan {
        self.source_end
            .span_since(self.source_start)
            .unwrap_or(TimeSpan::ZERO)
    }
}

/// Horizontal alignment of text lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlignment {
    /// Lines start on the left.
    Left,
    /// Lines are centred.
    #[default]
    Center,
    /// Lines end on the right.
    Right,
}

/// How a text looks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    /// Font name, for example `"Inter"`.
    pub font_family: String,
    /// Font size, in pixels of the project canvas.
    pub font_size_in_pixels: f32,
    /// Text colour.
    pub color: Color,
    /// Alignment of the lines.
    pub alignment: TextAlignment,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_family: "Inter".to_string(),
            font_size_in_pixels: 64.0,
            color: Color::WHITE,
            alignment: TextAlignment::Center,
        }
    }
}

/// A text shown for some time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextContent {
    /// The text. Line breaks are kept.
    pub text: String,
    /// How long the text stays on screen.
    pub duration: TimeSpan,
    /// How it looks.
    pub style: TextStyle,
    /// How the text appears, over its first moments.
    pub entry_animation: Option<TextAnimation>,
    /// How the text disappears, over its last moments.
    pub exit_animation: Option<TextAnimation>,
}

impl TextContent {
    /// A text without animations.
    pub fn new(text: impl Into<String>, duration: TimeSpan, style: TextStyle) -> Self {
        Self {
            text: text.into(),
            duration,
            style,
            entry_animation: None,
            exit_animation: None,
        }
    }

    /// The animation playing `time_in_clip` after the text appears, with where it
    /// is, or `None` while the text is still. Animations longer than the text
    /// are shortened to fit (see
    /// [`fitted_edge_durations`](crate::transition::fitted_edge_durations)).
    pub fn animation_at(&self, time_in_clip: TimeSpan) -> Option<(TextAnimationStyle, EdgePhase)> {
        let style_and_duration =
            |animation: Option<TextAnimation>| animation.map(|it| (it.style, it.duration));
        active_edge_style(
            style_and_duration(self.entry_animation),
            style_and_duration(self.exit_animation),
            time_in_clip,
            self.duration,
        )
    }
}

/// What a clip shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClipContent {
    /// A range of a video, audio or image file.
    Media(MediaSegment),
    /// A text.
    Text(TextContent),
}

/// Part of the picture removed from each edge of a clip, as a fraction of the
/// source picture (0 = nothing removed, 0.25 = a quarter).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct CropInsets {
    /// Fraction removed from the left edge.
    pub left_fraction: f32,
    /// Fraction removed from the top edge.
    pub top_fraction: f32,
    /// Fraction removed from the right edge.
    pub right_fraction: f32,
    /// Fraction removed from the bottom edge.
    pub bottom_fraction: f32,
}

/// Where and how a clip's picture is placed on the canvas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipTransform {
    /// Horizontal offset of the clip's centre from the canvas centre; right is positive.
    pub position_x_in_pixels: AnimatedValue,
    /// Vertical offset of the clip's centre from the canvas centre; down is positive.
    pub position_y_in_pixels: AnimatedValue,
    /// Size multiplier. At 1 the picture fits entirely inside the canvas.
    pub scale_factor: AnimatedValue,
    /// Clockwise rotation around the clip's centre.
    pub rotation_in_degrees: AnimatedValue,
    /// Edges removed from the source picture.
    pub crop: CropInsets,
}

impl Default for ClipTransform {
    fn default() -> Self {
        Self {
            position_x_in_pixels: AnimatedValue::constant(0.0),
            position_y_in_pixels: AnimatedValue::constant(0.0),
            scale_factor: AnimatedValue::constant(1.0),
            rotation_in_degrees: AnimatedValue::constant(0.0),
            crop: CropInsets::default(),
        }
    }
}

/// The clip properties that can be animated with keyframes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimatableProperty {
    /// Sound level multiplier: 0 silent, 1 unchanged, up to 4 (about +12 dB).
    Volume,
    /// 0 invisible to 1 opaque.
    Opacity,
    /// Horizontal position, in pixels from the canvas centre.
    PositionX,
    /// Vertical position, in pixels from the canvas centre.
    PositionY,
    /// Size multiplier.
    Scale,
    /// Clockwise rotation in degrees.
    Rotation,
}

impl AnimatableProperty {
    /// Every animatable property, in display order.
    pub const ALL: [AnimatableProperty; 6] = [
        AnimatableProperty::Volume,
        AnimatableProperty::Opacity,
        AnimatableProperty::PositionX,
        AnimatableProperty::PositionY,
        AnimatableProperty::Scale,
        AnimatableProperty::Rotation,
    ];

    /// The name used in messages, for example `"opacity"`.
    pub fn display_name(self) -> &'static str {
        match self {
            AnimatableProperty::Volume => "volume",
            AnimatableProperty::Opacity => "opacity",
            AnimatableProperty::PositionX => "horizontal position",
            AnimatableProperty::PositionY => "vertical position",
            AnimatableProperty::Scale => "scale",
            AnimatableProperty::Rotation => "rotation",
        }
    }

    /// The smallest and largest allowed values (both included).
    pub fn allowed_range(self) -> (f32, f32) {
        match self {
            AnimatableProperty::Volume => (0.0, 4.0),
            AnimatableProperty::Opacity => (0.0, 1.0),
            AnimatableProperty::PositionX | AnimatableProperty::PositionY => {
                (-100_000.0, 100_000.0)
            }
            AnimatableProperty::Scale => (0.01, 100.0),
            AnimatableProperty::Rotation => (-3_600.0, 3_600.0),
        }
    }

    /// Whether `value` is finite and inside [`Self::allowed_range`].
    pub fn accepts_value(self, value: f32) -> bool {
        let (minimum, maximum) = self.allowed_range();
        value.is_finite() && (minimum..=maximum).contains(&value)
    }
}

impl fmt::Display for AnimatableProperty {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.display_name())
    }
}

/// A clip placed on a track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    /// Identifier inside the project.
    pub id: ClipId,
    /// What the clip shows.
    pub content: ClipContent,
    /// Where the clip starts on the timeline.
    pub timeline_start: Timestamp,
    /// Sound level multiplier (see [`AnimatableProperty::Volume`]).
    pub volume_multiplier: AnimatedValue,
    /// 0 invisible to 1 opaque.
    pub opacity: AnimatedValue,
    /// Position, scale, rotation and crop.
    pub transform: ClipTransform,
    /// Effects, applied in order.
    pub effects: Vec<Effect>,
    /// How the picture enters, over the clip's first moments (videos and images).
    pub entry_transition: Option<Transition>,
    /// How the picture leaves, over the clip's last moments (videos and images).
    pub exit_transition: Option<Transition>,
}

impl Clip {
    /// A clip with default properties: full volume, opaque, centred, no effect,
    /// no transition.
    pub fn new(id: ClipId, content: ClipContent, timeline_start: Timestamp) -> Self {
        Self {
            id,
            content,
            timeline_start,
            volume_multiplier: AnimatedValue::constant(1.0),
            opacity: AnimatedValue::constant(1.0),
            transform: ClipTransform::default(),
            effects: Vec::new(),
            entry_transition: None,
            exit_transition: None,
        }
    }

    /// The transition playing `time_in_clip` after the clip's start, with where
    /// it is, or `None` while the picture is settled. Times past the clip's edges
    /// are clamped to them. Transitions longer than the clip are shortened to fit
    /// (see [`fitted_edge_durations`](crate::transition::fitted_edge_durations)).
    pub fn transition_at(&self, time_in_clip: TimeSpan) -> Option<(TransitionStyle, EdgePhase)> {
        let style_and_duration =
            |transition: Option<Transition>| transition.map(|it| (it.style, it.duration));
        active_edge_style(
            style_and_duration(self.entry_transition),
            style_and_duration(self.exit_transition),
            time_in_clip,
            self.timeline_duration(),
        )
    }

    /// How long the clip lasts on the timeline.
    pub fn timeline_duration(&self) -> TimeSpan {
        match &self.content {
            ClipContent::Media(segment) => segment
                .speed
                .timeline_span_for_source_span(segment.source_span()),
            ClipContent::Text(text) => text.duration,
        }
    }

    /// Where the clip ends on the timeline (excluded).
    pub fn timeline_end(&self) -> Timestamp {
        self.timeline_start + self.timeline_duration()
    }

    /// Whether `timeline_time` falls inside the clip (start included, end excluded).
    pub fn contains_timeline_time(&self, timeline_time: Timestamp) -> bool {
        self.timeline_start <= timeline_time && timeline_time < self.timeline_end()
    }

    /// The media file shown, or `None` for a text clip.
    pub fn media_asset_id(&self) -> Option<MediaAssetId> {
        match &self.content {
            ClipContent::Media(segment) => Some(segment.media_asset_id),
            ClipContent::Text(_) => None,
        }
    }

    /// The moment of the source file shown at `timeline_time`, for media clips
    /// whose range contains that time.
    pub fn source_time_at(&self, timeline_time: Timestamp) -> Option<Timestamp> {
        let ClipContent::Media(segment) = &self.content else {
            return None;
        };
        if !self.contains_timeline_time(timeline_time) {
            return None;
        }
        let elapsed_on_timeline = timeline_time.span_since(self.timeline_start)?;
        Some(
            segment.source_start
                + segment
                    .speed
                    .source_span_for_timeline_span(elapsed_on_timeline),
        )
    }

    /// The timeline moment showing `source_time` of the media file, for media
    /// clips whose source range contains it: the reverse of
    /// [`Clip::source_time_at`] (rounded down to a tick with speed changes).
    pub fn timeline_time_showing(&self, source_time: Timestamp) -> Option<Timestamp> {
        let ClipContent::Media(segment) = &self.content else {
            return None;
        };
        if source_time < segment.source_start || segment.source_end <= source_time {
            return None;
        }
        let elapsed_in_source = source_time.span_since(segment.source_start)?;
        Some(
            self.timeline_start
                + segment
                    .speed
                    .timeline_span_for_source_span(elapsed_in_source),
        )
    }

    /// The moment of the source file that would play at `timeline_time` if the
    /// clip went on past its edges at the same speed: inside the clip, the same
    /// as [`Clip::source_time_at`]; before its start, earlier in the file (never
    /// before the file's beginning); after its end, later in the file. Used to
    /// show a clip under a transition of the clip touching it. Keeping the time
    /// inside the file's length is left to the caller, which knows it. `None` for
    /// text clips.
    pub fn source_time_continued_at(&self, timeline_time: Timestamp) -> Option<Timestamp> {
        let ClipContent::Media(segment) = &self.content else {
            return None;
        };
        let speed = segment.speed;
        match timeline_time.span_since(self.timeline_start) {
            Some(elapsed) => {
                Some(segment.source_start + speed.source_span_for_timeline_span(elapsed))
            }
            None => {
                let ahead = self.timeline_start.span_since(timeline_time)?;
                Some(
                    segment
                        .source_start
                        .checked_sub(speed.source_span_for_timeline_span(ahead))
                        .unwrap_or(Timestamp::ZERO),
                )
            }
        }
    }

    /// The animated value of `property`.
    pub fn animated_property(&self, property: AnimatableProperty) -> &AnimatedValue {
        match property {
            AnimatableProperty::Volume => &self.volume_multiplier,
            AnimatableProperty::Opacity => &self.opacity,
            AnimatableProperty::PositionX => &self.transform.position_x_in_pixels,
            AnimatableProperty::PositionY => &self.transform.position_y_in_pixels,
            AnimatableProperty::Scale => &self.transform.scale_factor,
            AnimatableProperty::Rotation => &self.transform.rotation_in_degrees,
        }
    }

    /// The animated value of `property`, for modification by commands.
    pub(crate) fn animated_property_mut(
        &mut self,
        property: AnimatableProperty,
    ) -> &mut AnimatedValue {
        match property {
            AnimatableProperty::Volume => &mut self.volume_multiplier,
            AnimatableProperty::Opacity => &mut self.opacity,
            AnimatableProperty::PositionX => &mut self.transform.position_x_in_pixels,
            AnimatableProperty::PositionY => &mut self.transform.position_y_in_pixels,
            AnimatableProperty::Scale => &mut self.transform.scale_factor,
            AnimatableProperty::Rotation => &mut self.transform.rotation_in_degrees,
        }
    }

    /// Applies `change` to every animatable property.
    fn transform_every_animated_property(
        &mut self,
        change: impl Fn(&AnimatedValue) -> AnimatedValue,
    ) {
        for property in AnimatableProperty::ALL {
            let changed_value = change(self.animated_property(property));
            *self.animated_property_mut(property) = changed_value;
        }
    }

    /// Fails unless `timeline_time` is strictly inside the clip (not on its edges).
    fn ensure_strictly_inside(&self, timeline_time: Timestamp) -> Result<TimeSpan, EditError> {
        let offset_from_start = timeline_time
            .span_since(self.timeline_start)
            .filter(|offset| !offset.is_zero() && timeline_time < self.timeline_end());
        offset_from_start.ok_or(EditError::TimeOutsideClip {
            clip_id: self.id,
            requested_time: timeline_time,
            clip_start: self.timeline_start,
            clip_end: self.timeline_end(),
        })
    }

    /// Cuts the clip in two at `split_time` on the timeline.
    ///
    /// The first part keeps this clip's identifier; the second part and copies of
    /// the effects get new identifiers from `id_allocator`. Both parts keep the
    /// same properties, effects and animation, so the result looks and sounds
    /// exactly like the original.
    ///
    /// # Errors
    ///
    /// Fails when `split_time` is not strictly inside the clip.
    pub fn split_at(
        &self,
        split_time: Timestamp,
        id_allocator: &mut IdAllocator,
    ) -> Result<(Clip, Clip), EditError> {
        let offset_from_start = self.ensure_strictly_inside(split_time)?;

        let (first_content, second_content) = self.split_content_at(offset_from_start);

        // The entry stays at the start of the first part, the exit at the end of
        // the second part.
        let mut first_part = self.clone();
        first_part.content = first_content;
        first_part.exit_transition = None;

        let mut second_part = self.clone();
        second_part.content = second_content;
        second_part.entry_transition = None;
        second_part.id = ClipId::from_raw(id_allocator.allocate_raw_id());
        second_part.timeline_start = split_time;
        for effect in &mut second_part.effects {
            effect.id = EffectId::from_raw(id_allocator.allocate_raw_id());
        }

        for property in AnimatableProperty::ALL {
            let (first_animation, second_animation) =
                self.animated_property(property).split_at(offset_from_start);
            *first_part.animated_property_mut(property) = first_animation;
            *second_part.animated_property_mut(property) = second_animation;
        }
        Ok((first_part, second_part))
    }

    /// The content before and after `offset_from_start` (a timeline offset).
    fn split_content_at(&self, offset_from_start: TimeSpan) -> (ClipContent, ClipContent) {
        match &self.content {
            ClipContent::Media(segment) => {
                let source_split = segment.source_start
                    + segment
                        .speed
                        .source_span_for_timeline_span(offset_from_start);
                let first_segment = MediaSegment {
                    source_end: source_split,
                    ..segment.clone()
                };
                let second_segment = MediaSegment {
                    source_start: source_split,
                    ..segment.clone()
                };
                (
                    ClipContent::Media(first_segment),
                    ClipContent::Media(second_segment),
                )
            }
            ClipContent::Text(text) => {
                let first_text = TextContent {
                    duration: offset_from_start,
                    exit_animation: None,
                    ..text.clone()
                };
                let second_text = TextContent {
                    duration: text
                        .duration
                        .checked_sub(offset_from_start)
                        .unwrap_or(TimeSpan::ZERO),
                    entry_animation: None,
                    ..text.clone()
                };
                (
                    ClipContent::Text(first_text),
                    ClipContent::Text(second_text),
                )
            }
        }
    }

    /// A copy of the clip whose start is moved to `new_start` on the timeline,
    /// keeping its end in place. The content and the animation stay aligned
    /// with the timeline: trimming hides the beginning, extending reveals more of
    /// the source file.
    ///
    /// # Errors
    ///
    /// Fails when `new_start` is at or after the clip's end, or when extending a
    /// media clip would start before the beginning of its source file.
    pub fn with_start_moved_to(&self, new_start: Timestamp) -> Result<Clip, EditError> {
        let clip_end = self.timeline_end();
        if new_start >= clip_end {
            return Err(EditError::TrimWouldEmptyClip {
                clip_id: self.id,
                requested_time: new_start,
            });
        }
        let mut trimmed_clip = self.clone();
        trimmed_clip.timeline_start = new_start;

        if let Some(removed_span) = new_start.span_since(self.timeline_start) {
            self.shorten_start_of(&mut trimmed_clip, removed_span);
        } else if let Some(added_span) = self.timeline_start.span_since(new_start) {
            self.lengthen_start_of(&mut trimmed_clip, added_span)?;
        }
        Ok(trimmed_clip)
    }

    /// Removes `removed_span` from the beginning of `trimmed_clip`.
    fn shorten_start_of(&self, trimmed_clip: &mut Clip, removed_span: TimeSpan) {
        match &mut trimmed_clip.content {
            ClipContent::Media(segment) => {
                segment.source_start = segment.source_start
                    + segment.speed.source_span_for_timeline_span(removed_span);
            }
            ClipContent::Text(text) => {
                text.duration = text
                    .duration
                    .checked_sub(removed_span)
                    .unwrap_or(TimeSpan::ZERO);
            }
        }
        trimmed_clip.transform_every_animated_property(|animation| {
            animation.after_removing_start(removed_span)
        });
    }

    /// Adds `added_span` before the beginning of `trimmed_clip`.
    fn lengthen_start_of(
        &self,
        trimmed_clip: &mut Clip,
        added_span: TimeSpan,
    ) -> Result<(), EditError> {
        match &mut trimmed_clip.content {
            ClipContent::Media(segment) => {
                let source_span_to_reveal = segment.speed.source_span_for_timeline_span(added_span);
                segment.source_start = segment
                    .source_start
                    .checked_sub(source_span_to_reveal)
                    .ok_or(EditError::ExtendsBeforeSourceStart { clip_id: self.id })?;
            }
            ClipContent::Text(text) => text.duration = text.duration + added_span,
        }
        trimmed_clip.transform_every_animated_property(|animation| {
            animation.after_extending_start(added_span)
        });
        Ok(())
    }

    /// A copy of the clip whose end is moved to `new_end` on the timeline,
    /// keeping its start in place.
    ///
    /// Extending past the end of the source file is caught later by the
    /// project validation, which knows the file's duration.
    ///
    /// # Errors
    ///
    /// Fails when `new_end` is at or before the clip's start.
    pub fn with_end_moved_to(&self, new_end: Timestamp) -> Result<Clip, EditError> {
        let new_duration = new_end
            .span_since(self.timeline_start)
            .filter(|duration| !duration.is_zero())
            .ok_or(EditError::TrimWouldEmptyClip {
                clip_id: self.id,
                requested_time: new_end,
            })?;
        let mut trimmed_clip = self.clone();
        match &mut trimmed_clip.content {
            ClipContent::Media(segment) => {
                segment.source_end = segment.source_start
                    + segment.speed.source_span_for_timeline_span(new_duration);
            }
            ClipContent::Text(text) => text.duration = new_duration,
        }
        Ok(trimmed_clip)
    }

    /// The parts of the clip left once `removed_parts` (timeline times) are cut
    /// out, placed end to end from the clip's start.
    ///
    /// The parts may come in any order and overlap. The first part kept keeps
    /// the clip's identifier and its entry transition, the last one its exit
    /// transition; the others get new identifiers (and new effect identifiers)
    /// from `id_allocator`. Keyframes stay on the moments of the content they
    /// were on.
    ///
    /// # Errors
    ///
    /// Fails when no part is given, a part is not inside the clip, or nothing
    /// would be left.
    pub fn without_parts(
        &self,
        removed_parts: &[TimeRange],
        id_allocator: &mut IdAllocator,
    ) -> Result<Vec<Clip>, EditError> {
        let kept_ranges = self.ranges_kept_without(removed_parts)?;
        let mut pieces = Vec::with_capacity(kept_ranges.len());
        let mut remaining = self.clone();
        for (index, kept_range) in kept_ranges.iter().enumerate() {
            if kept_range.start() > remaining.timeline_start {
                remaining = remaining.with_start_moved_to(kept_range.start())?;
            }
            let is_last = index + 1 == kept_ranges.len();
            if is_last {
                if kept_range.end() < remaining.timeline_end() {
                    remaining = remaining.with_end_moved_to(kept_range.end())?;
                }
                pieces.push(remaining);
                break;
            }
            let (piece, rest) = remaining.split_at(kept_range.end(), id_allocator)?;
            pieces.push(piece);
            remaining = rest;
        }
        let mut next_start = self.timeline_start;
        for piece in &mut pieces {
            piece.timeline_start = next_start;
            next_start = next_start + piece.timeline_duration();
        }
        Ok(pieces)
    }

    /// The parts of the clip (timeline times) between `removed_parts`, in order.
    fn ranges_kept_without(
        &self,
        removed_parts: &[TimeRange],
    ) -> Result<Vec<TimeRange>, EditError> {
        if removed_parts.is_empty() {
            return Err(EditError::NoPartToRemove { clip_id: self.id });
        }
        let (clip_start, clip_end) = (self.timeline_start, self.timeline_end());
        if let Some(outside_part) = removed_parts
            .iter()
            .find(|part| part.start() < clip_start || part.end() > clip_end)
        {
            return Err(EditError::PartOutsideClip {
                clip_id: self.id,
                part: *outside_part,
                clip_start,
                clip_end,
            });
        }
        let mut sorted_parts = removed_parts.to_vec();
        sorted_parts.sort_by_key(|part| part.start());
        let mut kept_ranges = Vec::new();
        let mut cursor = clip_start;
        for part in sorted_parts {
            kept_ranges.extend(TimeRange::new(cursor, part.start()));
            cursor = cursor.max(part.end());
        }
        kept_ranges.extend(TimeRange::new(cursor, clip_end));
        if kept_ranges.is_empty() {
            return Err(EditError::RemovalWouldEmptyClip { clip_id: self.id });
        }
        Ok(kept_ranges)
    }
}

#[cfg(test)]
pub(crate) mod test_samples {
    //! Ready-made clips for the tests of every module.

    use super::*;

    /// A clip showing seconds `source_start_seconds` to `source_end_seconds` of
    /// asset `media_asset_id`, placed at `timeline_start_seconds`.
    pub(crate) fn media_clip(
        clip_id: u64,
        media_asset_id: MediaAssetId,
        source_start_seconds: u64,
        source_end_seconds: u64,
        timeline_start_seconds: u64,
    ) -> Clip {
        Clip::new(
            ClipId::from_raw(clip_id),
            ClipContent::Media(MediaSegment {
                media_asset_id,
                source_start: Timestamp::from_whole_seconds(source_start_seconds),
                source_end: Timestamp::from_whole_seconds(source_end_seconds),
                speed: PlaybackSpeed::NORMAL,
            }),
            Timestamp::from_whole_seconds(timeline_start_seconds),
        )
    }

    /// A text clip lasting `duration_seconds`, placed at `timeline_start_seconds`.
    pub(crate) fn text_clip(
        clip_id: u64,
        duration_seconds: u64,
        timeline_start_seconds: u64,
    ) -> Clip {
        Clip::new(
            ClipId::from_raw(clip_id),
            ClipContent::Text(TextContent::new(
                "Hello",
                TimeSpan::from_whole_seconds(duration_seconds),
                TextStyle::default(),
            )),
            Timestamp::from_whole_seconds(timeline_start_seconds),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::test_samples::{media_clip, text_clip};
    use super::*;
    use crate::animation::{Interpolation, Keyframe};

    fn seconds(value: u64) -> Timestamp {
        Timestamp::from_whole_seconds(value)
    }

    fn segment_of(clip: &Clip) -> &MediaSegment {
        match &clip.content {
            ClipContent::Media(segment) => segment,
            ClipContent::Text(_) => panic!("expected a media clip"),
        }
    }

    #[test]
    fn speed_is_stored_in_lowest_terms() {
        assert_eq!(
            PlaybackSpeed::from_fraction(4, 2).unwrap(),
            PlaybackSpeed::from_fraction(2, 1).unwrap()
        );
        assert_eq!(
            PlaybackSpeed::from_multiplier(1.5).unwrap(),
            PlaybackSpeed::from_fraction(3, 2).unwrap()
        );
    }

    #[test]
    fn speed_out_of_range_is_rejected() {
        for multiplier in [0.0, 0.05, 16.5, f64::NAN, -1.0] {
            assert!(
                PlaybackSpeed::from_multiplier(multiplier).is_err(),
                "{multiplier}"
            );
        }
        assert!(PlaybackSpeed::from_fraction(1, 0).is_err());
    }

    #[test]
    fn speed_changes_the_timeline_duration() {
        let mut clip = media_clip(1, MediaAssetId::from_raw(9), 0, 10, 0);
        assert_eq!(clip.timeline_duration(), TimeSpan::from_whole_seconds(10));

        if let ClipContent::Media(segment) = &mut clip.content {
            segment.speed = PlaybackSpeed::from_fraction(2, 1).unwrap();
        }
        assert_eq!(clip.timeline_duration(), TimeSpan::from_whole_seconds(5));
        assert_eq!(clip.source_time_at(seconds(2)), Some(seconds(4)));
        assert_eq!(clip.timeline_time_showing(seconds(4)), Some(seconds(2)));
        assert_eq!(clip.timeline_time_showing(seconds(10)), None);
    }

    #[test]
    fn the_source_continues_past_the_clip_edges() {
        let mut clip = media_clip(1, MediaAssetId::from_raw(9), 2, 8, 10);
        if let ClipContent::Media(segment) = &mut clip.content {
            segment.speed = PlaybackSpeed::from_fraction(2, 1).unwrap();
        }
        // Source 2–8 s at 2× lasts 3 s on the timeline, from 10 s to 13 s.
        assert_eq!(clip.source_time_continued_at(seconds(11)), Some(seconds(4)));
        assert_eq!(
            clip.source_time_continued_at(seconds(14)),
            Some(seconds(10))
        );
        assert_eq!(clip.source_time_continued_at(seconds(9)), Some(seconds(0)));
        assert_eq!(
            clip.source_time_continued_at(seconds(5)),
            Some(Timestamp::ZERO)
        );
        assert_eq!(
            text_clip(2, 4, 0).source_time_continued_at(seconds(1)),
            None
        );
    }

    #[test]
    fn splitting_keeps_the_entry_on_the_first_part_and_the_exit_on_the_second() {
        use crate::transition::{Transition, TransitionStyle};
        let mut id_allocator = IdAllocator::default();
        let mut clip = media_clip(
            id_allocator.allocate_raw_id(),
            MediaAssetId::from_raw(9),
            0,
            8,
            0,
        );
        let fade = Transition {
            style: TransitionStyle::Fade,
            duration: TimeSpan::from_whole_seconds(1),
        };
        clip.entry_transition = Some(fade);
        clip.exit_transition = Some(fade);

        let (first_part, second_part) = clip.split_at(seconds(4), &mut id_allocator).unwrap();
        assert_eq!(
            (first_part.entry_transition, first_part.exit_transition),
            (Some(fade), None)
        );
        assert_eq!(
            (second_part.entry_transition, second_part.exit_transition),
            (None, Some(fade))
        );
    }

    #[test]
    fn splitting_a_media_clip_keeps_the_total_duration_and_content() {
        let mut id_allocator = IdAllocator::default();
        let clip = media_clip(
            id_allocator.allocate_raw_id(),
            MediaAssetId::from_raw(9),
            2,
            8,
            10,
        );

        let (first_part, second_part) = clip.split_at(seconds(12), &mut id_allocator).unwrap();

        assert_eq!(first_part.id, clip.id);
        assert_ne!(second_part.id, clip.id);
        assert_eq!(first_part.timeline_end(), second_part.timeline_start);
        assert_eq!(
            first_part.timeline_duration() + second_part.timeline_duration(),
            clip.timeline_duration()
        );
        assert_eq!(segment_of(&first_part).source_end, seconds(4));
        assert_eq!(segment_of(&second_part).source_start, seconds(4));
        assert_eq!(
            second_part.source_time_at(seconds(13)),
            clip.source_time_at(seconds(13))
        );
    }

    #[test]
    fn splitting_at_an_edge_or_outside_fails() {
        let mut id_allocator = IdAllocator::default();
        let clip = text_clip(1, 4, 10);
        for split_second in [5, 10, 14, 20] {
            assert!(
                matches!(
                    clip.split_at(seconds(split_second), &mut id_allocator),
                    Err(EditError::TimeOutsideClip { .. })
                ),
                "{split_second}"
            );
        }
    }

    #[test]
    fn splitting_preserves_animation_and_gives_effects_new_ids() {
        let mut id_allocator = IdAllocator::default();
        let mut clip = text_clip(1, 4, 0);
        clip.opacity.set_keyframe(Keyframe {
            time_offset: TimeSpan::ZERO,
            value: 0.0,
            interpolation_to_next: Interpolation::Linear,
        });
        clip.opacity.set_keyframe(Keyframe {
            time_offset: TimeSpan::from_whole_seconds(4),
            value: 1.0,
            interpolation_to_next: Interpolation::Linear,
        });
        clip.effects.push(Effect {
            id: EffectId::from_raw(50),
            is_enabled: true,
            kind: crate::effect::EffectKind::ColorAdjustment(
                crate::effect::ColorAdjustment::NEUTRAL,
            ),
        });

        let (first_part, second_part) = clip.split_at(seconds(1), &mut id_allocator).unwrap();

        assert_eq!(
            first_part.opacity.value_at(TimeSpan::from_whole_seconds(1)),
            0.25
        );
        assert_eq!(second_part.opacity.value_at(TimeSpan::ZERO), 0.25);
        assert_eq!(
            second_part
                .opacity
                .value_at(TimeSpan::from_whole_seconds(3)),
            1.0
        );
        assert_eq!(first_part.effects[0].id, EffectId::from_raw(50));
        assert_ne!(second_part.effects[0].id, EffectId::from_raw(50));
    }

    #[test]
    fn trimming_the_start_hides_the_beginning_of_the_source() {
        let clip = media_clip(1, MediaAssetId::from_raw(9), 2, 8, 10);

        let trimmed = clip.with_start_moved_to(seconds(13)).unwrap();

        assert_eq!(trimmed.timeline_start, seconds(13));
        assert_eq!(trimmed.timeline_end(), clip.timeline_end());
        assert_eq!(segment_of(&trimmed).source_start, seconds(5));
    }

    #[test]
    fn extending_the_start_reveals_more_source_until_its_beginning() {
        let clip = media_clip(1, MediaAssetId::from_raw(9), 2, 8, 10);

        let extended = clip.with_start_moved_to(seconds(8)).unwrap();
        assert_eq!(segment_of(&extended).source_start, Timestamp::ZERO);

        assert!(matches!(
            clip.with_start_moved_to(seconds(7)),
            Err(EditError::ExtendsBeforeSourceStart { .. })
        ));
    }

    #[test]
    fn moving_the_end_changes_the_duration() {
        let clip = text_clip(1, 4, 10);
        assert_eq!(
            clip.with_end_moved_to(seconds(12))
                .unwrap()
                .timeline_duration(),
            TimeSpan::from_whole_seconds(2)
        );
        assert!(clip.with_end_moved_to(seconds(10)).is_err());
        assert!(clip.with_start_moved_to(seconds(14)).is_err());
    }

    fn range(start_second: u64, end_second: u64) -> TimeRange {
        TimeRange::new(seconds(start_second), seconds(end_second)).unwrap()
    }

    /// (timeline start, timeline end, source start, source end) of each piece, in seconds.
    fn piece_spans(pieces: &[Clip]) -> Vec<(f64, f64, f64, f64)> {
        pieces
            .iter()
            .map(|piece| {
                let segment = segment_of(piece);
                (
                    piece.timeline_start.as_seconds(),
                    piece.timeline_end().as_seconds(),
                    segment.source_start.as_seconds(),
                    segment.source_end.as_seconds(),
                )
            })
            .collect()
    }

    #[test]
    fn removing_parts_leaves_the_rest_end_to_end() {
        let mut id_allocator = IdAllocator::default();
        let clip = media_clip(
            id_allocator.allocate_raw_id(),
            MediaAssetId::from_raw(9),
            0,
            20,
            10,
        );

        let pieces = clip
            .without_parts(&[range(20, 22), range(13, 15)], &mut id_allocator)
            .unwrap();

        assert_eq!(
            piece_spans(&pieces),
            vec![
                (10.0, 13.0, 0.0, 3.0),
                (13.0, 18.0, 5.0, 10.0),
                (18.0, 26.0, 12.0, 20.0)
            ]
        );
        assert_eq!(pieces[0].id, clip.id);
        assert!(pieces[1..].iter().all(|piece| piece.id != clip.id));
        assert_ne!(pieces[1].id, pieces[2].id);
    }

    #[test]
    fn removing_parts_at_the_edges_keeps_the_transitions_and_keyframes_in_place() {
        use crate::transition::{Transition, TransitionStyle};
        let mut id_allocator = IdAllocator::default();
        let mut clip = media_clip(
            id_allocator.allocate_raw_id(),
            MediaAssetId::from_raw(9),
            0,
            20,
            10,
        );
        let fade = Transition {
            style: TransitionStyle::Fade,
            duration: TimeSpan::from_whole_seconds(1),
        };
        clip.entry_transition = Some(fade);
        clip.exit_transition = Some(fade);
        // Opacity rises from 0 at timeline 10 s to 1 at timeline 15 s, which is
        // 3 s into what is left once 10–12 s is removed.
        for (offset_in_seconds, value) in [(0, 0.0), (5, 1.0)] {
            clip.opacity.set_keyframe(Keyframe {
                time_offset: TimeSpan::from_whole_seconds(offset_in_seconds),
                value,
                interpolation_to_next: Interpolation::Linear,
            });
        }

        let pieces = clip
            .without_parts(
                &[range(10, 12), range(20, 21), range(28, 30)],
                &mut id_allocator,
            )
            .unwrap();

        assert_eq!(
            piece_spans(&pieces),
            vec![(10.0, 18.0, 2.0, 10.0), (18.0, 25.0, 11.0, 18.0)]
        );
        assert_eq!(
            (pieces[0].entry_transition, pieces[0].exit_transition),
            (Some(fade), None)
        );
        assert_eq!(
            (pieces[1].entry_transition, pieces[1].exit_transition),
            (None, Some(fade))
        );
        let opacity_at = |offset_in_seconds| {
            pieces[0]
                .opacity
                .value_at(TimeSpan::from_whole_seconds(offset_in_seconds))
        };
        assert!((opacity_at(0) - 0.4).abs() < 1e-6, "{}", opacity_at(0));
        assert!((opacity_at(3) - 1.0).abs() < 1e-6, "{}", opacity_at(3));
    }

    #[test]
    fn overlapping_parts_are_removed_once() {
        let mut id_allocator = IdAllocator::default();
        let clip = text_clip(id_allocator.allocate_raw_id(), 10, 0);

        let pieces = clip
            .without_parts(&[range(4, 7), range(2, 5)], &mut id_allocator)
            .unwrap();

        let spans: Vec<(f64, f64)> = pieces
            .iter()
            .map(|piece| {
                (
                    piece.timeline_start.as_seconds(),
                    piece.timeline_end().as_seconds(),
                )
            })
            .collect();
        assert_eq!(spans, vec![(0.0, 2.0), (2.0, 5.0)]);
    }

    #[test]
    fn parts_to_remove_must_be_given_inside_the_clip_and_leave_something() {
        let mut id_allocator = IdAllocator::default();
        let clip = text_clip(1, 10, 5);
        assert!(matches!(
            clip.without_parts(&[], &mut id_allocator),
            Err(EditError::NoPartToRemove { .. })
        ));
        assert!(matches!(
            clip.without_parts(&[range(4, 6)], &mut id_allocator),
            Err(EditError::PartOutsideClip { .. })
        ));
        assert!(matches!(
            clip.without_parts(&[range(14, 16)], &mut id_allocator),
            Err(EditError::PartOutsideClip { .. })
        ));
        assert!(matches!(
            clip.without_parts(&[range(5, 9), range(8, 15)], &mut id_allocator),
            Err(EditError::RemovalWouldEmptyClip { .. })
        ));
    }

    #[test]
    fn property_ranges_reject_out_of_range_and_non_finite_values() {
        assert!(AnimatableProperty::Opacity.accepts_value(0.5));
        assert!(!AnimatableProperty::Opacity.accepts_value(1.5));
        assert!(!AnimatableProperty::Scale.accepts_value(0.0));
        assert!(!AnimatableProperty::Rotation.accepts_value(f32::NAN));
    }
}
