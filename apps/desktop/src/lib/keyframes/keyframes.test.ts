import { describe, expect, it } from "vitest";

import type { KeyframeCurve, PropertyView } from "../api/types";
import { TICKS_PER_SECOND } from "../time/ticks";
import {
  bezierHandlesOf,
  clampedHandles,
  keyframeAt,
  keyframeOffsetsOf,
  nextKeyframeOffset,
  previousKeyframeOffset,
  progressAlong,
  stretchStartAt,
  valueAt,
} from "./keyframes";

const SECOND = TICKS_PER_SECOND;

/** From 0 at 0 s to 100 at 10 s with `curve`. */
function ramp(curve: KeyframeCurve): PropertyView {
  return {
    value: 0,
    keyframes: [
      { offsetTicks: 0, value: 0, curve },
      { offsetTicks: 10 * SECOND, value: 100, curve: { type: "linear" } },
    ],
  };
}

describe("the value at the playhead", () => {
  it("is the constant value without keyframes", () => {
    expect(valueAt({ value: 0.4, keyframes: [] }, 5 * SECOND)).toBe(0.4);
  });

  // The same numbers as the engine's tests (kadr-core, animation.rs).
  it("follows the preset curves like the engine", () => {
    expect(valueAt(ramp({ type: "linear" }), 3 * SECOND)).toBe(30);
    expect(valueAt(ramp({ type: "easeIn" }), 5 * SECOND)).toBe(25);
    expect(valueAt(ramp({ type: "easeOut" }), 5 * SECOND)).toBe(75);
    expect(valueAt(ramp({ type: "easeInOut" }), 5 * SECOND)).toBe(50);
    expect(valueAt(ramp({ type: "hold" }), 9 * SECOND)).toBe(0);
  });

  it("solves a Bézier curve like the engine", () => {
    // CSS ease-out: x(t) = 0.25 at t ≈ 0.41803, where y ≈ 0.37810.
    const cssEaseOut = ramp({ type: "bezier", x1: 0, y1: 0, x2: 0.58, y2: 1 });
    expect(valueAt(cssEaseOut, 2.5 * SECOND)).toBeCloseTo(37.81, 2);
  });

  it("holds the first and last values outside the keyframes", () => {
    const property: PropertyView = {
      value: 0,
      keyframes: [
        { offsetTicks: SECOND, value: 5, curve: { type: "linear" } },
        { offsetTicks: 2 * SECOND, value: 9, curve: { type: "linear" } },
      ],
    };
    expect(valueAt(property, 0)).toBe(5);
    expect(valueAt(property, 3 * SECOND)).toBe(9);
  });
});

describe("curves", () => {
  it("draw each preset as the Bézier curve of the same shape", () => {
    for (const type of ["linear", "easeIn", "easeOut", "easeInOut"] as const) {
      const handles = bezierHandlesOf({ type });
      expect(handles).not.toBeNull();
      const asBezier: KeyframeCurve = { type: "bezier", ...handles! };
      for (let step = 0; step <= 20; step += 1) {
        const progress = step / 20;
        expect(progressAlong(asBezier, progress)).toBeCloseTo(progressAlong({ type }, progress), 4);
      }
    }
    expect(bezierHandlesOf({ type: "hold" })).toBeNull();
  });

  it("keep their handles inside the unit square", () => {
    expect(clampedHandles({ x1: -0.2, y1: 1.4, x2: 0.33333, y2: 0.5 })).toEqual({ x1: 0, y1: 1, x2: 0.333, y2: 0.5 });
  });
});

describe("finding keyframes", () => {
  const property: PropertyView = {
    value: 0,
    keyframes: [
      { offsetTicks: 0, value: 0, curve: { type: "easeOut" } },
      { offsetTicks: SECOND, value: 1, curve: { type: "hold" } },
      { offsetTicks: 2 * SECOND, value: 2, curve: { type: "linear" } },
    ],
  };

  it("finds the stretch the playhead is in", () => {
    expect(stretchStartAt(property, SECOND / 2)?.curve.type).toBe("easeOut");
    expect(stretchStartAt(property, SECOND)?.curve.type).toBe("hold");
    // From the last keyframe on, there is no stretch to shape.
    expect(stretchStartAt(property, 2 * SECOND)).toBeUndefined();
    expect(stretchStartAt({ value: 0, keyframes: [] }, 0)).toBeUndefined();
  });

  it("finds the keyframe under the playhead and its neighbours", () => {
    expect(keyframeAt(property, SECOND)?.value).toBe(1);
    expect(keyframeAt(property, SECOND + 1)).toBeUndefined();
    const offsets = keyframeOffsetsOf([property, { value: 0, keyframes: [{ offsetTicks: SECOND / 2, value: 0, curve: { type: "linear" } }] }]);
    expect(offsets).toEqual([0, SECOND / 2, SECOND, 2 * SECOND]);
    expect(previousKeyframeOffset(offsets, SECOND)).toBe(SECOND / 2);
    expect(nextKeyframeOffset(offsets, SECOND)).toBe(2 * SECOND);
    expect(previousKeyframeOffset(offsets, 0)).toBeUndefined();
  });
});
