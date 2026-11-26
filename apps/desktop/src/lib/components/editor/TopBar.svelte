<!--
  The editor's top bar: back home, project name and save state, undo and redo,
  project format, help, settings and export (docs/ux.md, "Editor").
-->
<script lang="ts">
  import type { ProjectView } from "../../api/types";
  import { t, undoStepDisplayName } from "../../i18n/i18n.svelte";
  import { editor } from "../../state/editor.svelte";
  import Button from "../../ui/Button.svelte";
  import Icon from "../../ui/Icon.svelte";
  import IconButton from "../../ui/IconButton.svelte";
  import { aspectRatioName, formatFrameRate } from "./projectFormat";

  interface Props {
    view: ProjectView;
    onGoHome: () => void;
    onOpenSettings: () => void;
    onShowShortcuts: () => void;
    onExport: () => void;
  }

  let { view, onGoHome, onOpenSettings, onShowShortcuts, onExport }: Props = $props();

  let editedName = $state("");
  $effect(() => {
    editedName = view.name;
  });

  async function commitName(): Promise<void> {
    const newName = editedName.trim();
    if (newName === "" || newName === view.name) {
      editedName = view.name;
      return;
    }
    await editor.renameProject(newName);
    editedName = editor.view?.name ?? view.name;
  }

  function handleNameKeydown(event: KeyboardEvent): void {
    if (event.key === "Enter") {
      (event.currentTarget as HTMLInputElement).blur();
    } else if (event.key === "Escape") {
      editedName = view.name;
      (event.currentTarget as HTMLInputElement).blur();
    }
  }

  const formatLabel = $derived(
    view.adoptsFirstVideoFormat
      ? t("topBar.formatAutomatic")
      : t("topBar.formatValue", {
          ratio: aspectRatioName(view.settings.canvasWidth, view.settings.canvasHeight),
          width: view.settings.canvasWidth,
          height: view.settings.canvasHeight,
          rate: formatFrameRate(view.settings),
        }),
  );
  const hasClips = $derived(view.durationTicks > 0);
</script>

<header class="top-bar">
  <button type="button" class="home" title={t("topBar.home")} aria-label={t("topBar.home")} onclick={onGoHome}>K</button>
  <div class="project-name">
    <input
      aria-label={t("topBar.projectName")}
      bind:value={editedName}
      onblur={commitName}
      onkeydown={handleNameKeydown}
      spellcheck="false"
    />
    {#if view.saveFailure}
      <span class="save-state failed" title={view.saveFailure}><Icon name="alert" size={14} />{t("topBar.notSaved")}</span>
    {:else}
      <span class="save-state" title={view.filePath}><Icon name="check" size={14} />{t("topBar.saved")}</span>
    {/if}
  </div>
  <span class="spacer"></span>
  <IconButton
    label={view.undoLabel ? t("topBar.undoStep", { step: undoStepDisplayName(view.undoLabel) }) : t("topBar.undo")}
    shortcut="Ctrl+Z"
    disabled={!view.undoLabel}
    onclick={() => editor.undo()}
  >
    <Icon name="undo" />
  </IconButton>
  <IconButton
    label={view.redoLabel ? t("topBar.redoStep", { step: undoStepDisplayName(view.redoLabel) }) : t("topBar.redo")}
    shortcut="Ctrl+Y"
    disabled={!view.redoLabel}
    onclick={() => editor.redo()}
  >
    <Icon name="redo" />
  </IconButton>
  <span class="divider" aria-hidden="true"></span>
  <span class="format-chip" title={t("topBar.format")}><Icon name="monitor" size={14} />{formatLabel}</span>
  <IconButton label={t("topBar.shortcuts")} shortcut="?" onclick={onShowShortcuts}><Icon name="help" /></IconButton>
  <IconButton label={t("topBar.settings")} onclick={onOpenSettings}><Icon name="settings" /></IconButton>
  <Button variant="primary" disabled={!hasClips} title="{t('topBar.export')} (Ctrl+E)" onclick={onExport}>
    <Icon name="download" />{t("topBar.export")}
  </Button>
</header>

<style>
  .top-bar {
    grid-area: top;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 0 var(--space-2) 0 var(--space-3);
    background: var(--color-bg-panel);
    min-width: 0;
  }
  .home {
    width: 24px;
    height: 24px;
    border: 0;
    border-radius: 7px;
    background: linear-gradient(135deg, var(--color-accent), #c25cff);
    color: #ffffff;
    font-weight: 700;
    font-size: 13px;
    cursor: pointer;
    flex: none;
  }
  .project-name {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-left: var(--space-2);
    min-width: 0;
  }
  .project-name input {
    width: 240px;
    max-width: 30vw;
    padding: 3px 6px;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: transparent;
    font-weight: var(--font-weight-medium);
    text-overflow: ellipsis;
  }
  .project-name input:hover {
    border-color: var(--color-border-strong);
  }
  .project-name input:focus {
    border-color: var(--color-accent);
    background: var(--color-bg-inset);
  }
  .save-state {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
    white-space: nowrap;
  }
  .save-state :global(svg) {
    color: var(--color-success);
  }
  .save-state.failed {
    color: var(--color-warning);
  }
  .save-state.failed :global(svg) {
    color: var(--color-warning);
  }
  .spacer {
    flex: 1;
  }
  .divider {
    width: 1px;
    height: 20px;
    background: var(--color-border-strong);
    margin: 0 var(--space-1);
  }
  .format-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 10px;
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-full);
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
    white-space: nowrap;
  }
</style>
