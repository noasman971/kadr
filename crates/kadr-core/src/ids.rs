//! Identifiers of the objects in a project.
//!
//! Every object that can be referred to (a media asset, a track, a clip, an
//! effect) gets a number from a single counter owned by the project. Numbers are
//! small, readable ("clip 12") and **never shared between two objects**, even of
//! different kinds, which keeps error messages unambiguous.
//!
//! Each kind of object has its own type ([`ClipId`], [`TrackId`]…), so the compiler
//! refuses to pass a track identifier where a clip identifier is expected.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Declares an identifier type wrapping a `u64`. Written once as a macro so that
/// the four identifier types stay strictly identical.
macro_rules! define_object_id {
    ($type_name:ident, $object_description:literal) => {
        #[doc = concat!("Identifies ", $object_description, " inside a project.")]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $type_name(u64);

        impl $type_name {
            /// Wraps a raw number. Prefer identifiers handed out by the project.
            pub const fn from_raw(raw_value: u64) -> Self {
                Self(raw_value)
            }

            /// The raw number, as shown to people and to Claude.
            pub const fn raw(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $type_name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}", self.0)
            }
        }
    };
}

define_object_id!(MediaAssetId, "an imported media file");
define_object_id!(TrackId, "a track of the timeline");
define_object_id!(ClipId, "a clip placed on a track");
define_object_id!(EffectId, "an effect applied to a clip");

/// Hands out identifiers. Stored in the project file so that numbers keep
/// increasing across sessions.
///
/// Undoing a command restores the allocator to its previous state, so redoing
/// it hands out exactly the same identifiers again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdAllocator {
    next_raw_id: u64,
}

impl Default for IdAllocator {
    fn default() -> Self {
        Self { next_raw_id: 1 }
    }
}

impl IdAllocator {
    /// The number the next allocated identifier will get.
    pub const fn next_raw_id(self) -> u64 {
        self.next_raw_id
    }

    /// Takes the next free number.
    pub(crate) fn allocate_raw_id(&mut self) -> u64 {
        let allocated_raw_id = self.next_raw_id;
        self.next_raw_id = self.next_raw_id.saturating_add(1);
        allocated_raw_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocator_starts_at_one_and_never_repeats() {
        let mut allocator = IdAllocator::default();
        assert_eq!(allocator.allocate_raw_id(), 1);
        assert_eq!(allocator.allocate_raw_id(), 2);
        assert_eq!(allocator.next_raw_id(), 3);
    }

    #[test]
    fn identifiers_display_as_plain_numbers() {
        assert_eq!(ClipId::from_raw(12).to_string(), "12");
        assert_eq!(serde_json::to_string(&TrackId::from_raw(7)).unwrap(), "7");
    }
}
