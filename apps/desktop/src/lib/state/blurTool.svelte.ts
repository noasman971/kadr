// The blur tool (docs/ux.md, "Blurring a face"): drawing a box on the preview,
// adjusting it, then following it through the clip in the background.
//
//   off ── start ──► drawing ── box drawn ──► adjusting ── confirm ──► tracking
//    ▲                  │  Escape                 │  Escape               │ finished,
//    └──────────────────┴─────────────────────────┘                       │ failed or
//    ▲                                                                    │ cancelled
//    └────────────────────────────────────────────────────────────────────┘
//
// The box is kept in the source video's pixels, where tracked blurs live; the
// preview draws it through the clip's mapping (`blur/sourceMapping.ts`).

import { backend } from "../api/backend";
import type { BlurShapeChoice, ClipOverlay, SourceRegion, TrackingEvent } from "../api/types";
import { blurTargetClip } from "../blur/blurTarget";
import { isLargeEnough } from "../blur/sourceMapping";
import { t } from "../i18n/i18n.svelte";
import { snapToFrame } from "../time/ticks";
import { editor } from "./editor.svelte";
import { describeError } from "./errors";
import { playback } from "./playback.svelte";
import { toasts } from "./toasts.svelte";

export type BlurToolPhase = "off" | "drawing" | "adjusting" | "tracking";

export interface TrackingProgressView {
  analysedFrameCount: number;
  totalFrameCount: number;
  elapsedSeconds: number;
}

/** Notices about the analysis replace each other. */
const TRACKING_TOAST_GROUP = "tracking";

class BlurTool {
  phase = $state<BlurToolPhase>("off");
  /** The clip being blurred. */
  clipId = $state<number | null>(null);
  /** The box, in the source video's pixels, over the frame under the playhead. */
  region = $state<SourceRegion | null>(null);
  shape = $state<BlurShapeChoice>("ellipse");
  /** Where the clip's picture is under the playhead (fetched by the preview). */
  overlay = $state<ClipOverlay | null>(null);
  progress = $state<TrackingProgressView>({ analysedFrameCount: 0, totalFrameCount: 0, elapsedSeconds: 0 });

  readonly isActive = $derived(this.phase === "drawing" || this.phase === "adjusting");

  readonly isRegionLargeEnough = $derived(
    this.region !== null && this.overlay !== null && isLargeEnough(this.region, this.overlay.minimumRegionSide),
  );

  readonly percentDone = $derived(
    this.progress.totalFrameCount > 0
      ? Math.min(100, Math.floor((100 * this.progress.analysedFrameCount) / this.progress.totalFrameCount))
      : 0,
  );

  /** Seconds left, from the pace so far; `null` until there is a pace to go by. */
  readonly secondsLeft = $derived.by(() => {
    const { analysedFrameCount, totalFrameCount, elapsedSeconds } = this.progress;
    if (analysedFrameCount < 10 || elapsedSeconds <= 0) {
      return null;
    }
    return ((totalFrameCount - analysedFrameCount) * elapsedSeconds) / analysedFrameCount;
  });

  /** Why the tool cannot start now, as a sentence; `null` when it can. */
  unavailableReason(): string | null {
    const view = editor.view;
    if (this.phase === "tracking") {
      return t("blur.busy");
    }
    if (!view || !blurTargetClip(view, editor.selectedClipIds, editor.playheadTicks)) {
      return t("blur.noVideoHere");
    }
    return null;
  }

  /** Starts drawing on the clip under the playhead (the selected one if it shows). */
  start(): void {
    const view = editor.view;
    if (this.phase === "tracking" || !view) {
      return;
    }
    const clip = blurTargetClip(view, editor.selectedClipIds, editor.playheadTicks);
    if (!clip) {
      toasts.show({ tone: "info", message: t("blur.noVideoHere") });
      return;
    }
    playback.pause();
    this.clipId = clip.id;
    editor.selectedClipIds = [clip.id];
    this.region = null;
    this.overlay = null;
    this.phase = "drawing";
  }

  /** Leaves the tool without changing anything (not while tracking: see `cancelTracking`). */
  cancel(): void {
    if (this.isActive) {
      this.phase = "off";
      this.region = null;
    }
  }

  /** The box was drawn or changed. */
  setRegion(region: SourceRegion): void {
    this.region = region;
    if (this.phase === "drawing") {
      this.phase = "adjusting";
    }
  }

  /** Blur and follow: starts the analysis of the box. */
  async confirm(): Promise<void> {
    const { clipId, region } = this;
    if (this.phase !== "adjusting" || clipId === null || region === null || !this.isRegionLargeEnough) {
      return;
    }
    const timeTicks = snapToFrame(editor.playheadTicks, editor.frameRate);
    this.progress = { analysedFrameCount: 0, totalFrameCount: 0, elapsedSeconds: 0 };
    this.phase = "tracking";
    try {
      await backend.startRegionTracking(clipId, timeTicks, region, this.shape, (event) => this.handleEvent(event));
    } catch (error) {
      // Refused before anything ran (box too small, outside the picture…): fix the box.
      this.phase = "adjusting";
      editor.showError(error);
    }
  }

  /** Stops the analysis; nothing is added. */
  cancelTracking(): void {
    if (this.phase === "tracking") {
      void backend.cancelRegionTracking();
    }
  }

  private handleEvent(event: TrackingEvent): void {
    switch (event.type) {
      case "progress":
        this.progress = event;
        return;
      case "finished": {
        editor.acceptView(event.view);
        editor.selectedClipIds = [event.clipId];
        this.finish();
        const rangeCount = event.uncertainRangeCount + event.lostRangeCount;
        toasts.show({
          tone: "success",
          group: TRACKING_TOAST_GROUP,
          message: rangeCount === 0 ? t("blur.done") : t("blur.doneWithRanges", { count: rangeCount }),
          action: { label: t("toast.undo"), run: () => void editor.undo() },
        });
        return;
      }
      case "failed": {
        const described = describeError({ code: event.code, details: event.details, fileName: null });
        // The box can be fixed and tried again, unless the clip is gone.
        this.phase = this.region && editor.view ? "adjusting" : "off";
        toasts.show({ tone: "error", group: TRACKING_TOAST_GROUP, message: described.message, details: described.details });
        return;
      }
      case "cancelled":
        this.finish();
        toasts.show({ tone: "info", group: TRACKING_TOAST_GROUP, message: t("blur.cancelled") });
        return;
    }
  }

  private finish(): void {
    this.phase = "off";
    this.region = null;
  }
}

export const blurTool = new BlurTool();
