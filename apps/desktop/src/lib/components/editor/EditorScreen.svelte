<!--
  The editor (docs/ux.md, "Editor"): top bar, library, preview, inspector and
  timeline, plus the keyboard shortcuts.
-->
<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";

  import type { ProjectView } from "../../api/types";
  import { t } from "../../i18n/i18n.svelte";
  import { editor } from "../../state/editor.svelte";
  import { blurTool } from "../../state/blurTool.svelte";
  import { exportJob } from "../../state/exportJob.svelte";
  import ExportDialog from "../export/ExportDialog.svelte";
  import { mediaDrag } from "../../state/mediaDrag.svelte";
  import { playback } from "../../state/playback.svelte";
  import { preferences } from "../../state/preferences.svelte";
  import { toasts } from "../../state/toasts.svelte";
  import { backend } from "../../api/backend";
  import { nextUnsureMoment } from "../../blur/unsureMoments";
  import { cutTimes } from "../../timeline/placement";
  import { snapToFrame, ticksPerFrame } from "../../time/ticks";
  import Icon from "../../ui/Icon.svelte";
  import { unzoomedClientPosition } from "../../ui/pointer";
  import InspectorPanel from "./InspectorPanel.svelte";
  import LibraryPanel from "./LibraryPanel.svelte";
  import PreviewPanel from "./PreviewPanel.svelte";
  import TimelinePanel from "./TimelinePanel.svelte";
  import TopBar from "./TopBar.svelte";

  interface Props {
    view: ProjectView;
    onOpenSettings: () => void;
    onShowShortcuts: () => void;
  }

  let { view, onOpenSettings, onShowShortcuts }: Props = $props();

  /** Extensions offered by the import dialog (anything FFmpeg reads also works). */
  const MEDIA_EXTENSIONS = ["mp4", "mov", "mkv", "webm", "avi", "m4v", "mp3", "wav", "m4a", "aac", "flac", "ogg", "png", "jpg", "jpeg", "webp", "bmp", "gif"];

  let timeline: TimelinePanel | undefined = $state();

  async function importFromDialog(): Promise<void> {
    const selection = await open({
      title: t("dialog.importMedia"),
      multiple: true,
      directory: false,
      filters: [
        { name: t("dialog.mediaFiles"), extensions: MEDIA_EXTENSIONS },
        { name: t("dialog.allFiles"), extensions: ["*"] },
      ],
    });
    if (Array.isArray(selection) && selection.length > 0) {
      await editor.importFiles(selection);
    }
  }

  /**
   * S: cuts the selected clips at the playhead, or every clip under it when
   * nothing is selected. Clips the playhead only touches are not cut.
   */
  async function splitAtPlayhead(): Promise<void> {
    const timeTicks = snapToFrame(editor.playheadTicks, editor.frameRate);
    const selected = new Set(editor.selectedClipIds);
    const hasClipToCut = view.tracks.some((track) =>
      track.clips.some(
        (clip) =>
          (selected.size === 0 || selected.has(clip.id)) &&
          clip.startTicks < timeTicks &&
          timeTicks < clip.startTicks + clip.durationTicks,
      ),
    );
    if (!hasClipToCut) {
      toasts.show({ tone: "info", message: t("timeline.nothingToSplit") });
      return;
    }
    await editor.edit({ type: "splitClips", clipIds: editor.selectedClipIds, timeTicks });
  }

  /** Delete: removes the selected clips, leaving the gaps; the notice offers Undo. */
  async function deleteSelection(): Promise<void> {
    const clipIds = editor.selectedClipIds;
    if (clipIds.length === 0) {
      return;
    }
    await editor.edit({ type: "removeClips", clipIds });
    const remainingClipIds = new Set(editor.view?.tracks.flatMap((track) => track.clips.map((clip) => clip.id)));
    if (clipIds.every((clipId) => !remainingClipIds.has(clipId))) {
      toasts.show({
        tone: "info",
        message: t("toast.clipsDeleted", { count: clipIds.length }),
        action: { label: t("toast.undo"), run: () => void editor.undo() },
      });
    }
  }

  /** Shift+B: the playhead goes to the next moment a blur's tracker was unsure about. */
  function goToNextUnsureMoment(): void {
    const moment = nextUnsureMoment(view, editor.playheadTicks);
    if (!moment) {
      toasts.show({ tone: "info", message: t("blur.nothingToCheck") });
      return;
    }
    editor.selectedClipIds = [moment.clipId];
    playback.seek(moment.middleTicks);
  }

  /** ↑ / ↓: the playhead jumps to the previous or next cut of the edit. */
  function jumpToCut(direction: -1 | 1): void {
    const now = editor.playheadTicks;
    // Half a frame of tolerance, so a playhead sitting on a cut moves past it.
    const tolerance = ticksPerFrame(editor.frameRate) / 2;
    const times = cutTimes(view.tracks);
    const target =
      direction < 0
        ? times.filter((time) => time < now - tolerance).at(-1)
        : times.find((time) => time > now + tolerance);
    if (target !== undefined) {
      playback.seek(target);
    }
  }

  const dragBadgePosition = $derived(unzoomedClientPosition(mediaDrag.clientX, mediaDrag.clientY));
  /** Near the right edge of the window, the badge goes to the left of the pointer. */
  const DRAG_BADGE_ROOM_IN_PIXELS = 380;
  const isDragBadgeOnTheLeft = $derived(mediaDrag.clientX > window.innerWidth - DRAG_BADGE_ROOM_IN_PIXELS);

  /** Export (Ctrl+E): the dialog, or the running export's progress. */
  function openExport(): void {
    if (view.durationTicks > 0 || exportJob.phase !== "setup") {
      exportJob.openDialog();
    }
  }

  async function goHome(): Promise<void> {
    playback.pause();
    await editor.closeProject();
  }

  /** Whether a key press belongs to a text field or an open dialog rather than to the editor. */
  function isTypingOrInDialog(event: KeyboardEvent): boolean {
    const target = event.target as HTMLElement | null;
    const isField =
      target instanceof HTMLInputElement ||
      target instanceof HTMLTextAreaElement ||
      target instanceof HTMLSelectElement ||
      target?.isContentEditable === true;
    return isField || document.querySelector("dialog[open]") !== null;
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (isTypingOrInDialog(event)) {
      return;
    }
    const isCommand = event.ctrlKey || event.metaKey;
    const key = event.key.toLowerCase();
    // While a box is drawn for a blur, Enter follows it, Escape leaves the tool
    // and the arrows move the box (Shift: ten times more).
    if (blurTool.isActive && (event.key === "Enter" || event.key === "Escape" || event.key.startsWith("Arrow"))) {
      if (event.key === "Enter") {
        void blurTool.confirm();
      } else if (event.key === "Escape") {
        blurTool.cancel();
      } else {
        const step = event.shiftKey ? 20 : 2;
        const dx = event.key === "ArrowLeft" ? -step : event.key === "ArrowRight" ? step : 0;
        const dy = event.key === "ArrowUp" ? -step : event.key === "ArrowDown" ? step : 0;
        blurTool.nudge(dx, dy);
      }
      event.preventDefault();
      return;
    }
    let isHandled = true;
    if (isCommand && key === "z" && !event.shiftKey) {
      void editor.undo();
    } else if (isCommand && (key === "y" || (key === "z" && event.shiftKey))) {
      void editor.redo();
    } else if (isCommand && key === "e") {
      openExport();
    } else if (isCommand && key === "i") {
      void importFromDialog();
    } else if (isCommand && key === "s") {
      void backend.saveProject().then((saved) => editor.acceptView(saved), (error: unknown) => editor.showError(error));
    } else if (isCommand) {
      isHandled = false;
    } else if (event.key === " ") {
      playback.toggle();
    } else if (key === "j") {
      playback.shuttle(-1);
    } else if (key === "k") {
      playback.pause();
    } else if (key === "l") {
      playback.shuttle(1);
    } else if (event.key === "ArrowLeft") {
      if (event.shiftKey) {
        playback.stepSeconds(-1);
      } else {
        playback.stepFrames(-1);
      }
    } else if (event.key === "ArrowRight") {
      if (event.shiftKey) {
        playback.stepSeconds(1);
      } else {
        playback.stepFrames(1);
      }
    } else if (event.key === "Home") {
      playback.seek(0);
    } else if (event.key === "End") {
      playback.seek(view.durationTicks);
    } else if (event.key === "ArrowUp" || event.key === "ArrowDown") {
      jumpToCut(event.key === "ArrowUp" ? -1 : 1);
    } else if (key === "s" && !event.shiftKey) {
      void splitAtPlayhead();
    } else if (event.key === "Delete" || event.key === "Backspace") {
      void deleteSelection();
    } else if (key === "t") {
      void editor.addTitle("title");
    } else if (key === "b" && !event.shiftKey) {
      if (blurTool.isActive) {
        blurTool.cancel();
      } else {
        blurTool.start();
      }
    } else if (key === "b" && event.shiftKey) {
      goToNextUnsureMoment();
    } else if (key === "n") {
      preferences.update({ isSnappingOn: !preferences.isSnappingOn });
    } else if (event.key === "Escape") {
      editor.selectedClipIds = [];
    } else if (event.key === "+" || event.key === "=") {
      timeline?.handleZoomShortcut("in");
    } else if (event.key === "-") {
      timeline?.handleZoomShortcut("out");
    } else if (key === "z" && event.shiftKey) {
      timeline?.handleZoomShortcut("fit");
    } else if (event.key === "?") {
      onShowShortcuts();
    } else {
      isHandled = false;
    }
    if (isHandled) {
      event.preventDefault();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="editor">
  <TopBar {view} onGoHome={goHome} {onOpenSettings} {onShowShortcuts} onExport={openExport} />
  <LibraryPanel {view} onImport={importFromDialog} onMediaPointerDown={(event, media) => mediaDrag.press(event, media)} />
  <PreviewPanel {view} />
  <InspectorPanel {view} />
  <TimelinePanel {view} bind:this={timeline} onSplit={splitAtPlayhead} onDelete={deleteSelection} />
</div>

{#if exportJob.isDialogOpen}
  <ExportDialog {view} />
{/if}

{#if mediaDrag.media}
  <div
    class="drag-badge"
    class:refused={mediaDrag.isDropRefused}
    class:on-the-left={isDragBadgeOnTheLeft}
    style:left="{dragBadgePosition.x + 14}px"
    style:top="{dragBadgePosition.y + 14}px"
    aria-hidden="true"
  >
    <Icon name={mediaDrag.isDropRefused ? "alert" : mediaDrag.media.kind === "audio" ? "music" : "film"} size={14} />
    <span class="drag-name">{mediaDrag.media.name}</span>
    {#if mediaDrag.dropLabel}<span class="drag-where">{mediaDrag.dropLabel}</span>{/if}
  </div>
{/if}

<style>
  .editor {
    height: 100%;
    display: grid;
    grid-template-columns: var(--library-width) minmax(0, 1fr) var(--inspector-width);
    grid-template-rows: var(--topbar-height) minmax(0, 1fr) var(--timeline-height);
    grid-template-areas:
      "top top top"
      "library preview inspector"
      "timeline timeline timeline";
    gap: 1px;
    background: var(--color-border);
  }
  /* The media file being dragged from the library, under the pointer. */
  .drag-badge {
    position: fixed;
    z-index: 40;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    max-width: 360px;
    padding: 6px var(--space-3);
    border-radius: var(--radius-md);
    background: var(--color-bg-raised);
    box-shadow: var(--shadow-popover);
    border: 1px solid var(--color-border-strong);
    color: var(--color-text);
    font-size: var(--font-size-sm);
    pointer-events: none;
  }
  .drag-badge.on-the-left {
    /* Its offset from the pointer (14 px) on the other side. */
    transform: translateX(calc(-100% - 28px));
  }
  .drag-badge.refused {
    border-color: var(--color-danger);
    color: var(--color-danger);
  }
  .drag-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .drag-where {
    flex: none;
    color: var(--color-text-secondary);
    font-variant-numeric: tabular-nums;
  }
  .refused .drag-where {
    flex: 1 1 auto;
    color: inherit;
  }
</style>
