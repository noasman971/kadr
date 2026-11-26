<!-- A keyboard key, for shortcut lists. -->
<script lang="ts">
  let { text }: { text: string } = $props();
</script>

<kbd>{text}</kbd>

<style>
  kbd {
    display: inline-block;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    line-height: 16px;
    border: 1px solid var(--color-border-strong);
    border-bottom-width: 2px;
    border-radius: var(--radius-sm);
    background: var(--color-bg-raised);
    color: var(--color-text-secondary);
    font-family: inherit;
    font-size: var(--font-size-xs);
    text-align: center;
  }
</style>
