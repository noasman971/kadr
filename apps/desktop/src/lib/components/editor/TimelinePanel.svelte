<!--
  The timeline (docs/ux.md, "Cut and arrange"): ruler, track headers, clips and
  the playhead, and every gesture on them — moving and trimming clips, dropping
  media from the library, snapping. The scroll area holds everything; the ruler
  sticks to the top and the track headers to the left.

  During a gesture nothing is sent to the engine: a ghost shows the result (or
  turns red and says why it is refused); releasing sends one edit.
-->
<script lang="ts">
  import { onMount } from "svelte";

  import type { ClipKind, ClipView, MediaView, ProjectView, TrackView } from "../../api/types";
  import { quoted, t, trackDisplayName } from "../../i18n/i18n.svelte";
  import { editor } from "../../state/editor.svelte";
  import { mediaDrag } from "../../state/mediaDrag.svelte";
  import { playback } from "../../state/playback.svelte";
  import { preferences } from "../../state/preferences.svelte";
  import {
    SNAP_DISTANCE_IN_PIXELS,
    collectSnapPoints,
    findOverlappingClip,
    snapEdges,
    trackAcceptsClip,
    trimLimits,
  } from "../../timeline/placement";
  import {
    DEFAULT_PIXELS_PER_SECOND,
    clampZoom,
    pixelsToTicks,
    ticksToPixels,
    zoomToFit,
  } from "../../timeline/timelineGeometry";
  import { TICKS_PER_SECOND, formatTimecode, snapToFrame, ticksPerFrame } from "../../time/ticks";
  import Icon from "../../ui/Icon.svelte";
  import IconButton from "../../ui/IconButton.svelte";
  import TimelineClip from "./TimelineClip.svelte";
  import { pointerOffsetX } from "../../ui/pointer";
  import TimelineRuler from "./TimelineRuler.svelte";

  interface Props {
    view: ProjectView;
    onSplit: () => void;
    onDelete: () => void;
  }

  let { view, onSplit, onDelete }: Props = $props();

  /** Room left after the end of the edit, to drop clips there. */
  const TRAILING_SPACE_IN_SECONDS = 30;
  const ZOOM_STEP = 1.25;
  const RULER_HEIGHT_IN_PIXELS = 28;
  const NEW_TRACK_ZONE_HEIGHT_IN_PIXELS = 36;
  const TRACK_HEIGHTS_IN_PIXELS: Record<TrackView["kind"], number> = { video: 68, audio: 52 };
  const STILL_IMAGE_SECONDS = 5;
  /** A press must travel this far before it moves a clip. */
  const DRAG_START_DISTANCE_IN_PIXELS = 4;
  /** Near the panel's sides, a drag scrolls the timeline by this much per move. */
  const AUTO_SCROLL_EDGE_IN_PIXELS = 40;
  const AUTO_SCROLL_STEP_IN_PIXELS = 24;

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
  // Ruler, one row per track, the new-track zone, then the filler. `repeat(0, …)` is invalid CSS and
  // would drop the whole declaration, so an empty timeline lists no track rows.
  const gridRows = $derived(
    [`${RULER_HEIGHT_IN_PIXELS}px`, ...view.tracks.map(() => "auto"), `${NEW_TRACK_ZONE_HEIGHT_IN_PIXELS}px`, "1fr"].join(" "),
  );
  const playheadX = $derived(ticksToPixels(editor.playheadTicks, pixelsPerSecond));
  const selected = $derived(new Set(editor.selectedClipIds));
  const mediaById = $derived(new Map(view.media.map((media) => [media.id, media])));
  const minimumClipTicks = $derived(ticksPerFrame(editor.frameRate));

  // ---------- Gestures ----------

  /** Where a dragged clip or media file would land. */
  interface DropPreview {
    /** The destination track, or `null` for a new track. */
    trackId: number | null;
    startTicks: number;
    durationTicks: number;
    kind: ClipKind;
    name: string;
    snappedTo: number | null;
    refusal: string | null;
  }

  interface MoveGesture {
    type: "move";
    clip: ClipView;
    grabOffsetTicks: number;
    pressX: number;
    pressY: number;
    hasStarted: boolean;
    preview: DropPreview | null;
  }

  interface TrimGesture {
    type: "trim";
    clip: ClipView;
    edge: "start" | "end";
    edgeTicks: number;
    snappedTo: number | null;
  }

  let gesture = $state<MoveGesture | TrimGesture | null>(null);
  let mediaPreview = $state<DropPreview | null>(null);

  /** The content's x coordinate (pixels from time 0) under a pointer. */
  function contentX(clientX: number): number {
    return scroller ? pointerOffsetX(clientX, scroller) - headerWidth + scroller.scrollLeft : 0;
  }

  /** The track lane under a pointer: a track id, `null` for the new-track zone, `undefined` for neither. */
  function laneUnder(clientX: number, clientY: number): number | null | undefined {
    const element = document.elementFromPoint(clientX, clientY)?.closest<HTMLElement>("[data-lane]");
    if (!element) {
      return undefined;
    }
    return element.dataset.lane === "new" ? null : Number(element.dataset.lane);
  }

  function isSnappingActive(event: PointerEvent): boolean {
    return preferences.isSnappingOn && !event.altKey;
  }

  /** Works out where a clip of `kind` and length would land for a pointer position. */
  function previewDrop(
    event: PointerEvent,
    kind: ClipKind,
    name: string,
    durationTicks: number,
    grabOffsetTicks: number,
    movingClipId: number | null,
  ): DropPreview | null {
    const lane = laneUnder(event.clientX, event.clientY);
    if (lane === undefined) {
      return null;
    }
    const ignored = new Set(movingClipId === null ? [] : [movingClipId]);
    let startTicks = Math.max(0, pixelsToTicks(contentX(event.clientX), pixelsPerSecond) - grabOffsetTicks);
    let snappedTo: number | null = null;
    if (isSnappingActive(event)) {
      const snap = snapEdges(
        [startTicks, startTicks + durationTicks],
        collectSnapPoints(view.tracks, editor.playheadTicks, ignored),
        pixelsToTicks(SNAP_DISTANCE_IN_PIXELS, pixelsPerSecond),
      );
      startTicks = Math.max(0, startTicks + snap.shiftTicks);
      snappedTo = snap.snappedTo;
    }
    startTicks = snapToFrame(startTicks, editor.frameRate);
    const preview: DropPreview = { trackId: lane, startTicks, durationTicks, kind, name, snappedTo, refusal: null };
    if (lane === null) {
      return preview;
    }
    const track = view.tracks.find((candidate) => candidate.id === lane);
    if (!track) {
      return null;
    }
    if (!trackAcceptsClip(track, kind)) {
      preview.refusal = t(kind === "audio" ? "timeline.refused.needsAudioTrack" : "timeline.refused.needsVideoTrack");
      return preview;
    }
    const overlapping = findOverlappingClip(track, startTicks, durationTicks, ignored);
    if (overlapping) {
      preview.refusal = t("timeline.refused.overlap", { name: quoted(overlapping.name) });
    }
    return preview;
  }

  function autoScrollNear(clientX: number): void {
    if (!scroller) {
      return;
    }
    const pointerX = pointerOffsetX(clientX, scroller);
    if (pointerX < headerWidth + AUTO_SCROLL_EDGE_IN_PIXELS) {
      scroller.scrollLeft -= AUTO_SCROLL_STEP_IN_PIXELS;
    } else if (pointerX > scroller.clientWidth - AUTO_SCROLL_EDGE_IN_PIXELS) {
      scroller.scrollLeft += AUTO_SCROLL_STEP_IN_PIXELS;
    }
  }

  function pressClip(event: PointerEvent, clip: ClipView): void {
    if (event.button !== 0) {
      return;
    }
    event.stopPropagation();
    if (event.shiftKey) {
      editor.selectedClipIds = selected.has(clip.id)
        ? editor.selectedClipIds.filter((clipId) => clipId !== clip.id)
        : [...editor.selectedClipIds, clip.id];
      return;
    }
    if (!selected.has(clip.id)) {
      editor.selectedClipIds = [clip.id];
    }
    const pointerTicks = pixelsToTicks(contentX(event.clientX), pixelsPerSecond);
    gesture = {
      type: "move",
      clip,
      grabOffsetTicks: pointerTicks - clip.startTicks,
      pressX: event.clientX,
      pressY: event.clientY,
      hasStarted: false,
      preview: null,
    };
    window.addEventListener("pointermove", handleGestureMove);
    window.addEventListener("pointerup", handleGestureEnd, { once: true });
  }

  function pressEdge(event: PointerEvent, clip: ClipView, edge: "start" | "end"): void {
    if (event.button !== 0) {
      return;
    }
    gesture = {
      type: "trim",
      clip,
      edge,
      edgeTicks: edge === "start" ? clip.startTicks : clip.startTicks + clip.durationTicks,
      snappedTo: null,
    };
    window.addEventListener("pointermove", handleGestureMove);
    window.addEventListener("pointerup", handleGestureEnd, { once: true });
  }

  function handleGestureMove(event: PointerEvent): void {
    if (!gesture) {
      return;
    }
    if (gesture.type === "move") {
      const distance = Math.hypot(event.clientX - gesture.pressX, event.clientY - gesture.pressY);
      if (!gesture.hasStarted && distance < DRAG_START_DISTANCE_IN_PIXELS) {
        return;
      }
      gesture.hasStarted = true;
      autoScrollNear(event.clientX);
      gesture.preview = previewDrop(
        event,
        gesture.clip.kind,
        gesture.clip.name,
        gesture.clip.durationTicks,
        gesture.grabOffsetTicks,
        gesture.clip.id,
      );
      return;
    }
    autoScrollNear(event.clientX);
    const track = view.tracks.find((candidate) => candidate.id === gesture?.clip.trackId);
    if (!track) {
      return;
    }
    const limits = trimLimits(gesture.clip, gesture.edge, track, minimumClipTicks);
    let edgeTicks = pixelsToTicks(contentX(event.clientX), pixelsPerSecond);
    let snappedTo: number | null = null;
    if (isSnappingActive(event)) {
      const snap = snapEdges(
        [edgeTicks],
        collectSnapPoints(view.tracks, editor.playheadTicks, new Set([gesture.clip.id])),
        pixelsToTicks(SNAP_DISTANCE_IN_PIXELS, pixelsPerSecond),
      );
      edgeTicks += snap.shiftTicks;
      snappedTo = snap.snappedTo;
    }
    edgeTicks = Math.min(limits.latestTicks, Math.max(limits.earliestTicks, edgeTicks));
    gesture.edgeTicks = snapToFrame(edgeTicks, editor.frameRate);
    gesture.snappedTo = snappedTo !== null && Math.abs(snappedTo - gesture.edgeTicks) < minimumClipTicks ? snappedTo : null;
    // Show the frame at the edge being moved.
    editor.seek(gesture.edge === "start" ? gesture.edgeTicks : gesture.edgeTicks - minimumClipTicks);
  }

  function handleGestureEnd(): void {
    window.removeEventListener("pointermove", handleGestureMove);
    const finished = gesture;
    gesture = null;
    if (!finished) {
      return;
    }
    if (finished.type === "move") {
      const preview = finished.preview;
      if (!finished.hasStarted || !preview || preview.refusal) {
        return;
      }
      const isUnchanged = preview.trackId === finished.clip.trackId && preview.startTicks === finished.clip.startTicks;
      if (!isUnchanged) {
        void editor.edit({
          type: "moveClip",
          clipId: finished.clip.id,
          trackId: preview.trackId,
          startTicks: preview.startTicks,
        });
      }
      return;
    }
    const currentEdge =
      finished.edge === "start" ? finished.clip.startTicks : finished.clip.startTicks + finished.clip.durationTicks;
    if (finished.edgeTicks !== currentEdge) {
      void editor.edit({ type: "trimClip", clipId: finished.clip.id, edge: finished.edge, timeTicks: finished.edgeTicks });
    }
  }

  // ---------- Media dragged from the library ----------

  function mediaClipKind(media: MediaView): ClipKind {
    return media.kind;
  }

  function mediaDurationTicks(media: MediaView): number {
    return media.durationTicks ?? STILL_IMAGE_SECONDS * TICKS_PER_SECOND;
  }

  $effect(() => {
    const media = mediaDrag.media;
    if (!media) {
      mediaPreview = null;
      return;
    }
    const pointer = { clientX: mediaDrag.clientX, clientY: mediaDrag.clientY, altKey: false } as PointerEvent;
    // A local copy: reading `mediaPreview` back here would make the effect depend
    // on what it writes, and loop.
    const preview = previewDrop(pointer, mediaClipKind(media), media.name, mediaDurationTicks(media), 0, null);
    mediaPreview = preview;
    mediaDrag.dropLabel = preview?.refusal ?? (preview ? formatTimecode(preview.startTicks, editor.frameRate) : null);
    mediaDrag.isDropRefused = (preview?.refusal ?? null) !== null;
  });

  onMount(() =>
    mediaDrag.onDrop((media, clientX, clientY) => {
      const pointer = { clientX, clientY, altKey: false } as PointerEvent;
      const preview = previewDrop(pointer, mediaClipKind(media), media.name, mediaDurationTicks(media), 0, null);
      mediaPreview = null;
      if (!preview || preview.refusal) {
        return;
      }
      void editor
        .edit({ type: "addMediaAt", mediaId: media.id, trackId: preview.trackId, startTicks: preview.startTicks })
        .then((createdClipIds) => {
          if (createdClipIds.length > 0) {
            editor.selectedClipIds = createdClipIds;
          }
        });
    }),
  );

  const activePreview = $derived(gesture?.type === "move" && gesture.hasStarted ? gesture.preview : mediaPreview);
  const snapGuideTicks = $derived(
    gesture?.type === "trim" ? gesture.snappedTo : (activePreview?.snappedTo ?? null),
  );

  // ---------- Zoom and scrolling ----------

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
    const anchorX = pointerOffsetX(event.clientX, scroller) - headerWidth;
    zoomAround(pixelsPerSecond * (event.deltaY < 0 ? ZOOM_STEP : 1 / ZOOM_STEP), anchorX);
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

  function toggleMuted(track: TrackView): void {
    void editor.edit({ type: "setTrackMuted", trackId: track.id, isMuted: !track.isMuted });
  }

  function toggleHidden(track: TrackView): void {
    void editor.edit({ type: "setTrackHidden", trackId: track.id, isHidden: !track.isHidden });
  }

  /** The clip as the trim gesture would leave it, to draw instead of the real one. */
  function clipDuringTrim(clip: ClipView): ClipView {
    if (gesture?.type !== "trim" || gesture.clip.id !== clip.id) {
      return clip;
    }
    const end = clip.startTicks + clip.durationTicks;
    return gesture.edge === "start"
      ? { ...clip, startTicks: gesture.edgeTicks, durationTicks: end - gesture.edgeTicks }
      : { ...clip, durationTicks: gesture.edgeTicks - clip.startTicks };
  }
