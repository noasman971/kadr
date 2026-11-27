//! The timeline: an ordered stack of tracks holding clips.
//!
//! Tracks are stacked in order: for video, the first track is at the bottom and
//! each following track is drawn on top of it. On a single track, clips never
//! overlap and are kept sorted by start time.

use serde::{Deserialize, Serialize};

use crate::clip::Clip;
use crate::ids::{ClipId, TrackId};
use crate::time::{TimeSpan, Timestamp};

/// What a track can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    /// Videos, images and texts. A video clip also plays its own sound.
    Video,
    /// Sound only: music, voice-over, sound effects.
    Audio,
}

impl TrackKind {
    /// The name used in messages.
    pub fn display_name(self) -> &'static str {
        match self {
            TrackKind::Video => "video",
            TrackKind::Audio => "audio",
        }
    }
}

/// A row of the timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    /// Identifier inside the project.
    pub id: TrackId,
    /// What the track can hold.
    pub kind: TrackKind,
    /// Name shown in the interface, for example `"Video 1"`.
    pub name: String,
    /// A muted track plays no sound.
    pub is_muted: bool,
    /// A hidden track shows no picture.
    pub is_hidden: bool,
    clips: Vec<Clip>,
}

impl Track {
    /// An empty, audible and visible track.
    pub(crate) fn new(id: TrackId, kind: TrackKind, name: String) -> Self {
        Self {
            id,
            kind,
            name,
            is_muted: false,
            is_hidden: false,
            clips: Vec::new(),
        }
    }

    /// The clips, sorted by start time.
    pub fn clips(&self) -> &[Clip] {
        &self.clips
    }

    /// The clip with this identifier, if it is on this track.
    pub fn clip(&self, clip_id: ClipId) -> Option<&Clip> {
        self.clips.iter().find(|clip| clip.id == clip_id)
    }

    /// Where the last clip of the track ends (zero for an empty track).
    pub fn end(&self) -> Timestamp {
        self.clips
            .iter()
            .map(Clip::timeline_end)
            .max()
            .unwrap_or(Timestamp::ZERO)
    }

    /// Whether the clips are sorted by start time. Always true for tracks
    /// modified through commands; checked when loading files.
    pub fn has_clips_sorted_by_start(&self) -> bool {
        self.clips
            .windows(2)
            .all(|pair| sort_key(&pair[0]) <= sort_key(&pair[1]))
    }

    /// Adds a clip, keeping the clips sorted.
    pub(crate) fn insert_clip(&mut self, clip: Clip) {
        let insertion_index = self
            .clips
            .partition_point(|existing| sort_key(existing) <= sort_key(&clip));
        self.clips.insert(insertion_index, clip);
    }

    /// Removes and returns the clip with this identifier.
    pub(crate) fn remove_clip(&mut self, clip_id: ClipId) -> Option<Clip> {
        let index = self.clips.iter().position(|clip| clip.id == clip_id)?;
        Some(self.clips.remove(index))
    }

    /// Replaces the clip that has the same identifier as `new_version` and returns
    /// the previous version. Re-sorts the track in case the start time changed.
    pub(crate) fn replace_clip(&mut self, new_version: Clip) -> Option<Clip> {
        let previous_version = self.remove_clip(new_version.id)?;
        self.insert_clip(new_version);
        Some(previous_version)
    }

    /// Replaces every clip of the track by `new_clips` (sorted here) and returns
    /// the previous ones, in order.
    pub(crate) fn replace_all_clips(&mut self, mut new_clips: Vec<Clip>) -> Vec<Clip> {
        new_clips.sort_by_key(sort_key);
        std::mem::replace(&mut self.clips, new_clips)
    }
}

/// Clips are ordered by start time, then by identifier so the order is stable.
fn sort_key(clip: &Clip) -> (Timestamp, ClipId) {
    (clip.timeline_start, clip.id)
}

/// All the tracks of a project, from bottom to top.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Timeline {
    tracks: Vec<Track>,
}

