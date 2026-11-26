<!--
  The home screen (docs/ux.md, "Home"): a new project with its format, opening a
  project, and the recent projects.
-->
<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";

  import { backend } from "../../api/backend";
  import type { ProjectFormatChoice, RecentProject } from "../../api/types";
  import { t, translations } from "../../i18n/i18n.svelte";
  import type { MessageKey } from "../../i18n/messages.en";
  import { editor } from "../../state/editor.svelte";
  import Button from "../../ui/Button.svelte";
  import Icon from "../../ui/Icon.svelte";
  import IconButton from "../../ui/IconButton.svelte";

  interface Props {
    onOpenSettings: () => void;
  }

  let { onOpenSettings }: Props = $props();

  interface FormatOption {
    choice: ProjectFormatChoice;
    title: MessageKey;
    hint: MessageKey;
    shapeWidth: number;
    shapeHeight: number;
  }

  const FORMAT_OPTIONS: FormatOption[] = [
    { choice: "automatic", title: "home.format.automatic", hint: "home.format.automatic.hint", shapeWidth: 30, shapeHeight: 20 },
    { choice: "landscape", title: "home.format.landscape", hint: "home.format.landscape.hint", shapeWidth: 32, shapeHeight: 18 },
    { choice: "vertical", title: "home.format.vertical", hint: "home.format.vertical.hint", shapeWidth: 13, shapeHeight: 22 },
    { choice: "square", title: "home.format.square", hint: "home.format.square.hint", shapeWidth: 20, shapeHeight: 20 },
  ];

  let chosenFormat = $state<ProjectFormatChoice>("automatic");
  let recentProjects = $state<RecentProject[]>([]);
  let isBusy = $state(false);

  onMount(() => {
    void refreshRecentProjects();
  });

  async function refreshRecentProjects(): Promise<void> {
    try {
      recentProjects = await backend.listRecentProjects();
    } catch (error) {
      editor.showError(error);
    }
  }

  async function createProject(): Promise<void> {
    isBusy = true;
    await editor.createProject(chosenFormat);
    isBusy = false;
  }

  async function openProjectFromDialog(): Promise<void> {
    const filePath = await open({
      title: t("dialog.openProject"),
      multiple: false,
      directory: false,
      filters: [{ name: t("dialog.kadrProjects"), extensions: ["kadr"] }],
    });
    if (typeof filePath === "string") {
      await openRecent(filePath);
    }
  }

  async function openRecent(filePath: string): Promise<void> {
    isBusy = true;
    const isOpen = await editor.openProject(filePath);
    isBusy = false;
    if (!isOpen) {
      await refreshRecentProjects();
    }
  }

  async function forget(project: RecentProject): Promise<void> {
    await backend.forgetRecentProject(project.filePath).catch((error: unknown) => editor.showError(error));
    await refreshRecentProjects();
  }
</script>

