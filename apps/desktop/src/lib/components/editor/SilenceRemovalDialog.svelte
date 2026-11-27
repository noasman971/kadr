<!--
  Remove silences (docs/desktop-app.md, "Removing silences"): the clip's sound is
  searched again whenever a setting changes, the strip shows what would go, and
  one click cuts it all as one undo step. Nothing changes before that click.
-->
<script lang="ts">
  import { onDestroy, untrack } from "svelte";

  import { backend } from "../../api/backend";
  import type { ClipView, SilencePlan, SilenceSearchSettings } from "../../api/types";
  import { t, translations } from "../../i18n/i18n.svelte";
  import {
    DEFAULT_SILENCE_SEARCH_SETTINGS,
    SILENCE_SETTING_LIMITS,
    clampedSetting,
    clipStretches,
  } from "../../silences/silencePlan";
  import { describeError, isSupersededError } from "../../state/errors";
  import { editor } from "../../state/editor.svelte";
  import { toasts } from "../../state/toasts.svelte";
  import { formatShortDuration, ticksToSeconds } from "../../time/ticks";
  import Button from "../../ui/Button.svelte";
  import Dialog from "../../ui/Dialog.svelte";
  import Icon from "../../ui/Icon.svelte";
  import NumberField from "../../ui/NumberField.svelte";

  interface Props {
    clip: ClipView;
    onClose: () => void;
  }

  let { clip, onClose }: Props = $props();

  /** Settings are searched again this long after the last change. */
  const SEARCH_DELAY_IN_MILLISECONDS = 250;

  const clipId = untrack(() => clip.id);
  let settings = $state<SilenceSearchSettings>({ ...DEFAULT_SILENCE_SEARCH_SETTINGS });
  let phase = $state<"searching" | "found" | "failed">("searching");
  let fractionSearched = $state(0);
  let plan = $state<SilencePlan | null>(null);
  let failure = $state("");
  let isRemoving = $state(false);

  /** The clip as the project has it now (Claude or an undo may change it meanwhile). */
  const currentClip = $derived(
    editor.view?.tracks.flatMap((track) => track.clips).find((candidate) => candidate.id === clipId),
  );
  /** What the search depends on: the settings, and where the clip is and what it plays. */
  const searchKey = $derived(
    currentClip
      ? JSON.stringify([
          settings,
          currentClip.startTicks,
          currentClip.durationTicks,
          currentClip.sourceStartTicks,
          currentClip.speed,
        ])
      : null,
  );
  const stretches = $derived(
    plan && currentClip ? clipStretches(currentClip.startTicks, currentClip.durationTicks, plan.cuts) : [],
  );
  const canRemove = $derived(phase === "found" && plan !== null && plan.cuts.length > 0 && !isRemoving);

  let searchNumber = 0;
  let pendingSearch: number | undefined;

  $effect(() => {
    const key = searchKey;
    if (key === null) {
      onClose();
      return;
    }
    window.clearTimeout(pendingSearch);
    pendingSearch = window.setTimeout(() => void search(), SEARCH_DELAY_IN_MILLISECONDS);
  });

  onDestroy(() => {
    window.clearTimeout(pendingSearch);
    searchNumber += 1;
    void backend.stopSilenceSearch().catch(() => undefined);
  });

  async function search(): Promise<void> {
    const thisSearch = ++searchNumber;
    phase = "searching";
    fractionSearched = 0;
    try {
      const found = await backend.findClipSilences(clipId, $state.snapshot(settings), (fraction) => {
        if (thisSearch === searchNumber) {
          fractionSearched = fraction;
        }
      });
      if (thisSearch === searchNumber) {
        plan = found;
        phase = "found";
      }
    } catch (error) {
      if (thisSearch === searchNumber && !isSupersededError(error)) {
        failure = describeError(error).message;
        phase = "failed";
      }
    }
  }

  function changeSetting(name: keyof SilenceSearchSettings, value: number): void {
    settings = { ...settings, [name]: clampedSetting(name, value) };
  }

  function seconds(ticks: number): string {
    return translations.formatNumber(ticksToSeconds(ticks), 1);
  }

  async function removeSilences(): Promise<void> {
    if (!plan || !currentClip) {
      return;
    }
    const { cuts, removedTicks } = plan;
    const lengthBefore = currentClip.durationTicks;
    isRemoving = true;
    await editor.edit({ type: "removeSilences", clipId, cuts });
    isRemoving = false;
    const lengthAfter = currentClip?.durationTicks ?? lengthBefore;
    if (lengthAfter < lengthBefore) {
      toasts.show({
        tone: "success",
        message: t("toast.silencesRemoved", { count: cuts.length, seconds: seconds(removedTicks) }),
        action: { label: t("toast.undo"), run: () => void editor.undo() },
      });
      onClose();
    }
  }
