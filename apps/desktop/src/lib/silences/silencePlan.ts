// The silence dialog's settings and its picture of the clip, as pure functions
// (tested in silencePlan.test.ts). The search itself runs in the back end
// (`silence_search.rs`, `kadr_engine::silence_removal`).

import type { SilenceSearchSettings, TickRange } from "../api/types";

/** The engine's defaults (`kadr_engine::silence_removal`). */
export const DEFAULT_SILENCE_SEARCH_SETTINGS: SilenceSearchSettings = {
  thresholdInDecibels: -40,
  minimumSilenceInSeconds: 0.5,
  keptPauseInSeconds: 0.2,
};

/** The ranges the dialog offers (narrower than the engine's, which also serve Claude). */
export const SILENCE_SETTING_LIMITS = {
  thresholdInDecibels: { min: -80, max: -10 },
  minimumSilenceInSeconds: { min: 0.1, max: 10 },
  keptPauseInSeconds: { min: 0, max: 2 },
} as const;

/** One stretch of the clip in the dialog's strip: kept or cut, as fractions of the clip's length. */
export interface ClipStretch {
  isCut: boolean;
  startFraction: number;
  widthFraction: number;
}

/**
 * The clip (`clipStartTicks`, `clipDurationTicks` long) as alternating kept and
 * cut stretches, for the strip that shows what will go. Cuts outside the clip
 * are clipped to it; overlapping cuts merge.
 */
export function clipStretches(clipStartTicks: number, clipDurationTicks: number, cuts: TickRange[]): ClipStretch[] {
  if (clipDurationTicks <= 0) {
    return [];
  }
  const clipEndTicks = clipStartTicks + clipDurationTicks;
  const fractionAt = (ticks: number) => (ticks - clipStartTicks) / clipDurationTicks;
  const stretch = (isCut: boolean, fromTicks: number, toTicks: number): ClipStretch => ({
    isCut,
    startFraction: fractionAt(fromTicks),
    widthFraction: fractionAt(toTicks) - fractionAt(fromTicks),
  });
  const stretches: ClipStretch[] = [];
  let cursor = clipStartTicks;
  for (const cut of [...cuts].sort((first, second) => first.startTicks - second.startTicks)) {
    const cutStart = Math.max(cut.startTicks, cursor);
    const cutEnd = Math.min(cut.endTicks, clipEndTicks);
    if (cutEnd <= cutStart) {
      continue;
    }
    const previous = stretches.at(-1);
    if (cutStart > cursor) {
      stretches.push(stretch(false, cursor, cutStart), stretch(true, cutStart, cutEnd));
    } else if (previous?.isCut) {
      // Touches or overlaps the previous cut: one longer cut.
      previous.widthFraction = fractionAt(cutEnd) - previous.startFraction;
    } else {
      stretches.push(stretch(true, cutStart, cutEnd));
    }
    cursor = cutEnd;
  }
  if (cursor < clipEndTicks) {
    stretches.push(stretch(false, cursor, clipEndTicks));
  }
  return stretches;
}

/** A setting kept inside the dialog's limits (a typed value can be anything). */
export function clampedSetting(name: keyof SilenceSearchSettings, value: number): number {
  const { min, max } = SILENCE_SETTING_LIMITS[name];
  if (!Number.isFinite(value)) {
    return DEFAULT_SILENCE_SEARCH_SETTINGS[name];
  }
  return Math.min(max, Math.max(min, value));
}