impl Timeline {
    /// The tracks, from bottom (index 0) to top.
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// The track with this identifier.
    pub fn track(&self, track_id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|track| track.id == track_id)
    }

    /// The position of the track in the stack (0 is the bottom).
    pub fn track_index(&self, track_id: TrackId) -> Option<usize> {
        self.tracks.iter().position(|track| track.id == track_id)
    }

    /// The identifiers of every track, from bottom to top.
    pub fn track_ids(&self) -> Vec<TrackId> {
        self.tracks.iter().map(|track| track.id).collect()
    }

    /// Every clip of every track, with the track it is on.
    pub fn all_clips(&self) -> impl Iterator<Item = (&Track, &Clip)> {
        self.tracks
            .iter()
            .flat_map(|track| track.clips.iter().map(move |clip| (track, clip)))
    }

    /// The identifiers of every clip, in increasing order.
    pub fn clip_ids(&self) -> Vec<ClipId> {
        let mut clip_ids: Vec<ClipId> = self.all_clips().map(|(_, clip)| clip.id).collect();
        clip_ids.sort();
        clip_ids
    }

    /// The clip with this identifier and the track it is on.
    pub fn find_clip(&self, clip_id: ClipId) -> Option<(&Track, &Clip)> {
        self.all_clips().find(|(_, clip)| clip.id == clip_id)
    }

    /// How long the whole timeline lasts: up to the end of the last clip.
    pub fn duration(&self) -> TimeSpan {
        let timeline_end = self
            .tracks
            .iter()
            .map(Track::end)
            .max()
            .unwrap_or(Timestamp::ZERO);
        timeline_end
            .span_since(Timestamp::ZERO)
            .unwrap_or(TimeSpan::ZERO)
    }

    /// The track with this identifier, for modification by commands.
    pub(crate) fn track_mut(&mut self, track_id: TrackId) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|track| track.id == track_id)
    }

    /// The track holding this clip, for modification by commands.
    pub(crate) fn track_holding_clip_mut(&mut self, clip_id: ClipId) -> Option<&mut Track> {
        self.tracks
            .iter_mut()
            .find(|track| track.clip(clip_id).is_some())
    }

    /// Inserts a track at `index` in the stack (clamped to the top).
    pub(crate) fn insert_track(&mut self, index: usize, track: Track) {
        let clamped_index = index.min(self.tracks.len());
        self.tracks.insert(clamped_index, track);
    }

    /// Removes a track and returns it with its former position.
    pub(crate) fn remove_track(&mut self, track_id: TrackId) -> Option<(usize, Track)> {
        let index = self.track_index(track_id)?;
        Some((index, self.tracks.remove(index)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::test_samples::text_clip;

    #[test]
    fn clips_stay_sorted_when_inserted_and_replaced() {
        let mut track = Track::new(TrackId::from_raw(1), TrackKind::Video, "Video 1".into());
        track.insert_clip(text_clip(3, 2, 10));
        track.insert_clip(text_clip(4, 2, 0));
        track.insert_clip(text_clip(5, 2, 5));

        let starts: Vec<u64> = track
            .clips()
            .iter()
            .map(|clip| clip.timeline_start.ticks())
            .collect();
        assert!(starts.windows(2).all(|pair| pair[0] <= pair[1]));

        let mut moved_clip = track.clip(ClipId::from_raw(4)).cloned().unwrap();
        moved_clip.timeline_start = Timestamp::from_whole_seconds(20);
        track.replace_clip(moved_clip);
        assert_eq!(track.clips().last().unwrap().id, ClipId::from_raw(4));
        assert!(track.has_clips_sorted_by_start());
        assert_eq!(track.end(), Timestamp::from_whole_seconds(22));
    }

    #[test]
    fn timeline_finds_clips_and_measures_its_duration() {
        let mut timeline = Timeline::default();
        let mut first_track = Track::new(TrackId::from_raw(1), TrackKind::Video, "Video 1".into());
        first_track.insert_clip(text_clip(10, 3, 0));
        let mut second_track = Track::new(TrackId::from_raw(2), TrackKind::Video, "Video 2".into());
        second_track.insert_clip(text_clip(11, 3, 5));
        timeline.insert_track(0, first_track);
        timeline.insert_track(99, second_track);

        assert_eq!(
            timeline.track_ids(),
            vec![TrackId::from_raw(1), TrackId::from_raw(2)]
        );
        assert_eq!(
            timeline.find_clip(ClipId::from_raw(11)).unwrap().0.id,
            TrackId::from_raw(2)
        );
        assert_eq!(timeline.duration(), TimeSpan::from_whole_seconds(8));
        assert_eq!(
            timeline.clip_ids(),
            vec![ClipId::from_raw(10), ClipId::from_raw(11)]
        );
    }
}
