import { describe, expect, it } from "vitest";

import { estimatedFileSizeInBytes, haveSameShape, pictureSizeOf, shapeOf } from "./exportSizes";

describe("export sizes", () => {
  const canvas = { width: 1280, height: 720 };

  it("uses the project's canvas for the project format", () => {
    expect(pictureSizeOf("project", canvas)).toEqual(canvas);
    expect(pictureSizeOf("youtube4k", canvas)).toEqual({ width: 3840, height: 2160 });
  });

  it("tells shapes apart exactly", () => {
    expect(haveSameShape(canvas, { width: 3840, height: 2160 })).toBe(true);
    expect(haveSameShape(canvas, { width: 1080, height: 1920 })).toBe(false);
    expect(shapeOf({ width: 1080, height: 1920 })).toBe("vertical");
    expect(shapeOf({ width: 1080, height: 1080 })).toBe("square");
    expect(shapeOf(canvas)).toBe("horizontal");
  });

  it("estimates about 18 MB for 24 s of 1080p at 30 fps", () => {
    const bytes = estimatedFileSizeInBytes({ width: 1920, height: 1080 }, 30, 24, "high");
    expect(bytes / 1_000_000).toBeGreaterThan(17);
    expect(bytes / 1_000_000).toBeLessThan(20);
    expect(estimatedFileSizeInBytes({ width: 1920, height: 1080 }, 30, 24, "fast")).toBeGreaterThan(bytes);
  });
});
