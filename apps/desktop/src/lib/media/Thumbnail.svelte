<!--
  The picture of a media file at a given time, drawn on a canvas once the back
  end has made it. Shows a neutral block until then; a transparent picture
  shows a checkerboard behind it.
-->
<script lang="ts">
  import { thumbnailOf } from "./mediaCaches";

  interface Props {
    mediaId: number;
    filePath: string;
    sourceTimeTicks: number;
    maxWidth: number;
    maxHeight: number;
    fit?: "cover" | "contain";
  }

  let { mediaId, filePath, sourceTimeTicks, maxWidth, maxHeight, fit = "cover" }: Props = $props();

  let canvas: HTMLCanvasElement | undefined = $state();
  let isLoaded = $state(false);

  $effect(() => {
    const request = { mediaId, filePath, sourceTimeTicks, maxWidth, maxHeight };
    let isCurrent = true;
    thumbnailOf(request)
      .then((picture) => {
        if (!isCurrent || !canvas) {
          return;
        }
        canvas.width = picture.width;
        canvas.height = picture.height;
        canvas.getContext("2d")?.putImageData(new ImageData(picture.pixels, picture.width, picture.height), 0, 0);
        isLoaded = true;
      })
      .catch(() => {
        isLoaded = false;
      });
    return () => {
      isCurrent = false;
    };
  });
</script>

<canvas bind:this={canvas} class:loaded={isLoaded} style:object-fit={fit} aria-hidden="true"></canvas>

<style>
  canvas {
    display: block;
    width: 100%;
    height: 100%;
    background:
      repeating-conic-gradient(var(--color-checkerboard-light) 0 25%, var(--color-checkerboard-dark) 0 50%) 0 0 / 12px 12px;
    opacity: 0;
    transition: opacity var(--duration-normal) var(--easing-standard);
  }
  canvas.loaded {
    opacity: 1;
  }
</style>
