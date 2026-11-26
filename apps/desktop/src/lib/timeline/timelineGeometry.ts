// Converting between timeline time (ticks) and screen pixels, and choosing ruler
// marks that stay readable at every zoom.

import { TICKS_PER_SECOND } from "../time/ticks";

/** Zoom limits, in pixels per second: about 4 h on a screen, down to single frames. */
export const SMALLEST_PIXELS_PER_SECOND = 0.1;
export const LARGEST_PIXELS_PER_SECOND = 1_200;
export const DEFAULT_PIXELS_PER_SECOND = 48;
/** Ruler labels need about this much room. */
const MINIMUM_LABEL_SPACING_IN_PIXELS = 72;
/** Steps between ruler labels, in seconds, from fine to coarse. */
const RULER_STEPS_IN_SECONDS = [
  0.1, 0.2, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1_800, 3_600, 7_200,
];

export function ticksToPixels(ticks: number, pixelsPerSecond: number): number {
  return (ticks / TICKS_PER_SECOND) * pixelsPerSecond;
}

export function pixelsToTicks(pixels: number, pixelsPerSecond: number): number {
  return (pixels / pixelsPerSecond) * TICKS_PER_SECOND;
}

export function clampZoom(pixelsPerSecond: number): number {
  return Math.min(LARGEST_PIXELS_PER_SECOND, Math.max(SMALLEST_PIXELS_PER_SECOND, pixelsPerSecond));
}

/** The zoom that shows `durationTicks` in `widthInPixels` (with a little room after). */
export function zoomToFit(durationTicks: number, widthInPixels: number): number {
  if (durationTicks <= 0 || widthInPixels <= 0) {
    return DEFAULT_PIXELS_PER_SECOND;
  }
  const FIT_MARGIN = 0.92;
  return clampZoom((widthInPixels * FIT_MARGIN) / (durationTicks / TICKS_PER_SECOND));
}

/** Seconds between labelled ruler marks at this zoom. */
export function rulerStepInSeconds(pixelsPerSecond: number): number {
  for (const step of RULER_STEPS_IN_SECONDS) {
    if (step * pixelsPerSecond >= MINIMUM_LABEL_SPACING_IN_PIXELS) {
      return step;
    }
  }
  return RULER_STEPS_IN_SECONDS[RULER_STEPS_IN_SECONDS.length - 1] ?? 3_600;
}

/** A ruler label: `0:05`, `1:30`, `1:00:00`, or `0:01.5` for steps under a second. */
export function formatRulerLabel(seconds: number, stepInSeconds: number): string {
  const wholeSeconds = Math.floor(seconds + 1e-6);
  const hours = Math.floor(wholeSeconds / 3600);
  const minutes = Math.floor((wholeSeconds % 3600) / 60);
  const secondsText = String(wholeSeconds % 60).padStart(2, "0");
  const fraction = stepInSeconds < 1 ? `.${Math.round((seconds - wholeSeconds) * 10) % 10}` : "";
  if (hours > 0) {
    return `${hours}:${String(minutes).padStart(2, "0")}:${secondsText}${fraction}`;
  }
  return `${minutes}:${secondsText}${fraction}`;
}
