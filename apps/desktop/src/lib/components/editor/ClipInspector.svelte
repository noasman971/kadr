<!--
  The settings of one selected clip (docs/ux.md, "Adjust a clip"): picture,
  sound, speed, colour and text, each shown only for the clips it applies to,
  each with a Reset. Every finished change is one edit, so one undo step.
-->
<script lang="ts">
  import { tick, untrack } from "svelte";

  import type { ClipValueName, ClipView, ColorSettings, CropView, EditRequest, TextAlignment, TextSettings } from "../../api/types";
  import { t, translations } from "../../i18n/i18n.svelte";
  import { editor } from "../../state/editor.svelte";
  import { TICKS_PER_SECOND } from "../../time/ticks";
  import Button from "../../ui/Button.svelte";
  import Icon from "../../ui/Icon.svelte";
  import IconButton from "../../ui/IconButton.svelte";
  import type { IconName } from "../../ui/icons";
  import NumberField from "../../ui/NumberField.svelte";
  import BlurInspector from "./BlurInspector.svelte";
  import SilenceRemovalDialog from "./SilenceRemovalDialog.svelte";

  interface Props {
    clip: ClipView;
  }

  let { clip }: Props = $props();

  // The inspector panel keys this component by clip id, so it edits one clip
  // for its whole life. Changes name that clip by its id and read its settings
  // from the project, never from `clip`: a field can send its value while the
  // inspector closes (the colour picker does when it is removed), and by then
  // `clip` is no longer this clip.
  const clipId = untrack(() => clip.id);

  /** This clip as the project has it now, or `undefined` once it is gone. */
  function currentClip(): ClipView | undefined {
    return editor.view?.tracks.flatMap((track) => track.clips).find((candidate) => candidate.id === clipId);
  }

  function colorSettingsOf(view: ClipView): ColorSettings {
    return view.colorAdjustment
      ? {
          exposure: view.colorAdjustment.exposure,
          contrast: view.colorAdjustment.contrast,
          saturation: view.colorAdjustment.saturation,
          temperature: view.colorAdjustment.temperature,
        }
      : NEUTRAL_COLOR;
  }

  const KIND_ICONS: Record<ClipView["kind"], IconName> = { video: "film", audio: "music", image: "image", text: "type" };
  const SPEED_CHOICES = [0.5, 1, 1.5, 2];
  const FONT_FAMILIES = ["Inter", "Arial", "Georgia", "Times New Roman", "Courier New", "Impact", "Segoe UI", "Verdana"];
  const ALIGNMENTS: Array<{ value: TextAlignment; icon: IconName; label: "inspector.alignLeft" | "inspector.alignCenter" | "inspector.alignRight" }> = [
    { value: "left", icon: "alignLeft", label: "inspector.alignLeft" },
    { value: "center", icon: "alignCenter", label: "inspector.alignCenter" },
    { value: "right", icon: "alignRight", label: "inspector.alignRight" },
  ];
  const NEUTRAL_COLOR: ColorSettings = { exposure: 0, contrast: 1, saturation: 1, temperature: 0 };

  let isSilenceDialogOpen = $state(false);

  const hasPictureSettings = $derived(clip.kind !== "audio");
  const hasSpeed = $derived(clip.kind === "video" || clip.kind === "audio");
  const hasColor = $derived(clip.kind === "video" || clip.kind === "image");
  const colorSettings: ColorSettings = $derived(colorSettingsOf(clip));

  let textArea: HTMLTextAreaElement | undefined = $state();
  let draftText = $state("");
  let customSpeed = $state(false);

  /** The title's words as the project has them. */
  const savedWords = $derived(clip.text?.text ?? "");
  /** The words when the field got the focus; `null` while it does not have it. */
  let wordsAtFocus: string | null = null;

  // The field follows the project's words, but not while the user types in it:
  // the answer to an earlier change (the font size, say) can arrive meanwhile,
  // and must not replace what is being typed.
  $effect(() => {
    const words = savedWords;
    if (wordsAtFocus === null) {
      draftText = words;
    }
  });

  // A title just added asks for its words: focus the text field and select its
  // placeholder words, once they are rendered, so typing replaces them.
  $effect(() => {
    const field = textArea;
    if (editor.clipToFocusForText === clipId && field) {
      editor.clipToFocusForText = null;
      void tick().then(() => {
        field.focus();
        field.select();
      });
    }
  });

  function edit(request: EditRequest): void {
    void editor.edit(request);
  }

  function setValue(value: ClipValueName, amount: number): void {
    edit({ type: "setClipValue", clipId, value, amount });
  }

  // Crop, colour and text changes name only what changed. The project here can
  // be behind the back end's (an earlier change not answered yet): sending the
  // other settings from it would put their old values back.

  function setCrop(side: keyof CropView, percent: number): void {
    if (currentClip()) {
      const changes: Partial<CropView> = {};
      changes[side] = percent / 100;
      edit({ type: "setClipCrop", clipId, changes });
    }
  }

  function setColor(changes: Partial<ColorSettings>): void {
    if (currentClip()) {
      edit({ type: "setClipColor", clipId, changes });
    }
  }

  function setText(changes: Partial<TextSettings>): void {
    if (currentClip()?.text) {
      edit({ type: "setClipText", clipId, changes });
    }
  }

  function startTypingWords(): void {
    wordsAtFocus = draftText;
  }

  /** Sends the words typed, if the user changed them. */
  function commitDraftText(): void {
    const typedOver = wordsAtFocus;
    wordsAtFocus = null;
    const current = currentClip()?.text;
    if (!current) {
      return;
    }
    const isChanged = draftText !== typedOver && draftText !== current.text;
    if (draftText.trim() !== "" && isChanged) {
      setText({ text: draftText });
    } else {
      draftText = current.text;
    }
  }

  function handleTextKeydown(event: KeyboardEvent): void {
    // Typing words is not using shortcuts. Ctrl+Enter validates.
    event.stopPropagation();
    if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
      textArea?.blur();
    } else if (event.key === "Escape") {
      draftText = savedWords;
      textArea?.blur();
    }
  }

  function setTextDuration(seconds: number): void {
    const current = currentClip();
    if (current) {
      edit({ type: "trimClip", clipId, edge: "end", timeTicks: current.startTicks + Math.round(seconds * TICKS_PER_SECOND) });
    }
  }
