<!--
  The app: the home screen or the editor, the dialogs, and files dropped on the
  window from the file explorer.
-->
<script lang="ts">
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { onMount } from "svelte";

  import EditorScreen from "./lib/components/editor/EditorScreen.svelte";
  import HomeScreen from "./lib/components/home/HomeScreen.svelte";
  import SettingsDialog from "./lib/components/SettingsDialog.svelte";
  import ShortcutsDialog from "./lib/components/ShortcutsDialog.svelte";
  import { t } from "./lib/i18n/i18n.svelte";
  import { editor } from "./lib/state/editor.svelte";
  import Icon from "./lib/ui/Icon.svelte";
  import ToastStack from "./lib/ui/ToastStack.svelte";

  let openDialog = $state<"settings" | "shortcuts" | null>(null);
  let isDraggingFilesOver = $state(false);

  /** Dropped files: imported into the open project, or into a new automatic project. */
  async function importDroppedFiles(filePaths: string[]): Promise<void> {
    if (filePaths.length === 0) {
      return;
    }
    if (!editor.view) {
      const isCreated = await editor.createProject("automatic");
      if (!isCreated) {
        return;
      }
    }
    await editor.importFiles(filePaths);
  }

  onMount(() => {
    let stopListening: (() => void) | undefined;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "enter" || payload.type === "over") {
          isDraggingFilesOver = true;
        } else if (payload.type === "drop") {
          isDraggingFilesOver = false;
          void importDroppedFiles(payload.paths);
        } else {
          isDraggingFilesOver = false;
        }
      })
      .then((unlisten) => {
        stopListening = unlisten;
      });
    return () => stopListening?.();
  });
</script>

{#if editor.view}
  <EditorScreen
    view={editor.view}
    onOpenSettings={() => (openDialog = "settings")}
    onShowShortcuts={() => (openDialog = "shortcuts")}
  />
{:else}
  <HomeScreen onOpenSettings={() => (openDialog = "settings")} />
  <div class="home-toasts"><ToastStack /></div>
{/if}

{#if openDialog === "settings"}
  <SettingsDialog onClose={() => (openDialog = null)} />
{:else if openDialog === "shortcuts"}
  <ShortcutsDialog onClose={() => (openDialog = null)} />
{/if}

{#if isDraggingFilesOver}
  <div class="drop-overlay" aria-hidden="true">
    <div class="drop-message"><Icon name="upload" size={32} />{t("library.dropOverlay")}</div>
  </div>
{/if}

<style>
  .home-toasts {
    position: fixed;
    left: 0;
    right: 0;
    bottom: 0;
    height: 120px;
    pointer-events: none;
  }
  .drop-overlay {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    background: var(--backdrop);
    border: 3px dashed var(--color-accent-text);
    pointer-events: none;
  }
  .drop-message {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-6);
    border-radius: var(--radius-xl);
    background: var(--color-bg-raised);
    box-shadow: var(--shadow-popover);
    color: var(--color-text);
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-medium);
  }
</style>
