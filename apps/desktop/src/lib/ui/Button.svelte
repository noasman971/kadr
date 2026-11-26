<!--
  A text button, with an optional icon before the label.
  Variants: default, primary (one per area), ghost, danger. Sizes: normal, large.
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";

  interface Props extends HTMLButtonAttributes {
    variant?: "default" | "primary" | "ghost" | "danger";
    size?: "normal" | "large";
    children: Snippet;
  }

  let { variant = "default", size = "normal", children, type = "button", ...attributes }: Props = $props();
</script>

<button {type} class="button {variant} {size}" {...attributes}>
  {@render children()}
</button>

<style>
  .button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    height: var(--control-height);
    padding: 0 var(--space-3);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    background: var(--color-bg-raised);
    color: var(--color-text);
    cursor: pointer;
    white-space: nowrap;
    transition: background var(--duration-fast) var(--easing-standard);
  }
  .button:hover:not(:disabled) {
    background: var(--color-bg-hover);
  }
  .button:active:not(:disabled) {
    background: var(--color-bg-active);
  }
  .button:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .primary {
    background: var(--color-accent);
    border-color: transparent;
    color: var(--color-on-accent);
    font-weight: var(--font-weight-medium);
  }
  .primary:hover:not(:disabled) {
    background: var(--color-accent-hover);
  }
  .primary:active:not(:disabled) {
    background: var(--color-accent);
  }
  .ghost {
    background: transparent;
    border-color: transparent;
  }
  .danger {
    color: var(--color-danger);
  }
  .large {
    height: var(--control-height-large);
    padding: 0 var(--space-5);
    font-size: var(--font-size-lg);
    border-radius: var(--radius-lg);
  }
</style>
