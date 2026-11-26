// Geometry of the blur tool: from the source video's pixels (where tracked
// blurs live) to the preview's pixels and back.
//
// The back end gives the renderer's own mapping from source pixels to canvas
// pixels (`ClipOverlay.sourceToCanvas`, SVG's `matrix(a, b, c, d, e, f)` order);
// the preview shows the canvas scaled to its size. Boxes stay upright rectangles
// in the source: a rotated clip shows them rotated.

import type { SourceRegion } from "../api/types";

/** An affine map in SVG's `matrix(a, b, c, d, e, f)` order: x' = a·x + c·y + e, y' = b·x + d·y + f. */
export type AffineMatrix = [number, number, number, number, number, number];

export interface Point {
  x: number;
  y: number;
}

/** The map from source pixels to the preview's pixels, for a canvas shown `displayScale` times its size. */
export function sourceToDisplayMatrix(sourceToCanvas: AffineMatrix, displayScale: number): AffineMatrix {
  return sourceToCanvas.map((value) => value * displayScale) as AffineMatrix;
}

export function applyMatrix(matrix: AffineMatrix, point: Point): Point {
  const [a, b, c, d, e, f] = matrix;
  return { x: a * point.x + c * point.y + e, y: b * point.x + d * point.y + f };
}

/** The reverse map, or `null` when the matrix flattens everything (zero scale). */
export function invertMatrix(matrix: AffineMatrix): AffineMatrix | null {
  const [a, b, c, d, e, f] = matrix;
  const determinant = a * d - b * c;
  if (Math.abs(determinant) < 1e-12) {
    return null;
  }
  const inverseA = d / determinant;
  const inverseB = -b / determinant;
  const inverseC = -c / determinant;
  const inverseD = a / determinant;
  return [inverseA, inverseB, inverseC, inverseD, -(inverseA * e + inverseC * f), -(inverseB * e + inverseD * f)];
}

/** The upright source box with `first` and `second` as opposite corners, kept inside `bounds` (left, top, right, bottom). */
export function regionBetween(first: Point, second: Point, bounds: [number, number, number, number]): SourceRegion {
  const [boundLeft, boundTop, boundRight, boundBottom] = bounds;
  const clampX = (x: number) => Math.min(Math.max(x, boundLeft), boundRight);
  const clampY = (y: number) => Math.min(Math.max(y, boundTop), boundBottom);
  const left = clampX(Math.min(first.x, second.x));
  const right = clampX(Math.max(first.x, second.x));
  const top = clampY(Math.min(first.y, second.y));
  const bottom = clampY(Math.max(first.y, second.y));
  return { left, top, width: right - left, height: bottom - top };
}

/** `region` moved by `offset` (source pixels), kept inside `bounds`. */
export function movedRegion(region: SourceRegion, offset: Point, bounds: [number, number, number, number]): SourceRegion {
  const [boundLeft, boundTop, boundRight, boundBottom] = bounds;
  const left = Math.min(Math.max(region.left + offset.x, boundLeft), Math.max(boundLeft, boundRight - region.width));
  const top = Math.min(Math.max(region.top + offset.y, boundTop), Math.max(boundTop, boundBottom - region.height));
  return { ...region, left, top };
}

/** The handles of a box: corners and middles of the sides, named by the sides they move. */
export type HandleName = "n" | "s" | "e" | "w" | "ne" | "nw" | "se" | "sw";

export const HANDLE_NAMES: HandleName[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];

/** Where handle `handle` of `region` is, in source pixels. */
export function handlePosition(region: SourceRegion, handle: HandleName): Point {
  const right = region.left + region.width;
  const bottom = region.top + region.height;
  const middleX = region.left + region.width / 2;
  const middleY = region.top + region.height / 2;
  const x = handle.includes("w") ? region.left : handle.includes("e") ? right : middleX;
  const y = handle.includes("n") ? region.top : handle.includes("s") ? bottom : middleY;
  return { x, y };
}

/** `region` with the sides of `handle` dragged to `point` (source pixels), kept inside `bounds` and never inverted. */
export function resizedRegion(
  region: SourceRegion,
  handle: HandleName,
  point: Point,
  bounds: [number, number, number, number],
): SourceRegion {
  let left = region.left;
  let top = region.top;
  let right = region.left + region.width;
  let bottom = region.top + region.height;
  if (handle.includes("w")) {
    left = Math.min(point.x, right - 1);
  }
  if (handle.includes("e")) {
    right = Math.max(point.x, left + 1);
  }
  if (handle.includes("n")) {
    top = Math.min(point.y, bottom - 1);
  }
  if (handle.includes("s")) {
    bottom = Math.max(point.y, top + 1);
  }
  return regionBetween({ x: left, y: top }, { x: right, y: bottom }, bounds);
}

/** The four corners of `region` in another space, in drawing order (for an SVG polygon). */
export function regionCorners(region: SourceRegion, matrix: AffineMatrix): Point[] {
  const right = region.left + region.width;
  const bottom = region.top + region.height;
  return [
    { x: region.left, y: region.top },
    { x: right, y: region.top },
    { x: right, y: bottom },
    { x: region.left, y: bottom },
  ].map((corner) => applyMatrix(matrix, corner));
}

/** Whether `region` is large enough for the tracker. */
export function isLargeEnough(region: SourceRegion, minimumSide: number): boolean {
  return region.width >= minimumSide && region.height >= minimumSide;
}
