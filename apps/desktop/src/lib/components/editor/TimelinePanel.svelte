<!--
  The timeline (docs/ux.md, "Cut and arrange"): ruler, track headers, clips and
  the playhead. The scroll area holds everything; the ruler sticks to the top and
  the track headers to the left.
-->
<script lang="ts">
  import type { ClipView, ProjectView, TrackView } from "../../api/types";
  import { t, trackDisplayName } from "../../i18n/i18n.svelte";
  import { editor } from "../../state/editor.svelte";
  import { playback } from "../../state/playback.svelte";
  import {
    DEFAULT_PIXELS_PER_SECOND,
    clampZoom,
    pixelsToTicks,
    ticksToPixels,
    zoomToFit,
  } from "../../timeline/timelineGeometry";
  import { TICKS_PER_SECOND } from "../../time/ticks";
  import Icon from "../../ui/Icon.svelte";
  import IconButton from "../../ui/IconButton.svelte";
  import type { IconName } from "../../ui/icons";
  import TimelineRuler from "./TimelineRuler.svelte";

  interface Props {
    view: ProjectView;
  }

  let { view }: Props = $props();

  /** Room left after the end of the edit, to drop clips there. */
  const TRAILING_SPACE_IN_SECONDS = 30;
  const ZOOM_STEP = 1.25;
  const RULER_HEIGHT_IN_PIXELS = 28;
  const TRACK_HEIGHTS_IN_PIXELS: Record<TrackView["kind"], number> = { video: 68, audio: 52 };
  const CLIP_ICONS: Record<ClipView["kind"], IconName> = { video: "film", audio: "music", image: "image", text: "type" };

  let scroller: HTMLDivElement | undefined = $state();
  let pixelsPerSecond = $state(DEFAULT_PIXELS_PER_SECOND);
  let scrollLeft = $state(0);
  let viewportWidth = $state(0);
  let headerWidth = $state(168);

  const contentWidth = $derived(
    Math.max(
      viewportWidth - headerWidth,
      ticksToPixels(view.durationTicks, pixelsPerSecond) + TRAILING_SPACE_IN_SECONDS * pixelsPerSecond,
    ),
  );
  const playheadX = $derived(ticksToPixels(editor.playheadTicks, pixelsPerSecond));
  const selected = $derived(new Set(editor.selectedClipIds));

  function zoomAround(newPixelsPerSecond: number, anchorX: number): void {
    const clamped = clampZoom(newPixelsPerSecond);
    if (!scroller) {
      pixelsPerSecond = clamped;
      return;
    }
    const anchorTicks = pixelsToTicks(scroller.scrollLeft + anchorX, pixelsPerSecond);
    pixelsPerSecond = clamped;
    // Keep the time under the anchor where it was on screen.
    queueMicrotask(() => {
      if (scroller) {
        scroller.scrollLeft = ticksToPixels(anchorTicks, clamped) - anchorX;
      }
    });
  }

  function zoomBy(factor: number): void {
    zoomAround(pixelsPerSecond * factor, Math.max(0, playheadX - scrollLeft));
  }

  function fitWholeEdit(): void {
    pixelsPerSecond = zoomToFit(view.durationTicks, viewportWidth - headerWidth);
    if (scroller) {
      scroller.scrollLeft = 0;
    }
  }

  function handleWheel(event: WheelEvent): void {
    if (!event.ctrlKey || !scroller) {
      return;
    }
    event.preventDefault();
    const bounds = scroller.getBoundingClientRect();
    zoomAround(pixelsPerSecond * (event.deltaY < 0 ? ZOOM_STEP : 1 / ZOOM_STEP), event.clientX - bounds.left - headerWidth);
  }

  function selectClip(event: MouseEvent, clip: ClipView): void {
    event.stopPropagation();
    if (event.shiftKey) {
      editor.selectedClipIds = selected.has(clip.id)
        ? editor.selectedClipIds.filter((clipId) => clipId !== clip.id)
        : [...editor.selectedClipIds, clip.id];
    } else {
      editor.selectedClipIds = [clip.id];
    }
  }

  // While playing, keep the playhead on screen by turning pages.
  $effect(() => {
    if (!scroller || !playback.isPlaying) {
      return;
    }
    const visibleWidth = viewportWidth - headerWidth;
    if (playheadX < scrollLeft || playheadX > scrollLeft + visibleWidth - 24) {
      scroller.scrollLeft = Math.max(0, playheadX - visibleWidth * 0.1);
    }
  });

  /** Zoom shortcuts (+, −, Shift+Z) are routed here by the editor screen. */
  export function handleZoomShortcut(action: "in" | "out" | "fit"): void {
    if (action === "fit") {
      fitWholeEdit();
    } else {
      zoomBy(action === "in" ? ZOOM_STEP : 1 / ZOOM_STEP);
    }
  }
</script>

