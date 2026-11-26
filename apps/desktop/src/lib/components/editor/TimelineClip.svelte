<!--
  One clip on the timeline: its kind's colour and icon, its name, badges for a
  changed speed or volume, thumbnails along pictures, a waveform along sound, and
  trim handles on its edges when it is selected. Never colour alone: every clip
  has an icon and a name (docs/ux.md).
-->
<script lang="ts">
  import type { ClipView, MediaView } from "../../api/types";
  import { t, translations } from "../../i18n/i18n.svelte";
  import { ticksToPixels } from "../../timeline/timelineGeometry";
  import Icon from "../../ui/Icon.svelte";
  import type { IconName } from "../../ui/icons";
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
  const speedBadge = $derived(clip.speed !== 1 ? `${translations.formatNumber(clip.speed, 2)}×` : null);
  const volumeBadge = $derived(
    clip.hasSound && clip.volume.keyframeCount === 0 && Math.abs(clip.volume.value - 1) > 0.005
      ? `${Math.round(clip.volume.value * 100)} %`
      : null,
  );
</script>

<div
  class="clip {clip.kind}"
  class:selected={isSelected}
  class:ghost={isGhost}
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
