<!-- The notifications, at the bottom of the preview area. -->
<script lang="ts">
  import { t } from "../i18n/i18n.svelte";
  import { toasts } from "../state/toasts.svelte";
  import Icon from "./Icon.svelte";
</script>

<div class="stack" role="status" aria-live="polite">
  {#each toasts.visible as toast (toast.id)}
    <div class="toast {toast.tone}">
      <span class="tone-icon">
        <Icon name={toast.tone === "error" ? "circleAlert" : toast.tone === "success" ? "circleCheck" : "info"} />
      </span>
      <span class="message" title={toast.details}>{toast.message}</span>
      {#if toast.action}
        <button
          type="button"
          class="action"
          onclick={() => {
            toast.action?.run();
            toasts.dismiss(toast.id);
          }}>{toast.action.label}</button
        >
      {/if}
      <button type="button" class="close" aria-label={t("toast.close")} onclick={() => toasts.dismiss(toast.id)}>
        <Icon name="close" size={14} />
      </button>
    </div>
  {/each}
</div>

<style>
  .stack {
    position: absolute;
    left: 50%;
    bottom: var(--space-4);
    transform: translateX(-50%);
    z-index: 30;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    pointer-events: none;
    width: max-content;
    max-width: min(640px, 90%);
  }
  .toast {
    pointer-events: auto;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 6px 6px 6px 14px;
    border-radius: var(--radius-lg);
    background: var(--color-bg-raised);
    box-shadow: var(--shadow-toast), 0 0 0 1px var(--color-border-strong);
    animation: rise var(--duration-normal) var(--easing-standard);
  }
  .tone-icon {
    flex: none;
  }
  .success .tone-icon {
    color: var(--color-success);
  }
  .info .tone-icon {
    color: var(--color-accent-text);
  }
  .error .tone-icon {
    color: var(--color-danger);
  }
  .message {
    line-height: 1.4;
  }
  .action {
    border: 0;
    background: transparent;
    color: var(--color-accent-text);
    font-weight: var(--font-weight-medium);
    padding: 4px 8px;
    border-radius: var(--radius-md);
    cursor: pointer;
  }
  .action:hover,
  .close:hover {
    background: var(--color-bg-hover);
  }
  .close {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border: 0;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--color-text-secondary);
    cursor: pointer;
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
  }
</style>
