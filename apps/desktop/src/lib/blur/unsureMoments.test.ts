import { describe, expect, it } from "vitest";

import type { ClipView, ProjectView, TrackedBlurView } from "../api/types";
import { nextUnsureMoment, unsureMoments } from "./unsureMoments";

const SECOND = 705_600_000;

function blur(effectId: number, ranges: Array<[number, number]>, isEnabled = true): TrackedBlurView {
  return {
    effectId,
    isEnabled,
    settings: { style: "blur", strength: 30, maskColor: "#000000", shape: "ellipse", marginInPercent: 20 },
    correctionCount: 0,
    unsureRanges: ranges.map(([start, end]) => ({ startTicks: start * SECOND, endTicks: end * SECOND, confidence: "lost" })),
  };
}

function clip(id: number, trackedBlurs: TrackedBlurView[]): ClipView {
  const property = { value: 1, keyframeCount: 0 };
  return {
    id,
    trackId: 1,
    kind: "video",
    name: `clip ${id}`,
    startTicks: 0,
    durationTicks: 20 * SECOND,
    mediaId: 1,
    sourceStartTicks: 0,
    sourceEndTicks: 20 * SECOND,
    sourceDurationTicks: 20 * SECOND,
    speed: 1,
    hasPicture: true,
    hasSound: false,
    volume: property,
    opacity: property,
    positionX: { value: 0, keyframeCount: 0 },
    positionY: { value: 0, keyframeCount: 0 },
    scale: property,
    rotation: { value: 0, keyframeCount: 0 },
    crop: { left: 0, top: 0, right: 0, bottom: 0 },
    text: null,
    colorAdjustment: null,
    trackedBlurs,
    effectCount: trackedBlurs.length,
  };
}

function project(clips: ClipView[]): ProjectView {
  return {
    revision: 1,
    name: "p",
    filePath: "p.kadr",
    saveFailure: null,
    settings: {
      canvasWidth: 1920,
      canvasHeight: 1080,
      frameRateNumerator: 30,
      frameRateDenominator: 1,
      audioSampleRate: 48_000,
      backgroundColor: "#000000FF",
    },
    adoptsFirstVideoFormat: false,
    durationTicks: 20 * SECOND,
    media: [],
    tracks: [{ id: 1, kind: "video", name: "Video 1", isMuted: false, isHidden: false, clips }],
    undoLabel: null,
    redoLabel: null,
  };
}

describe("moments to check", () => {
  const view = project([
    clip(1, [blur(10, [[8, 9]]), blur(11, [[2, 4]]), blur(12, [[5, 6]], false)]),
    clip(2, [blur(20, [[3, 3.5]])]),
  ]);

  it("are listed in time order, without turned-off blurs", () => {
    expect(unsureMoments(view).map((moment) => [moment.clipId, moment.startTicks / SECOND])).toEqual([
      [1, 2],
      [2, 3],
      [1, 8],
    ]);
    expect(unsureMoments(view, 2)).toHaveLength(1);
    expect(unsureMoments(view)[0]?.middleTicks).toBe(3 * SECOND);
  });

  it("go on after the playhead, then round to the first", () => {
    expect(nextUnsureMoment(view, 0)?.startTicks).toBe(2 * SECOND);
    // Inside the first moment: the next one.
    expect(nextUnsureMoment(view, 3 * SECOND - 1)?.clipId).toBe(2);
    expect(nextUnsureMoment(view, 8.5 * SECOND)?.startTicks).toBe(2 * SECOND);
    expect(nextUnsureMoment(view, 0, 2)?.clipId).toBe(2);
    expect(nextUnsureMoment(project([clip(3, [])]), 0)).toBeNull();
  });
});
