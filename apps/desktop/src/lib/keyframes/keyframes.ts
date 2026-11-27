// Keyframes in the front end: the value an animated setting has at the playhead,
// the curves between keyframes, and where keyframes sit (docs/ux.md, "Animate a
// clip").
//
// The preview and the export are computed by the engine (`kadr-core`'s
// `AnimatedValue::value_at`); this module repeats the same arithmetic only to
// show numbers in the inspector and to draw curves. Keep both in step.

import type { KeyframeCurve, KeyframeView, PropertyView } from "../api/types";

/** The handles of a cubic Bézier curve, as in CSS `cubic-bezier(x1, y1, x2, y2)`. */
export interface BezierHandles {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
}

/** The curves offered by name, in menu order. */
export const PRESET_CURVES = ["linear", "easeIn", "easeOut", "easeInOut", "hold"] as const;
export type PresetCurveName = (typeof PRESET_CURVES)[number];
export type CurveName = KeyframeCurve["type"];

const ONE_THIRD = 1 / 3;
const TWO_THIRDS = 2 / 3;

/**
 * The cubic Bézier curve drawing the same shape as `curve`, or `null` for a hold
 * (a jump is not a curve). The presets are Bézier curves whose handles sit at a
 * third and two thirds of the time (`Interpolation::as_bezier_curve` in the
 * engine), so the curve editor starts from them without changing the animation.
 */
export function bezierHandlesOf(curve: KeyframeCurve): BezierHandles | null {
  switch (curve.type) {
    case "linear":
      return { x1: ONE_THIRD, y1: ONE_THIRD, x2: TWO_THIRDS, y2: TWO_THIRDS };
    case "easeIn":
      return { x1: ONE_THIRD, y1: 0, x2: TWO_THIRDS, y2: ONE_THIRD };
    case "easeOut":
      return { x1: ONE_THIRD, y1: TWO_THIRDS, x2: TWO_THIRDS, y2: 1 };
    case "easeInOut":
      return { x1: ONE_THIRD, y1: 0, x2: TWO_THIRDS, y2: 1 };
    case "hold":
      return null;
    case "bezier":
      return { x1: curve.x1, y1: curve.y1, x2: curve.x2, y2: curve.y2 };
  }
}

/** Handles kept inside the unit square, as the engine requires, and rounded to 1/1000. */
export function clampedHandles(handles: BezierHandles): BezierHandles {
  const clamp = (fraction: number) => Math.round(Math.min(1, Math.max(0, fraction)) * 1000) / 1000;
  return { x1: clamp(handles.x1), y1: clamp(handles.y1), x2: clamp(handles.x2), y2: clamp(handles.y2) };
}

function bezierCoordinate(first: number, second: number, t: number): number {
  const rest = 1 - t;
  return 3 * rest * rest * t * first + 3 * rest * t * t * second + t * t * t;
}

function bezierSlope(first: number, second: number, t: number): number {
  const rest = 1 - t;
  return 3 * rest * rest * first + 6 * rest * t * (second - first) + 3 * t * t * (1 - second);
}

const TIME_TOLERANCE = 1e-9;

/** The curve parameter where x is `target`: Newton's method, then bisection (as the engine). */
function parameterAt(x1: number, x2: number, target: number): number {
  let parameter = target;
  for (let step = 0; step < 8; step += 1) {
    const error = bezierCoordinate(x1, x2, parameter) - target;
    if (Math.abs(error) < TIME_TOLERANCE) {
      return parameter;
    }
    const slope = bezierSlope(x1, x2, parameter);
    if (Math.abs(slope) < TIME_TOLERANCE) {
      break;
    }
    parameter -= error / slope;
    if (parameter < 0 || parameter > 1) {
      break;
    }
  }
  let low = 0;
  let high = 1;
  parameter = target;
  for (let step = 0; step < 64; step += 1) {
    const error = bezierCoordinate(x1, x2, parameter) - target;
    if (Math.abs(error) < TIME_TOLERANCE) {
      break;
    }
    if (error < 0) {
      low = parameter;
    } else {
      high = parameter;
    }
    parameter = (low + high) / 2;
  }
  return parameter;
}

