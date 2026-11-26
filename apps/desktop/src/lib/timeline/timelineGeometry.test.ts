import { describe, expect, it } from "vitest";

import { TICKS_PER_SECOND } from "../time/ticks";
import {
  LARGEST_PIXELS_PER_SECOND,
  formatRulerLabel,
  pixelsToTicks,
  rulerStepInSeconds,
  ticksToPixels,
  zoomToFit,
} from "./timelineGeometry";

describe("time and pixels", () => {
  it("convert both ways", () => {
    expect(ticksToPixels(TICKS_PER_SECOND * 2, 48)).toBe(96);
    expect(pixelsToTicks(96, 48)).toBe(TICKS_PER_SECOND * 2);
  });

  it("fit an edit in the visible width", () => {
    expect(zoomToFit(TICKS_PER_SECOND * 100, 1_000)).toBeCloseTo(9.2);
    expect(zoomToFit(1, 1_000)).toBe(LARGEST_PIXELS_PER_SECOND);
  });
});

describe("ruler", () => {
  it("keeps labels apart at every zoom", () => {
    for (const pixelsPerSecond of [0.1, 1, 10, 48, 200, 1_200]) {
      const step = rulerStepInSeconds(pixelsPerSecond);
      expect(step * pixelsPerSecond).toBeGreaterThanOrEqual(72);
    }
  });

  it("labels minutes, seconds and tenths", () => {
    expect(formatRulerLabel(5, 1)).toBe("0:05");
    expect(formatRulerLabel(90, 10)).toBe("1:30");
    expect(formatRulerLabel(3_600, 600)).toBe("1:00:00");
    expect(formatRulerLabel(1.5, 0.5)).toBe("0:01.5");
  });
});
