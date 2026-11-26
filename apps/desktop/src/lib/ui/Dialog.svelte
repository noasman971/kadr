<!--
  A modal dialog: header with title and close button, scrolling body, optional
  footer. Built on <dialog>, so Esc closes it, focus stays inside, and the rest
  of the window is inert while it is open.
-->
<script lang="ts">
  import type { Snippet } from "svelte";

  import { t } from "../i18n/i18n.svelte";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";

  interface Props {
    title: string;
    widthInPixels?: number;
    onClose: () => void;
    children: Snippet;
    footer?: Snippet;
  }

  let { title, widthInPixels = 720, onClose, children, footer }: Props = $props();

  let dialog: HTMLDialogElement | undefined = $state();

  $effect(() => {
    dialog?.showModal();
  });
</script>

<dialog
  bind:this={dialog}
  style:width="min({widthInPixels}px, calc(100vw - 48px))"
  aria-labelledby="dialog-title"
  oncancel={(event) => {
    event.preventDefault();
    onClose();
  }}
>
  <header>
    <h2 id="dialog-title">{title}</h2>
    <span class="spacer"></span>
    <IconButton label={t("common.close")} shortcut={t("key.escape")} onclick={onClose}><Icon name="close" /></IconButton>
  </header>
  <div class="body">{@render children()}</div>
  {#if footer}
    <footer>{@render footer()}</footer>
  {/if}
</dialog>

<style>
  dialog {
    max-height: calc(100vh - 48px);
    padding: 0;
    border: 0;
    border-radius: var(--radius-xl);
    background: var(--color-bg-panel);
    color: var(--color-text);
    box-shadow: var(--shadow-popover);
    overflow: hidden;
  }
  dialog[open] {
    display: flex;
    flex-direction: column;
    animation: appear var(--duration-normal) var(--easing-standard);
  }
  dialog::backdrop {
    background: var(--backdrop);
  }
  header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-3) var(--space-3) var(--space-5);
    border-bottom: 1px solid var(--color-border);
  }
  h2 {
    margin: 0;
    font-size: var(--font-size-xl);
    font-weight: var(--font-weight-semibold);
  }
  .spacer {
    flex: 1;
  }
  .body {
    padding: var(--space-5);
    overflow: auto;
  }
  footer {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-5);
    border-top: 1px solid var(--color-border);
    background: var(--color-bg-raised);
  }
  @keyframes appear {
    from {
      opacity: 0;
      transform: translateY(8px) scale(0.98);
    }
  }
</style>
