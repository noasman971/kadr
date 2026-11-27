<!--
  One clip on the timeline: its kind's colour and icon, its name, badges for a
  changed speed or volume, thumbnails along pictures, a waveform along sound, its
  keyframes as diamonds along the bottom (click: go there; drag: move every
  keyframe at that moment), and trim handles on its edges when it is selected.
  Never colour alone: every clip has an icon and a name (docs/ux.md).
-->
<script lang="ts">
  import type { ClipValueName, ClipView, MediaView } from "../../api/types";
  import { t, translations } from "../../i18n/i18n.svelte";
  import { keyframeOffsetsOf } from "../../keyframes/keyframes";
  import { pixelsToTicks, ticksToPixels } from "../../timeline/timelineGeometry";
  import { pointerOffsetX } from "../../ui/pointer";
  import Icon from "../../ui/Icon.svelte";
  import type { IconName } from "../../ui/icons";
  import { claudeActivity } from "../../state/claude.svelte";
  import { editor } from "../../state/editor.svelte";
  import { playback } from "../../state/playback.svelte";
  import { formatTimecode, snapToFrame, ticksPerFrame } from "../../time/ticks";
  import ClipFilmstrip from "./ClipFilmstrip.svelte";
  import ClipWaveform from "./ClipWaveform.svelte";

  interface Props {
    clip: ClipView;
    media: MediaView | undefined;
    pixelsPerSecond: number;
    heightInPixels: number;
    isSelected: boolean;
    isGhost?: boolean;
    /** Visible horizontal range of the timeline content, in pixels. */
    visibleStartInPixels: number;
    visibleWidthInPixels: number;
    onBodyPointerDown?: (event: PointerEvent, clip: ClipView) => void;
    onEdgePointerDown?: (event: PointerEvent, clip: ClipView, edge: "start" | "end") => void;
  }

  let {
    clip,
    media,
    pixelsPerSecond,
    heightInPixels,
    isSelected,
    isGhost = false,
    visibleStartInPixels,
    visibleWidthInPixels,
    onBodyPointerDown,
    onEdgePointerDown,
  }: Props = $props();

  const ICONS: Record<ClipView["kind"], IconName> = { video: "film", audio: "music", image: "image", text: "type" };
  /** Height of the label row and of the waveform under pictures. */
  const LABEL_HEIGHT_IN_PIXELS = 18;
  const WAVEFORM_UNDER_PICTURE_HEIGHT_IN_PIXELS = 14;

  const left = $derived(ticksToPixels(clip.startTicks, pixelsPerSecond));
  const width = $derived(Math.max(2, ticksToPixels(clip.durationTicks, pixelsPerSecond)));
  // The part of the clip on screen, in pixels from its left edge.
  const visibleStart = $derived(Math.max(0, visibleStartInPixels - left));
  const visibleEnd = $derived(Math.min(width, visibleStartInPixels + visibleWidthInPixels - left));
  const isOnScreen = $derived(visibleEnd > visibleStart);
  const innerHeight = $derived(heightInPixels - 6);
  const showsPictures = $derived(
    !isGhost && media !== undefined && !media.isMissing && media.hasPicture && (clip.kind === "video" || clip.kind === "image"),
  );
  const showsWaveform = $derived(!isGhost && media !== undefined && !media.isMissing && clip.hasSound);
  const waveformHeight = $derived(
    clip.kind === "audio" ? innerHeight - LABEL_HEIGHT_IN_PIXELS : WAVEFORM_UNDER_PICTURE_HEIGHT_IN_PIXELS,
  );
  const filmstripHeight = $derived(
    innerHeight - LABEL_HEIGHT_IN_PIXELS - (showsWaveform ? WAVEFORM_UNDER_PICTURE_HEIGHT_IN_PIXELS : 0),
  );
  /** The stretches where a blur's tracker was unsure, in pixels from the clip's left edge. */
  const unsureBands = $derived(
    clip.trackedBlurs
      .filter((blur) => blur.isEnabled)
      .flatMap((blur) => blur.unsureRanges)
      .map((range) => ({
        range,
        left: ticksToPixels(range.startTicks - clip.startTicks, pixelsPerSecond),
        width: Math.max(3, ticksToPixels(range.endTicks - range.startTicks, pixelsPerSecond)),
      })),
  );

  /** A click on a band puts the playhead in its middle, to look at it and correct it. */
  function lookAtRange(event: PointerEvent, range: { startTicks: number; endTicks: number }): void {
    event.stopPropagation();
    editor.selectedClipIds = [clip.id];
    playback.seek(Math.floor((range.startTicks + range.endTicks) / 2));
  }

  /** The animated values, in the inspector's order, with the names it shows. */
  const ANIMATED_VALUES: Array<{ value: ClipValueName; label: "inspector.positionX" | "inspector.positionY" | "inspector.scale" | "inspector.rotation" | "inspector.opacity" | "inspector.volume" }> = [
    { value: "positionX", label: "inspector.positionX" },
    { value: "positionY", label: "inspector.positionY" },
    { value: "scale", label: "inspector.scale" },
    { value: "rotation", label: "inspector.rotation" },
    { value: "opacity", label: "inspector.opacity" },
    { value: "volume", label: "inspector.volume" },
  ];
  /** Below this many pixels of movement, a press on a mark is a click. */
  const DRAG_THRESHOLD_IN_PIXELS = 3;
  /** A mark is drawn only when it has this much room from the previous one. */
  const MINIMUM_MARK_SPACING_IN_PIXELS = 4;
  /** Half a mark's width, so a mark on the clip's edge is not cut. */
  const MARK_HALF_WIDTH_IN_PIXELS = 6;

  let clipElement: HTMLDivElement | undefined = $state();
  /** The keyframe mark being dragged, and where it would land (from the clip's start). */
  let draggedMark = $state<{ fromOffsetTicks: number; toOffsetTicks: number; isRefused: boolean } | null>(null);

  const keyframeOffsets = $derived(keyframeOffsetsOf(ANIMATED_VALUES.map(({ value }) => clip[value])));
  /** Keyframe marks on screen, with the names of the values keyed there. */
  const keyframeMarks = $derived.by(() => {
    const marks: Array<{ offsetTicks: number; left: number; names: string[] }> = [];
    for (const offsetTicks of keyframeOffsets) {
      const markLeft = markPosition(offsetTicks);
      const previous = marks.at(-1);
      const isCrowded = previous !== undefined && markLeft - previous.left < MINIMUM_MARK_SPACING_IN_PIXELS;
      if (markLeft < visibleStart - 8 || markLeft > visibleEnd + 8 || isCrowded) {
        continue;
      }
      const names = ANIMATED_VALUES.filter(({ value }) => clip[value].keyframes.some((keyframe) => keyframe.offsetTicks === offsetTicks)).map(
        ({ label }) => t(label),
      );
      marks.push({ offsetTicks, left: markLeft, names });
    }
    return marks;
  });

  /** Where a mark is drawn, kept whole inside the clip's edges. */
  function markPosition(offsetTicks: number): number {
    const position = ticksToPixels(Math.min(offsetTicks, clip.durationTicks), pixelsPerSecond);
    return Math.min(Math.max(position, MARK_HALF_WIDTH_IN_PIXELS), width - MARK_HALF_WIDTH_IN_PIXELS);
  }

  /** The moment under a pointer, from the clip's start, on a frame of the timeline. */
  function markOffsetAtPointer(clientX: number): number {
    if (!clipElement) {
      return 0;
    }
    const offset = pixelsToTicks(pointerOffsetX(clientX, clipElement), pixelsPerSecond);
    const snapped = snapToFrame(clip.startTicks + Math.max(0, offset), editor.frameRate) - clip.startTicks;
    return Math.min(Math.max(0, clip.durationTicks - ticksPerFrame(editor.frameRate)), Math.max(0, snapped));
  }

  /** A click goes to the keyframes' moment; a drag moves them all (one undo step). */
  function startMarkDrag(event: PointerEvent, fromOffsetTicks: number): void {
    if (event.button !== 0 || isGhost) {
      return;
    }
    event.stopPropagation();
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture(event.pointerId);
    const clipId = clip.id;
    const startX = event.clientX;
    let hasMoved = false;
    const move = (moveEvent: PointerEvent) => {
      hasMoved ||= Math.abs(moveEvent.clientX - startX) >= DRAG_THRESHOLD_IN_PIXELS;
      if (hasMoved) {
        const toOffsetTicks = markOffsetAtPointer(moveEvent.clientX);
        const isRefused = keyframeOffsets.some((offset) => offset === toOffsetTicks && offset !== fromOffsetTicks);
        draggedMark = { fromOffsetTicks, toOffsetTicks, isRefused };
      }
    };
    const end = () => {
      target.removeEventListener("pointermove", move);
      const landing = draggedMark;
      draggedMark = null;
      editor.selectedClipIds = [clipId];
      if (!hasMoved) {
        playback.seek(clip.startTicks + fromOffsetTicks);
      } else if (landing && !landing.isRefused && landing.toOffsetTicks !== fromOffsetTicks) {
        const toOffsetTicks = landing.toOffsetTicks;
        void editor
          .edit({ type: "moveKeyframe", clipId, values: [], fromOffsetTicks, toOffsetTicks })
          .then(() => playback.seek(clip.startTicks + toOffsetTicks));
      }
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", end, { once: true });
    target.addEventListener(
      "pointercancel",
      () => {
        target.removeEventListener("pointermove", move);
        draggedMark = null;
      },
      { once: true },
    );
  }

  const speedBadge = $derived(clip.speed !== 1 ? `${translations.formatNumber(clip.speed, 2)}Ã—` : null);
  const volumeBadge = $derived(
    clip.hasSound && clip.volume.keyframes.length === 0 && Math.abs(clip.volume.value - 1) > 0.005
      ? `${Math.round(clip.volume.value * 100)} %`
      : null,
  );
</script>

<div
  class="clip {clip.kind}"
  class:selected={isSelected}
  class:ghost={isGhost}
  class:changed-by-claude={claudeActivity.highlightedClipIds.includes(clip.id)}
  bind:this={clipElement}
  style:left="{left}px"
  style:width="{width}px"
  role="button"
  tabindex="-1"
  aria-pressed={isSelected}
  aria-label={clip.name}
  onpointerdown={(event) => onBodyPointerDown?.(event, clip)}
>
  <div class="label" style:padding-left="{Math.max(6, visibleStart + 6)}px">
    <Icon name={ICONS[clip.kind]} size={12} />
    <span class="name">{clip.name}</span>
    {#if speedBadge}<span class="badge">{speedBadge}</span>{/if}
    {#if volumeBadge}<span class="badge">{volumeBadge}</span>{/if}
    {#if clip.trackedBlurs.length > 0}
      <span class="badge blurred" title={t("blur.clipBadge")} aria-label={t("blur.clipBadge")}><Icon name="scanFace" size={11} /></span>
    {/if}
  </div>
  {#if isOnScreen && showsPictures && media}
    <div class="pictures" style:height="{filmstripHeight}px">
      <ClipFilmstrip
        mediaId={media.id}
        filePath={media.filePath}
        isStill={clip.kind === "image"}
        sourceStartTicks={clip.sourceStartTicks ?? 0}
        sourceEndTicks={clip.sourceEndTicks ?? 0}
        speed={clip.speed}
        {pixelsPerSecond}
        aspectRatio={(media.width ?? 16) / (media.height ?? 9)}
        heightInPixels={filmstripHeight}
        visibleStartInPixels={visibleStart}
        visibleWidthInPixels={visibleEnd - visibleStart}
      />
    </div>
  {/if}
  {#if isOnScreen && showsWaveform && media}
    <div class="sound" style:height="{waveformHeight}px">
      <ClipWaveform
        mediaId={media.id}
        filePath={media.filePath}
        sourceStartTicks={clip.sourceStartTicks ?? 0}
        speed={clip.speed}
        {pixelsPerSecond}
        visibleStartInPixels={visibleStart}
        visibleWidthInPixels={visibleEnd - visibleStart}
        heightInPixels={waveformHeight}
      />
    </div>
  {/if}
  {#each unsureBands as band (band.range.startTicks)}
    {@const description = t(band.range.confidence === "lost" ? "blur.rangeLost" : "blur.rangeUncertain", {
      start: formatTimecode(band.range.startTicks, editor.frameRate),
      end: formatTimecode(band.range.endTicks, editor.frameRate),
    })}
    <button
      type="button"
      class="unsure-band {band.range.confidence}"
      style:left="{band.left}px"
      style:width="{band.width}px"
      title={description}
      aria-label={description}
      onpointerdown={(event) => lookAtRange(event, band.range)}
    >
      {#if band.width >= 16}<Icon name="alert" size={10} />{/if}
    </button>
  {/each}
  {#if !isGhost}
    {#each keyframeMarks as mark (mark.offsetTicks)}
      {@const isDragged = draggedMark?.fromOffsetTicks === mark.offsetTicks}
      {@const description = t("keyframes.timelineMark", {
        time: formatTimecode(clip.startTicks + mark.offsetTicks, editor.frameRate),
        names: mark.names.join(", "),
      })}
      <button
        type="button"
        class="keyframe-mark"
        class:dragged={isDragged}
        class:refused={isDragged && draggedMark?.isRefused}
        class:current={clip.startTicks + mark.offsetTicks === editor.playheadTicks}
        style:left="{isDragged && draggedMark ? markPosition(draggedMark.toOffsetTicks) : mark.left}px"
        title={isDragged && draggedMark?.isRefused ? t("keyframes.cannotLandOnAnother") : description}
        aria-label={description}
        tabindex="-1"
        onpointerdown={(event) => startMarkDrag(event, mark.offsetTicks)}
      ></button>
    {/each}
  {/if}
  {#if isSelected && !isGhost}
    <span
      class="handle start"
      role="presentation"
      onpointerdown={(event) => {
        event.stopPropagation();
        onEdgePointerDown?.(event, clip, "start");
      }}
    ></span>
    <span
      class="handle end"
      role="presentation"
      onpointerdown={(event) => {
        event.stopPropagation();
        onEdgePointerDown?.(event, clip, "end");
      }}
    ></span>
  {/if}
</div>

<style>
  .clip {
    position: absolute;
    top: 3px;
    bottom: 3px;
    display: flex;
    flex-direction: column;
    border: 1px solid;
    border-radius: var(--radius-md);
    overflow: hidden;
    color: #ffffff;
    cursor: grab;
    touch-action: none;
  }
  .clip.video {
    background: var(--color-clip-video);
    border-color: var(--color-clip-video-edge);
  }
  .clip.audio {
    background: var(--color-clip-audio);
    border-color: var(--color-clip-audio-edge);
  }
  .clip.text {
    background: var(--color-clip-text);
    border-color: var(--color-clip-text-edge);
  }
  .clip.image {
    background: var(--color-clip-image);
    border-color: var(--color-clip-image-edge);
  }
  .clip.selected {
    box-shadow:
      0 0 0 2px var(--color-text),
      0 0 0 4px var(--color-accent);
    z-index: 1;
  }
  /* Just changed by Claude: an outline that fades, on top of the selection. */
  .clip.changed-by-claude {
    animation: changed-by-claude 2.5s var(--easing-standard);
  }
  @keyframes changed-by-claude {
    0%,
    60% {
      box-shadow:
        0 0 0 2px var(--color-text),
        0 0 0 5px var(--color-success);
    }
    100% {
      box-shadow: none;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .clip.changed-by-claude {
      animation: none;
      box-shadow:
        0 0 0 2px var(--color-text),
        0 0 0 5px var(--color-success);
    }
  }
  .clip.ghost {
    opacity: 0.4;
    pointer-events: none;
  }
  .label {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 18px;
    padding-right: 6px;
    flex: none;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    text-shadow: 0 1px 2px rgba(0, 0, 0, 0.5);
    white-space: nowrap;
    overflow: hidden;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .badge {
    flex: none;
    height: 15px;
    padding: 0 4px;
    border-radius: var(--radius-sm);
    background: rgba(0, 0, 0, 0.35);
    line-height: 15px;
  }
  .badge.blurred {
    display: grid;
    place-items: center;
  }
  /* Never colour alone: the stripes carry a warning icon and a tooltip. */
  .unsure-band {
    position: absolute;
    top: 18px;
    height: 12px;
    padding: 0 2px;
    border: 0;
    border-radius: 0 0 var(--radius-sm) var(--radius-sm);
    display: flex;
    align-items: center;
    color: var(--color-bg-inset);
    cursor: pointer;
    z-index: 1;
  }
  .unsure-band.uncertain {
    background: repeating-linear-gradient(135deg, var(--color-over-video-unsure) 0 4px, transparent 4px 7px),
      var(--color-warning-soft);
  }
  .unsure-band.lost {
    background: repeating-linear-gradient(135deg, var(--color-over-video-lost) 0 4px, transparent 4px 7px),
      var(--color-danger-soft);
  }
  .unsure-band :global(svg) {
    color: var(--color-over-video);
    filter: drop-shadow(0 0 1px var(--color-over-video-shadow));
  }
  /* Keyframes: diamonds along the bottom edge, above the pictures. */
  .keyframe-mark {
    position: absolute;
    bottom: 3px;
    width: 9px;
    height: 9px;
    margin-left: -4.5px;
    padding: 0;
    border: 1.5px solid var(--color-over-video);
    border-radius: 2px;
    background: var(--color-accent);
    box-shadow: 0 0 2px var(--color-over-video-shadow);
    transform: rotate(45deg);
    cursor: ew-resize;
    z-index: 2;
  }
  .keyframe-mark.current {
    background: var(--color-over-video);
  }
  .keyframe-mark.dragged {
    z-index: 3;
  }
  .keyframe-mark.refused {
    border-color: var(--color-danger);
    background: var(--color-danger-soft);
  }
  .pictures,
  .sound {
    position: relative;
    flex: none;
    overflow: hidden;
  }
  .handle {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 8px;
    background: rgba(255, 255, 255, 0.9);
    cursor: ew-resize;
    z-index: 2;
  }
  .handle::after {
    content: "";
    position: absolute;
    top: 50%;
    left: 3px;
    width: 2px;
    height: 14px;
    margin-top: -7px;
    border-radius: 1px;
    background: rgba(0, 0, 0, 0.5);
  }
  .handle.start {
    left: 0;
  }
  .handle.end {
    right: 0;
  }
</style>
