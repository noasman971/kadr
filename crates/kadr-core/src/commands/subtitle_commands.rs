//! Commands that add, edit and restyle subtitle cues.
//!
//! A subtitle track is added with [`AddTrack`](super::AddTrack) (kind
//! [`TrackKind::Subtitle`], optionally
//! [`with_subtitle_style`](super::AddTrack::with_subtitle_style)), and filled with
//! [`AddSubtitleCues`]. Cues are clips: [`MoveClip`](super::MoveClip),
//! [`TrimClip`](super::TrimClip), [`SplitClip`](super::SplitClip) (which cuts the
//! words between the two parts) and [`RemoveClip`](super::RemoveClip) work on them
//! too.

use super::{
    ClipChangeRecord, CreatedObject, EditCommand, IdAllocatorRecord, object_vanished,
    revert_without_apply,
};
use crate::clip::{Clip, ClipContent};
use crate::error::EditError;
use crate::ids::{ClipId, TrackId};
use crate::project::Project;
use crate::subtitle::{SubtitleCue, SubtitleStyle, TimedSubtitleCue, joined_cue_texts};
use crate::time::{TimeRange, TimeSpan, Timestamp};
use crate::timeline::{Track, TrackKind};

/// The subtitle track with this identifier, or why it is not one.
fn require_subtitle_track(project: &Project, track_id: TrackId) -> Result<&Track, EditError> {
    let track = project.require_track(track_id)?;
    if track.kind != TrackKind::Subtitle {
        return Err(not_a_subtitle_track(project, track_id));
    }
    Ok(track)
}

fn not_a_subtitle_track(project: &Project, track_id: TrackId) -> EditError {
    EditError::NotASubtitleTrack {
        track_id,
        subtitle_track_ids: project
            .timeline()
            .subtitle_tracks()
            .map(|track| track.id)
            .collect(),
    }
}

/// The cue with this identifier and its track, or why it is not one.
fn require_cue(
    project: &Project,
    clip_id: ClipId,
) -> Result<(&Track, &Clip, &SubtitleCue), EditError> {
    let (track, clip) = project.require_clip(clip_id)?;
    match &clip.content {
        ClipContent::Subtitle(cue) => Ok((track, clip, cue)),
        ClipContent::Media(_) | ClipContent::Text(_) => Err(EditError::NotASubtitleCue { clip_id }),
    }
}

/// The time between `start` and `end`, refused if it is not positive.
fn cue_duration(start: Timestamp, end: Timestamp) -> Result<TimeSpan, EditError> {
    end.span_since(start)
        .filter(|duration| !duration.is_zero())
        .ok_or(EditError::CueEndsBeforeItStarts { start, end })
}

/// Adds cues to a subtitle track, in one step however many.
#[derive(Debug)]
pub struct AddSubtitleCues {
    track_id: TrackId,
    cues: Vec<TimedSubtitleCue>,
    id_allocator_record: IdAllocatorRecord,
    created_clip_ids: Vec<ClipId>,
}

impl AddSubtitleCues {
    /// Prepares adding `cues` (in any order; they must not overlap each other
    /// nor the track's cues) to the subtitle track `track_id`.
    pub fn new(track_id: TrackId, cues: Vec<TimedSubtitleCue>) -> Self {
        Self {
            track_id,
            cues,
            id_allocator_record: IdAllocatorRecord::default(),
            created_clip_ids: Vec::new(),
        }
    }
}