</script>

<Dialog title={t("silences.title")} widthInPixels={600} {onClose}>
  <p class="intro">{t("silences.intro")}</p>

  <div class="settings">
    <NumberField
      label={t("silences.threshold")}
      unit="dB"
      min={SILENCE_SETTING_LIMITS.thresholdInDecibels.min}
      max={SILENCE_SETTING_LIMITS.thresholdInDecibels.max}
      value={settings.thresholdInDecibels}
      onCommit={(value) => changeSetting("thresholdInDecibels", value)}
    />
    <NumberField
      label={t("silences.minimumSilence")}
      unit="s"
      step={0.1}
      decimals={1}
      min={SILENCE_SETTING_LIMITS.minimumSilenceInSeconds.min}
      max={SILENCE_SETTING_LIMITS.minimumSilenceInSeconds.max}
      value={settings.minimumSilenceInSeconds}
      onCommit={(value) => changeSetting("minimumSilenceInSeconds", value)}
    />
    <NumberField
      label={t("silences.keptPause")}
      unit="s"
      step={0.05}
      decimals={2}
      min={SILENCE_SETTING_LIMITS.keptPauseInSeconds.min}
      max={SILENCE_SETTING_LIMITS.keptPauseInSeconds.max}
      value={settings.keptPauseInSeconds}
      onCommit={(value) => changeSetting("keptPauseInSeconds", value)}
    />
  </div>
  <p class="hint">{t("silences.thresholdHint")}</p>

  <div class="result" aria-live="polite">
    {#if phase === "searching"}
      {@const percent = Math.floor(fractionSearched * 100)}
      <p>{t("silences.searching", { percent })}</p>
      <div class="bar" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={percent}>
        <span style:width="{percent}%"></span>
      </div>
    {:else if phase === "failed"}
      <p class="failure"><Icon name="alert" size={16} />{failure}</p>
    {:else if plan && currentClip}
      <div class="strip" aria-hidden="true">
        {#each stretches as stretch, index (index)}
          <span
            class:cut={stretch.isCut}
            style:left="{stretch.startFraction * 100}%"
            style:width="{stretch.widthFraction * 100}%"
          ></span>
        {/each}
      </div>
      <div class="legend">
        <span><i class="swatch kept"></i>{t("silences.kept")}</span>
        <span><i class="swatch cut"></i>{t("silences.removed")}</span>
      </div>
      {#if plan.wholeClipIsSilent}
        <p>{t("silences.wholeClipSilent")}</p>
      {:else if plan.cuts.length === 0}
        <p>{t("silences.none")}</p>
      {:else}
        <p class="summary tabular">
          {t("silences.found", {
            count: plan.cuts.length,
            seconds: seconds(plan.removedTicks),
            before: formatShortDuration(currentClip.durationTicks),
            after: formatShortDuration(currentClip.durationTicks - plan.removedTicks),
          })}
        </p>
      {/if}
    {/if}
  </div>

  {#snippet footer()}
    <span class="spacer"></span>
    <Button onclick={onClose}>{t("common.cancel")}</Button>
    <Button variant="primary" disabled={!canRemove} onclick={() => void removeSilences()}>
      <Icon name="scissors" />{t("silences.remove")}
    </Button>
  {/snippet}
</Dialog>

<style>
  .intro,
  .hint {
    margin: 0;
    color: var(--color-text-secondary);
  }
  .hint {
    font-size: var(--font-size-sm);
  }
  .settings {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: var(--space-4);
    margin: var(--space-5) 0 var(--space-2);
  }
  .result {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-height: 112px;
    margin-top: var(--space-5);
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--color-bg-inset);
  }
  .result p {
    margin: 0;
  }
  .bar {
    height: 8px;
    border-radius: 4px;
    background: var(--color-bg-raised);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--color-accent);
    transition: width var(--duration-fast) linear;
  }
  .strip {
    position: relative;
    height: 28px;
    border-radius: var(--radius-sm);
    overflow: hidden;
    background: var(--color-bg-raised);
  }
  .strip span {
    position: absolute;
    top: 0;
    bottom: 0;
    background: var(--color-clip-video);
  }
  .strip span.cut,
  .swatch.cut {
    background: repeating-linear-gradient(
      135deg,
      var(--color-danger) 0 4px,
      var(--color-danger-soft) 4px 8px
    );
  }
  .legend {
    display: flex;
    gap: var(--space-5);
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }
  .legend span {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }
  .swatch {
    display: inline-block;
    width: 12px;
    height: 12px;
    border-radius: 3px;
  }
  .swatch.kept {
    background: var(--color-clip-video);
  }
  .failure {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--color-danger);
  }
  .spacer {
    flex: 1;
  }
</style>
