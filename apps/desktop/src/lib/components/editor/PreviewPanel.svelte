<!--
  The preview (docs/ux.md, "Watch and move around"): the frame under the playhead,
  rendered by the engine at the size the panel shows, and the transport bar.

  Only one frame is asked for at a time. While it is being made, newer playhead
  positions replace each other, and the newest is asked for next: the preview
  never falls behind, it skips frames instead.
-->
<script lang="ts">
  import { onDestroy, onMount } from "svelte";

  import { backend } from "../../api/backend";
  import type { ProjectView } from "../../api/types";
  import { t } from "../../i18n/i18n.svelte";
  import { createFrameRenderer, type FrameRenderer } from "../../preview/yuvFrameRenderer";
  import { editor } from "../../state/editor.svelte";
  import { describeError, isSupersededError } from "../../state/errors";
  import { playback } from "../../state/playback.svelte";
  import { preferences, type PreviewQuality } from "../../state/preferences.svelte";
  import { formatTimecode } from "../../time/ticks";
  import Icon from "../../ui/Icon.svelte";
  import IconButton from "../../ui/IconButton.svelte";
  import ToastStack from "../../ui/ToastStack.svelte";

  interface Props {
    view: ProjectView;
  }

  let { view }: Props = $props();

  /** Space around the picture inside the panel. */
  const STAGE_PADDING_IN_PIXELS = 16;
  const QUALITY_FACTORS: Record<Exclude<PreviewQuality, "auto">, number> = { full: 1, half: 0.5, quarter: 0.25 };

  let canvas: HTMLCanvasElement | undefined = $state();
  let renderer: FrameRenderer | null = null;
  let stageWidth = $state(0);
  let stageHeight = $state(0);
  let hasFrame = $state(false);
  let failure = $state<string | null>(null);

  const hasContent = $derived(view.durationTicks > 0);
  const canvasAspect = $derived(view.settings.canvasWidth / view.settings.canvasHeight);
  const displaySize = $derived.by(() => {
    const availableWidth = Math.max(0, stageWidth - 2 * STAGE_PADDING_IN_PIXELS);
    const availableHeight = Math.max(0, stageHeight - 2 * STAGE_PADDING_IN_PIXELS);
    const width = Math.min(availableWidth, availableHeight * canvasAspect);
    return { width, height: width / canvasAspect };
  });
  const requestedSize = $derived.by(() => {
    const quality = preferences.previewQuality;
    if (quality === "auto") {
      const pixelRatio = window.devicePixelRatio || 1;
      return { width: displaySize.width * pixelRatio, height: displaySize.height * pixelRatio };
    }
    const factor = QUALITY_FACTORS[quality];
    return { width: view.settings.canvasWidth * factor, height: view.settings.canvasHeight * factor };
  });

  // What the picture must show; a change asks for a new frame.
  const wantedFrame = $derived({
    revision: view.revision,
    filePath: view.filePath,
    timeTicks: Math.min(editor.playheadTicks, Math.max(0, view.durationTicks - 1)),
    width: Math.round(requestedSize.width),
    height: Math.round(requestedSize.height),
  });

  let isRendering = false;
  let isNewerFrameWanted = false;

  $effect(() => {
    if (hasContent && wantedFrame.width >= 16 && wantedFrame.height >= 16) {
      void renderWantedFrame();
    }
  });

  async function renderWantedFrame(): Promise<void> {
    if (isRendering) {
      isNewerFrameWanted = true;
      return;
    }
    isRendering = true;
    do {
      isNewerFrameWanted = false;
      const wanted = wantedFrame;
      try {
        const frame = await backend.renderPreviewFrame(wanted.timeTicks, wanted.width, wanted.height);
        renderer?.draw(frame);
        hasFrame = true;
        failure = null;
      } catch (error) {
        if (!isSupersededError(error)) {
          failure = describeError(error).details;
        }
      }
    } while (isNewerFrameWanted);
    isRendering = false;
  }

  onMount(() => {
    if (canvas) {
      renderer = createFrameRenderer(canvas);
    }
  });

  onDestroy(() => renderer?.dispose());

  function changeQuality(event: Event): void {
    preferences.update({ previewQuality: (event.currentTarget as HTMLSelectElement).value as PreviewQuality });
  }
</script>