<main class="home" aria-busy={isBusy}>
  <div class="columns">
    <section class="start">
      <div class="brand">
        <div class="brand-mark" aria-hidden="true">K</div>
        <span class="brand-name">{t("app.name")}</span>
      </div>
      <h1>{t("home.title")}</h1>
      <p class="subtitle">{t("home.subtitle")}</p>
      <div class="format-picker" role="radiogroup" aria-label={t("inspector.format")}>
        {#each FORMAT_OPTIONS as option (option.choice)}
          <button
            type="button"
            role="radio"
            aria-checked={chosenFormat === option.choice}
            class="format-option"
            onclick={() => (chosenFormat = option.choice)}
          >
            <span
              class="shape"
              class:dashed={option.choice === "automatic"}
              style:width="{option.shapeWidth}px"
              style:height="{option.shapeHeight}px"
            ></span>
            <strong>{t(option.title)}</strong>
            <span class="hint">{t(option.hint)}</span>
          </button>
        {/each}
      </div>
      <div class="actions">
        <Button variant="primary" size="large" disabled={isBusy} onclick={createProject}>
          <Icon name="plus" />{t("home.newProject")}
        </Button>
        <Button size="large" disabled={isBusy} onclick={openProjectFromDialog}>
          <Icon name="folderOpen" />{t("home.open")}
        </Button>
      </div>
      <p class="privacy"><Icon name="shield" />{t("home.privacy")}</p>
    </section>

    <section class="recent" aria-labelledby="recent-title">
      <h2 id="recent-title">{t("home.recent")}</h2>
      {#if recentProjects.length === 0}
        <p class="recent-empty">{t("home.recent.empty")}</p>
      {:else}
        <ul class="recent-grid">
          {#each recentProjects as project (project.filePath)}
            <li class="recent-card" class:missing={!project.exists}>
              <button
                type="button"
                class="recent-open"
                disabled={!project.exists || isBusy}
                title={project.filePath}
                onclick={() => openRecent(project.filePath)}
              >
                <span class="recent-picture" aria-hidden="true"><Icon name="film" size={28} /></span>
                <span class="recent-meta">
                  <strong>{project.name}</strong>
                  <span>
                    {project.exists
                      ? t("home.recent.openedAt", {
                          date: translations.formatDateTime(project.lastOpenedInMilliseconds),
                        })
                      : t("home.recent.missing")}
                  </span>
                </span>
              </button>
              <span class="recent-forget">
                <IconButton label={t("home.recent.forget")} size="small" onclick={() => forget(project)}>
                  <Icon name="close" size={14} />
                </IconButton>
              </span>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  </div>

  <footer class="footer">
    <span>{t("app.name")} 0.1 · {t("home.footer.license")}</span>
    <span class="spacer"></span>
    <Button variant="ghost" onclick={onOpenSettings}><Icon name="settings" />{t("settings.title")}</Button>
  </footer>
</main>

<style>
  .home {
    height: 100%;
    overflow: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-8);
    padding: var(--space-12) var(--space-8) var(--space-6);
    background: var(--color-bg-app);
  }
  .columns {
    width: min(1080px, 100%);
    display: grid;
    grid-template-columns: 340px 1fr;
    gap: var(--space-10);
    flex: 1;
  }
  .start {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .brand-mark {
    width: 36px;
    height: 36px;
    border-radius: var(--radius-lg);
    background: linear-gradient(135deg, var(--color-accent), #c25cff);
    display: grid;
    place-items: center;
    color: #ffffff;
    font-weight: 700;
    font-size: 20px;
  }
  .brand-name {
    font-size: var(--font-size-2xl);
    font-weight: var(--font-weight-semibold);
    letter-spacing: -0.02em;
  }
  h1 {
    margin: var(--space-3) 0 0;
    font-size: var(--font-size-3xl);
    line-height: 1.15;
    font-weight: var(--font-weight-semibold);
    letter-spacing: -0.02em;
  }
  .subtitle {
    margin: 0;
    color: var(--color-text-secondary);
    line-height: 1.5;
  }
  .format-picker {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: var(--space-2);
  }
  .format-option {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    border: 1px solid var(--color-border-strong);
    background: var(--color-bg-panel);
    cursor: pointer;
    text-align: left;
  }
  .format-option:hover {
    background: var(--color-bg-hover);
  }
  .format-option[aria-checked="true"] {
    border-color: var(--color-accent);
    box-shadow: 0 0 0 1px var(--color-accent);
    background: var(--color-accent-soft);
  }
  .shape {
    border: 2px solid var(--color-text-secondary);
    border-radius: 3px;
  }
  .shape.dashed {
    border-style: dashed;
  }
  .format-option strong {
    font-weight: var(--font-weight-medium);
  }
  .hint {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }
  .actions {
    display: flex;
    gap: var(--space-2);
  }
  .actions :global(.button:first-child) {
    flex: 1;
  }
  .privacy {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
  }
  .privacy :global(svg) {
    color: var(--color-success);
  }
  h2 {
    margin: 0 0 var(--space-4);
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
  }
  .recent-empty {
    color: var(--color-text-muted);
  }
  .recent-grid {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: var(--space-3);
  }
  .recent-card {
    position: relative;
  }
  .recent-open {
    width: 100%;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    border: 1px solid var(--color-border);
    background: var(--color-bg-panel);
    cursor: pointer;
    text-align: left;
  }
  .recent-open:hover:not(:disabled) {
    border-color: var(--color-border-strong);
    background: var(--color-bg-raised);
  }
  .recent-open:disabled {
    cursor: default;
    opacity: 0.6;
  }
  .recent-picture {
    width: 56px;
    height: 40px;
    flex: none;
    display: grid;
    place-items: center;
    border-radius: var(--radius-md);
    background: var(--color-bg-inset);
    color: var(--color-text-muted);
  }
  .recent-meta {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding-right: var(--space-6);
  }
  .recent-meta strong {
    font-weight: var(--font-weight-medium);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .recent-meta span {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }
  .missing .recent-meta span {
    color: var(--color-warning);
  }
  .recent-forget {
    position: absolute;
    top: 6px;
    right: 6px;
    opacity: 0;
  }
  .recent-card:hover .recent-forget,
  .recent-forget:focus-within {
    opacity: 1;
  }
  .footer {
    width: min(1080px, 100%);
    display: flex;
    align-items: center;
    gap: var(--space-4);
    padding-top: var(--space-4);
    border-top: 1px solid var(--color-border);
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }
  .spacer {
    flex: 1;
  }
</style>
