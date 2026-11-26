<!--
  The media library (docs/ux.md, "Import media"): imported files, files being
  read, files that could not be read, and the drop zone of an empty project.
-->
<script lang="ts">
  import type { MediaView, ProjectView, TitleStyle } from "../../api/types";
  import type { MessageKey } from "../../i18n/messages.en";
  import { t } from "../../i18n/i18n.svelte";
  import { editor } from "../../state/editor.svelte";
  import Button from "../../ui/Button.svelte";
  import Icon from "../../ui/Icon.svelte";
  import IconButton from "../../ui/IconButton.svelte";
  import KeyCap from "../../ui/KeyCap.svelte";
  import MediaTile from "./MediaTile.svelte";

  interface Props {
    view: ProjectView;
    onImport: () => void;
    onMediaPointerDown?: (event: PointerEvent, media: MediaView) => void;
  }

  let { view, onImport, onMediaPointerDown }: Props = $props();

  let activeTab = $state<"media" | "text">("media");

  const TITLE_STYLES: Array<{ style: TitleStyle; label: MessageKey; sample: MessageKey }> = [
    { style: "title", label: "title.style.title", sample: "title.titleText" },
    { style: "subtitle", label: "title.style.subtitle", sample: "title.subtitleText" },
    { style: "bigWord", label: "title.style.bigWord", sample: "title.bigWordText" },
  ];
  let searchQuery = $state("");

  const visibleMedia = $derived(
    view.media.filter((media) => media.name.toLowerCase().includes(searchQuery.trim().toLowerCase())),
  );
  const isEmpty = $derived(
    view.media.length === 0 && editor.pendingImports.length === 0 && editor.importFailures.length === 0,
  );

  async function addAtEnd(media: MediaView): Promise<void> {
    const createdClipIds = await editor.edit({ type: "addMediaAtEnd", mediaId: media.id });
    if (createdClipIds.length > 0) {
      editor.selectedClipIds = createdClipIds;
    }
  }
</script>

