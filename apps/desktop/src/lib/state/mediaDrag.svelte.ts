// A media file being dragged from the library. The gesture crosses panels
// (library → timeline), so its state lives here: the library starts it, the
// timeline shows where the clip would land and places it on release.
//
// Drags use pointer events, not HTML drag and drop: Tauri's file drop, needed to
// import files from the explorer, turns HTML drag and drop off on Windows.

import type { MediaView } from "../api/types";

/** The pointer must travel this far before a press becomes a drag. */
export const DRAG_START_DISTANCE_IN_PIXELS = 4;

class MediaDrag {
  /** The file being dragged, once the pointer has moved enough. */
  media = $state<MediaView | null>(null);
  clientX = $state(0);
  clientY = $state(0);
  /** Set by the timeline while the pointer is over it: where the clip would go, or why it cannot. */
  dropLabel = $state<string | null>(null);
  /** Set by the timeline: `dropLabel` explains a refusal. */
  isDropRefused = $state(false);

  private pressed: { media: MediaView; startX: number; startY: number } | null = null;
  private dropHandlers: Array<(media: MediaView, clientX: number, clientY: number) => void> = [];

  /** Called by a library tile on pointer down. */
  press(event: PointerEvent, media: MediaView): void {
    if (event.button !== 0) {
      return;
    }
    this.pressed = { media, startX: event.clientX, startY: event.clientY };
    window.addEventListener("pointermove", this.handleMove);
    window.addEventListener("pointerup", this.handleUp, { once: true });
  }

  /** The timeline registers here to receive drops. */
  onDrop(handler: (media: MediaView, clientX: number, clientY: number) => void): () => void {
    this.dropHandlers.push(handler);
    return () => {
      this.dropHandlers = this.dropHandlers.filter((registered) => registered !== handler);
    };
  }

  private handleMove = (event: PointerEvent): void => {
    if (!this.pressed) {
      return;
    }
    const distance = Math.hypot(event.clientX - this.pressed.startX, event.clientY - this.pressed.startY);
    if (!this.media && distance >= DRAG_START_DISTANCE_IN_PIXELS) {
      this.media = this.pressed.media;
    }
    this.clientX = event.clientX;
    this.clientY = event.clientY;
  };

  private handleUp = (event: PointerEvent): void => {
    window.removeEventListener("pointermove", this.handleMove);
    const media = this.media;
    this.pressed = null;
    this.media = null;
    this.dropLabel = null;
    this.isDropRefused = false;
    if (media) {
      for (const handler of this.dropHandlers) {
        handler(media, event.clientX, event.clientY);
      }
    }
  };
}

export const mediaDrag = new MediaDrag();
