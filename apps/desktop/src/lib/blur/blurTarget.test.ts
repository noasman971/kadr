import { describe, expect, it } from "vitest";

import type { ClipView, ProjectView, TrackView } from "../api/types";
import { blurTargetClip } from "./blurTarget";

const SECOND = 705_600_000;

function clip(id: number, kind: ClipView["kind"], startSeconds: number, durationSeconds: number): ClipView {
  const property = { value: 1, keyframeCount: 0 };
  return {
    id,
    trackId: 1,
    kind,
    name: `clip ${id}`,
    startTicks: startSeconds * SECOND,
    durationTicks: durationSeconds * SECOND,
    mediaId: 1,
    sourceStartTicks: 0,
    sourceEndTicks: durationSeconds * SECOND,
    sourceDurationTicks: durationSeconds * SECOND,
    speed: 1,
    hasPicture: kind !== "audio",
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
    trackedBlurs: [],
    effectCount: 0,
  };
}

function track(id: number, clips: ClipView[], overrides: Partial<TrackView> = {}): TrackView {
  return { id, kind: "video", name: `Video ${id}`, isMuted: false, isHidden: false, clips, ...overrides };
}

function project(tracks: TrackView[]): ProjectView {
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
    durationTicks: 10 * SECOND,
    media: [],
    tracks,
    undoLabel: null,
    redoLabel: null,
  };
}

describe("the clip to blur", () => {
  const view = project([
    track(2, [clip(20, "video", 0, 4), clip(21, "text", 4, 2)]),
    track(1, [clip(10, "video", 0, 10)]),
    track(3, [clip(30, "audio", 0, 10)], { kind: "audio" }),
  ]);

  it("is the top video under the playhead", () => {
    expect(blurTargetClip(view, [], 1 * SECOND)?.id).toBe(20);
    expect(blurTargetClip(view, [], 5 * SECOND)?.id).toBe(10);
  });

  it("is the selected video when it shows at the playhead", () => {
    expect(blurTargetClip(view, [10], 1 * SECOND)?.id).toBe(10);
    expect(blurTargetClip(view, [21], 5 * SECOND)?.id).toBe(10);
  });

  it("ignores hidden tracks, titles and sounds", () => {
    const hidden = project([track(2, [clip(20, "video", 0, 4)], { isHidden: true }), track(3, [clip(30, "audio", 0, 10)], { kind: "audio" })]);
    expect(blurTargetClip(hidden, [], 1 * SECOND)).toBeNull();
    expect(blurTargetClip(view, [], 11 * SECOND)).toBeNull();
  });
});