<section class="timeline" aria-label={t("timeline.title")}>
  <div class="toolbar">
    <span class="spacer"></span>
    <div class="zoom">
      <IconButton label={t("timeline.zoomOut")} shortcut="−" onclick={() => zoomBy(1 / ZOOM_STEP)}>
        <Icon name="zoomOut" />
      </IconButton>
      <input
        type="range"
        min={Math.log(0.1)}
        max={Math.log(1200)}
        step="0.01"
        aria-label={t("timeline.zoom")}
        value={Math.log(pixelsPerSecond)}
        oninput={(event) => zoomAround(Math.exp(Number(event.currentTarget.value)), Math.max(0, playheadX - scrollLeft))}
      />
      <IconButton label={t("timeline.zoomIn")} shortcut="+" onclick={() => zoomBy(ZOOM_STEP)}>
        <Icon name="zoomIn" />
      </IconButton>
    </div>
    <IconButton label={t("timeline.fit")} shortcut="Shift+Z" onclick={fitWholeEdit}><Icon name="fit" /></IconButton>
  </div>

  <div
    class="scroller"
    bind:this={scroller}
    bind:clientWidth={viewportWidth}
    onscroll={() => (scrollLeft = scroller?.scrollLeft ?? 0)}
    onwheel={handleWheel}
  >
    <div
      class="grid"
      style:grid-template-columns="var(--track-header-width) {contentWidth}px"
      style:grid-template-rows="{RULER_HEIGHT_IN_PIXELS}px repeat({view.tracks.length}, auto) 1fr"
    >
      <div class="corner" bind:clientWidth={headerWidth}></div>
      <div class="ruler-cell">
        <TimelineRuler
          {pixelsPerSecond}
          widthInPixels={contentWidth}
          visibleStartInPixels={scrollLeft}
          visibleWidthInPixels={viewportWidth}
          onSeek={(ticks) => playback.seek(ticks)}
        />
        <div class="playhead-head" style:left="{playheadX}px"></div>
      </div>

      {#each view.tracks as track (track.id)}
        <div class="track-header" style:height="{TRACK_HEIGHTS_IN_PIXELS[track.kind]}px">
          <Icon name={track.kind === "video" ? "film" : "music"} size={14} />
          <span class="track-name" title={trackDisplayName(track.name)}>{trackDisplayName(track.name)}</span>
        </div>
        <div
          class="lane {track.kind}"
          role="presentation"
          style:height="{TRACK_HEIGHTS_IN_PIXELS[track.kind]}px"
          onclick={() => (editor.selectedClipIds = [])}
        >
          {#each track.clips as clip (clip.id)}
            <button
              type="button"
              class="clip {clip.kind}"
              class:selected={selected.has(clip.id)}
              style:left="{ticksToPixels(clip.startTicks, pixelsPerSecond)}px"
              style:width="{Math.max(2, ticksToPixels(clip.durationTicks, pixelsPerSecond))}px"
              title="{clip.name} · {(clip.durationTicks / TICKS_PER_SECOND).toFixed(1)} s"
              aria-pressed={selected.has(clip.id)}
              onclick={(event) => selectClip(event, clip)}
            >
              <span class="clip-label"><Icon name={CLIP_ICONS[clip.kind]} size={12} />{clip.name}</span>
            </button>
          {/each}
        </div>
      {/each}

      <div class="filler-header"></div>
      <div class="filler" role="presentation" onclick={() => (editor.selectedClipIds = [])}>
        {#if view.tracks.length === 0}
          <div class="empty">
            <strong>{t("timeline.empty.title")}</strong>
            <span>{t("timeline.empty.hint")}</span>
          </div>
        {/if}
      </div>

      <div class="playhead" style:left="calc(var(--track-header-width) + {playheadX}px)"></div>
    </div>
  </div>
</section>

<style>
  .timeline {
    grid-area: timeline;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    background: var(--color-bg-panel);
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 2px;
    height: var(--panel-header-height);
    padding: 0 var(--space-2);
    border-bottom: 1px solid var(--color-border);
    flex: none;
  }
  .spacer {
    flex: 1;
  }
  .zoom {
    display: flex;
    align-items: center;
    gap: 4px;
    width: 220px;
  }
  .zoom input {
    flex: 1;
    accent-color: var(--color-accent);
  }
  .scroller {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .grid {
    position: relative;
    display: grid;
    min-height: 100%;
  }
  .corner {
    position: sticky;
    top: 0;
    left: 0;
    z-index: 4;
    background: var(--color-bg-panel);
    border-right: 1px solid var(--color-border);
    border-bottom: 1px solid var(--color-border);
  }
  .ruler-cell {
    position: sticky;
    top: 0;
    z-index: 3;
    background: var(--color-bg-panel);
    border-bottom: 1px solid var(--color-border);
  }
  .track-header,
  .filler-header {
    position: sticky;
    left: 0;
    z-index: 2;
    background: var(--color-bg-panel);
    border-right: 1px solid var(--color-border);
  }
  .track-header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 var(--space-1) 0 var(--space-3);
    border-bottom: 1px solid var(--color-border);
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }
  .track-name {
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .lane {
    position: relative;
    border-bottom: 1px solid var(--color-border);
  }
  .lane.video {
    background: color-mix(in srgb, var(--color-bg-panel) 88%, var(--color-clip-video));
  }
  .lane.audio {
    background: color-mix(in srgb, var(--color-bg-panel) 90%, var(--color-clip-audio));
  }
  .clip {
    position: absolute;
    top: 3px;
    bottom: 3px;
    display: flex;
    flex-direction: column;
    padding: 0;
    border: 1px solid;
    border-radius: var(--radius-md);
    overflow: hidden;
    color: #ffffff;
    text-align: left;
    cursor: pointer;
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
  .clip-label {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 2px 6px;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    text-shadow: 0 1px 2px rgba(0, 0, 0, 0.5);
  }
  .filler {
    position: relative;
    min-height: 40px;
  }
  .empty {
    position: sticky;
    left: 0;
    width: min(100%, 720px);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-8) var(--space-4);
    color: var(--color-text-muted);
    text-align: center;
  }
  .empty strong {
    color: var(--color-text-secondary);
    font-weight: var(--font-weight-medium);
  }
  .playhead {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    margin-left: -1px;
    background: var(--color-playhead);
    z-index: 1;
    pointer-events: none;
  }
  .playhead-head {
    position: absolute;
    top: 0;
    width: 14px;
    height: 16px;
    margin-left: -7px;
    background: var(--color-playhead);
    clip-path: polygon(0 0, 100% 0, 100% 60%, 50% 100%, 0 60%);
    pointer-events: none;
  }
</style>
