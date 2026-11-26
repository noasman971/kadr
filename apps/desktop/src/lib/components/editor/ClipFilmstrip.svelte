<!--
  Thumbnails along a video or picture clip. Only the thumbnails on screen are
  asked for, and their times are rounded to a step that depends on the zoom, so
  scrolling and zooming reuse thumbnails already made.
-->
<script lang="ts">
  import Thumbnail from "../../media/Thumbnail.svelte";
  import { TICKS_PER_SECOND } from "../../time/ticks";

  interface Props {
    mediaId: number;
    filePath: string;
    isStill: boolean;
    sourceStartTicks: number;
    sourceEndTicks: number;
    speed: number;
    pixelsPerSecond: number;
    /** Picture proportions (width / height) of the file. */
    aspectRatio: number;
    heightInPixels: number;
    visibleStartInPixels: number;
    visibleWidthInPixels: number;
  }

  let {
    mediaId,
    filePath,
    isStill,
    sourceStartTicks,
    sourceEndTicks,
    speed,
    pixelsPerSecond,
    aspectRatio,
    heightInPixels,
    visibleStartInPixels,
    visibleWidthInPixels,
  }: Props = $props();

  /** Thumbnail times snap to multiples of these steps (seconds of the file). */
  const TIME_STEPS_IN_SECONDS = [0.1, 0.25, 0.5, 1, 2, 5, 10, 30, 60, 300];
  /** Thumbnails are made at this height (pixels) whatever the track height. */
  const THUMBNAIL_HEIGHT_IN_PIXELS = 90;

  const thumbnailWidth = $derived(Math.max(16, heightInPixels * Math.min(Math.max(aspectRatio, 0.5), 2.4)));
  const thumbnails = $derived.by(() => {
    const first = Math.max(0, Math.floor(visibleStartInPixels / thumbnailWidth));
    const last = Math.ceil((visibleStartInPixels + visibleWidthInPixels) / thumbnailWidth);
    const secondsPerThumbnail = (thumbnailWidth / pixelsPerSecond) * speed;
    const step = TIME_STEPS_IN_SECONDS.find((candidate) => candidate >= secondsPerThumbnail) ?? 300;
    const result: Array<{ x: number; sourceTimeTicks: number }> = [];
    for (let index = first; index < last; index += 1) {
      const x = index * thumbnailWidth;
      const sourceSeconds = sourceStartTicks / TICKS_PER_SECOND + (x / pixelsPerSecond) * speed;
      const roundedSeconds = Math.round(sourceSeconds / step) * step;
      const sourceTimeTicks = isStill
        ? 0
        : Math.min(Math.max(0, Math.round(roundedSeconds * TICKS_PER_SECOND)), Math.max(0, sourceEndTicks - 1));
      result.push({ x, sourceTimeTicks });
    }
    return result;
  });
</script>

{#each thumbnails as thumbnail (thumbnail.x)}
  <div class="thumbnail" style:left="{thumbnail.x}px" style:width="{thumbnailWidth}px" style:height="{heightInPixels}px">
    <Thumbnail
      {mediaId}
      {filePath}
      sourceTimeTicks={thumbnail.sourceTimeTicks}
      maxWidth={Math.round(THUMBNAIL_HEIGHT_IN_PIXELS * aspectRatio)}
      maxHeight={THUMBNAIL_HEIGHT_IN_PIXELS}
    />
  </div>
{/each}

<style>
  .thumbnail {
    position: absolute;
    top: 0;
    overflow: hidden;
    opacity: 0.9;
    pointer-events: none;
  }
</style>
