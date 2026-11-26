//! How layers move during transitions and text animations.
//!
//! `kadr-core` says where a clip is in its entry or exit
//! ([`EdgePhase`]); this module turns that into a [`LayerMotion`] (a change of
//! the layer's opacity, position and scale) and, for texts, into a
//! [`TextReveal`] (how many letters or words are visible). Nothing here touches
//! pixels: both renderers draw the moved layers exactly like any other, so they
//! agree on transitions without any extra shader.
//!
//! | Transition | Entering picture | Leaving picture |
//! |---|---|---|
//! | Fade | fades in | fades out |
//! | Slide | comes in from one canvas away, moving in the slide's direction | goes out one canvas away, same direction |
//! | Zoom in | grows from half size to full size while fading in | grows to twice its size while fading out |
//! | Zoom out | shrinks from twice its size while fading in | shrinks to half size while fading out |
//!
//! The clip **touching** the transitioning clip on the same track is drawn
//! underneath with the opposite motion and no fading: during an entry it leaves
//! (pushed out by a slide, zooming with a zoom, simply covered by a fade, which
//! makes a dissolve); during an exit it enters.
//!
//! Positions and sizes move with an ease-in-out curve; opacity changes linearly.

use kadr_core::media_asset::PictureSize;
use kadr_core::text_animation::TextAnimationStyle;
use kadr_core::transition::{EdgePhase, TransitionStyle, ZoomDirection};

use crate::layers::LayerPlacement;

/// How much a zoom transition scales pictures: an entering "zoom in" picture
/// starts at half size, a leaving one ends at twice its size.
const ZOOM_TRANSITION_FACTOR: f32 = 2.0;
/// How far a sliding text travels, in multiples of its font size.
const TEXT_SLIDE_DISTANCE_IN_FONT_SIZES: f32 = 1.5;
/// How fast a popping text becomes opaque compared with its growth: at 4, it is
/// opaque after the first quarter of the animation.
const TEXT_POP_FADE_SPEED: f32 = 4.0;
/// How far the "back" easing of a popping text overshoots (the classic value
/// from Robert Penner's easing equations, about 10 % past full size).
const TEXT_POP_OVERSHOOT: f32 = 1.701_58;

/// A change to a layer's placement: multiply its opacity and scale, shift its
/// position. [`LayerMotion::NONE`] changes nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerMotion {
    /// Multiplies the layer's opacity (0 invisible to 1 unchanged).
    pub opacity_multiplier: f32,
    /// Added to the layer's horizontal position; right is positive.
    pub offset_x_in_pixels: f32,
    /// Added to the layer's vertical position; down is positive.
    pub offset_y_in_pixels: f32,
    /// Multiplies the layer's scale, around its centre.
    pub scale_multiplier: f32,
}

impl LayerMotion {
    /// No change at all.
    pub const NONE: Self = Self {
        opacity_multiplier: 1.0,
        offset_x_in_pixels: 0.0,
        offset_y_in_pixels: 0.0,
        scale_multiplier: 1.0,
    };

    /// This motion followed by `other`: opacities and scales multiply, offsets add.
    pub fn combined_with(self, other: LayerMotion) -> LayerMotion {
        LayerMotion {
            opacity_multiplier: self.opacity_multiplier * other.opacity_multiplier,
            offset_x_in_pixels: self.offset_x_in_pixels + other.offset_x_in_pixels,
            offset_y_in_pixels: self.offset_y_in_pixels + other.offset_y_in_pixels,
            scale_multiplier: self.scale_multiplier * other.scale_multiplier,
        }
    }

    /// The same motion with the opacity left unchanged.
    pub fn without_fading(self) -> LayerMotion {
        LayerMotion {
            opacity_multiplier: 1.0,
            ..self
        }
    }
}

impl LayerPlacement {
    /// This placement changed by `motion`.
    pub fn moved_by(self, motion: LayerMotion) -> LayerPlacement {
        LayerPlacement {
            offset_x_in_pixels: self.offset_x_in_pixels + motion.offset_x_in_pixels,
            offset_y_in_pixels: self.offset_y_in_pixels + motion.offset_y_in_pixels,
            scale_factor: self.scale_factor * motion.scale_multiplier,
            opacity: self.opacity * motion.opacity_multiplier,
            ..self
        }
    }
}

