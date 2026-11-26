//! Text animations: how the words of a text clip appear and disappear.
//!
//! A text clip has an optional **entry** animation, played over its first
//! moments, and an optional **exit** animation, played over its last moments,
//! with the same timing rules as the transitions of videos and images
//! ([`edge_phase_at`](crate::transition::edge_phase_at)). Texts do not take
//! transitions: their animations act on the letters themselves (type-on) and
//! move by a distance that follows the font size.

use serde::{Deserialize, Serialize};

use crate::time::TimeSpan;
use crate::transition::SlideDirection;

/// How a text appears or disappears, and for how long.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TextAnimation {
    /// What the text does.
    pub style: TextAnimationStyle,
    /// How long it takes, from the clip's edge.
    pub duration: TimeSpan,
}

/// What a text does while it appears or disappears.
///
/// The exit of each style mirrors its entry: a typed-on text is erased from its
/// last letter, a popped-in text shrinks away. Slides keep moving the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TextAnimationStyle {
    /// The text fades in or out.
    Fade,
    /// The text glides into place over a short distance while fading in, and
    /// glides on the same way when it leaves.
    Slide {
        /// Which way the text moves.
        direction: SlideDirection,
    },
    /// The text grows from nothing, slightly overshoots its size and settles.
    Pop,
    /// The letters appear one after another, as if typed.
    TypeOn,
    /// The words appear one after another.
    WordByWord,
}

impl TextAnimationStyle {
    /// A short name for messages, for example `"type-on"`.
    pub fn display_name(&self) -> &'static str {
        match self {
            TextAnimationStyle::Fade => "fade",
            TextAnimationStyle::Slide { .. } => "slide",
            TextAnimationStyle::Pop => "pop",
            TextAnimationStyle::TypeOn => "type-on",
            TextAnimationStyle::WordByWord => "word by word",
        }
    }
}
