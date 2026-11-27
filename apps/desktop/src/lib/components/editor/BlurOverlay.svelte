<!--
  Drawn over the preview picture (docs/ux.md, "Blurring a face"):

  - with the blur tool on, the box being drawn or adjusted, with its handles,
    and a bar to confirm or cancel; then the analysis' progress;
  - otherwise, for the selected clip, each tracked blur's box at the playhead:
    solid where the tracker was sure, dashed with ⚠ where it was not. Dragging
    a box starts correcting that blur from this frame (docs/ux.md, "Check and
    correct").

  Boxes live in the source video's pixels; they are drawn through the clip's
  mapping (`blur/sourceMapping.ts`), so they sit on what the export shows.
  Nothing here is ever exported.
-->
<script lang="ts">
  import type { ClipOverlay, ProjectView, SourceRegion } from "../../api/types";
  import { findClip } from "../../blur/blurTarget";
  import {
    HANDLE_NAMES,
    applyMatrix,
    handlePosition,
    invertMatrix,
    movedRegion,
    regionBetween,
    regionCorners,
    resizedRegion,
    sourceToDisplayMatrix,
    type AffineMatrix,
    type HandleName,
    type Point,
  } from "../../blur/sourceMapping";
  import { t, translations } from "../../i18n/i18n.svelte";
  import { backend } from "../../api/backend";
  import { blurTool } from "../../state/blurTool.svelte";
  import { editor } from "../../state/editor.svelte";
  import { playback } from "../../state/playback.svelte";
  import Button from "../../ui/Button.svelte";
  import Icon from "../../ui/Icon.svelte";
  import { pointerOffsetX, pointerOffsetY } from "../../ui/pointer";

  interface Props {
    view: ProjectView;
    /** Displayed size of the picture, in CSS pixels. */
    displayWidth: number;
    displayHeight: number;
  }

  let { view, displayWidth, displayHeight }: Props = $props();

  /** A drag shorter than this (display pixels) is a click, not a box. */
  const MINIMUM_DRAWN_SIDE_IN_PIXELS = 4;
  const HANDLE_SIZE_IN_PIXELS = 10;

  let surface: SVGSVGElement | undefined = $state();
  let overlay = $state<ClipOverlay | null>(null);

  /** The clip whose boxes are shown: the tool's, or the selected clip with blurs. */
  const overlayClipId = $derived.by(() => {
    if (blurTool.phase !== "off") {
      return blurTool.clipId;
    }
    if (editor.selectedClipIds.length !== 1) {
      return null;
    }
    const clip = findClip(view, editor.selectedClipIds[0] ?? -1);
    return clip && clip.trackedBlurs.length > 0 ? clip.id : null;
  });

  // The overlay of that clip under the playhead, fetched when anything changes.
  // Not while playing: the boxes would lag behind the picture.
  $effect(() => {
    const clipId = overlayClipId;
    const timeTicks = editor.playheadTicks;
    const revision = view.revision;
    if (clipId === null || playback.isPlaying) {
      overlay = null;
      blurTool.overlay = null;
      return;
    }
    let isCurrent = true;
    void backend
      .getClipOverlay(clipId, timeTicks)
      .then((fetched) => {
        if (isCurrent && revision === view.revision) {
          overlay = fetched;
          if (blurTool.clipId === clipId) {
            blurTool.overlay = fetched;
          }
        }
      })
      .catch(() => {
        // The clip is gone (undone, deleted): leave the tool.
        if (isCurrent) {
          overlay = null;
          blurTool.cancel();
        }
      });
    return () => {
      isCurrent = false;
    };
  });

  const displayScale = $derived(view.settings.canvasWidth > 0 ? displayWidth / view.settings.canvasWidth : 0);
  const displayMatrix = $derived<AffineMatrix | null>(
    overlay ? sourceToDisplayMatrix(overlay.sourceToCanvas, displayScale) : null,
  );
  const inverseMatrix = $derived(displayMatrix ? invertMatrix(displayMatrix) : null);
  const bounds = $derived<[number, number, number, number]>(overlay?.visibleSource ?? [0, 0, 0, 0]);

  function pointsOf(region: SourceRegion): string {
    if (!displayMatrix) {
      return "";
    }
    return regionCorners(region, displayMatrix)
      .map((corner) => `${corner.x},${corner.y}`)
      .join(" ");
  }

  function matrixAttribute(matrix: AffineMatrix): string {
    return `matrix(${matrix.join(" ")})`;
  }

  // ----- Drawing, moving and resizing the tool's box -----

  type Drag =
    | { kind: "draw"; start: Point }
    | { kind: "move"; start: Point; original: SourceRegion }
    | { kind: "resize"; handle: HandleName; original: SourceRegion };

  let drag: Drag | null = null;

  function sourcePointOf(event: PointerEvent): Point | null {
    if (!surface || !inverseMatrix) {
      return null;
    }
    const displayPoint = { x: pointerOffsetX(event.clientX, surface), y: pointerOffsetY(event.clientY, surface) };
    return applyMatrix(inverseMatrix, displayPoint);
  }

  function isInside(point: Point, region: SourceRegion): boolean {
    return (
      point.x >= region.left &&
      point.x <= region.left + region.width &&
      point.y >= region.top &&
      point.y <= region.top + region.height
    );
  }

  /** Dragging an existing blur's box starts correcting it, the drag moving the box. */
  function startCorrectionFrom(event: PointerEvent): boolean {
    const effectId = Number((event.target as Element | null)?.getAttribute("data-effect-id") ?? NaN);
    const box = overlay?.blurBoxes.find((candidate) => candidate.effectId === effectId);
    if (!overlay || !box || blurTool.phase !== "off") {
      return false;
    }
    blurTool.startCorrection(overlay.clipId, box.effectId, box.region, box.shape);
    blurTool.overlay = overlay;
    return true;
  }

  function handlePointerDown(event: PointerEvent): void {
    if (event.button !== 0 || !(blurTool.isActive || startCorrectionFrom(event))) {
      return;
    }
    const point = sourcePointOf(event);
    if (!point) {
      return;
    }
    const handle = (event.target as Element | null)?.getAttribute("data-handle") as HandleName | null;
    const region = blurTool.region;
    if (region && handle) {
      drag = { kind: "resize", handle, original: region };
    } else if (region && isInside(point, region)) {
      drag = { kind: "move", start: point, original: region };
    } else {
      drag = { kind: "draw", start: point };
    }
    surface?.setPointerCapture(event.pointerId);
    event.preventDefault();
  }

  function handlePointerMove(event: PointerEvent): void {
    const point = drag ? sourcePointOf(event) : null;
    if (!drag || !point) {
      return;
    }
    if (drag.kind === "draw") {
      blurTool.region = regionBetween(drag.start, point, bounds);
    } else if (drag.kind === "move") {
      blurTool.region = movedRegion(drag.original, { x: point.x - drag.start.x, y: point.y - drag.start.y }, bounds);
    } else {
      blurTool.region = resizedRegion(drag.original, drag.handle, point, bounds);
    }
  }

  function handlePointerUp(event: PointerEvent): void {
    if (!drag) {
      return;
    }
    surface?.releasePointerCapture(event.pointerId);
    const wasDrawing = drag.kind === "draw";
    drag = null;
    const region = blurTool.region;
    if (!region) {
      return;
    }
    const drawnSize = displayMatrix ? regionDisplaySize(region, displayMatrix) : 0;
    if (wasDrawing && drawnSize < MINIMUM_DRAWN_SIDE_IN_PIXELS) {
      // A click, not a box.
      blurTool.region = null;
      return;
    }
    blurTool.setRegion(region);
  }

  /** The smaller side of `region` on screen, in display pixels. */
  function regionDisplaySize(region: SourceRegion, matrix: AffineMatrix): number {
    const [topLeft, topRight, , bottomLeft] = regionCorners(region, matrix);
    if (!topLeft || !topRight || !bottomLeft) {
      return 0;
    }
    return Math.min(
      Math.hypot(topRight.x - topLeft.x, topRight.y - topLeft.y),
      Math.hypot(bottomLeft.x - topLeft.x, bottomLeft.y - topLeft.y),
    );
  }

  function handleDisplayPosition(region: SourceRegion, handle: HandleName): Point {
    return displayMatrix ? applyMatrix(displayMatrix, handlePosition(region, handle)) : { x: 0, y: 0 };
  }

  const secondsLeftText = $derived(
    blurTool.secondsLeft === null ? "" : t("blur.secondsLeft", { seconds: translations.formatNumber(Math.ceil(blurTool.secondsLeft), 0) }),
  );
  const isPlayheadOutsideClip = $derived(blurTool.isActive && overlay === null && !playback.isPlaying);
  const isCorrecting = $derived(blurTool.correctedEffectId !== null);
</script>

<svg
  bind:this={surface}
  class="surface"
  class:drawing={blurTool.isActive}
  width={displayWidth}
  height={displayHeight}
  viewBox="0 0 {displayWidth} {displayHeight}"
  role="presentation"
  onpointerdown={handlePointerDown}
  onpointermove={handlePointerMove}
  onpointerup={handlePointerUp}
  onpointercancel={handlePointerUp}
>
  {#if overlay && displayMatrix}
    {#each overlay.blurBoxes.filter((box) => box.effectId !== blurTool.correctedEffectId) as box (box.effectId)}
      {@const isSure = box.confidence === "confident"}
      <g class="blur-box" class:unsure={box.confidence === "uncertain"} class:lost={box.confidence === "lost"} class:off={!box.isEnabled}>
        {#if box.shape === "ellipse"}
          <g transform={matrixAttribute(displayMatrix)}>
            <ellipse
              class="shadow"
              cx={box.region.left + box.region.width / 2}
              cy={box.region.top + box.region.height / 2}
              rx={box.region.width / 2}
              ry={box.region.height / 2}
            />
            <ellipse
              class="line"
              class:dashed={!isSure}
              cx={box.region.left + box.region.width / 2}
              cy={box.region.top + box.region.height / 2}
              rx={box.region.width / 2}
              ry={box.region.height / 2}
            />
          </g>
        {:else}
          <polygon class="shadow" points={pointsOf(box.region)} />
          <polygon class="line" class:dashed={!isSure} points={pointsOf(box.region)} />
        {/if}
        {#if !isSure}
          {@const corner = applyMatrix(displayMatrix, { x: box.region.left, y: box.region.top })}
          <text class="warning-mark" x={corner.x + 4} y={corner.y - 6}>⚠</text>
        {/if}
        {#if blurTool.phase === "off" && box.isEnabled}
          <polygon class="grab" data-effect-id={box.effectId} points={pointsOf(box.region)}>
            <title>{t("blur.dragToCorrect")}</title>
          </polygon>
        {/if}
      </g>
    {/each}

    {#if blurTool.phase !== "off" && blurTool.region}
      {@const region = blurTool.region}
      <g class="tool-box" class:too-small={!blurTool.isRegionLargeEnough}>
        {#if blurTool.shape === "ellipse"}
          <polygon class="frame" points={pointsOf(region)} />
          <g transform={matrixAttribute(displayMatrix)}>
            <ellipse
              class="shadow"
              cx={region.left + region.width / 2}
              cy={region.top + region.height / 2}
              rx={region.width / 2}
              ry={region.height / 2}
            />
            <ellipse
              class="line"
              cx={region.left + region.width / 2}
              cy={region.top + region.height / 2}
              rx={region.width / 2}
              ry={region.height / 2}
            />
          </g>
        {:else}
          <polygon class="shadow" points={pointsOf(region)} />
          <polygon class="line" points={pointsOf(region)} />
        {/if}
        {#if blurTool.phase === "adjusting"}
          {#each HANDLE_NAMES as handle (handle)}
            {@const position = handleDisplayPosition(region, handle)}
            <rect
              class="handle {handle}"
              data-handle={handle}
              x={position.x - HANDLE_SIZE_IN_PIXELS / 2}
              y={position.y - HANDLE_SIZE_IN_PIXELS / 2}
              width={HANDLE_SIZE_IN_PIXELS}
              height={HANDLE_SIZE_IN_PIXELS}
            />
          {/each}
        {/if}
      </g>
    {/if}
  {/if}
</svg>

{#if blurTool.phase !== "off"}
  <div class="tool-bar" role="toolbar" aria-label={t("blur.toolName")}>
    {#if blurTool.phase === "tracking"}
      <Icon name="scanFace" size={16} />
      <span class="message">
        {t("blur.following", { percent: blurTool.percentDone })}
        {#if secondsLeftText}<span class="muted"> · {secondsLeftText}</span>{/if}
      </span>
      <div
        class="progress"
        role="progressbar"
        aria-label={t("blur.followingLabel")}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={blurTool.percentDone}
      >
        <div class="progress-fill" style:width="{blurTool.percentDone}%"></div>
      </div>
      <Button variant="ghost" onclick={() => blurTool.cancelTracking()}>{t("blur.stop")}</Button>
    {:else}
      <Icon name="scanFace" size={16} />
      <span class="message">
        {#if isPlayheadOutsideClip}
          {t("blur.playheadOutside")}
        {:else if blurTool.phase === "drawing"}
          {t("blur.drawHint")}
        {:else if !blurTool.isRegionLargeEnough}
          {t("blur.tooSmall")}
        {:else if isCorrecting}
          {t("blur.correctHint")}
        {:else}
          {t("blur.adjustHint")}
        {/if}
      </span>
      {#if !isCorrecting}
        <div class="segmented" role="group" aria-label={t("blur.shape")}>
          <button type="button" aria-pressed={blurTool.shape === "ellipse"} onclick={() => (blurTool.shape = "ellipse")}>{t("blur.shape.ellipse")}</button>
          <button type="button" aria-pressed={blurTool.shape === "rectangle"} onclick={() => (blurTool.shape = "rectangle")}>{t("blur.shape.rectangle")}</button>
        </div>
      {/if}
      <Button variant="ghost" title="{t('blur.cancel')} (Esc)" onclick={() => blurTool.cancel()}>{t("blur.cancel")}</Button>
      {#if blurTool.phase === "adjusting"}
        <Button
          variant="primary"
          title="{isCorrecting ? t('blur.correct') : t('blur.confirm')} (Enter)"
          disabled={!blurTool.isRegionLargeEnough || isPlayheadOutsideClip}
          onclick={() => void blurTool.confirm()}
        >
          {isCorrecting ? t("blur.correct") : t("blur.confirm")}
        </Button>
      {/if}
    {/if}
  </div>
{/if}

<style>
  .surface {
    position: absolute;
    inset: 0;
    overflow: visible;
    pointer-events: none;
  }
  .surface.drawing {
    pointer-events: auto;
    cursor: crosshair;
    touch-action: none;
  }
  .shadow {
    fill: none;
    stroke: var(--color-over-video-shadow);
    stroke-width: 4;
    vector-effect: non-scaling-stroke;
  }
  .line {
    fill: none;
    stroke: var(--color-over-video);
    stroke-width: 2;
    vector-effect: non-scaling-stroke;
  }
  .line.dashed {
    stroke-dasharray: 6 4;
  }
  .blur-box.unsure .line {
    stroke: var(--color-over-video-unsure);
  }
  .blur-box.lost .line {
    stroke: var(--color-over-video-lost);
  }
  .blur-box.off {
    opacity: 0.45;
  }
  .grab {
    fill: transparent;
    stroke: none;
    pointer-events: all;
    cursor: move;
  }
  .warning-mark {
    fill: var(--color-over-video-unsure);
    font-size: var(--font-size-lg);
    paint-order: stroke;
    stroke: var(--color-over-video-shadow);
    stroke-width: 3;
  }
  .tool-box .frame {
    fill: var(--color-over-video-fill);
    stroke: var(--color-over-video);
    stroke-width: 1;
    stroke-dasharray: 3 3;
    vector-effect: non-scaling-stroke;
  }
  .tool-box .line {
    stroke: var(--color-accent-text);
  }
  .tool-box.too-small .line {
    stroke: var(--color-over-video-unsure);
  }
  .handle {
    fill: var(--color-over-video);
    stroke: var(--color-over-video-shadow);
    stroke-width: 1;
  }
  .handle.nw,
  .handle.se {
    cursor: nwse-resize;
  }
  .handle.ne,
  .handle.sw {
    cursor: nesw-resize;
  }
  .handle.n,
  .handle.s {
    cursor: ns-resize;
  }
  .handle.e,
  .handle.w {
    cursor: ew-resize;
  }
  .tool-bar {
    position: absolute;
    top: var(--space-2);
    left: 50%;
    transform: translateX(-50%);
    max-width: calc(100% - 2 * var(--space-2));
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2) var(--space-1) var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--color-bg-raised);
    box-shadow: var(--shadow-popover);
    color: var(--color-text);
    font-size: var(--font-size-sm);
    white-space: nowrap;
  }
  .message {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .muted {
    color: var(--color-text-secondary);
  }
  .progress {
    width: 120px;
    height: 6px;
    border-radius: var(--radius-full);
    background: var(--color-bg-inset);
    overflow: hidden;
    flex: none;
  }
  .progress-fill {
    height: 100%;
    background: var(--color-accent);
    transition: width var(--duration-normal) var(--easing-standard);
  }
  .segmented {
    display: flex;
    flex: none;
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    overflow: hidden;
  }
  .segmented button {
    min-height: 26px;
    padding: 0 var(--space-2);
    border: 0;
    background: transparent;
    color: var(--color-text-secondary);
    cursor: pointer;
    font-size: var(--font-size-sm);
  }
  .segmented button[aria-pressed="true"] {
    background: var(--color-accent-soft);
    color: var(--color-text);
  }
  @media (prefers-reduced-motion: reduce) {
    .progress-fill {
      transition: none;
    }
  }
</style>
