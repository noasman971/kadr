import { describe, expect, it } from "vitest";

import type { ClipView, TrackView } from "../api/types";
import { collectSnapPoints, cutTimes, findOverlappingClip, snapEdges, trackAcceptsClip, trimLimits } from "./placement";

const SECOND = 705_600_000;
const FRAME = SECOND / 30;

function clip(id: number, startSeconds: number, durationSeconds: number, overrides: Partial<ClipView> = {}): ClipView {
  const property = { value: 1, keyframeCount: 0 };
  return {
    id,
    trackId: 1,
    kind: "video",
    name: `clip ${id}`,
    startTicks: startSeconds * SECOND,
    durationTicks: durationSeconds * SECOND,
    mediaId: 1,
    sourceStartTicks: 0,
    sourceEndTicks: durationSeconds * SECOND,
    sourceDurationTicks: 60 * SECOND,
    speed: 1,
    hasPicture: true,
    hasSound: true,
    volume: property,
    opacity: property,
    positionX: { value: 0, keyframeCount: 0 },
    positionY: { value: 0, keyframeCount: 0 },
    scale: property,
    rotation: { value: 0, keyframeCount: 0 },
    crop: { left: 0, top: 0, right: 0, bottom: 0 },
    text: null,
    colorAdjustment: null,
    colorLut: null,
    trackedBlurs: [],
    effectCount: 0,
    ...overrides,
  };
}

function track(clips: ClipView[], kind: TrackView["kind"] = "video"): TrackView {
  return { id: 1, kind, name: "Video 1", isMuted: false, isHidden: false, clips };
}

describe("snapping", () => {
  const points = collectSnapPoints([track([clip(1, 0, 4), clip(2, 6, 2)])], 5 * SECOND, new Set([2]));

  it("collects clip edges, the playhead and the start, except ignored clips", () => {
    expect(points.map((point) => point.ticks / SECOND).sort((a, b) => a - b)).toEqual([0, 0, 4, 5]);
  });

  it("picks the closest point for any moving edge", () => {
    // A 2-second clip dragged to 3.9 s: its start is 0.1 s from 4 s.
    const result = snapEdges([3.9 * SECOND, 5.9 * SECOND], points, 0.2 * SECOND);
    expect(result.snappedTo).toBe(4 * SECOND);
    expect(result.shiftTicks).toBeCloseTo(0.1 * SECOND);
  });

  it("leaves edges alone when nothing is close", () => {
    expect(snapEdges([2 * SECOND], points, 0.2 * SECOND)).toEqual({ shiftTicks: 0, snappedTo: null });
  });
});

describe("overlaps", () => {
  const lane = track([clip(1, 0, 4), clip(2, 6, 2)]);

  it("finds the clip in the way", () => {
    expect(findOverlappingClip(lane, 3 * SECOND, 2 * SECOND)?.id).toBe(1);
    expect(findOverlappingClip(lane, 4 * SECOND, 2 * SECOND)).toBeNull();
  });

  it("ignores the dragged clip itself", () => {
    expect(findOverlappingClip(lane, 1 * SECOND, 2 * SECOND, new Set([1]))).toBeNull();
  });

  it("keeps sounds on audio tracks and pictures on video tracks", () => {
    expect(trackAcceptsClip(track([], "audio"), "audio")).toBe(true);
    expect(trackAcceptsClip(track([], "audio"), "text")).toBe(false);
    expect(trackAcceptsClip(track([], "video"), "image")).toBe(true);
  });
});

describe("trimming", () => {
  it("stops at the file's start and end, at the speed of the clip", () => {
    const trimmed = clip(1, 10, 4, { sourceStartTicks: 2 * SECOND, sourceEndTicks: 10 * SECOND, speed: 2 });
    const lane = track([trimmed]);
    expect(trimLimits(trimmed, "start", lane, FRAME).earliestTicks).toBe(9 * SECOND);
    // 50 s of file left after the end, played at 2×: 25 s of timeline.
    expect(trimLimits(trimmed, "end", lane, FRAME).latestTicks).toBe(39 * SECOND);
  });

  it("stops at the neighbours and keeps one frame", () => {
    const middle = clip(2, 5, 2, { sourceStartTicks: 30 * SECOND, sourceEndTicks: 32 * SECOND });
    const lane = track([clip(1, 0, 4), middle, clip(3, 8, 1)]);
    expect(trimLimits(middle, "start", lane, FRAME)).toEqual({ earliestTicks: 4 * SECOND, latestTicks: 7 * SECOND - FRAME });
    expect(trimLimits(middle, "end", lane, FRAME)).toEqual({ earliestTicks: 5 * SECOND + FRAME, latestTicks: 8 * SECOND });
  });

  it("lets pictures and titles grow freely", () => {
    const picture = clip(1, 3, 5, { kind: "image", sourceStartTicks: 0 });
    expect(trimLimits(picture, "start", track([picture]), FRAME).earliestTicks).toBe(0);
    expect(trimLimits(picture, "end", track([picture]), FRAME).latestTicks).toBe(Number.POSITIVE_INFINITY);
  });
});

describe("cut times", () => {
  it("lists every start and end once, in order", () => {
    expect(cutTimes([track([clip(1, 0, 4), clip(2, 4, 2)]), track([clip(3, 1, 2)], "audio")]).map((ticks) => ticks / SECOND)).toEqual([
      0, 1, 3, 4, 6,
    ]);
  });
});