impl EditCommand for AddSubtitleCues {
    fn label(&self) -> String {
        format!(
            "Add {} subtitle cue(s) to track {}",
            self.cues.len(),
            self.track_id
        )
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        require_subtitle_track(project, self.track_id)?;
        let durations = self
            .cues
            .iter()
            .map(|cue| cue_duration(cue.start, cue.end))
            .collect::<Result<Vec<_>, _>>()?;

        self.id_allocator_record.remember(project);
        let mut new_clips = Vec::with_capacity(self.cues.len());
        for (cue, duration) in self.cues.iter().zip(durations) {
            let clip_id = ClipId::from_raw(project.id_allocator_mut().allocate_raw_id());
            let content = ClipContent::Subtitle(SubtitleCue::new(cue.text.clone(), duration));
            new_clips.push(Clip::new(clip_id, content, cue.start));
        }
        let track = project
            .timeline_mut()
            .track_mut(self.track_id)
            .ok_or_else(|| object_vanished(format!("track {}", self.track_id)))?;
        self.created_clip_ids = new_clips.iter().map(|clip| clip.id).collect();
        for clip in new_clips {
            track.insert_clip(clip);
        }
        Ok(self
            .created_clip_ids
            .iter()
            .map(|clip_id| CreatedObject::Clip(*clip_id))
            .collect())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let label = self.label();
        if self.id_allocator_record.state_before_apply.is_none() {
            return Err(revert_without_apply(&label));
        }
        let track = project
            .timeline_mut()
            .track_mut(self.track_id)
            .ok_or_else(|| object_vanished(format!("track {}", self.track_id)))?;
        for clip_id in std::mem::take(&mut self.created_clip_ids) {
            track
                .remove_clip(clip_id)
                .ok_or_else(|| object_vanished(format!("clip {clip_id}")))?;
        }
        self.id_allocator_record.restore(project, &label)
    }
}

/// Changes the style every cue of a subtitle track is drawn with.
#[derive(Debug)]
pub struct SetSubtitleStyle {
    track_id: TrackId,
    style: SubtitleStyle,
    previous_style: Option<Option<SubtitleStyle>>,
}

impl SetSubtitleStyle {
    /// Prepares the change.
    pub fn new(track_id: TrackId, style: SubtitleStyle) -> Self {
        Self {
            track_id,
            style,
            previous_style: None,
        }
    }
}

impl EditCommand for SetSubtitleStyle {
    fn label(&self) -> String {
        format!("Change the subtitle style of track {}", self.track_id)
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        require_subtitle_track(project, self.track_id)?;
        let track = project
            .timeline_mut()
            .track_mut(self.track_id)
            .ok_or_else(|| object_vanished(format!("track {}", self.track_id)))?;
        let previous_style = track.subtitle_style.replace(self.style.clone());
        self.previous_style = Some(previous_style);
        Ok(Vec::new())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let previous_style = self
            .previous_style
            .take()
            .ok_or_else(|| revert_without_apply(&self.label()))?;
        project
            .timeline_mut()
            .track_mut(self.track_id)
            .ok_or_else(|| object_vanished(format!("track {}", self.track_id)))?
            .subtitle_style = previous_style;
        Ok(())
    }
}

/// Changes the words of a cue.
#[derive(Debug)]
pub struct SetSubtitleCueText {
    clip_id: ClipId,
    new_text: String,
    change_record: ClipChangeRecord,
}

impl SetSubtitleCueText {
    /// Prepares the change. `new_text` may hold line breaks and `<i>` / `<b>` tags.
    pub fn new(clip_id: ClipId, new_text: impl Into<String>) -> Self {
        Self {
            clip_id,
            new_text: new_text.into(),
            change_record: ClipChangeRecord::default(),
        }
    }
}

impl EditCommand for SetSubtitleCueText {
    fn label(&self) -> String {
        format!("Edit the words of subtitle cue {}", self.clip_id)
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        require_cue(project, self.clip_id)?;
        let new_text = self.new_text.clone();
        self.change_record
            .replace_clip(project, self.clip_id, |clip| {
                let mut new_version = clip.clone();
                if let ClipContent::Subtitle(cue) = &mut new_version.content {
                    cue.text = new_text;
                }
                Ok(new_version)
            })?;
        Ok(Vec::new())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let label = self.label();
        self.change_record.restore_previous_version(project, &label)
    }
}

/// Moves a cue's start and end at once (on its track).
#[derive(Debug)]
pub struct SetSubtitleCueTiming {
    clip_id: ClipId,
    start: Timestamp,
    end: Timestamp,
    change_record: ClipChangeRecord,
}

