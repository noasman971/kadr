<!--
  A square button showing only an icon. `label` is required: it becomes the
  tooltip and the accessible name, with the shortcut when there is one.
  `pressed` turns it into a toggle (snapping, mute, hide).
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";

  interface Props extends HTMLButtonAttributes {
    label: string;
    shortcut?: string;
    pressed?: boolean;
    size?: "normal" | "small";
    children: Snippet;
  }

  let { label, shortcut, pressed, size = "normal", children, ...attributes }: Props = $props();
  const tooltip = $derived(shortcut ? `${label} (${shortcut})` : label);
</script>

<button
  type="button"
  class="icon-button {size}"
  title={tooltip}
  aria-label={label}
  aria-pressed={pressed}
  {...attributes}
>
  {@render children()}
</button>

<style>
  .icon-button {
    display: inline-grid;
    place-items: center;
    width: var(--control-height);
    height: var(--control-height);
    padding: 0;
    border: 0;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--color-text-secondary);
    cursor: pointer;
    flex: none;
  }
  .small {
    width: 24px;
    height: 24px;
  }
  .icon-button:hover:not(:disabled) {
    background: var(--color-bg-hover);
    color: var(--color-text);
  }
  .icon-button[aria-pressed="true"] {
    background: var(--color-accent-soft);
    color: var(--color-accent-text);
  }
  .icon-button:disabled {
    opacity: 0.4;
    cursor: default;
  }
</style>
