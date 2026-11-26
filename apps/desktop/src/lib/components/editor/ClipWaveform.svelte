<!--
  The waveform drawn along a clip: the loudest peak under each screen pixel, for
  the part of the clip that is on screen only (a long clip at a high zoom would
  otherwise need a canvas wider than browsers allow).
-->
<script lang="ts">
  import type { Waveform } from "../../api/binaryMessages";
  import { waveformOf } from "../../media/mediaCaches";
  import { TICKS_PER_SECOND } from "../../time/ticks";

  interface Props {
    mediaId: number;
    filePath: string;
    /** Range of the file the clip plays. */
    sourceStartTicks: number;
    speed: number;
    pixelsPerSecond: number;
    /** The visible part of the clip, in pixels from the clip's left edge. */
    visibleStartInPixels: number;
    visibleWidthInPixels: number;
    heightInPixels: number;
  }

  let {
    mediaId,
    filePath,
    sourceStartTicks,
    speed,
    pixelsPerSecond,
    visibleStartInPixels,
    visibleWidthInPixels,
    heightInPixels,
  }: Props = $props();

  let canvas: HTMLCanvasElement | undefined = $state();
  let waveform = $state<Waveform | null>(null);

  $effect(() => {
    let isCurrent = true;
    waveformOf(mediaId, filePath)
      .then((loaded) => {
        if (isCurrent) {
          waveform = loaded;
        }
      })
      .catch(() => {
        // No waveform (no sound, unreadable file): the clip simply shows none.
      });
    return () => {
      isCurrent = false;
    };
  });

  $effect(() => {
    if (!canvas || !waveform) {
      return;
    }
    const width = Math.max(1, Math.round(visibleWidthInPixels));
    const height = Math.max(1, Math.round(heightInPixels));
    const pixelRatio = window.devicePixelRatio || 1;
    canvas.width = Math.round(width * pixelRatio);
    canvas.height = Math.round(height * pixelRatio);
    const context = canvas.getContext("2d");
    if (!context) {
      return;
    }
    context.setTransform(pixelRatio, 0, 0, pixelRatio, 0, 0);
    context.clearRect(0, 0, width, height);
    context.fillStyle = getComputedStyle(canvas).getPropertyValue("--color-wave") || "rgba(255,255,255,0.6)";
    const { peaks, peaksPerSecond } = waveform;
    const sourceStartInSeconds = sourceStartTicks / TICKS_PER_SECOND;
    const middle = height / 2;
    for (let column = 0; column < width; column += 1) {
      // Source seconds covered by this screen column.
      const timelineSeconds = (visibleStartInPixels + column) / pixelsPerSecond;
      const nextTimelineSeconds = (visibleStartInPixels + column + 1) / pixelsPerSecond;
      const firstPeak = Math.floor((sourceStartInSeconds + timelineSeconds * speed) * peaksPerSecond);
      const endPeak = Math.max(firstPeak + 1, Math.ceil((sourceStartInSeconds + nextTimelineSeconds * speed) * peaksPerSecond));
      let loudest = 0;
      for (let peakIndex = firstPeak; peakIndex < endPeak && peakIndex < peaks.length; peakIndex += 1) {
        loudest = Math.max(loudest, peaks[peakIndex] ?? 0);
      }
      // Square root: quiet passages stay visible next to loud ones.
      const halfHeight = Math.max(0.5, Math.sqrt(loudest / 255) * middle * 0.95);
      context.fillRect(column, middle - halfHeight, 1, halfHeight * 2);
    }
  });
</script>

<canvas
  bind:this={canvas}
  aria-hidden="true"
  style:left="{visibleStartInPixels}px"
  style:width="{visibleWidthInPixels}px"
  style:height="{heightInPixels}px"
></canvas>

<style>
  canvas {
    position: absolute;
    bottom: 0;
    display: block;
    pointer-events: none;
  }
</style>