impl SetSubtitleCueTiming {
    /// Prepares showing the cue from `start` to `end` on the timeline.
    pub fn new(clip_id: ClipId, start: Timestamp, end: Timestamp) -> Self {
        Self {
            clip_id,
            start,
            end,
            change_record: ClipChangeRecord::default(),
        }
    }
}

impl EditCommand for SetSubtitleCueTiming {
    fn label(&self) -> String {
        format!(
            "Show subtitle cue {} from {} to {}",
            self.clip_id, self.start, self.end
        )
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        require_cue(project, self.clip_id)?;
        let (start, duration) = (self.start, cue_duration(self.start, self.end)?);
        self.change_record
            .replace_clip(project, self.clip_id, |clip| {
                let mut new_version = clip.clone();
                new_version.timeline_start = start;
                if let ClipContent::Subtitle(cue) = &mut new_version.content {
                    cue.duration = duration;
                }
                Ok(new_version)
            })?;
        Ok(Vec::new())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let label = self.label();
        self.change_record.restore_previous_version(project, &label)
    }
}

/// Merges a cue with the next one of its track: the first cue lasts until the
/// end of the second (over any gap between them) and shows both texts, joined
/// by a space; the second cue is removed.
#[derive(Debug)]
pub struct MergeSubtitleCues {
    first_clip_id: ClipId,
    second_clip_id: ClipId,
    change_record: ClipChangeRecord,
    removed_second_cue: Option<(TrackId, Clip)>,
}

impl MergeSubtitleCues {
    /// Prepares merging `first_clip_id` with `second_clip_id`, the cue right
    /// after it on the same track.
    pub fn new(first_clip_id: ClipId, second_clip_id: ClipId) -> Self {
        Self {
            first_clip_id,
            second_clip_id,
            change_record: ClipChangeRecord::default(),
            removed_second_cue: None,
        }
    }

    /// The merged first cue, or why the two cannot be merged.
    fn merged_first_cue(&self, project: &Project) -> Result<Clip, EditError> {
        let cannot_merge = |reason: &str| EditError::CuesCannotBeMerged {
            first_clip_id: self.first_clip_id,
            second_clip_id: self.second_clip_id,
            reason: reason.to_string(),
        };
        let (first_track, first_clip, first_cue) = require_cue(project, self.first_clip_id)?;
        let (second_track, second_clip, second_cue) = require_cue(project, self.second_clip_id)?;
        if first_track.id != second_track.id {
            return Err(cannot_merge("they are on different tracks"));
        }
        let clips = first_track.clips();
        let first_index = clips.iter().position(|clip| clip.id == first_clip.id);
        let second_index = clips.iter().position(|clip| clip.id == second_clip.id);
        let are_neighbours = matches!(
            (first_index, second_index),
            (Some(first), Some(second)) if second == first + 1
        );
        if !are_neighbours {
            return Err(cannot_merge(
                "the second must be the cue right after the first on their track",
            ));
        }
        let merged_duration = second_clip
            .timeline_end()
            .span_since(first_clip.timeline_start)
            .ok_or_else(|| cannot_merge("the second starts before the first"))?;
        let mut merged = first_clip.clone();
        merged.content = ClipContent::Subtitle(SubtitleCue::new(
            joined_cue_texts(&first_cue.text, &second_cue.text),
            merged_duration,
        ));
        Ok(merged)
    }
}

