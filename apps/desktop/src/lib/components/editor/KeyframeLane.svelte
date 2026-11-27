<!--
  The keyframes of one animated setting along its clip (docs/ux.md, "Animate a
  clip"): a diamond per keyframe, the playhead, and the stretch the playhead is
  in. Click a diamond to put the playhead on it; drag it to move it in time
  (snapped to frames; it cannot land on another one). With the keyboard: ← / →
  move the focused keyframe by a frame (Shift: ten), Delete removes it. Each
  finished gesture is one request, so one undo step.
-->
<script lang="ts">
  import type { KeyframeView } from "../../api/types";
  import { t } from "../../i18n/i18n.svelte";
  import { formatTimecode, snapToFrame, ticksPerFrame, type FrameRate } from "../../time/ticks";
  import { pointerOffsetX } from "../../ui/pointer";

  interface Props {
    /** The setting's name, for the diamonds' accessible names. */
    label: string;
    keyframes: KeyframeView[];
    /** The clip's start on the timeline, so moves land on the timeline's frames. */
    clipStartTicks: number;
    clipDurationTicks: number;
    /** The playhead, from the clip's start (may be outside the clip). */
    playheadOffsetTicks: number;
    /** The keyframe starting the stretch the playhead is in, if any. */
    stretchStartTicks: number | undefined;
    frameRate: FrameRate;
    onSeek: (offsetTicks: number) => void;
    onMove: (fromOffsetTicks: number, toOffsetTicks: number) => void;
    onRemove: (offsetTicks: number) => void;
  }

  let {
    label,
    keyframes,
    clipStartTicks,
    clipDurationTicks,
    playheadOffsetTicks,
    stretchStartTicks,
    frameRate,
    onSeek,
    onMove,
    onRemove,
  }: Props = $props();

  /** Below this many pixels of movement, a press on a diamond is a click. */
  const DRAG_THRESHOLD_IN_PIXELS = 3;
  const BIG_STEP_FRAMES = 10;

  let lane: HTMLDivElement | undefined = $state();
  /** The keyframe being dragged, and where it would land. */
  let dragged = $state<{ fromOffsetTicks: number; toOffsetTicks: number; isRefused: boolean } | null>(null);

  const lastFrameOffset = $derived(Math.max(0, clipDurationTicks - ticksPerFrame(frameRate)));

  /** Where an offset is along the lane, as a percentage (keyframes past the end sit on it). */
  function percentOf(offsetTicks: number): number {
    if (clipDurationTicks <= 0) {
      return 0;
    }
    return (Math.min(Math.max(offsetTicks, 0), clipDurationTicks) / clipDurationTicks) * 100;
  }

  /** The offset under a pointer, on a frame of the timeline, kept on the clip. */
  function offsetAtPointer(clientX: number): number {
    if (!lane) {
      return 0;
    }
    const fraction = Math.min(1, Math.max(0, pointerOffsetX(clientX, lane) / lane.clientWidth));
    return frameOffset(fraction * clipDurationTicks);
  }

  /** `offsetTicks` moved to the start of its frame on the timeline, kept on the clip. */
  function frameOffset(offsetTicks: number): number {
    const snapped = snapToFrame(clipStartTicks + offsetTicks, frameRate) - clipStartTicks;
    return Math.min(lastFrameOffset, Math.max(0, snapped));
  }

  function isTaken(offsetTicks: number, ignoredOffsetTicks: number): boolean {
    return keyframes.some((keyframe) => keyframe.offsetTicks === offsetTicks && keyframe.offsetTicks !== ignoredOffsetTicks);
  }

  function startDrag(event: PointerEvent, keyframe: KeyframeView): void {
    if (event.button !== 0) {
      return;
    }
    event.stopPropagation();
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture(event.pointerId);
    const startX = event.clientX;
    const fromOffsetTicks = keyframe.offsetTicks;
    let hasMoved = false;
    const move = (moveEvent: PointerEvent) => {
      hasMoved ||= Math.abs(moveEvent.clientX - startX) >= DRAG_THRESHOLD_IN_PIXELS;
      if (hasMoved) {
        const toOffsetTicks = offsetAtPointer(moveEvent.clientX);
        dragged = { fromOffsetTicks, toOffsetTicks, isRefused: isTaken(toOffsetTicks, fromOffsetTicks) };
      }
    };
    const end = () => {
      target.removeEventListener("pointermove", move);
      const landing = dragged;
      dragged = null;
      if (!hasMoved) {
        onSeek(fromOffsetTicks);
      } else if (landing && !landing.isRefused && landing.toOffsetTicks !== fromOffsetTicks) {
        onMove(fromOffsetTicks, landing.toOffsetTicks);
      }
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", end, { once: true });
    target.addEventListener("pointercancel", () => {
      target.removeEventListener("pointermove", move);
      dragged = null;
    }, { once: true });
  }

  function seekAtPointer(event: PointerEvent): void {
    if (event.button === 0) {
      onSeek(offsetAtPointer(event.clientX));
    }
  }

  function handleKeydown(event: KeyboardEvent, keyframe: KeyframeView): void {
    if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      event.preventDefault();
      event.stopPropagation();
      const frames = (event.key === "ArrowLeft" ? -1 : 1) * (event.shiftKey ? BIG_STEP_FRAMES : 1);
      const toOffsetTicks = frameOffset(keyframe.offsetTicks + frames * ticksPerFrame(frameRate) + 1);
      if (toOffsetTicks !== keyframe.offsetTicks && !isTaken(toOffsetTicks, keyframe.offsetTicks)) {
        onMove(keyframe.offsetTicks, toOffsetTicks);
      }
    } else if (event.key === "Delete" || event.key === "Backspace") {
      event.preventDefault();
      event.stopPropagation();
      onRemove(keyframe.offsetTicks);
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      event.stopPropagation();
      onSeek(keyframe.offsetTicks);
    }
  }

  function keyframeLabel(keyframe: KeyframeView): string {
    return t("keyframes.keyframeAt", { name: label, time: formatTimecode(clipStartTicks + keyframe.offsetTicks, frameRate) });
  }

  const stretch = $derived.by(() => {
    if (stretchStartTicks === undefined) {
      return null;
    }
    const next = keyframes.find((keyframe) => keyframe.offsetTicks > stretchStartTicks);
    return next ? { left: percentOf(stretchStartTicks), right: percentOf(next.offsetTicks) } : null;
  });
  const playheadIsOnClip = $derived(playheadOffsetTicks >= 0 && playheadOffsetTicks < clipDurationTicks);
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="lane" bind:this={lane} onpointerdown={seekAtPointer}>
  <span class="track"></span>
  {#if stretch}
    <span class="stretch" style:left="{stretch.left}%" style:width="{stretch.right - stretch.left}%"></span>
  {/if}
  {#if playheadIsOnClip}
    <span class="playhead" style:left="{percentOf(playheadOffsetTicks)}%"></span>
  {/if}
  {#each keyframes as keyframe (keyframe.offsetTicks)}
    {@const isDragged = dragged?.fromOffsetTicks === keyframe.offsetTicks}
    {@const shownOffset = isDragged && dragged ? dragged.toOffsetTicks : keyframe.offsetTicks}
    <button
      type="button"
      class="diamond"
      class:current={keyframe.offsetTicks === playheadOffsetTicks}
      class:dragged={isDragged}
      class:refused={isDragged && dragged?.isRefused}
      class:past-the-end={keyframe.offsetTicks > clipDurationTicks}
      style:left="{percentOf(shownOffset)}%"
      title={isDragged && dragged?.isRefused ? t("keyframes.cannotLandOnAnother") : keyframeLabel(keyframe)}
      aria-label={keyframeLabel(keyframe)}
      onpointerdown={(event) => startDrag(event, keyframe)}
      onkeydown={(event) => handleKeydown(event, keyframe)}
    ></button>
  {/each}
</div>

<style>
  .lane {
    position: relative;
    flex: 1;
    min-width: 0;
    height: 20px;
    cursor: pointer;
    touch-action: none;
  }
  .track {
    position: absolute;
    left: 0;
    right: 0;
    top: 9px;
    height: 2px;
    border-radius: 1px;
    background: var(--color-border-strong);
  }
  .stretch {
    position: absolute;
    top: 8px;
    height: 4px;
    border-radius: 2px;
    background: var(--color-accent);
    opacity: 0.6;
  }
  .playhead {
    position: absolute;
    top: 2px;
    bottom: 2px;
    width: 2px;
    margin-left: -1px;
    background: var(--color-playhead);
    pointer-events: none;
  }
  .diamond {
    position: absolute;
    top: 5px;
    width: 10px;
    height: 10px;
    margin-left: -5px;
    padding: 0;
    border: 1.5px solid var(--color-text);
    border-radius: 2px;
    background: var(--color-bg-panel);
    transform: rotate(45deg);
    cursor: grab;
  }
  .diamond:hover {
    border-color: var(--color-accent-text);
  }
  .diamond.current {
    background: var(--color-accent);
    border-color: var(--color-accent-text);
  }
  .diamond.dragged {
    cursor: grabbing;
    z-index: 1;
  }
  .diamond.refused {
    border-color: var(--color-danger);
    background: var(--color-danger-soft);
  }
  .diamond.past-the-end {
    border-style: dashed;
  }
  .diamond:focus-visible {
    outline: 2px solid var(--color-focus-ring);
    outline-offset: 2px;
  }
</style>
