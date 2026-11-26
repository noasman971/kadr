// The export presets as the dialog shows them, their shapes, and an estimate of
// the file size. The back end owns the real sizes (`kadr_engine::export_presets`);
// these mirror them for display.

import type { ExportSize } from "../api/types";

export interface PictureSize {
  width: number;
  height: number;
}

export type Shape = "horizontal" | "vertical" | "square";

/** The presets after "Project format", in the dialog's order. */
export const PRESET_SIZES: ReadonlyArray<{ size: Exclude<ExportSize, "project">; picture: PictureSize }> = [
  { size: "youtube1080p", picture: { width: 1920, height: 1080 } },
  { size: "youtube4k", picture: { width: 3840, height: 2160 } },
  { size: "verticalReelsShorts", picture: { width: 1080, height: 1920 } },
  { size: "square1080", picture: { width: 1080, height: 1080 } },
];

/** The picture size of an export size for a project of `canvas`. */
export function pictureSizeOf(size: ExportSize, canvas: PictureSize): PictureSize {
  return PRESET_SIZES.find((preset) => preset.size === size)?.picture ?? canvas;
}

export function shapeOf(picture: PictureSize): Shape {
  if (picture.width === picture.height) {
    return "square";
  }
  return picture.width > picture.height ? "horizontal" : "vertical";
}

/** Exactly the same proportions (1280 × 720 and 3840 × 2160 do; 1080 × 1920 does not). */
export function haveSameShape(first: PictureSize, second: PictureSize): boolean {
  return first.width * second.height === first.height * second.width;
}

/** H.264 bits per pixel and frame at "High" quality: about 6 Mbit/s for 1080p30. */
const BITS_PER_PIXEL_PER_FRAME = 0.1;
/** "Fast" files are larger for the same picture. */
const FAST_SIZE_FACTOR = 1.6;
const AUDIO_BITS_PER_SECOND = 192_000;

/** An estimate of the exported file's size, in bytes. */
export function estimatedFileSizeInBytes(
  picture: PictureSize,
  framesPerSecond: number,
  durationSeconds: number,
  quality: "high" | "fast",
): number {
  const videoBitsPerSecond =
    picture.width * picture.height * framesPerSecond * BITS_PER_PIXEL_PER_FRAME * (quality === "fast" ? FAST_SIZE_FACTOR : 1);
  return ((videoBitsPerSecond + AUDIO_BITS_PER_SECOND) * durationSeconds) / 8;
}
