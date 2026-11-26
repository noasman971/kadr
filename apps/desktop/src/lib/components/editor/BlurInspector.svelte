<!--
  The inspector's Blur section for a video clip (docs/ux.md, "Blurring a face"):
  add a blur with the preview's tool, then, for each tracked blur, its style,
  strength, shape and margin, turn it off or delete it. Every finished change
  is one undo step.
-->
<script lang="ts">
  import type { BlurSettings, BlurStyleChoice, ClipView, TrackedBlurView } from "../../api/types";
  import { t } from "../../i18n/i18n.svelte";
  import { backend } from "../../api/backend";
  import { nextUnsureMoment } from "../../blur/unsureMoments";
  import { blurTool } from "../../state/blurTool.svelte";
  import { playback } from "../../state/playback.svelte";
  import { editor } from "../../state/editor.svelte";
  import Button from "../../ui/Button.svelte";
  import Icon from "../../ui/Icon.svelte";
  import IconButton from "../../ui/IconButton.svelte";
  import NumberField from "../../ui/NumberField.svelte";

  interface Props {
    clip: ClipView;
  }

  let { clip }: Props = $props();

  /** The strength a style starts with when the user switches to it. */
  const DEFAULT_STRENGTH: Record<BlurStyleChoice, number> = { blur: 30, pixels: 24, mask: 30 };
  const STYLES: Array<{ value: BlurStyleChoice; label: "blur.style.blur" | "blur.style.pixels" | "blur.style.mask" }> = [
    { value: "blur", label: "blur.style.blur" },
    { value: "pixels", label: "blur.style.pixels" },
    { value: "mask", label: "blur.style.mask" },
  ];

  const isToolBusy = $derived(blurTool.phase === "tracking");
  const unavailableReason = $derived(isToolBusy ? t("blur.busy") : null);

  function change(blur: TrackedBlurView, changes: Partial<BlurSettings>): void {
    void editor.edit({
      type: "setBlurSettings",
      clipId: clip.id,
      effectId: blur.effectId,
      settings: { ...blur.settings, ...changes },
    });
  }

  function changeStyle(blur: TrackedBlurView, style: BlurStyleChoice): void {
    if (style !== blur.settings.style) {
      change(blur, { style, strength: DEFAULT_STRENGTH[style] });
    }
  }

  function addBlur(): void {
    // The tool works on the clip at the playhead: bring the playhead into this clip first.
    const end = clip.startTicks + clip.durationTicks;
    if (editor.playheadTicks < clip.startTicks || editor.playheadTicks >= end) {
      editor.seek(clip.startTicks);
    }
    editor.selectedClipIds = [clip.id];
    blurTool.start();
  }

  function rangeCountOf(blur: TrackedBlurView): number {
    return blur.unsureRanges.length;
  }

  /** Puts the playhead in the middle of this clip's next moment to check. */
  function goToNextMoment(): void {
    const view = editor.view;
    const moment = view ? nextUnsureMoment(view, editor.playheadTicks, clip.id) : null;
    if (moment) {
      playback.seek(moment.middleTicks);
    }
  }

  const isPlayheadInClip = $derived(
    editor.playheadTicks >= clip.startTicks && editor.playheadTicks < clip.startTicks + clip.durationTicks,
  );

  /** Starts correcting `blur` from its box under the playhead (the keyboard way to drag it). */
  async function correctHere(blur: TrackedBlurView): Promise<void> {
    playback.pause();
    const overlay = await backend.getClipOverlay(clip.id, editor.playheadTicks).catch(() => null);
    const box = overlay?.blurBoxes.find((candidate) => candidate.effectId === blur.effectId);
    if (overlay && box) {
      blurTool.startCorrection(clip.id, blur.effectId, box.region, box.shape);
      blurTool.overlay = overlay;
    }
  }
</script>

