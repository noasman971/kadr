<!--
  The time ruler above the tracks. Clicking or dragging on it moves the playhead.
-->
<script lang="ts">
  import {
    formatRulerLabel,
    pixelsToTicks,
    rulerStepInSeconds,
  } from "../../timeline/timelineGeometry";
  import { pointerOffsetX } from "../../ui/pointer";

  interface Props {
    pixelsPerSecond: number;
    widthInPixels: number;
    /** Visible horizontal range, to draw only the labels on screen. */
    visibleStartInPixels: number;
    visibleWidthInPixels: number;
    onSeek: (ticks: number) => void;
  }

  let { pixelsPerSecond, widthInPixels, visibleStartInPixels, visibleWidthInPixels, onSeek }: Props = $props();

  const stepInSeconds = $derived(rulerStepInSeconds(pixelsPerSecond));
  const labels = $derived.by(() => {
    const firstStep = Math.max(0, Math.floor(visibleStartInPixels / pixelsPerSecond / stepInSeconds) - 1);
    const lastStep = Math.ceil((visibleStartInPixels + visibleWidthInPixels) / pixelsPerSecond / stepInSeconds) + 1;
    const result: Array<{ x: number; text: string }> = [];
    for (let step = firstStep; step <= lastStep; step += 1) {
      const seconds = step * stepInSeconds;
      result.push({ x: seconds * pixelsPerSecond, text: formatRulerLabel(seconds, stepInSeconds) });
    }
    return result;
  });
  /** Four small ticks between labels. */
  const minorSpacing = $derived((stepInSeconds * pixelsPerSecond) / 4);

  let isDragging = false;

  function seekTo(event: PointerEvent): void {
    const offsetX = pointerOffsetX(event.clientX, event.currentTarget as HTMLElement);
    onSeek(Math.max(0, pixelsToTicks(offsetX, pixelsPerSecond)));
  }

  function startDrag(event: PointerEvent): void {
    if (event.button !== 0) {
      return;
    }
    isDragging = true;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    seekTo(event);
  }

  function drag(event: PointerEvent): void {
    if (isDragging) {
      seekTo(event);
    }
  }

  function endDrag(): void {
    isDragging = false;
  }
</script>

<div
  class="ruler"
  role="presentation"
  style:width="{widthInPixels}px"
  style:--major-spacing="{stepInSeconds * pixelsPerSecond}px"
  style:--minor-spacing="{minorSpacing}px"
  onpointerdown={startDrag}
  onpointermove={drag}
  onpointerup={endDrag}
  onpointercancel={endDrag}
>
  {#each labels as label (label.x)}
    <span class="label tabular" style:left="{label.x}px">{label.text}</span>
  {/each}
</div>

<style>
  .ruler {
    position: relative;
    height: 100%;
    cursor: pointer;
    background:
      repeating-linear-gradient(to right, var(--color-border-strong) 0 1px, transparent 1px var(--major-spacing)) 0 100% /
        100% 9px no-repeat,
      repeating-linear-gradient(to right, var(--color-border) 0 1px, transparent 1px var(--minor-spacing)) 0 100% / 100%
        4px no-repeat;
  }
  .label {
    position: absolute;
    top: 6px;
    transform: translateX(4px);
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
    white-space: nowrap;
    pointer-events: none;
  }
</style>