/// Which picture of a transition is being moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionRole {
    /// The clip whose transition it is, drawn on top.
    TransitioningClip,
    /// The clip touching that edge on the same track, drawn underneath.
    TouchingClip,
}

/// How a picture moves during a transition.
///
/// `phase` is where the **transitioning** clip is in its entry or exit (from
/// [`Clip::transition_at`](kadr_core::clip::Clip::transition_at)); `role` says
/// whether the picture is that clip or the one touching it.
pub fn transition_motion(
    style: TransitionStyle,
    phase: EdgePhase,
    role: TransitionRole,
    canvas_size: PictureSize,
) -> LayerMotion {
    let (is_entering, progress) = match phase {
        EdgePhase::Entering { progress } => (true, progress),
        EdgePhase::Leaving { progress } => (false, progress),
        EdgePhase::Settled => return LayerMotion::NONE,
    };
    let progress = progress.clamp(0.0, 1.0);
    match role {
        TransitionRole::TransitioningClip => {
            picture_motion(style, is_entering, progress, canvas_size)
        }
        // The touching clip does the opposite, and stays opaque underneath.
        TransitionRole::TouchingClip => {
            picture_motion(style, !is_entering, progress, canvas_size).without_fading()
        }
    }
}

/// The motion of a picture entering (or leaving) with `style`, `progress` of
/// the way through.
fn picture_motion(
    style: TransitionStyle,
    is_entering: bool,
    progress: f32,
    canvas_size: PictureSize,
) -> LayerMotion {
    let eased = ease_in_out(progress);
    let fade = if is_entering {
        progress
    } else {
        1.0 - progress
    };
    match style {
        TransitionStyle::Fade => LayerMotion {
            opacity_multiplier: fade,
            ..LayerMotion::NONE
        },
        TransitionStyle::Slide { direction } => {
            let (step_x, step_y) = direction.unit_step();
            let travel_x = step_x * canvas_size.width_in_pixels as f32;
            let travel_y = step_y * canvas_size.height_in_pixels as f32;
            // Entering: from one canvas behind to in place. Leaving: from in place
            // to one canvas ahead.
            let distance_fraction = if is_entering { eased - 1.0 } else { eased };
            LayerMotion {
                offset_x_in_pixels: travel_x * distance_fraction,
                offset_y_in_pixels: travel_y * distance_fraction,
                ..LayerMotion::NONE
            }
        }
        TransitionStyle::Zoom { direction } => {
            let growth = match direction {
                ZoomDirection::In => ZOOM_TRANSITION_FACTOR,
                ZoomDirection::Out => 1.0 / ZOOM_TRANSITION_FACTOR,
            };
            let (start_scale, end_scale) = if is_entering {
                (1.0 / growth, 1.0)
            } else {
                (1.0, growth)
            };
            LayerMotion {
                opacity_multiplier: fade,
                scale_multiplier: start_scale + (end_scale - start_scale) * eased,
                ..LayerMotion::NONE
            }
        }
    }
}

/// How much of a text is visible.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum TextReveal {
    /// The whole text.
    #[default]
    Everything,
    /// The first letters: `shown_fraction` of them (0 to 1). The letter being
    /// typed fades in.
    Characters {
        /// From 0 (nothing) to 1 (every letter).
        shown_fraction: f32,
    },
    /// The first words: `shown_fraction` of them (0 to 1). The word appearing
    /// fades in.
    Words {
        /// From 0 (nothing) to 1 (every word).
        shown_fraction: f32,
    },
}

