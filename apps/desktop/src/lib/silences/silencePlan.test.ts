import { describe, expect, it } from "vitest";

import { DEFAULT_SILENCE_SEARCH_SETTINGS, clampedSetting, clipStretches } from "./silencePlan";

describe("the strip of a clip", () => {
  it("alternates kept and cut stretches that cover the whole clip", () => {
    // A clip from 100 to 1100 ticks, cuts 300–400 and 900–1100 (given out of order).
    const stretches = clipStretches(100, 1000, [
      { startTicks: 900, endTicks: 1100 },
      { startTicks: 300, endTicks: 400 },
    ]);
    const expected = [
      { isCut: false, startFraction: 0, widthFraction: 0.2 },
      { isCut: true, startFraction: 0.2, widthFraction: 0.1 },
      { isCut: false, startFraction: 0.3, widthFraction: 0.5 },
      { isCut: true, startFraction: 0.8, widthFraction: 0.2 },
    ];
    expect(stretches).toHaveLength(expected.length);
    stretches.forEach((stretch, index) => {
      const wanted = expected[index];
      expect(stretch.isCut).toBe(wanted?.isCut);
      expect(stretch.startFraction).toBeCloseTo(wanted?.startFraction ?? Number.NaN);
      expect(stretch.widthFraction).toBeCloseTo(wanted?.widthFraction ?? Number.NaN);
    });
  });

  it("clips cuts to the clip and merges overlapping ones", () => {
    const stretches = clipStretches(0, 100, [
      { startTicks: -50, endTicks: 10 },
      { startTicks: 40, endTicks: 60 },
      { startTicks: 50, endTicks: 70 },
    ]);
    expect(stretches.map((stretch) => stretch.isCut)).toEqual([true, false, true, false]);
    expect(stretches[2]?.startFraction).toBeCloseTo(0.4);
    expect(stretches[2]?.widthFraction).toBeCloseTo(0.3);
  });

  it("is empty for an empty clip and whole without cuts", () => {
    expect(clipStretches(0, 0, [])).toEqual([]);
    expect(clipStretches(5, 10, [])).toEqual([{ isCut: false, startFraction: 0, widthFraction: 1 }]);
  });
});

describe("settings", () => {
  it("are kept inside the dialog's limits", () => {
    expect(clampedSetting("thresholdInDecibels", -120)).toBe(-80);
    expect(clampedSetting("keptPauseInSeconds", 3)).toBe(2);
    expect(clampedSetting("minimumSilenceInSeconds", Number.NaN)).toBe(
      DEFAULT_SILENCE_SEARCH_SETTINGS.minimumSilenceInSeconds,
    );
  });
});
