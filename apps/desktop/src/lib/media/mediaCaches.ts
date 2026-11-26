// Thumbnails and waveforms already fetched, so scrolling the library or the
// timeline does not ask the back end again. Keys use the file path rather than
// the media number, which is only unique within one project.

import { backend } from "../api/backend";
import type { RgbaPicture, Waveform } from "../api/binaryMessages";

/** Thumbnails kept in memory (each is at most 640 × 640 RGBA, usually 160 × 90). */
const MAXIMUM_CACHED_THUMBNAILS = 600;

const thumbnails = new Map<string, Promise<RgbaPicture>>();
const waveforms = new Map<string, Promise<Waveform>>();

export interface ThumbnailRequest {
  mediaId: number;
  filePath: string;
  sourceTimeTicks: number;
  maxWidth: number;
  maxHeight: number;
}

export function thumbnailOf(request: ThumbnailRequest): Promise<RgbaPicture> {
  const key = `${request.filePath}|${request.sourceTimeTicks}|${request.maxWidth}x${request.maxHeight}`;
  const cached = thumbnails.get(key);
  if (cached) {
    // Most recently used last, so the oldest go first.
    thumbnails.delete(key);
    thumbnails.set(key, cached);
    return cached;
  }
  const pending = backend.getThumbnail(
    request.mediaId,
    request.sourceTimeTicks,
    request.maxWidth,
    request.maxHeight,
  );
  pending.catch(() => thumbnails.delete(key));
  thumbnails.set(key, pending);
  while (thumbnails.size > MAXIMUM_CACHED_THUMBNAILS) {
    const oldestKey = thumbnails.keys().next().value;
    if (oldestKey === undefined) {
      break;
    }
    thumbnails.delete(oldestKey);
  }
  return pending;
}

export function waveformOf(mediaId: number, filePath: string): Promise<Waveform> {
  const cached = waveforms.get(filePath);
  if (cached) {
    return cached;
  }
  const pending = backend.getWaveform(mediaId);
  pending.catch(() => waveforms.delete(filePath));
  waveforms.set(filePath, pending);
  return pending;
}