impl EditCommand for MergeSubtitleCues {
    fn label(&self) -> String {
        format!(
            "Merge subtitle cues {} and {}",
            self.first_clip_id, self.second_clip_id
        )
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let merged = self.merged_first_cue(project)?;
        let track = project
            .timeline_mut()
            .track_holding_clip_mut(self.second_clip_id)
            .ok_or_else(|| object_vanished(format!("clip {}", self.second_clip_id)))?;
        let track_id = track.id;
        let second_cue = track
            .remove_clip(self.second_clip_id)
            .ok_or_else(|| object_vanished(format!("clip {}", self.second_clip_id)))?;
        self.removed_second_cue = Some((track_id, second_cue));
        self.change_record
            .replace_clip(project, self.first_clip_id, |_| Ok(merged))?;
        Ok(Vec::new())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        let label = self.label();
        self.change_record
            .restore_previous_version(project, &label)?;
        let (track_id, second_cue) = self
            .removed_second_cue
            .take()
            .ok_or_else(|| revert_without_apply(&label))?;
        project
            .timeline_mut()
            .track_mut(track_id)
            .ok_or_else(|| object_vanished(format!("track {track_id}")))?
            .insert_clip(second_cue);
        Ok(())
    }
}

/// Removes moments of the timeline from every subtitle track and closes the
/// gaps: cue parts inside `removed_ranges` go, and what follows moves earlier.
/// Used when the same moments are cut out of the clips the subtitles describe
/// ([`RemoveClipParts`](super::RemoveClipParts) does it by itself).
#[derive(Debug)]
pub struct RemoveTimeFromSubtitleTracks {
    removed_ranges: Vec<TimeRange>,
    previous_cues: Vec<(TrackId, Vec<Clip>)>,
    was_applied: bool,
}

impl RemoveTimeFromSubtitleTracks {
    /// Prepares removing `removed_ranges` (timeline times, in any order,
    /// possibly overlapping).
    pub fn new(removed_ranges: Vec<TimeRange>) -> Self {
        Self {
            removed_ranges,
            previous_cues: Vec::new(),
            was_applied: false,
        }
    }
}

impl EditCommand for RemoveTimeFromSubtitleTracks {
    fn label(&self) -> String {
        format!(
            "Remove {} range(s) of time from the subtitle tracks",
            self.removed_ranges.len()
        )
    }

    fn apply(&mut self, project: &mut Project) -> Result<Vec<CreatedObject>, EditError> {
        let new_cues = subtitle_cues_without_time(project, &self.removed_ranges);
        self.previous_cues = replace_track_clips(project, new_cues)?;
        self.was_applied = true;
        Ok(Vec::new())
    }

    fn revert(&mut self, project: &mut Project) -> Result<(), EditError> {
        if !std::mem::take(&mut self.was_applied) {
            return Err(revert_without_apply(&self.label()));
        }
        replace_track_clips(project, std::mem::take(&mut self.previous_cues))?;
        Ok(())
    }
}

/// Puts each list of clips on its track and returns the lists they replace.
pub(super) fn replace_track_clips(
    project: &mut Project,
    clips_by_track: Vec<(TrackId, Vec<Clip>)>,
) -> Result<Vec<(TrackId, Vec<Clip>)>, EditError> {
    let mut previous = Vec::with_capacity(clips_by_track.len());
    for (track_id, clips) in clips_by_track {
        let track = project
            .timeline_mut()
            .track_mut(track_id)
            .ok_or_else(|| object_vanished(format!("track {track_id}")))?;
        previous.push((track_id, track.replace_all_clips(clips)));
    }
    Ok(previous)
}

