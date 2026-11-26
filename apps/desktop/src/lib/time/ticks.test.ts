import { describe, expect, it } from "vitest";

import {
  TICKS_PER_SECOND,
  formatShortDuration,
  formatTimecode,
  frameIndexAt,
  snapToFrame,
  ticksOfFrame,
  ticksPerFrame,
} from "./ticks";

const FPS_30 = { numerator: 30, denominator: 1 };
const FPS_29_97 = { numerator: 30_000, denominator: 1_001 };

describe("frames", () => {
  it("are whole numbers of ticks at common rates", () => {
    expect(ticksPerFrame(FPS_30)).toBe(23_520_000);
    expect(Number.isInteger(ticksPerFrame(FPS_29_97))).toBe(true);
  });

  it("are found from any time inside them", () => {
    expect(frameIndexAt(0, FPS_30)).toBe(0);
    expect(frameIndexAt(23_519_999, FPS_30)).toBe(0);
    expect(frameIndexAt(23_520_000, FPS_30)).toBe(1);
    expect(frameIndexAt(ticksOfFrame(1234, FPS_29_97), FPS_29_97)).toBe(1234);
  });

  it("snap times to their frame's start", () => {
    expect(snapToFrame(23_520_000 * 7 + 5, FPS_30)).toBe(23_520_000 * 7);
  });
});

describe("timecodes", () => {
  it("show minutes, seconds and frames", () => {
    expect(formatTimecode(0, FPS_30)).toBe("00:00:00");
    expect(formatTimecode(ticksOfFrame(7 * 30 + 12, FPS_30), FPS_30)).toBe("00:07:12");
    expect(formatTimecode(TICKS_PER_SECOND * 95, FPS_30)).toBe("01:35:00");
  });

  it("show hours from one hour", () => {
    expect(formatTimecode(TICKS_PER_SECOND * 3_725, FPS_30)).toBe("1:02:05:00");
  });

  it("never show a frame number past the rate at 29.97 fps", () => {
    for (let frameIndex = 0; frameIndex < 400; frameIndex += 1) {
      const frames = Number(formatTimecode(ticksOfFrame(frameIndex, FPS_29_97), FPS_29_97).split(":")[2]);
      expect(frames).toBeLessThan(30);
    }
  });
});

describe("short durations", () => {
  it("round to the second", () => {
    expect(formatShortDuration(TICKS_PER_SECOND * 12.4)).toBe("0:12");
    expect(formatShortDuration(TICKS_PER_SECOND * 151)).toBe("2:31");
    expect(formatShortDuration(TICKS_PER_SECOND * 3_725)).toBe("1:02:05");
  });
});
