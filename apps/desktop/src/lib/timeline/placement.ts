// The rules of dragging on the timeline, as pure functions: snapping, overlap
// checks and trimming limits. The engine refuses invalid edits anyway; these
// rules let the interface show the result (ghost, guide line, red refusal)
// during the gesture, before anything is sent.

import type { ClipView, TrackView } from "../api/types";

/** Snapping reaches this far on screen, whatever the zoom (docs/ux.md). */
export const SNAP_DISTANCE_IN_PIXELS = 8;

/** A time a dragged edge can stick to. */
export interface SnapPoint {
  ticks: number;
}

/** Every clip edge (except those of `ignoredClipIds`), the playhead and the start. */
export function collectSnapPoints(
  tracks: TrackView[],
  playheadTicks: number,
  ignoredClipIds: ReadonlySet<number>,
): SnapPoint[] {
  const points: SnapPoint[] = [{ ticks: 0 }, { ticks: playheadTicks }];
  for (const track of tracks) {
    for (const clip of track.clips) {
      if (!ignoredClipIds.has(clip.id)) {
        points.push({ ticks: clip.startTicks }, { ticks: clip.startTicks + clip.durationTicks });
      }
    }
  }
  return points;
}

export interface SnapResult {
  /** The time after snapping. */
  ticks: number;
  /** The snap point used, or `null` when nothing was close enough. */
  snappedTo: number | null;
}

/**
 * Moves `edges` (one or more times that move together, like a clip's start and
 * end) by the smallest shift that puts one of them on a snap point within
 * `thresholdTicks`. Returns the shift to apply and the point used.
 */
export function snapEdges(
  edges: number[],
  points: SnapPoint[],
  thresholdTicks: number,
): { shiftTicks: number; snappedTo: number | null } {
  let bestShift = 0;
  let bestDistance = Number.POSITIVE_INFINITY;
  let snappedTo: number | null = null;
  for (const edge of edges) {
    for (const point of points) {
      const distance = Math.abs(point.ticks - edge);
      if (distance <= thresholdTicks && distance < bestDistance) {
        bestDistance = distance;
        bestShift = point.ticks - edge;
        snappedTo = point.ticks;
      }
    }
  }
  return { shiftTicks: bestShift, snappedTo };
}

/** The first clip of `track` (other than `ignoredClipId`) overlapping `[start, start + duration)`. */
export function findOverlappingClip(
  track: TrackView,
  startTicks: number,
  durationTicks: number,
  ignoredClipIds: ReadonlySet<number> = new Set(),
): ClipView | null {
  const endTicks = startTicks + durationTicks;
  return (
    track.clips.find(
      (clip) =>
        !ignoredClipIds.has(clip.id) &&
        clip.startTicks < endTicks &&
        startTicks < clip.startTicks + clip.durationTicks,
    ) ?? null
  );
}

/** Which tracks a clip can go on: same kind as its own track (videos, pictures and titles on video tracks). */
export function trackAcceptsClip(track: TrackView, clipKind: ClipView["kind"]): boolean {
  return clipKind === "audio" ? track.kind === "audio" : track.kind === "video";
}

/** Limits of a trim gesture on one edge of a clip, in timeline ticks. */
export interface TrimLimits {
  earliestTicks: number;
  latestTicks: number;
}

/**
 * How far an edge of `clip` can go: not past the other edge (keeping at least
 * one frame), not beyond the media file (for media clips), and not into the
 * neighbouring clips of `track`.
 */
export function trimLimits(
  clip: ClipView,
  edge: "start" | "end",
  track: TrackView,
  minimumDurationTicks: number,
): TrimLimits {
  const clipEnd = clip.startTicks + clip.durationTicks;
  const previousEnd = Math.max(
    0,
    ...track.clips
      .filter((other) => other.id !== clip.id && other.startTicks + other.durationTicks <= clip.startTicks)
      .map((other) => other.startTicks + other.durationTicks),
  );
  const nextStart = Math.min(
    Number.POSITIVE_INFINITY,
    ...track.clips.filter((other) => other.id !== clip.id && other.startTicks >= clipEnd).map((other) => other.startTicks),
  );
  const isStill = clip.kind === "image" || clip.kind === "text";
  if (edge === "start") {
    // Source time available before the clip's current start, at the clip's speed.
    const roomInSource = isStill ? Number.POSITIVE_INFINITY : (clip.sourceStartTicks ?? 0) / clip.speed;
    return {
      earliestTicks: Math.max(previousEnd, clip.startTicks - roomInSource),
      latestTicks: clipEnd - minimumDurationTicks,
    };
  }
  const roomAfter =
    isStill || clip.sourceDurationTicks === null || clip.sourceEndTicks === null
      ? Number.POSITIVE_INFINITY
      : (clip.sourceDurationTicks - clip.sourceEndTicks) / clip.speed;
  return {
    earliestTicks: clip.startTicks + minimumDurationTicks,
    latestTicks: Math.min(nextStart, clipEnd + roomAfter),
  };
}

/** The cut times of the edit (clip starts and ends), sorted, for ↑ and ↓. */
export function cutTimes(tracks: TrackView[]): number[] {
  const times = new Set<number>([0]);
  for (const track of tracks) {
    for (const clip of track.clips) {
      times.add(clip.startTicks);
      times.add(clip.startTicks + clip.durationTicks);
    }
  }
  return [...times].sort((first, second) => first - second);
}
