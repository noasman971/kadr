<!--
  The editor (docs/ux.md, "Editor"): top bar, library, preview, inspector and
  timeline, plus the keyboard shortcuts.
-->
<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";

  import type { ProjectView } from "../../api/types";
  import { t } from "../../i18n/i18n.svelte";
  import { editor } from "../../state/editor.svelte";
  import { playback } from "../../state/playback.svelte";
  import { backend } from "../../api/backend";
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
    let isHandled = true;
    if (isCommand && key === "z" && !event.shiftKey) {
      void editor.undo();
    } else if (isCommand && (key === "y" || (key === "z" && event.shiftKey))) {
      void editor.redo();
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
  <TopBar {view} onGoHome={goHome} {onOpenSettings} {onShowShortcuts} onExport={() => undefined} />
  <LibraryPanel {view} onImport={importFromDialog} />
  <PreviewPanel {view} />
  <InspectorPanel {view} />
  <TimelinePanel {view} bind:this={timeline} />
</div>

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
</style>