/// The cues of every subtitle track touched by removing `removed_ranges` and
/// closing the gaps, by track (tracks left unchanged are not listed).
///
/// A moment `t` moves earlier by the removed time before it, so a cue inside a
/// removed range disappears, a cue overlapping one gets shorter, and later cues
/// keep their length. Texts do not change.
pub(super) fn subtitle_cues_without_time(
    project: &Project,
    removed_ranges: &[TimeRange],
) -> Vec<(TrackId, Vec<Clip>)> {
    let merged_ranges = merged_time_ranges(removed_ranges);
    if merged_ranges.is_empty() {
        return Vec::new();
    }
    let moved = |time: Timestamp| {
        time.checked_sub(time_removed_before(&merged_ranges, time))
            .unwrap_or(Timestamp::ZERO)
    };
    let mut changed_tracks = Vec::new();
    for track in project.timeline().subtitle_tracks() {
        let mut is_changed = false;
        let mut new_clips = Vec::with_capacity(track.clips().len());
        for clip in track.clips() {
            let (new_start, new_end) = (moved(clip.timeline_start), moved(clip.timeline_end()));
            if (new_start, new_end) == (clip.timeline_start, clip.timeline_end()) {
                new_clips.push(clip.clone());
                continue;
            }
            is_changed = true;
            let Some(new_duration) = new_end.span_since(new_start).filter(|span| !span.is_zero())
            else {
                continue;
            };
            let mut moved_clip = clip.clone();
            moved_clip.timeline_start = new_start;
            if let ClipContent::Subtitle(cue) = &mut moved_clip.content {
                cue.duration = new_duration;
            }
            new_clips.push(moved_clip);
        }
        if is_changed {
            changed_tracks.push((track.id, new_clips));
        }
    }
    changed_tracks
}

/// The ranges sorted by start, overlapping and touching ones joined.
fn merged_time_ranges(ranges: &[TimeRange]) -> Vec<TimeRange> {
    let mut sorted = ranges.to_vec();
    sorted.sort_by_key(|range| range.start());
    let mut merged: Vec<TimeRange> = Vec::with_capacity(sorted.len());
    for range in sorted {
        match merged.last_mut() {
            Some(last) if range.start() <= last.end() => {
                if range.end() > last.end()
                    && let Some(joined) = TimeRange::new(last.start(), range.end())
                {
                    *last = joined;
                }
            }
            _ => merged.push(range),
        }
    }
    merged
}