</script>

<section class="timeline" aria-label={t("timeline.title")}>
  <div class="toolbar">
    <IconButton label={t("timeline.split")} shortcut="S" disabled={view.durationTicks === 0} onclick={onSplit}>
      <Icon name="scissors" />
    </IconButton>
    <IconButton
      label={t("timeline.delete")}
      shortcut={t("key.delete")}
      disabled={editor.selectedClipIds.length === 0}
      onclick={onDelete}
    >
      <Icon name="trash" />
    </IconButton>
    <span class="spacer"></span>
    <IconButton
      label={preferences.isSnappingOn ? t("timeline.snappingOn") : t("timeline.snappingOff")}
      shortcut="N"
      pressed={preferences.isSnappingOn}
      onclick={() => preferences.update({ isSnappingOn: !preferences.isSnappingOn })}
    >
      <Icon name="magnet" />
    </IconButton>
    <span class="divider" aria-hidden="true"></span>
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
      style:grid-template-rows={gridRows}
    >
      <div class="corner tabular" bind:clientWidth={headerWidth}>{formatTimecode(editor.playheadTicks, editor.frameRate)}</div>
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
        <div class="track-header" class:dimmed={track.isHidden || track.isMuted} style:height="{TRACK_HEIGHTS_IN_PIXELS[track.kind]}px">
          <Icon name={track.kind === "video" ? "film" : "music"} size={14} />
          <span class="track-name" title={trackDisplayName(track.name)}>{trackDisplayName(track.name)}</span>
          {#if track.kind === "video"}
            <IconButton
              size="small"
              label={track.isHidden ? t("timeline.showTrack") : t("timeline.hideTrack")}
              pressed={track.isHidden}
              onclick={() => toggleHidden(track)}
            >
              <Icon name={track.isHidden ? "eyeOff" : "eye"} size={14} />
            </IconButton>
          {/if}
          <IconButton
            size="small"
            label={track.isMuted ? t("timeline.unmuteTrack") : t("timeline.muteTrack")}
            pressed={track.isMuted}
            onclick={() => toggleMuted(track)}
          >
            <Icon name={track.isMuted ? "volumeOff" : "volume"} size={14} />
          </IconButton>
          {#if track.clips.length === 0}
            <IconButton size="small" label={t("timeline.removeTrack")} onclick={() => editor.edit({ type: "removeTrack", trackId: track.id })}>
              <Icon name="close" size={14} />
            </IconButton>
          {/if}
        </div>
        <div
          class="lane {track.kind}"
          class:muted={track.isMuted || track.isHidden}
          data-lane={track.id}
          role="presentation"
          style:height="{TRACK_HEIGHTS_IN_PIXELS[track.kind]}px"
          onpointerdown={() => (editor.selectedClipIds = [])}
        >
          {#each track.clips as clip (clip.id)}
            <TimelineClip
              clip={clipDuringTrim(clip)}
              media={clip.mediaId === null ? undefined : mediaById.get(clip.mediaId)}
              {pixelsPerSecond}
              heightInPixels={TRACK_HEIGHTS_IN_PIXELS[track.kind]}
              isSelected={selected.has(clip.id)}
              isGhost={gesture?.type === "move" && gesture.hasStarted && gesture.clip.id === clip.id}
              visibleStartInPixels={scrollLeft}
              visibleWidthInPixels={viewportWidth}
              onBodyPointerDown={pressClip}
              onEdgePointerDown={pressEdge}
            />
          {/each}
          {#if activePreview && activePreview.trackId === track.id}
            <div
              class="drop-ghost"
              class:refused={activePreview.refusal !== null}
              style:left="{ticksToPixels(activePreview.startTicks, pixelsPerSecond)}px"
              style:width="{Math.max(2, ticksToPixels(activePreview.durationTicks, pixelsPerSecond))}px"
            ></div>
          {/if}
        </div>
      {/each}

      <div class="new-track-header"></div>
      <div class="new-track-lane" data-lane="new" role="presentation" onpointerdown={() => (editor.selectedClipIds = [])}>
        {#if activePreview && activePreview.trackId === null}
          <div class="new-track-zone is-target">
            {t(activePreview.kind === "audio" ? "timeline.newAudioTrack" : "timeline.newVideoTrack", { name: activePreview.name })}
          </div>
        {:else if activePreview}
          <div class="new-track-zone">{t("timeline.dropForNewTrack")}</div>
        {/if}
      </div>

      <div class="filler-header"></div>
      <div class="filler" role="presentation" onpointerdown={() => (editor.selectedClipIds = [])}>
        {#if view.tracks.length === 0 && !activePreview}
          <div class="empty">
            <strong>{t("timeline.empty.title")}</strong>
            <span>{t("timeline.empty.hint")}</span>
          </div>
        {/if}
      </div>

      {#if snapGuideTicks !== null}
        <div class="snap-guide" style:left="calc(var(--track-header-width) + {ticksToPixels(snapGuideTicks, pixelsPerSecond)}px)"></div>
      {/if}
      <div class="playhead" style:left="calc(var(--track-header-width) + {playheadX}px)"></div>
    </div>
  </div>

  {#if gesture?.type === "trim"}
    <div class="gesture-label tabular">
      {t("timeline.trimLength", {
        length: formatTimecode(clipDuringTrim(gesture.clip).durationTicks, editor.frameRate),
      })}
    </div>
  {:else if gesture?.type === "move" && activePreview?.refusal}
    <!-- A media file dragged from the library shows its refusal on the badge under the pointer. -->
    <div class="gesture-label refused"><Icon name="alert" size={14} />{activePreview.refusal}</div>
  {/if}
</section>

<style>
  .timeline {
    position: relative;
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
  .divider {
    width: 1px;
    height: 20px;
    margin: 0 var(--space-1);
    background: var(--color-border-strong);
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
    display: flex;
    align-items: center;
    padding: 0 var(--space-3);
    background: var(--color-bg-panel);
    border-right: 1px solid var(--color-border);
    border-bottom: 1px solid var(--color-border);
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }
  .ruler-cell {
    position: sticky;
    top: 0;
    z-index: 3;
    background: var(--color-bg-panel);
    border-bottom: 1px solid var(--color-border);
  }
  .track-header,
  .new-track-header,
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
    gap: 4px;
    padding: 0 var(--space-1) 0 var(--space-3);
    border-bottom: 1px solid var(--color-border);
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }
  .track-header.dimmed .track-name {
    color: var(--color-text-muted);
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
  .lane.muted {
    background-image: repeating-linear-gradient(
      -45deg,
      transparent 0 8px,
      color-mix(in srgb, var(--color-bg-app) 40%, transparent) 8px 16px
    );
  }
  .drop-ghost {
    position: absolute;
    top: 3px;
    bottom: 3px;
    border: 2px dashed var(--color-accent-text);
    border-radius: var(--radius-md);
    background: var(--color-accent-soft);
    pointer-events: none;
    z-index: 2;
  }
  .drop-ghost.refused {
    border-color: var(--color-danger);
    background: var(--color-danger-soft);
  }
  .new-track-lane {
    position: relative;
    padding: 3px 0;
  }
  .new-track-zone {
    position: sticky;
    left: 12px;
    width: min(calc(100vw - var(--track-header-width) - 48px), 640px);
    height: 100%;
    display: grid;
    place-items: center;
    border: 1px dashed var(--color-border-strong);
    border-radius: var(--radius-md);
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
    pointer-events: none;
  }
  .new-track-zone.is-target {
    border-color: var(--color-accent-text);
    background: var(--color-accent-soft);
    color: var(--color-accent-text);
  }
  .filler {
    position: relative;
    min-height: 24px;
  }
  .empty {
    position: sticky;
    left: 0;
    width: min(100%, 720px);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-6) var(--space-4);
    color: var(--color-text-muted);
    text-align: center;
  }
  .empty strong {
    color: var(--color-text-secondary);
    font-weight: var(--font-weight-medium);
  }
  .playhead,
  .snap-guide {
    position: absolute;
    top: 0;
    bottom: 0;
    pointer-events: none;
  }
  .playhead {
    width: 2px;
    margin-left: -1px;
    background: var(--color-playhead);
    z-index: 1;
  }
  .snap-guide {
    width: 0;
    border-left: 1px dashed var(--color-snap-guide);
    z-index: 3;
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
  .gesture-label {
    position: absolute;
    right: var(--space-4);
    bottom: var(--space-3);
    z-index: 6;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-radius: var(--radius-md);
    background: var(--color-bg-raised);
    box-shadow: var(--shadow-popover);
    font-size: var(--font-size-sm);
    pointer-events: none;
  }
  .gesture-label.refused {
    color: var(--color-danger);
  }
</style>
