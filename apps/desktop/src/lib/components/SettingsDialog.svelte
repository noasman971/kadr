<!-- Settings (docs/ux.md): language, theme and interface size. -->
<script lang="ts">
  import { systemLanguage, t, type LanguageChoice } from "../i18n/i18n.svelte";
  import {
    LARGEST_INTERFACE_SCALE,
    SMALLEST_INTERFACE_SCALE,
    preferences,
    type ThemeChoice,
  } from "../state/preferences.svelte";
  import Dialog from "../ui/Dialog.svelte";

  interface Props {
    onClose: () => void;
  }

  let { onClose }: Props = $props();

  const LANGUAGE_NAMES = { fr: "Français", en: "English" } as const;
  const THEMES: ThemeChoice[] = ["dark", "light", "system"];
</script>

<Dialog title={t("settings.title")} widthInPixels={560} {onClose}>
  <div class="setting">
    <label for="language-setting">{t("settings.language")}</label>
    <select
      id="language-setting"
      value={preferences.language}
      onchange={(event) => preferences.update({ language: event.currentTarget.value as LanguageChoice })}
    >
      <option value="automatic">{t("settings.language.automatic", { language: LANGUAGE_NAMES[systemLanguage()] })}</option>
      <option value="fr">{LANGUAGE_NAMES.fr}</option>
      <option value="en">{LANGUAGE_NAMES.en}</option>
    </select>
  </div>
  <div class="setting">
    <span id="theme-setting">{t("settings.theme")}</span>
    <div class="segmented" role="radiogroup" aria-labelledby="theme-setting">
      {#each THEMES as theme (theme)}
        <button
          type="button"
          role="radio"
          aria-checked={preferences.theme === theme}
          onclick={() => preferences.update({ theme })}>{t(`settings.theme.${theme}`)}</button
        >
      {/each}
    </div>
  </div>
  <div class="setting">
    <label for="scale-setting">{t("settings.uiScale")}</label>
    <div class="scale">
      <input
        id="scale-setting"
        type="range"
        min={SMALLEST_INTERFACE_SCALE}
        max={LARGEST_INTERFACE_SCALE}
        step="0.05"
        value={preferences.interfaceScale}
        onchange={(event) => preferences.update({ interfaceScale: Number(event.currentTarget.value) })}
      />
      <span class="tabular">{Math.round(preferences.interfaceScale * 100)} %</span>
    </div>
  </div>
</Dialog>

<style>
  .setting {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-3) 0;
    border-bottom: 1px solid var(--color-border);
  }
  .setting:last-child {
    border-bottom: 0;
  }
  label,
  .setting > span {
    font-weight: var(--font-weight-medium);
  }
  select {
    width: 220px;
    height: var(--control-height);
    padding: 0 var(--space-2);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    background: var(--color-bg-inset);
  }
  .segmented {
    display: inline-flex;
    padding: 2px;
    gap: 2px;
    border-radius: var(--radius-md);
    background: var(--color-bg-inset);
  }
  .segmented button {
    padding: 4px 12px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-secondary);
    cursor: pointer;
  }
  .segmented button[aria-checked="true"] {
    background: var(--color-bg-raised);
    color: var(--color-text);
    box-shadow: 0 0 0 1px var(--color-border-strong);
  }
  .scale {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 220px;
  }
  .scale input {
    flex: 1;
    accent-color: var(--color-accent);
  }
</style>