/// How much of `merged_ranges` lies before `time`.
fn time_removed_before(merged_ranges: &[TimeRange], time: Timestamp) -> TimeSpan {
    merged_ranges
        .iter()
        .take_while(|range| range.start() < time)
        .map(|range| {
            range
                .end()
                .min(time)
                .span_since(range.start())
                .unwrap_or(TimeSpan::ZERO)
        })
        .fold(TimeSpan::ZERO, |total, span| total + span)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::{apply_and_check_revert, apply_expecting_error};
    use crate::commands::{
        AddClip, AddTrack, CreatedObjectsExt, ImportMedia, RemoveClipParts, SplitClip,
        TrackPosition,
    };
    use crate::media_asset::test_samples::ten_second_video;
    use crate::project::ProjectSettings;
    use crate::subtitle::SubtitleBackdrop;

    fn seconds(value: u64) -> Timestamp {
        Timestamp::from_whole_seconds(value)
    }

    fn milliseconds(value: u64) -> Timestamp {
        Timestamp::from_milliseconds(value)
    }

    fn timed(start_millisecond: u64, end_millisecond: u64, text: &str) -> TimedSubtitleCue {
        TimedSubtitleCue {
            start: milliseconds(start_millisecond),
            end: milliseconds(end_millisecond),
            text: text.to_string(),
        }
    }

    /// A video track with 6 s of video, and a subtitle track with three cues:
    /// 0.5–1.5 s, 2–4 s and 4.5–5.5 s.
    struct Fixture {
        project: Project,
        video_clip_id: ClipId,
        subtitle_track_id: TrackId,
        cue_ids: Vec<ClipId>,
    }

    fn fixture() -> Fixture {
        let project = Project::new(ProjectSettings::vertical_1080x1920());
        let (project, created) =
            apply_and_check_revert(&project, ImportMedia::new(ten_second_video()));
        let media_asset_id = created.first_media_asset_id().unwrap();
        let (project, created) = apply_and_check_revert(
            &project,
            AddTrack::new(TrackKind::Video, TrackPosition::Top),
        );
        let video_track_id = created.first_track_id().unwrap();
        let (project, created) = apply_and_check_revert(
            &project,
            AddClip::media(
                video_track_id,
                media_asset_id,
                seconds(0),
                seconds(6),
                seconds(0),
            ),
        );
        let video_clip_id = created.first_clip_id().unwrap();
        let (project, created) = apply_and_check_revert(
            &project,
            AddTrack::new(TrackKind::Subtitle, TrackPosition::Top),
        );
        let subtitle_track_id = created.first_track_id().unwrap();
        let (project, created) = apply_and_check_revert(
            &project,
            AddSubtitleCues::new(
                subtitle_track_id,
                vec![
                    timed(2_000, 4_000, "Second cue"),
                    timed(500, 1_500, "First cue"),
                    timed(4_500, 5_500, "<i>Third</i> cue"),
                ],
            ),
        );
        let mut cue_ids: Vec<ClipId> = created
            .iter()
            .filter_map(|object| match object {
                CreatedObject::Clip(clip_id) => Some(*clip_id),
                _ => None,
            })
            .collect();
        // Created in the given order: put them in time order.
        cue_ids.swap(0, 1);
        Fixture {
            project,
            video_clip_id,
            subtitle_track_id,
            cue_ids,
        }
    }

    fn cue_spans(project: &Project, track_id: TrackId) -> Vec<(u64, u64, String)> {
        project
            .timeline()
            .track(track_id)
            .unwrap()
            .clips()
            .iter()
            .map(|clip| {
                let ClipContent::Subtitle(cue) = &clip.content else {
                    panic!("not a cue");
                };
                (
                    clip.timeline_start.ticks() / 705_600,
                    clip.timeline_end().ticks() / 705_600,
                    cue.text.clone(),
                )
            })
            .collect()
    }

    #[test]
    fn a_new_subtitle_track_gets_the_default_style_of_the_canvas() {
        let fixture = fixture();
        let track = fixture
            .project
            .timeline()
            .track(fixture.subtitle_track_id)
            .unwrap();
        assert_eq!(track.name, "Subtitles 1");
        let style = track.subtitle_style.as_ref().unwrap();
        assert_eq!(style.margin_in_pixels, 480.0, "clear of TikTok's buttons");
        assert_eq!(
            cue_spans(&fixture.project, fixture.subtitle_track_id),
            vec![
                (500, 1_500, "First cue".to_string()),
                (2_000, 4_000, "Second cue".to_string()),
                (4_500, 5_500, "<i>Third</i> cue".to_string()),
            ]
        );
    }

    #[test]
    fn cues_go_only_on_subtitle_tracks_and_must_last() {
        let fixture = fixture();
        let video_track_id = fixture.project.timeline().tracks()[0].id;
        assert!(matches!(
            apply_expecting_error(
                &fixture.project,
                AddSubtitleCues::new(video_track_id, vec![timed(0, 100, "x")])
            ),
            EditError::NotASubtitleTrack { .. }
        ));
        assert!(matches!(
            apply_expecting_error(
                &fixture.project,
                AddSubtitleCues::new(fixture.subtitle_track_id, vec![timed(7_000, 7_000, "x")])
            ),
            EditError::CueEndsBeforeItStarts { .. }
        ));
    }

    #[test]
    fn the_style_text_and_timing_of_cues_change_and_revert() {
        let fixture = fixture();
        let mut style = SubtitleStyle::for_canvas(fixture.project.settings().canvas_size);
        style.backdrop = SubtitleBackdrop::Box {
            color: crate::color::Color::BLACK,
        };
        let (project, _) = apply_and_check_revert(
            &fixture.project,
            SetSubtitleStyle::new(fixture.subtitle_track_id, style.clone()),
        );
        assert_eq!(
            project
                .timeline()
                .track(fixture.subtitle_track_id)
                .unwrap()
                .subtitle_style,
            Some(style)
        );
        let (project, _) = apply_and_check_revert(
            &project,
            SetSubtitleCueText::new(fixture.cue_ids[0], "Fixed\n<b>words</b>"),
        );
        let (project, _) = apply_and_check_revert(
            &project,
            SetSubtitleCueTiming::new(fixture.cue_ids[0], milliseconds(100), milliseconds(1_900)),
        );
        assert_eq!(
            cue_spans(&project, fixture.subtitle_track_id)[0],
            (100, 1_900, "Fixed\n<b>words</b>".to_string())
        );
        assert!(matches!(
            apply_expecting_error(
                &project,
                SetSubtitleCueTiming::new(fixture.cue_ids[0], seconds(2), seconds(1))
            ),
            EditError::CueEndsBeforeItStarts { .. }
        ));
        assert!(matches!(
            apply_expecting_error(
                &project,
                SetSubtitleCueText::new(fixture.video_clip_id, "x")
            ),
            EditError::NotASubtitleCue { .. }
        ));
    }

    #[test]
    fn neighbouring_cues_merge_into_one() {
        let fixture = fixture();
        let (project, _) = apply_and_check_revert(
            &fixture.project,
            MergeSubtitleCues::new(fixture.cue_ids[0], fixture.cue_ids[1]),
        );
        assert_eq!(
            cue_spans(&project, fixture.subtitle_track_id),
            vec![
                (500, 4_000, "First cue Second cue".to_string()),
                (4_500, 5_500, "<i>Third</i> cue".to_string()),
            ]
        );
        assert!(matches!(
            apply_expecting_error(
                &fixture.project,
                MergeSubtitleCues::new(fixture.cue_ids[0], fixture.cue_ids[2])
            ),
            EditError::CuesCannotBeMerged { .. }
        ));
    }

    #[test]
    fn splitting_a_cue_shares_its_words() {
        let fixture = fixture();
        let (project, created) = apply_and_check_revert(
            &fixture.project,
            SplitClip::new(fixture.cue_ids[1], milliseconds(3_000)),
        );
        assert_eq!(created.len(), 1);
        assert_eq!(
            cue_spans(&project, fixture.subtitle_track_id)[1..3],
            [
                (2_000, 3_000, "Second".to_string()),
                (3_000, 4_000, "cue".to_string()),
            ]
        );
    }

    #[test]
    fn cutting_parts_of_a_video_closes_up_the_subtitles_too() {
        let fixture = fixture();
        let range = |start, end| TimeRange::new(milliseconds(start), milliseconds(end)).unwrap();
        // Remove 1–2.5 s (the end of the first cue and the start of the second)
        // and 4.6–5 s (inside the third).
        let (project, _) = apply_and_check_revert(
            &fixture.project,
            RemoveClipParts::new(
                fixture.video_clip_id,
                vec![range(4_600, 5_000), range(1_000, 2_500)],
            ),
        );
        assert_eq!(
            cue_spans(&project, fixture.subtitle_track_id),
            vec![
                (500, 1_000, "First cue".to_string()),
                (1_000, 2_500, "Second cue".to_string()),
                (3_000, 3_600, "<i>Third</i> cue".to_string()),
            ]
        );
        assert!(matches!(
            apply_expecting_error(
                &fixture.project,
                RemoveClipParts::new(fixture.cue_ids[0], vec![range(600, 700)])
            ),
            EditError::PartsOfSubtitleCue { .. }
        ));
    }

    #[test]
    fn removing_time_drops_the_cues_inside_it() {
        let fixture = fixture();
        let range = |start, end| TimeRange::new(milliseconds(start), milliseconds(end)).unwrap();
        let (project, _) = apply_and_check_revert(
            &fixture.project,
            RemoveTimeFromSubtitleTracks::new(vec![range(1_800, 4_200), range(2_000, 3_000)]),
        );
        assert_eq!(
            cue_spans(&project, fixture.subtitle_track_id),
            vec![
                (500, 1_500, "First cue".to_string()),
                (2_100, 3_100, "<i>Third</i> cue".to_string()),
            ]
        );
        // Nothing to move: nothing changes, and it still reverts.
        let (unchanged, _) = apply_and_check_revert(
            &fixture.project,
            RemoveTimeFromSubtitleTracks::new(vec![range(8_000, 9_000)]),
        );
        assert_eq!(unchanged, fixture.project);
    }
}