<section class="preview" aria-label={t("preview.quality")}>
  <div class="stage" bind:clientWidth={stageWidth} bind:clientHeight={stageHeight}>
    <div
      class="picture"
      class:hidden={!hasContent}
      style:width="{displaySize.width}px"
      style:height="{displaySize.height}px"
      style:background={view.settings.backgroundColor.slice(0, 7)}
    >
      <canvas bind:this={canvas} class:visible={hasFrame}></canvas>
      {#if failure}
        <div class="failure" title={failure}>
          <Icon name="alert" size={24} />
          <span>{t("preview.failed")}</span>
        </div>
      {/if}
    </div>
    {#if !hasContent}
      <div
        class="empty"
        style:width="{displaySize.width}px"
        style:height="{displaySize.height}px"
      >
        <Icon name="film" size={32} />
        <span>{t("preview.empty")}</span>
      </div>
    {/if}
    <ToastStack />
  </div>

  <div class="transport">
    <div class="timecode tabular" title={t("transport.timecode")}>
      {formatTimecode(editor.playheadTicks, editor.frameRate)}
      <small>/ {formatTimecode(view.durationTicks, editor.frameRate)}</small>
    </div>
    <div class="buttons">
      <IconButton label={t("transport.start")} shortcut="Home" disabled={!hasContent} onclick={() => playback.seek(0)}>
        <Icon name="skipBack" />
      </IconButton>
      <IconButton label={t("transport.previousFrame")} shortcut="←" disabled={!hasContent} onclick={() => playback.stepFrames(-1)}>
        <Icon name="chevronLeft" />
      </IconButton>
      <button
        type="button"
        class="play"
        disabled={!hasContent}
        title="{playback.isPlaying ? t('transport.pause') : t('transport.play')} (Space)"
        aria-label={playback.isPlaying ? t("transport.pause") : t("transport.play")}
        onclick={() => playback.toggle()}
      >
        <Icon name={playback.isPlaying || playback.isStarting ? "pause" : "play"} size={18} filled />
      </button>
      <IconButton label={t("transport.nextFrame")} shortcut="→" disabled={!hasContent} onclick={() => playback.stepFrames(1)}>
        <Icon name="chevronRight" />
      </IconButton>
      <IconButton
        label={t("transport.end")}
        shortcut="End"
        disabled={!hasContent}
        onclick={() => playback.seek(view.durationTicks)}
      >
        <Icon name="skipForward" />
      </IconButton>
    </div>
    <div class="right">
      {#if playback.isPlaying && Math.abs(playback.speed) !== 1}
        <span class="speed tabular">{playback.speed > 0 ? "" : "−"}{Math.abs(playback.speed)}×</span>
      {/if}
      <select aria-label={t("preview.quality")} value={preferences.previewQuality} onchange={changeQuality}>
        <option value="auto">{t("preview.quality.auto")}</option>
        <option value="full">{t("preview.quality.full")}</option>
        <option value="half">{t("preview.quality.half")}</option>
        <option value="quarter">{t("preview.quality.quarter")}</option>
      </select>
    </div>
  </div>
</section>

<style>
  .preview {
    grid-area: preview;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--color-bg-inset);
  }
  .stage {
    position: relative;
    flex: 1;
    min-height: 0;
    display: grid;
    place-items: center;
    overflow: hidden;
  }
  .picture {
    position: relative;
    box-shadow: 0 0 0 1px var(--color-border);
    overflow: hidden;
  }
  .picture.hidden {
    display: none;
  }
  canvas {
    display: block;
    width: 100%;
    height: 100%;
    visibility: hidden;
  }
  canvas.visible {
    visibility: visible;
  }
  .failure {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    background: rgba(0, 0, 0, 0.6);
    color: #ffffff;
    text-align: center;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    border: 1px dashed var(--color-border-strong);
    border-radius: var(--radius-md);
    color: var(--color-text-muted);
    text-align: center;
  }
  .transport {
    flex: none;
    height: 48px;
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    padding: 0 var(--space-3);
    background: var(--color-bg-panel);
    border-top: 1px solid var(--color-border);
  }
  .timecode {
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-medium);
    white-space: nowrap;
  }
  .timecode small {
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-regular);
  }
  .buttons {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .play {
    width: 38px;
    height: 38px;
    margin: 0 var(--space-2);
    border: 0;
    border-radius: 50%;
    background: var(--color-text);
    color: var(--color-bg-panel);
    display: grid;
    place-items: center;
    cursor: pointer;
  }
  .play:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .right {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: var(--space-2);
  }
  .speed {
    font-size: var(--font-size-sm);
    color: var(--color-accent-text);
    font-weight: var(--font-weight-semibold);
  }
  select {
    height: 26px;
    padding: 0 var(--space-2);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    background: var(--color-bg-inset);
    font-size: var(--font-size-sm);
  }
</style>