/// How a text looks at one moment of its entry or exit animation: how its layer
/// moves and how much of it is visible.
///
/// `phase` comes from
/// [`TextContent::animation_at`](kadr_core::clip::TextContent::animation_at).
pub fn text_animation_frame(
    style: TextAnimationStyle,
    phase: EdgePhase,
    font_size_in_pixels: f32,
) -> (LayerMotion, TextReveal) {
    let (is_entering, progress) = match phase {
        EdgePhase::Entering { progress } => (true, progress),
        EdgePhase::Leaving { progress } => (false, progress),
        EdgePhase::Settled => return (LayerMotion::NONE, TextReveal::Everything),
    };
    let progress = progress.clamp(0.0, 1.0);
    // How much of the text shows: rises during the entry, falls during the exit.
    let shown_fraction = if is_entering {
        progress
    } else {
        1.0 - progress
    };
    let fading = LayerMotion {
        opacity_multiplier: shown_fraction,
        ..LayerMotion::NONE
    };
    match style {
        TextAnimationStyle::Fade => (fading, TextReveal::Everything),
        TextAnimationStyle::Slide { direction } => {
            let (step_x, step_y) = direction.unit_step();
            let distance = TEXT_SLIDE_DISTANCE_IN_FONT_SIZES * font_size_in_pixels;
            let eased = ease_in_out(progress);
            let distance_fraction = if is_entering { eased - 1.0 } else { eased };
            let sliding = LayerMotion {
                offset_x_in_pixels: step_x * distance * distance_fraction,
                offset_y_in_pixels: step_y * distance * distance_fraction,
                ..fading
            };
            (sliding, TextReveal::Everything)
        }
        TextAnimationStyle::Pop => {
            let popping = LayerMotion {
                opacity_multiplier: (shown_fraction * TEXT_POP_FADE_SPEED).min(1.0),
                scale_multiplier: ease_out_back(shown_fraction),
                ..LayerMotion::NONE
            };
            (popping, TextReveal::Everything)
        }
        TextAnimationStyle::TypeOn => {
            (LayerMotion::NONE, TextReveal::Characters { shown_fraction })
        }
        TextAnimationStyle::WordByWord => (LayerMotion::NONE, TextReveal::Words { shown_fraction }),
    }
}

/// Smoothstep (3p² − 2p³): starts and ends gently, the same curve as the
/// ease-in-out keyframes of `kadr-core`.
fn ease_in_out(progress: f32) -> f32 {
    progress * progress * (3.0 - 2.0 * progress)
}