</script>

<div class="clip-header">
  <Icon name={KIND_ICONS[clip.kind]} size={16} />
  <span class="clip-name" title={clip.name}>{clip.name}</span>
</div>

{#if hasPictureSettings}
  <section class="section">
    <div class="section-head">
      <h3>{t("inspector.picture")}</h3>
      <IconButton size="small" label={t("inspector.reset")} onclick={() => edit({ type: "resetClipPicture", clipId })}>
        <Icon name="undo" size={14} />
      </IconButton>
    </div>
    <NumberField label={t("inspector.positionX")} unit="px" value={clip.positionX.value} keyframeCount={clip.positionX.keyframeCount} onCommit={(value) => setValue("positionX", value)} />
    <NumberField label={t("inspector.positionY")} unit="px" value={clip.positionY.value} keyframeCount={clip.positionY.keyframeCount} onCommit={(value) => setValue("positionY", value)} />
    <NumberField label={t("inspector.scale")} unit="%" min={1} max={1000} value={clip.scale.value * 100} keyframeCount={clip.scale.keyframeCount} onCommit={(value) => setValue("scale", value / 100)} />
    <NumberField label={t("inspector.rotation")} unit="°" min={-360} max={360} value={clip.rotation.value} keyframeCount={clip.rotation.keyframeCount} onCommit={(value) => setValue("rotation", value)} />
    <NumberField label={t("inspector.opacity")} unit="%" min={0} max={100} value={clip.opacity.value * 100} keyframeCount={clip.opacity.keyframeCount} onCommit={(value) => setValue("opacity", value / 100)} />
    {#if clip.kind !== "text"}
      <details>
        <summary>{t("inspector.crop")}</summary>
        <div class="details-body">
          <NumberField label={t("inspector.cropLeft")} unit="%" min={0} max={90} value={clip.crop.left * 100} onCommit={(value) => setCrop("left", value)} />
          <NumberField label={t("inspector.cropTop")} unit="%" min={0} max={90} value={clip.crop.top * 100} onCommit={(value) => setCrop("top", value)} />
          <NumberField label={t("inspector.cropRight")} unit="%" min={0} max={90} value={clip.crop.right * 100} onCommit={(value) => setCrop("right", value)} />
          <NumberField label={t("inspector.cropBottom")} unit="%" min={0} max={90} value={clip.crop.bottom * 100} onCommit={(value) => setCrop("bottom", value)} />
        </div>
      </details>
    {/if}
  </section>
{/if}

{#if clip.hasSound}
  <section class="section">
    <div class="section-head">
      <h3>{t("inspector.sound")}</h3>
      <IconButton size="small" label={t("inspector.reset")} disabled={clip.volume.keyframeCount > 0} onclick={() => setValue("volume", 1)}>
        <Icon name="undo" size={14} />
      </IconButton>
    </div>
    <NumberField label={t("inspector.volume")} unit="%" min={0} max={400} value={clip.volume.value * 100} keyframeCount={clip.volume.keyframeCount} onCommit={(value) => setValue("volume", value / 100)} />
    {#if clip.volume.keyframeCount === 0}
      <input
        class="slider"
        type="range"
        min="0"
        max="400"
        step="1"
        aria-label={t("inspector.volume")}
        value={clip.volume.value * 100}
        onchange={(event) => setValue("volume", Number(event.currentTarget.value) / 100)}
      />
    {/if}
    <Button onclick={() => (isSilenceDialogOpen = true)}><Icon name="scissors" size={14} />{t("inspector.removeSilences")}</Button>
  </section>
  {#if isSilenceDialogOpen}
    <SilenceRemovalDialog {clip} onClose={() => (isSilenceDialogOpen = false)} />
  {/if}
{/if}

{#if hasSpeed}
  <section class="section">
    <div class="section-head">
      <h3>{t("inspector.speed")}</h3>
      <IconButton size="small" label={t("inspector.reset")} disabled={clip.speed === 1} onclick={() => edit({ type: "setClipSpeed", clipId, speed: 1 })}>
        <Icon name="undo" size={14} />
      </IconButton>
    </div>
    <div class="segmented" role="group" aria-label={t("inspector.speed")}>
      {#each SPEED_CHOICES as speed (speed)}
        <button type="button" aria-pressed={!customSpeed && clip.speed === speed} onclick={() => { customSpeed = false; edit({ type: "setClipSpeed", clipId, speed }); }}>
          {translations.formatNumber(speed, 1)}×
        </button>
      {/each}
      <button type="button" aria-pressed={customSpeed || !SPEED_CHOICES.includes(clip.speed)} onclick={() => (customSpeed = true)}>
        {t("inspector.speedCustom")}
      </button>
    </div>
    {#if customSpeed || !SPEED_CHOICES.includes(clip.speed)}
      <NumberField label={t("inspector.speedValue")} unit="×" step={0.05} decimals={2} min={0.1} max={16} value={clip.speed} onCommit={(speed) => edit({ type: "setClipSpeed", clipId, speed })} />
    {/if}
    <p class="hint">{t("inspector.speedHint")}</p>
  </section>
{/if}

{#if hasColor}
  <section class="section">
    <div class="section-head">
      <h3>{t("inspector.color")}</h3>
      <IconButton size="small" label={t("inspector.reset")} disabled={clip.colorAdjustment === null} onclick={() => edit({ type: "resetClipColor", clipId })}>
        <Icon name="undo" size={14} />
      </IconButton>
    </div>
    {#if clip.colorAdjustment === null}<p class="hint">{t("inspector.colorOff")}</p>{/if}
    <NumberField label={t("inspector.exposure")} step={0.1} decimals={1} min={-4} max={4} value={colorSettings.exposure} onCommit={(exposure) => setColor({ exposure })} />
    <NumberField label={t("inspector.contrast")} unit="%" min={0} max={200} value={colorSettings.contrast * 100} onCommit={(value) => setColor({ contrast: value / 100 })} />
    <NumberField label={t("inspector.saturation")} unit="%" min={0} max={200} value={colorSettings.saturation * 100} onCommit={(value) => setColor({ saturation: value / 100 })} />
    <NumberField label={t("inspector.temperature")} min={-100} max={100} value={colorSettings.temperature * 100} onCommit={(value) => setColor({ temperature: value / 100 })} />
  </section>
{/if}

{#if clip.kind === "video"}
  <BlurInspector {clip} />
{/if}

{#if clip.text}
  {@const text = clip.text}
  <section class="section">
    <div class="section-head"><h3>{t("inspector.text")}</h3></div>
    <textarea
      bind:this={textArea}
      bind:value={draftText}
      rows="3"
      aria-label={t("inspector.words")}
      onfocus={startTypingWords}
      onblur={commitDraftText}
      onkeydown={handleTextKeydown}
    ></textarea>
    <label class="row">
      <span>{t("inspector.font")}</span>
      <select value={text.fontFamily} onchange={(event) => setText({ fontFamily: event.currentTarget.value })}>
        {#each FONT_FAMILIES.includes(text.fontFamily) ? FONT_FAMILIES : [text.fontFamily, ...FONT_FAMILIES] as family (family)}
          <option value={family}>{family}</option>
        {/each}
      </select>
    </label>
    <NumberField label={t("inspector.fontSize")} unit="px" min={4} max={1000} value={text.fontSize} onCommit={(fontSize) => setText({ fontSize })} />
    <label class="row">
      <span>{t("inspector.textColor")}</span>
      <input type="color" value={text.color.slice(0, 7)} onchange={(event) => setText({ color: event.currentTarget.value })} />
    </label>
    <div class="row">
      <span>{t("inspector.alignment")}</span>
      <div class="segmented" role="group" aria-label={t("inspector.alignment")}>
        {#each ALIGNMENTS as alignment (alignment.value)}
          <button type="button" aria-pressed={text.alignment === alignment.value} aria-label={t(alignment.label)} title={t(alignment.label)} onclick={() => setText({ alignment: alignment.value })}>
            <Icon name={alignment.icon} size={14} />
          </button>
        {/each}
      </div>
    </div>
    <NumberField label={t("inspector.duration")} unit="s" step={0.5} decimals={2} min={0.1} max={3600} value={clip.durationTicks / TICKS_PER_SECOND} onCommit={setTextDuration} />
  </section>
{/if}

<style>
  .clip-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3);
    border-bottom: 1px solid var(--color-border);
    color: var(--color-text-secondary);
  }
  .clip-name {
    color: var(--color-text);
    font-weight: var(--font-weight-medium);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
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
  details summary {
    cursor: pointer;
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
  }
  .details-body {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding-top: var(--space-2);
  }
  .slider {
    width: 100%;
    accent-color: var(--color-accent);
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
    background: var(--color-bg-inset);
    color: var(--color-text-secondary);
    font-variant-numeric: tabular-nums;
    cursor: pointer;
    display: grid;
    place-items: center;
  }
  .segmented button:last-child {
    border-right: 0;
  }
  .segmented button[aria-pressed="true"] {
    background: var(--color-accent-soft);
    color: var(--color-text);
  }
  textarea {
    width: 100%;
    resize: vertical;
    padding: var(--space-2);
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    background: var(--color-bg-inset);
    font: inherit;
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
  select {
    height: 28px;
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    background: var(--color-bg-inset);
    padding: 0 6px;
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