<aside class="library" aria-label={t("library.media")}>
  <div class="header">
    <div class="tabs" role="tablist">
      <button type="button" role="tab" aria-selected={activeTab === "media"} onclick={() => (activeTab = "media")}>
        <Icon name="film" size={14} />{t("library.media")}
      </button>
      <button type="button" role="tab" aria-selected={activeTab === "text"} onclick={() => (activeTab = "text")}>
        <Icon name="type" size={14} />{t("library.text")}
      </button>
    </div>
    <span class="spacer"></span>
    {#if activeTab === "media" && !isEmpty}
      <IconButton label={t("library.import")} shortcut="Ctrl+I" onclick={onImport}><Icon name="plus" /></IconButton>
    {/if}
  </div>

  <div class="body">
    {#if activeTab === "text"}
      <p class="hint">{t("title.styles")}</p>
      <div class="title-styles">
        {#each TITLE_STYLES as style (style.style)}
          <button type="button" class="title-style {style.style}" onclick={() => void editor.addTitle(style.style)}>
            <span class="title-sample">{t(style.sample)}</span>
            <span class="title-name"><Icon name="plus" size={12} />{t(style.label)}</span>
          </button>
        {/each}
      </div>
    {:else if isEmpty}
      <div class="dropzone">
        <span class="dropzone-icon"><Icon name="upload" size={32} /></span>
        <strong>{t("library.drop.title")}</strong>
        <p>{t("library.drop.body")}</p>
        <Button variant="primary" onclick={onImport}><Icon name="filePlus" />{t("library.import")}</Button>
        <p class="formats"><KeyCap text="Ctrl" /> <KeyCap text="I" /> · {t("library.drop.formats")}</p>
      </div>
    {:else}
      <label class="search">
        <Icon name="search" size={14} />
        <input type="search" placeholder={t("library.search")} aria-label={t("library.search")} bind:value={searchQuery} />
      </label>
      <div class="grid" role="list">
        {#each visibleMedia as media (media.id)}
          <MediaTile {media} onAddAtEnd={addAtEnd} onPointerDown={onMediaPointerDown} />
        {/each}
        {#each editor.pendingImports as pending (pending.filePath)}
          <div class="pending" role="listitem" title={pending.filePath}>
            <div class="picture"><span class="spinner" aria-hidden="true"></span></div>
            <span class="name">{t("library.reading")} {pending.fileName}</span>
          </div>
        {/each}
        {#each editor.importFailures as failure (failure.filePath)}
          <div class="failure" role="listitem" title={failure.details}>
            <div class="picture">
              <Icon name="alert" size={20} />
              <span class="dismiss">
                <IconButton
                  label={t("common.close")}
                  size="small"
                  onclick={() => editor.dismissImportFailure(failure.filePath)}
                >
                  <Icon name="close" size={12} />
                </IconButton>
              </span>
            </div>
            <span class="name">{t("library.unreadable", { name: failure.fileName })}</span>
          </div>
        {/each}
      </div>
      {#if visibleMedia.length === 0 && searchQuery.trim() !== ""}
        <p class="hint">{t("library.noResults", { query: searchQuery.trim() })}</p>
      {:else}
        <p class="hint">{t("library.hint")}</p>
      {/if}
    {/if}
  </div>
</aside>

<style>
  .library {
    grid-area: library;
    background: var(--color-bg-panel);
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
  .header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: var(--panel-header-height);
    padding: 0 var(--space-2) 0 var(--space-3);
    border-bottom: 1px solid var(--color-border);
    flex: none;
  }
  .tabs {
    display: flex;
    gap: 2px;
    height: 100%;
  }
  .tabs button {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 var(--space-2);
    border: 0;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--color-text-secondary);
    font-weight: var(--font-weight-medium);
    cursor: pointer;
  }
  .tabs button[aria-selected="true"] {
    color: var(--color-text);
    border-bottom-color: var(--color-accent);
  }
  .spacer {
    flex: 1;
  }
  .body {
    flex: 1;
    overflow: auto;
    padding: var(--space-3);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .search {
    position: relative;
    display: block;
    color: var(--color-text-muted);
  }
  .search :global(svg) {
    position: absolute;
    left: 8px;
    top: 8px;
  }
  .search input {
    width: 100%;
    height: var(--control-height);
    padding: 0 var(--space-2) 0 28px;
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    background: var(--color-bg-inset);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-3) var(--space-2);
  }
  .pending,
  .failure {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .picture {
    position: relative;
    aspect-ratio: 16 / 9;
    border-radius: var(--radius-md);
    border: 1px solid var(--color-border);
    background: var(--color-bg-inset);
    display: grid;
    place-items: center;
  }
  .failure .picture {
    border-color: color-mix(in srgb, var(--color-danger) 60%, transparent);
    background: var(--color-danger-soft);
    color: var(--color-danger);
  }
  .dismiss {
    position: absolute;
    top: 2px;
    right: 2px;
  }
  .name {
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .failure .name {
    color: var(--color-danger);
  }
  .spinner {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 2px solid var(--color-bg-active);
    border-top-color: var(--color-accent-text);
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .title-styles {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .title-style {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: var(--space-3);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    background: var(--color-bg-inset);
    color: var(--color-text);
    text-align: left;
    cursor: pointer;
  }
  .title-style:hover {
    border-color: var(--color-accent);
  }
  .title-sample {
    min-height: 44px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-sm);
    background: #000000;
    color: #ffffff;
    font-weight: var(--font-weight-semibold);
  }
  .title-style.title .title-sample {
    font-size: var(--font-size-lg);
  }
  .title-style.subtitle .title-sample {
    place-items: end center;
    padding-bottom: 4px;
    font-size: var(--font-size-xs);
  }
  .title-style.bigWord .title-sample {
    font-size: 26px;
  }
  .title-name {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
  }
  .hint {
    margin: 0;
    padding: var(--space-2) 0;
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
    text-align: center;
    line-height: 1.5;
  }
  .dropzone {
    flex: 1;
    min-height: 240px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    padding: var(--space-6);
    border: 2px dashed var(--color-border-strong);
    border-radius: var(--radius-lg);
    text-align: center;
    color: var(--color-text-secondary);
  }
  .dropzone-icon {
    color: var(--color-accent-text);
  }
  .dropzone strong {
    color: var(--color-text);
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-medium);
  }
  .dropzone p {
    margin: 0;
    font-size: var(--font-size-sm);
    line-height: 1.5;
  }
  .formats {
    color: var(--color-text-muted);
  }
</style>