/// "Back" easing out: rises past 1, then settles on 1 (Robert Penner's
/// `easeOutBack`). 0 at the start, 1 at the end.
fn ease_out_back(progress: f32) -> f32 {
    let from_end = progress - 1.0;
    1.0 + (TEXT_POP_OVERSHOOT + 1.0) * from_end.powi(3) + TEXT_POP_OVERSHOOT * from_end.powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kadr_core::transition::SlideDirection;

    const CANVAS: PictureSize = PictureSize {
        width_in_pixels: 1920,
        height_in_pixels: 1080,
    };

    fn entering(progress: f32) -> EdgePhase {
        EdgePhase::Entering { progress }
    }

    fn leaving(progress: f32) -> EdgePhase {
        EdgePhase::Leaving { progress }
    }

    fn assert_close(found: f32, expected: f32) {
        assert!(
            (found - expected).abs() < 1e-4,
            "{found} instead of {expected}"
        );
    }

    #[test]
    fn a_fade_is_a_dissolve_over_the_touching_clip() {
        let on_top = transition_motion(
            TransitionStyle::Fade,
            entering(0.25),
            TransitionRole::TransitioningClip,
            CANVAS,
        );
        let underneath = transition_motion(
            TransitionStyle::Fade,
            entering(0.25),
            TransitionRole::TouchingClip,
            CANVAS,
        );
        assert_close(on_top.opacity_multiplier, 0.25);
        assert_eq!(underneath, LayerMotion::NONE);
        let leaving_fade = transition_motion(
            TransitionStyle::Fade,
            leaving(0.25),
            TransitionRole::TransitioningClip,
            CANVAS,
        );
        assert_close(leaving_fade.opacity_multiplier, 0.75);
    }

    #[test]
    fn a_slide_pushes_the_touching_clip_out() {
        let slide_left = TransitionStyle::Slide {
            direction: SlideDirection::Left,
        };
        let motion_at =
            |progress, role| transition_motion(slide_left, entering(progress), role, CANVAS);

        // At the start the entering picture is one canvas to the right, the
        // touching one in place; halfway they share the canvas; at the end the
        // entering picture is in place and the other one canvas to the left.
        assert_close(
            motion_at(0.0, TransitionRole::TransitioningClip).offset_x_in_pixels,
            1920.0,
        );
        assert_close(
            motion_at(0.0, TransitionRole::TouchingClip).offset_x_in_pixels,
            0.0,
        );
        assert_close(
            motion_at(0.5, TransitionRole::TransitioningClip).offset_x_in_pixels,
            960.0,
        );
        assert_close(
            motion_at(0.5, TransitionRole::TouchingClip).offset_x_in_pixels,
            -960.0,
        );
        assert_close(
            motion_at(1.0, TransitionRole::TransitioningClip).offset_x_in_pixels,
            0.0,
        );
        assert_close(
            motion_at(1.0, TransitionRole::TouchingClip).offset_x_in_pixels,
            -1920.0,
        );
        assert_close(
            motion_at(0.5, TransitionRole::TransitioningClip).opacity_multiplier,
            1.0,
        );
    }

    #[test]
    fn zooms_grow_or_shrink_both_pictures() {
        let zoom_in = TransitionStyle::Zoom {
            direction: ZoomDirection::In,
        };
        let entering_start = transition_motion(
            zoom_in,
            entering(0.0),
            TransitionRole::TransitioningClip,
            CANVAS,
        );
        let touching_end =
            transition_motion(zoom_in, entering(1.0), TransitionRole::TouchingClip, CANVAS);
        assert_close(entering_start.scale_multiplier, 0.5);
        assert_close(entering_start.opacity_multiplier, 0.0);
        assert_close(touching_end.scale_multiplier, 2.0);
        assert_close(touching_end.opacity_multiplier, 1.0);

        let zoom_out = TransitionStyle::Zoom {
            direction: ZoomDirection::Out,
        };
        let leaving_end = transition_motion(
            zoom_out,
            leaving(1.0),
            TransitionRole::TransitioningClip,
            CANVAS,
        );
        assert_close(leaving_end.scale_multiplier, 0.5);
        assert_close(leaving_end.opacity_multiplier, 0.0);
    }

    #[test]
    fn a_settled_clip_does_not_move() {
        assert_eq!(
            transition_motion(
                TransitionStyle::Fade,
                EdgePhase::Settled,
                TransitionRole::TransitioningClip,
                CANVAS
            ),
            LayerMotion::NONE
        );
        assert_eq!(
            text_animation_frame(TextAnimationStyle::TypeOn, EdgePhase::Settled, 64.0),
            (LayerMotion::NONE, TextReveal::Everything)
        );
    }

    #[test]
    fn text_animations_mirror_their_entry_when_leaving() {
        let (_, typed) = text_animation_frame(TextAnimationStyle::TypeOn, entering(0.3), 64.0);
        let (_, erased) = text_animation_frame(TextAnimationStyle::TypeOn, leaving(0.7), 64.0);
        assert_eq!(typed, erased);
        assert!(
            matches!(typed, TextReveal::Characters { shown_fraction } if (shown_fraction - 0.3).abs() < 1e-6)
        );

        let (popping, _) = text_animation_frame(TextAnimationStyle::Pop, entering(0.0), 64.0);
        assert_close(popping.scale_multiplier, 0.0);
        let (popped, _) = text_animation_frame(TextAnimationStyle::Pop, entering(0.8), 64.0);
        assert!(
            popped.scale_multiplier > 1.0,
            "the pop overshoots before settling"
        );
        let (settled_pop, _) = text_animation_frame(TextAnimationStyle::Pop, entering(1.0), 64.0);
        assert_close(settled_pop.scale_multiplier, 1.0);
    }

    #[test]
    fn a_sliding_text_travels_a_distance_following_its_size() {
        let slide_up = TextAnimationStyle::Slide {
            direction: SlideDirection::Up,
        };
        let (arriving, _) = text_animation_frame(slide_up, entering(0.0), 40.0);
        // Starts 1.5 font sizes below its place, invisible.
        assert_close(arriving.offset_y_in_pixels, 60.0);
        assert_close(arriving.opacity_multiplier, 0.0);
        let (departing, _) = text_animation_frame(slide_up, leaving(1.0), 40.0);
        assert_close(departing.offset_y_in_pixels, -60.0);
    }

    #[test]
    fn moving_a_placement_combines_with_its_own_values() {
        let placement = LayerPlacement {
            offset_x_in_pixels: 10.0,
            scale_factor: 2.0,
            opacity: 0.5,
            ..LayerPlacement::FITTED
        };
        let motion = LayerMotion {
            opacity_multiplier: 0.5,
            offset_x_in_pixels: 5.0,
            offset_y_in_pixels: -3.0,
            scale_multiplier: 1.5,
        };
        let moved = placement.moved_by(motion.combined_with(LayerMotion::NONE));
        assert_close(moved.offset_x_in_pixels, 15.0);
        assert_close(moved.offset_y_in_pixels, -3.0);
        assert_close(moved.scale_factor, 3.0);
        assert_close(moved.opacity, 0.25);
    }
}
