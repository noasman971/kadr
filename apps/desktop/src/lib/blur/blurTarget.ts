// Which clip the blur tool works on (docs/ux.md, "Blurring a face"): the selected
// video clip under the playhead, or else the top video clip there.

import type { ClipView, ProjectView } from "../api/types";

/** Whether `clip` shows a video at `timeTicks`. */
export function showsVideoAt(clip: ClipView, timeTicks: number): boolean {
  return clip.kind === "video" && clip.startTicks <= timeTicks && timeTicks < clip.startTicks + clip.durationTicks;
}

/**
 * The clip to blur at `timeTicks`: a selected video clip showing then, or else
 * the video clip of the top visible track showing then; `null` when no video is
 * on screen.
 */
export function blurTargetClip(view: ProjectView, selectedClipIds: number[], timeTicks: number): ClipView | null {
  const candidates = view.tracks
    .filter((track) => track.kind === "video" && !track.isHidden)
    .flatMap((track) => track.clips)
    .filter((clip) => showsVideoAt(clip, timeTicks));
  const selected = candidates.find((clip) => selectedClipIds.includes(clip.id));
  // Tracks come in display order: the first video track is the top layer.
  return selected ?? candidates[0] ?? null;
}

/** The clip with id `clipId`, if it still exists. */
export function findClip(view: ProjectView, clipId: number): ClipView | undefined {
  return view.tracks.flatMap((track) => track.clips).find((clip) => clip.id === clipId);
}
