// The moments to check (docs/ux.md, "Check and correct"): the stretches where a
// blur's tracker was unsure, across the edit, and the next one to look at.

import type { ProjectView, UnsureRangeView } from "../api/types";

export interface UnsureMoment {
  clipId: number;
  effectId: number;
  startTicks: number;
  /** Excluded. */
  endTicks: number;
  /** Where to put the playhead to look at it. */
  middleTicks: number;
  confidence: UnsureRangeView["confidence"];
}

/** Every moment to check of the edit (of one clip when `clipId` is given), in time order. */
export function unsureMoments(view: ProjectView, clipId?: number): UnsureMoment[] {
  return view.tracks
    .flatMap((track) => track.clips)
    .filter((clip) => clipId === undefined || clip.id === clipId)
    .flatMap((clip) =>
      clip.trackedBlurs
        .filter((blur) => blur.isEnabled)
        .flatMap((blur) =>
          blur.unsureRanges.map((range) => ({
            clipId: clip.id,
            effectId: blur.effectId,
            startTicks: range.startTicks,
            endTicks: range.endTicks,
            middleTicks: Math.floor((range.startTicks + range.endTicks) / 2),
            confidence: range.confidence,
          })),
        ),
    )
    .sort((first, second) => first.startTicks - second.startTicks || first.clipId - second.clipId);
}

/**
 * The moment to look at after `playheadTicks`: the first one starting after
 * it, or the first of all when none is left (the search goes round). A moment
 * the playhead is already in counts as seen, so pressing again moves on.
 */
export function nextUnsureMoment(view: ProjectView, playheadTicks: number, clipId?: number): UnsureMoment | null {
  const moments = unsureMoments(view, clipId);
  return moments.find((moment) => moment.startTicks > playheadTicks) ?? moments[0] ?? null;
}