<section class="section">
  <div class="section-head">
    <h3>{t("blur.section")}</h3>
    <Button disabled={isToolBusy} title={unavailableReason ?? `${t("blur.add")} (B)`} onclick={addBlur}>
      <Icon name="scanFace" size={14} />
      {t("blur.add")}
    </Button>
  </div>

  {#if clip.trackedBlurs.length === 0}
    <p class="hint">{t("blur.empty")}</p>
  {/if}

  {#each clip.trackedBlurs as blur, index (blur.effectId)}
    <div class="blur" class:off={!blur.isEnabled}>
      <div class="blur-head">
        <span class="blur-name">{t("blur.name", { number: index + 1 })}</span>
        <IconButton
          size="small"
          label={blur.isEnabled ? t("blur.turnOff") : t("blur.turnOn")}
          pressed={!blur.isEnabled}
          onclick={() => void editor.edit({ type: "setBlurEnabled", clipId: clip.id, effectId: blur.effectId, isEnabled: !blur.isEnabled })}
        >
          <Icon name={blur.isEnabled ? "eye" : "eyeOff"} size={14} />
        </IconButton>
        <IconButton
          size="small"
          label={t("blur.remove")}
          onclick={() => void editor.edit({ type: "removeBlur", clipId: clip.id, effectId: blur.effectId })}
        >
          <Icon name="trash" size={14} />
        </IconButton>
      </div>

      {#if rangeCountOf(blur) === 0}
        <p class="status sure"><Icon name="circleCheck" size={14} />{t("blur.allSure")}</p>
      {:else}
        <div class="status-row">
          <p class="status unsure"><Icon name="alert" size={14} />{t("blur.rangesToCheck", { count: rangeCountOf(blur) })}</p>
          <Button variant="ghost" title="{t('blur.nextMoment')} (Shift+B)" onclick={goToNextMoment}>{t("blur.nextMoment")}</Button>
        </div>
      {/if}
      <Button
        disabled={!isPlayheadInClip || !blur.isEnabled || isToolBusy}
        title={isPlayheadInClip ? t("blur.correctHereHint") : t("blur.playheadOutside")}
        onclick={() => void correctHere(blur)}
      >
        {t("blur.correctHere")}
      </Button>

      <div class="segmented" role="group" aria-label={t("blur.style")}>
        {#each STYLES as style (style.value)}
          <button type="button" aria-pressed={blur.settings.style === style.value} onclick={() => changeStyle(blur, style.value)}>
            {t(style.label)}
          </button>
        {/each}
      </div>
      {#if blur.settings.style === "mask"}
        <label class="row">
          <span>{t("blur.maskColor")}</span>
          <input type="color" value={blur.settings.maskColor} onchange={(event) => change(blur, { maskColor: event.currentTarget.value })} />
        </label>
      {:else}
        <NumberField
          label={blur.settings.style === "pixels" ? t("blur.pixelSize") : t("blur.strength")}
          unit="px"
          min={blur.settings.style === "pixels" ? 2 : 1}
          max={blur.settings.style === "pixels" ? 512 : 500}
          value={blur.settings.strength}
          onCommit={(strength) => change(blur, { strength })}
        />
      {/if}
      <div class="segmented" role="group" aria-label={t("blur.shape")}>
        <button type="button" aria-pressed={blur.settings.shape === "ellipse"} onclick={() => change(blur, { shape: "ellipse" })}>{t("blur.shape.ellipse")}</button>
        <button type="button" aria-pressed={blur.settings.shape === "rectangle"} onclick={() => change(blur, { shape: "rectangle" })}>{t("blur.shape.rectangle")}</button>
      </div>
      <NumberField
        label={t("blur.margin")}
        unit="%"
        min={0}
        max={200}
        value={blur.settings.marginInPercent}
        onCommit={(marginInPercent) => change(blur, { marginInPercent })}
      />
    </div>
  {/each}
</section>

<style>
  .section {
    padding: var(--space-3);
    border-bottom: 1px solid var(--color-border);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .section-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    min-height: 24px;
  }
  h3 {
    margin: 0;
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
  }
  .hint {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
    line-height: 1.45;
  }
  .blur {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    background: var(--color-bg-inset);
  }
  .blur.off {
    opacity: 0.6;
  }
  .blur-head {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  .blur-name {
    flex: 1;
    font-weight: var(--font-weight-medium);
  }
  .status {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    margin: 0;
    font-size: var(--font-size-xs);
  }
  .status-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }
  .status.sure {
    color: var(--color-success);
  }
  .status.unsure {
    color: var(--color-warning);
  }
  .segmented {
    display: flex;
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    overflow: hidden;
  }
  .segmented button {
    flex: 1;
    min-height: 28px;
    padding: 0 6px;
    border: 0;
    border-right: 1px solid var(--color-border);
    background: var(--color-bg-panel);
    color: var(--color-text-secondary);
    cursor: pointer;
  }
  .segmented button:last-child {
    border-right: 0;
  }
  .segmented button[aria-pressed="true"] {
    background: var(--color-accent-soft);
    color: var(--color-text);
  }
  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 140px;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--font-size-sm);
  }
  .row > span:first-child {
    color: var(--color-text-secondary);
  }
  input[type="color"] {
    width: 100%;
    height: 28px;
    padding: 0;
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    background: transparent;
  }
</style>
