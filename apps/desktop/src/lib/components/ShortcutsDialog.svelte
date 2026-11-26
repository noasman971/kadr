<!-- The keyboard shortcut list, opened with "?" (docs/ux.md, "Keyboard shortcuts"). -->
<script lang="ts">
  import { t } from "../i18n/i18n.svelte";
  import type { MessageKey } from "../i18n/messages.en";
  import Dialog from "../ui/Dialog.svelte";
  import KeyCap from "../ui/KeyCap.svelte";

  interface Props {
    onClose: () => void;
  }

  let { onClose }: Props = $props();

  interface Shortcut {
    action: MessageKey;
    /** Key caps; entries starting with "key." are translated. */
    keys: string[];
  }

  interface ShortcutGroup {
    title: MessageKey;
    shortcuts: Shortcut[];
  }

  const GROUPS: ShortcutGroup[] = [
    {
      title: "shortcuts.playback",
      shortcuts: [
        { action: "shortcuts.playPause", keys: ["key.space"] },
        { action: "shortcuts.shuttle", keys: ["J", "K", "L"] },
        { action: "shortcuts.frameStep", keys: ["←", "→"] },
        { action: "shortcuts.secondStep", keys: ["key.shift", "←", "→"] },
        { action: "shortcuts.startEnd", keys: ["key.home", "key.end"] },
      ],
    },
    {
      title: "shortcuts.editing",
      shortcuts: [
        { action: "shortcuts.undo", keys: ["Ctrl", "Z"] },
        { action: "shortcuts.redo", keys: ["Ctrl", "Y"] },
        { action: "shortcuts.deselect", keys: ["key.escape"] },
      ],
    },
    {
      title: "shortcuts.timeline",
      shortcuts: [
        { action: "shortcuts.zoom", keys: ["+", "−"] },
        { action: "shortcuts.fit", keys: ["key.shift", "Z"] },
      ],
    },
    {
      title: "shortcuts.project",
      shortcuts: [
        { action: "shortcuts.import", keys: ["Ctrl", "I"] },
        { action: "shortcuts.save", keys: ["Ctrl", "S"] },
        { action: "shortcuts.list", keys: ["?"] },
      ],
    },
  ];

  function keyText(key: string): string {
    return key.startsWith("key.") ? t(key as MessageKey) : key;
  }
</script>

<Dialog title={t("shortcuts.title")} widthInPixels={820} {onClose}>
  <div class="columns">
    {#each GROUPS as group (group.title)}
      <section>
        <h3>{t(group.title)}</h3>
        {#each group.shortcuts as shortcut (shortcut.action)}
          <div class="shortcut">
            <span>{t(shortcut.action)}</span>
            <span class="keys">
              {#each shortcut.keys as key (key)}<KeyCap text={keyText(key)} />{/each}
            </span>
          </div>
        {/each}
      </section>
    {/each}
  </div>
</Dialog>

<style>
  .columns {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: var(--space-2) var(--space-8);
  }
  h3 {
    margin: var(--space-3) 0 var(--space-2);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-semibold);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--color-text-muted);
  }
  .shortcut {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: var(--space-4);
    padding: 5px 0;
    border-bottom: 1px solid var(--color-border);
    font-size: var(--font-size-sm);
  }
  .keys {
    display: flex;
    gap: 4px;
    flex: none;
  }
</style>
