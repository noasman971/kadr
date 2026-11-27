<!--
  Connect to Claude (docs/ux.md, "Connect to Claude"; decision record 0031): the
  folders Claude may use, and Claude Desktop's configuration written in one click.
  Nothing here talks to Anthropic: it only writes files on this computer.
-->
<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";

  import { backend } from "../api/backend";
  import type { ClaudeSetupView } from "../api/types";
  import { t } from "../i18n/i18n.svelte";
  import { editor } from "../state/editor.svelte";
  import { toasts } from "../state/toasts.svelte";
  import Button from "../ui/Button.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import Icon from "../ui/Icon.svelte";
  import IconButton from "../ui/IconButton.svelte";

  interface Props {
    onClose: () => void;
  }

  let { onClose }: Props = $props();

  let setup = $state<ClaudeSetupView | null>(null);
  /** The configuration file the person chose, when not the one found. */
  let chosenConfig = $state<string | null>(null);
  let isConnecting = $state(false);
  let hasJustConnected = $state(false);

  async function run(request: () => Promise<ClaudeSetupView>): Promise<boolean> {
    try {
      setup = await request();
      return true;
    } catch (error) {
      editor.showError(error);
      return false;
    }
  }

  onMount(() => {
    void run(() => backend.getClaudeSetup(chosenConfig));
  });

  async function addFolder(): Promise<void> {
    const folder = await open({ title: t("claude.addFolder"), directory: true, multiple: false });
    if (typeof folder === "string") {
      await run(() => backend.addClaudeFolder(folder, chosenConfig));
    }
  }

  async function chooseConfig(): Promise<void> {
    const file = await open({
      title: t("claude.chooseConfig"),
      multiple: false,
      directory: false,
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (typeof file === "string") {
      chosenConfig = file;
      await run(() => backend.getClaudeSetup(file));
    }
  }

  async function connect(): Promise<void> {
    isConnecting = true;
    hasJustConnected = await run(() => backend.connectClaudeDesktop(chosenConfig));
    isConnecting = false;
  }

  async function copyCommand(command: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(command);
      toasts.show({ tone: "success", message: t("claude.copied") });
    } catch {
      toasts.show({ tone: "error", message: t("claude.copyFailed") });
    }
  }

  const statusKey = $derived(
    setup === null
      ? null
      : setup.kadrMcpPath === null
        ? "claude.status.kadrMcpMissing"
        : setup.connection === "connected"
          ? "claude.status.connected"
          : setup.connection === "otherKadr"
            ? "claude.status.otherKadr"
            : setup.connection === "unreadable"
              ? "claude.status.unreadable"
              : "claude.status.notConnected",
  );
</script>

<Dialog title={t("claude.title")} widthInPixels={640} {onClose}>
  <p class="intro">{t("claude.intro")}</p>

  {#if setup}
    <section class="step" aria-labelledby="claude-folders">
      <h3 id="claude-folders"><span class="number">1</span>{t("claude.folders")}</h3>
      <p class="hint">{t("claude.foldersHint")}</p>
      {#if setup.allowedFolders.length === 0}
        <p class="empty">{t("claude.noFolder")}</p>
      {:else}
        <ul class="folders">
          {#each setup.allowedFolders as folder (folder.path)}
            <li>
              <Icon name={folder.exists ? "folder" : "alert"} size={16} />
              <span class="path" title={folder.path}>{folder.path}</span>
              {#if !folder.exists}<span class="missing">{t("claude.folderMissing")}</span>{/if}
              <IconButton
                size="small"
                label={t("claude.removeFolder")}
                onclick={() => {
                  const path = folder.path;
                  void run(() => backend.removeClaudeFolder(path, chosenConfig));
                }}
              >
                <Icon name="close" size={14} />
              </IconButton>
            </li>
          {/each}
        </ul>
      {/if}
      <div>
        <Button onclick={() => void addFolder()}><Icon name="folderOpen" size={14} />{t("claude.addFolder")}</Button>
      </div>
    </section>

    <section class="step" aria-labelledby="claude-desktop">
      <h3 id="claude-desktop"><span class="number">2</span>{t("claude.desktop")}</h3>
      {#if statusKey}
        <p class="status" class:connected={setup.connection === "connected" && setup.kadrMcpPath !== null}>
          <Icon name={setup.connection === "connected" && setup.kadrMcpPath !== null ? "circleCheck" : "info"} size={16} />
          {t(statusKey)}
        </p>
      {/if}
      <div class="config">
        <span class="label">{t("claude.configFile")}</span>
        <span class="path" title={setup.claudeDesktopConfig}>{setup.claudeDesktopConfig}</span>
        <Button variant="ghost" onclick={() => void chooseConfig()}>{t("claude.chooseConfig")}</Button>
      </div>
      {#if !setup.claudeDesktopConfigExists}
        <p class="hint">{t("claude.configWillBeCreated")}</p>
      {/if}
      <div class="actions">
        <Button
          variant="primary"
          disabled={setup.kadrMcpPath === null || setup.connection === "unreadable" || isConnecting}
          onclick={() => void connect()}
        >
          <Icon name="check" size={14} />
          {setup.connection === "connected" ? t("claude.reconnect") : t("claude.connect")}
        </Button>
      </div>
      {#if hasJustConnected}
        <p class="next" role="status">{t("claude.restartClaude")}</p>
      {/if}
    </section>

    {#if setup.claudeCodeCommand}
      {@const command = setup.claudeCodeCommand}
      <details class="step">
        <summary>{t("claude.code")}</summary>
        <p class="hint">{t("claude.codeHint")}</p>
        <div class="command">
          <code>{command}</code>
          <IconButton label={t("claude.copy")} onclick={() => void copyCommand(command)}>
            <Icon name="copy" size={14} />
          </IconButton>
        </div>
      </details>
    {/if}
  {:else}
    <p class="hint">{t("claude.loading")}</p>
  {/if}
</Dialog>

<style>
  .intro {
    margin: 0 0 var(--space-4);
    color: var(--color-text-secondary);
    line-height: 1.5;
  }
  .step {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) 0;
    border-top: 1px solid var(--color-border);
  }
  h3 {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
  }
  .number {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border-radius: var(--radius-full);
    background: var(--color-accent-soft);
    color: var(--color-accent-text);
    font-size: var(--font-size-xs);
  }
  .hint,
  .empty {
    margin: 0;
    font-size: var(--font-size-sm);
    color: var(--color-text-muted);
    line-height: 1.45;
  }
  .folders {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .folders li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-md);
    background: var(--color-bg-inset);
  }
  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--font-size-sm);
  }
  .missing {
    color: var(--color-warning);
    font-size: var(--font-size-xs);
  }
  .status {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
    color: var(--color-text-secondary);
  }
  .status.connected {
    color: var(--color-success);
  }
  .config {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .label {
    flex: none;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }
  .actions {
    display: flex;
    gap: var(--space-2);
  }
  .next {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--color-accent-soft);
    line-height: 1.45;
  }
  summary {
    cursor: pointer;
    font-weight: var(--font-weight-medium);
  }
  .command {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  code {
    flex: 1;
    min-width: 0;
    padding: var(--space-2);
    border-radius: var(--radius-md);
    background: var(--color-bg-inset);
    font-size: var(--font-size-xs);
    overflow-x: auto;
    white-space: nowrap;
  }
</style>