/** The share of the value change done once `timeProgress` (0 to 1) of the time has passed. */
export function progressAlong(curve: KeyframeCurve, timeProgress: number): number {
  const progress = Math.min(1, Math.max(0, timeProgress));
  let eased: number;
  switch (curve.type) {
    case "linear":
      eased = progress;
      break;
    case "easeIn":
      eased = progress * progress;
      break;
    case "easeOut":
      eased = 1 - (1 - progress) * (1 - progress);
      break;
    case "easeInOut":
      eased = progress * progress * (3 - 2 * progress);
      break;
    case "hold":
      eased = 0;
      break;
    case "bezier":
      eased = bezierCoordinate(curve.y1, curve.y2, parameterAt(curve.x1, curve.x2, progress));
      break;
  }
  return Math.min(1, Math.max(0, eased));
}

/** The value of `property` at `offsetTicks` from the clip's start. */
export function valueAt(property: PropertyView, offsetTicks: number): number {
  const keyframes = property.keyframes;
  const first = keyframes[0];
  const last = keyframes[keyframes.length - 1];
  if (!first || !last) {
    return property.value;
  }
  if (offsetTicks <= first.offsetTicks) {
    return first.value;
  }
  if (offsetTicks >= last.offsetTicks) {
    return last.value;
  }
  const nextIndex = keyframes.findIndex((keyframe) => keyframe.offsetTicks > offsetTicks);
  const previous = keyframes[nextIndex - 1];
  const next = keyframes[nextIndex];
  if (!previous || !next) {
    return last.value;
  }
  const timeProgress = (offsetTicks - previous.offsetTicks) / (next.offsetTicks - previous.offsetTicks);
  return previous.value + (next.value - previous.value) * progressAlong(previous.curve, timeProgress);
}

/** The keyframe exactly at `offsetTicks`, if there is one. */
export function keyframeAt(property: PropertyView, offsetTicks: number): KeyframeView | undefined {
  return property.keyframes.find((keyframe) => keyframe.offsetTicks === offsetTicks);
}

/**
 * The keyframe starting the stretch the playhead is in: the last one at or
 * before `offsetTicks`, if another one follows it. Its curve is the one the
 * inspector shows and changes. `undefined` before the first keyframe and from
 * the last one on.
 */
export function stretchStartAt(property: PropertyView, offsetTicks: number): KeyframeView | undefined {
  const followingIndex = property.keyframes.findIndex((keyframe) => keyframe.offsetTicks > offsetTicks);
  // The stretch needs a keyframe at or before the playhead and one after it.
  return followingIndex > 0 ? property.keyframes[followingIndex - 1] : undefined;
}

/** The closest keyframe time before `offsetTicks`, for "previous keyframe". */
export function previousKeyframeOffset(offsets: number[], offsetTicks: number): number | undefined {
  return offsets.filter((offset) => offset < offsetTicks).at(-1);
}

/** The closest keyframe time after `offsetTicks`, for "next keyframe". */
export function nextKeyframeOffset(offsets: number[], offsetTicks: number): number | undefined {
  return offsets.find((offset) => offset > offsetTicks);
}

/** Every moment with a keyframe of any of `properties`, sorted, each once. */
export function keyframeOffsetsOf(properties: PropertyView[]): number[] {
  const offsets = new Set(properties.flatMap((property) => property.keyframes.map((keyframe) => keyframe.offsetTicks)));
  return [...offsets].sort((first, second) => first - second);
}

/** The curve's name: a preset's own, or "bezier". */
export function curveName(curve: KeyframeCurve): CurveName {
  return curve.type;
}

/** A preset curve from its name. */
export function presetCurve(name: PresetCurveName): KeyframeCurve {
  return { type: name };
}
