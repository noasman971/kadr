import { describe, expect, it } from "vitest";

import {
  applyMatrix,
  handlePosition,
  invertMatrix,
  isLargeEnough,
  movedRegion,
  regionBetween,
  regionCorners,
  resizedRegion,
  sourceToDisplayMatrix,
  type AffineMatrix,
} from "./sourceMapping";

const BOUNDS: [number, number, number, number] = [0, 0, 1920, 1080];

describe("source mapping", () => {
  it("goes from source to display and back", () => {
    // A 1920 × 1080 source fitted in a 1080 × 1920 canvas, rotated 30°, moved.
    const scale = 1080 / 1920;
    const angle = (30 * Math.PI) / 180;
    const sourceToCanvas: AffineMatrix = [
      scale * Math.cos(angle),
      scale * Math.sin(angle),
      -scale * Math.sin(angle),
      scale * Math.cos(angle),
      400,
      700,
    ];
    const display = sourceToDisplayMatrix(sourceToCanvas, 0.25);
    const inverse = invertMatrix(display);
    expect(inverse).not.toBeNull();
    const point = { x: 812.5, y: 301.25 };
    const back = applyMatrix(inverse!, applyMatrix(display, point));
    expect(back.x).toBeCloseTo(point.x, 6);
    expect(back.y).toBeCloseTo(point.y, 6);
    expect(invertMatrix([0, 0, 0, 0, 1, 1])).toBeNull();
  });

  it("makes upright boxes from any two corners, inside the picture", () => {
    expect(regionBetween({ x: 300, y: 200 }, { x: 100, y: 50 }, BOUNDS)).toEqual({ left: 100, top: 50, width: 200, height: 150 });
    expect(regionBetween({ x: -50, y: 1000 }, { x: 100, y: 1200 }, BOUNDS)).toEqual({ left: 0, top: 1000, width: 100, height: 80 });
  });

  it("moves and resizes boxes without leaving the picture or turning them inside out", () => {
    const region = { left: 100, top: 100, width: 200, height: 100 };
    expect(movedRegion(region, { x: 2000, y: -500 }, BOUNDS)).toEqual({ left: 1720, top: 0, width: 200, height: 100 });
    expect(resizedRegion(region, "se", { x: 400, y: 260 }, BOUNDS)).toEqual({ left: 100, top: 100, width: 300, height: 160 });
    expect(resizedRegion(region, "w", { x: 500, y: 0 }, BOUNDS)).toEqual({ left: 299, top: 100, width: 1, height: 100 });
    expect(handlePosition(region, "ne")).toEqual({ x: 300, y: 100 });
    expect(handlePosition(region, "s")).toEqual({ x: 200, y: 200 });
  });

  it("gives the corners in drawing order and checks the size", () => {
    const corners = regionCorners({ left: 10, top: 20, width: 30, height: 40 }, [2, 0, 0, 2, 5, 5]);
    expect(corners).toEqual([
      { x: 25, y: 45 },
      { x: 85, y: 45 },
      { x: 85, y: 125 },
      { x: 25, y: 125 },
    ]);
    expect(isLargeEnough({ left: 0, top: 0, width: 24, height: 30 }, 24)).toBe(true);
    expect(isLargeEnough({ left: 0, top: 0, width: 23, height: 30 }, 24)).toBe(false);
  });
});
