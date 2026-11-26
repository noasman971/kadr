// Timeline time in the front end.
//
// The engine counts time in integer ticks: 705 600 000 per second, so that every
// common frame and sample duration is a whole number (see docs/data-model.md).
// The front end keeps ticks too, and only converts to seconds, frames and pixels
// to display them. Ticks fit exactly in JavaScript numbers for any edit shorter
// than about 147 days.

export const TICKS_PER_SECOND = 705_600_000;

/** An exact frame rate, as the engine stores it. */
export interface FrameRate {
  numerator: number;
  denominator: number;
}

export function secondsToTicks(seconds: number): number {
  return Math.round(seconds * TICKS_PER_SECOND);
}

export function ticksToSeconds(ticks: number): number {
  return ticks / TICKS_PER_SECOND;
}

/** Ticks per frame; exact for every common rate (24, 25, 30, 29.97, 50, 60…). */
export function ticksPerFrame(frameRate: FrameRate): number {
  return (TICKS_PER_SECOND * frameRate.denominator) / frameRate.numerator;
}

/** The index of the frame showing at `ticks` (the last frame starting at or before it). */
export function frameIndexAt(ticks: number, frameRate: FrameRate): number {
  return Math.floor((Math.max(0, ticks) * frameRate.numerator) / (TICKS_PER_SECOND * frameRate.denominator) + 1e-9);
}

/** Where frame `frameIndex` starts, in ticks (rounded like the engine: down). */
export function ticksOfFrame(frameIndex: number, frameRate: FrameRate): number {
  return Math.floor((frameIndex * TICKS_PER_SECOND * frameRate.denominator) / frameRate.numerator);
}

/** `ticks` moved to the start of the frame it falls in. */
export function snapToFrame(ticks: number, frameRate: FrameRate): number {
  return ticksOfFrame(frameIndexAt(ticks, frameRate), frameRate);
}

/** Frames per second, rounded up for display (29.97 → 30 frame numbers). */
function nominalFramesPerSecond(frameRate: FrameRate): number {
  return Math.ceil(frameRate.numerator / frameRate.denominator - 1e-9);
}

/**
 * A timecode `MM:SS:FF` (minutes, seconds, frames), or `H:MM:SS:FF` from one hour.
 * Frames count from 0 within each second.
 */
export function formatTimecode(ticks: number, frameRate: FrameRate): string {
  const frameIndex = frameIndexAt(ticks, frameRate);
  const framesPerSecond = nominalFramesPerSecond(frameRate);
  const exactSeconds = ticksToSeconds(ticksOfFrame(frameIndex, frameRate));
  const wholeSeconds = Math.floor(exactSeconds + 1e-9);
  const frameInSecond = Math.max(
    0,
    frameIndex - frameIndexAt(secondsToTicks(wholeSeconds), frameRate),
  );
  const hours = Math.floor(wholeSeconds / 3600);
  const minutes = Math.floor((wholeSeconds % 3600) / 60);
  const seconds = wholeSeconds % 60;
  const pad = (value: number) => String(value).padStart(2, "0");
  const frames = pad(Math.min(frameInSecond, framesPerSecond - 1));
  const head = hours > 0 ? `${hours}:${pad(minutes)}` : pad(minutes);
  return `${head}:${pad(seconds)}:${frames}`;
}

/** A short length for badges and tiles: `0:12`, `2:31`, `1:02:05`. */
export function formatShortDuration(ticks: number): string {
  const totalSeconds = Math.round(ticksToSeconds(ticks));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = String(totalSeconds % 60).padStart(2, "0");
  if (hours > 0) {
    return `${hours}:${String(minutes).padStart(2, "0")}:${seconds}`;
  }
  return `${minutes}:${seconds}`;
}
