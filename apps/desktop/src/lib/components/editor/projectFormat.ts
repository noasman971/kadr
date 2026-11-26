// Describing a project format to people: "16:9", "30 fps".

import type { SettingsView } from "../../api/types";
import { translations } from "../../i18n/i18n.svelte";

/** Common ratios, recognised within 1 % so 1080 × 1920 and 1088 × 1920 both say 9:16. */
const NAMED_RATIOS: Array<[string, number]> = [
  ["16:9", 16 / 9],
  ["9:16", 9 / 16],
  ["1:1", 1],
  ["4:3", 4 / 3],
  ["3:4", 3 / 4],
  ["4:5", 4 / 5],
  ["21:9", 21 / 9],
];
const RATIO_TOLERANCE = 0.01;

export function aspectRatioName(width: number, height: number): string {
  const ratio = width / height;
  const named = NAMED_RATIOS.find(([, value]) => Math.abs(ratio - value) / value < RATIO_TOLERANCE);
  return named ? named[0] : `${translations.formatNumber(ratio, 2)}:1`;
}

/** 30, 25, 29,97 / 29.97… */
export function formatFrameRate(settings: SettingsView): string {
  return translations.formatNumber(settings.frameRateNumerator / settings.frameRateDenominator, 2);
}
