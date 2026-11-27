<!--
  The inspector of one subtitle cue (decision record 0056): its words and times,
  split, merge and delete, then the style of its track, shared by every cue, and
  the export of the track as an .srt file. Every finished change is one edit, so
  one undo step; requests name only what changes.
-->
<script lang="ts">
  import { save } from "@tauri-apps/plugin-dialog";
  import { tick, untrack } from "svelte";

  import type {
    ClipView,
    SubtitleBackdrop,
    SubtitlePlacement,
    SubtitleStyleChanges,
    SubtitleStyleView,
    TrackView,
  } from "../../api/types";
  import type { MessageKey } from "../../i18n/messages.en";
  import { t } from "../../i18n/i18n.svelte";
  import { editor } from "../../state/editor.svelte";
  import { TICKS_PER_SECOND, snapToFrame } from "../../time/ticks";
  import Button from "../../ui/Button.svelte";
  import Icon from "../../ui/Icon.svelte";
  import NumberField from "../../ui/NumberField.svelte";
  import { subtitleFileNameFor, withHexAlpha } from "../../subtitles/subtitleSettings";

  interface Props {
    clip: ClipView;
  }

  let { clip }: Props = $props();

  // Keyed by clip id: edits name this cue by its id and read the project as it
  // is now, never `clip`, which can already be another item while the inspector
  // closes (see ClipInspector).
  const clipId = untrack(() => clip.id);
  const trackId = untrack(() => clip.trackId);

  const FONT_FAMILIES = ["Inter", "Arial", "Verdana", "Segoe UI", "Georgia", "Impact", "Courier New"];
  const BACKDROPS: Array<{ value: SubtitleBackdrop; label: MessageKey }> = [
    { value: "outline", label: "subtitles.backdrop.outline" },
    { value: "box", label: "subtitles.backdrop.box" },
    { value: "none", label: "subtitles.backdrop.none" },
  ];
  const PLACEMENTS: Array<{ value: SubtitlePlacement; label: MessageKey }> = [
    { value: "bottom", label: "subtitles.placement.bottom" },
    { value: "top", label: "subtitles.placement.top" },
  ];

  function currentTrack(): TrackView | undefined {
    return editor.view?.tracks.find((candidate) => candidate.id === trackId);
  }

  function currentCue(): ClipView | undefined {
    return currentTrack()?.clips.find((candidate) => candidate.id === clipId);
  }

  const track = $derived(editor.view?.tracks.find((candidate) => candidate.id === clip.trackId));
  const style: SubtitleStyleView | null = $derived(track?.subtitleStyle ?? null);
  const cueIndex = $derived(track?.clips.findIndex((candidate) => candidate.id === clip.id) ?? -1);
  const hasNextCue = $derived(track !== undefined && cueIndex >= 0 && cueIndex + 1 < track.clips.length);
  const playheadIsInside = $derived(
    editor.playheadTicks > clip.startTicks && editor.playheadTicks < clip.startTicks + clip.durationTicks,
  );

  let textArea: HTMLTextAreaElement | undefined = $state();
  let draftText = $state("");
  /** The words when the field got the focus; `null` while it does not have it. */
  let wordsAtFocus: string | null = null;
  const savedWords = $derived(clip.cueText ?? "");

  // The field follows the project's words, except while the user types.
  $effect(() => {
    const words = savedWords;
    if (wordsAtFocus === null) {
      draftText = words;
    }
  });

  // A cue just added asks for its words.
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

  function startTypingWords(): void {
    wordsAtFocus = draftText;
  }

  /** Sends the words typed, if the user changed them. */
  function commitDraftText(): void {
    const typedOver = wordsAtFocus;
    wordsAtFocus = null;
    const current = currentCue();
    if (!current) {
      return;
    }
    const savedText = current.cueText ?? "";
    const isChanged = draftText !== typedOver && draftText !== savedText;
    if (draftText.trim() !== "" && isChanged) {
      void editor.edit({ type: "setSubtitleCue", clipId, changes: { text: draftText } });
    } else {
      draftText = savedText;
    }
  }

  function handleTextKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
      textArea?.blur();
    } else if (event.key === "Escape") {
      draftText = savedWords;
      textArea?.blur();
    }
  }

  function secondsToTicks(seconds: number): number {
    return Math.round(seconds * TICKS_PER_SECOND);
  }

  function setStart(seconds: number): void {
    if (currentCue()) {
      void editor.edit({ type: "setSubtitleCue", clipId, changes: { startTicks: secondsToTicks(seconds) } });
    }
  }

  function setEnd(seconds: number): void {
    if (currentCue()) {
      void editor.edit({ type: "setSubtitleCue", clipId, changes: { endTicks: secondsToTicks(seconds) } });
    }
  }

  function splitAtPlayhead(): void {
    const timeTicks = snapToFrame(editor.playheadTicks, editor.frameRate);
    void editor.edit({ type: "splitClips", clipIds: [clipId], timeTicks });
  }

  function mergeWithNext(): void {
    void editor.edit({ type: "mergeSubtitleWithNext", clipId });
  }

  function deleteCue(): void {
    void editor.edit({ type: "removeClips", clipIds: [clipId] });
  }

  function setStyle(changes: SubtitleStyleChanges): void {
    if (currentTrack()?.subtitleStyle) {
      void editor.edit({ type: "setSubtitleStyle", trackId, changes });
    }
  }

  async function exportTrack(): Promise<void> {
    const current = currentTrack();
    if (!current) {
      return;
    }
    const filePath = await save({
      title: t("dialog.exportSubtitles"),
      defaultPath: subtitleFileNameFor(current.name),
      filters: [{ name: t("dialog.subtitleFiles"), extensions: ["srt"] }],
    });
    if (typeof filePath === "string") {
      await editor.exportSubtitleFile(trackId, filePath);
    }
  }
</script>

<div class="clip-header">
  <Icon name="captions" size={16} />
  <span class="clip-name" title={clip.name}>{clip.name}</span>
</div>

<section class="section">
  <div class="section-head"><h3>{t("subtitles.cue")}</h3></div>
  <textarea
    bind:this={textArea}
    bind:value={draftText}
    rows="3"
    aria-label={t("inspector.words")}
    onfocus={startTypingWords}
    onblur={commitDraftText}
    onkeydown={handleTextKeydown}
  ></textarea>
  <p class="hint">{t("subtitles.markupHint")}</p>
  <NumberField label={t("subtitles.start")} unit="s" step={0.1} decimals={3} min={0} max={86_400} value={clip.startTicks / TICKS_PER_SECOND} onCommit={setStart} />
  <NumberField
    label={t("subtitles.end")}
    unit="s"
    step={0.1}
    decimals={3}
    min={0}
    max={86_400}
    value={(clip.startTicks + clip.durationTicks) / TICKS_PER_SECOND}
    onCommit={setEnd}
  />
  <div class="actions">
    <Button disabled={!playheadIsInside} onclick={splitAtPlayhead}><Icon name="scissors" size={14} />{t("subtitles.split")}</Button>
    <Button disabled={!hasNextCue} onclick={mergeWithNext}><Icon name="merge" size={14} />{t("subtitles.mergeWithNext")}</Button>
    <Button onclick={deleteCue}><Icon name="trash" size={14} />{t("subtitles.delete")}</Button>
  </div>
</section>

{#if style}
  <section class="section">
    <div class="section-head"><h3>{t("subtitles.style")}</h3></div>
    <p class="hint">{t("subtitles.styleHint")}</p>
    <label class="row">
      <span>{t("inspector.font")}</span>
      <select value={style.fontFamily} onchange={(event) => setStyle({ fontFamily: event.currentTarget.value })}>
        {#each FONT_FAMILIES.includes(style.fontFamily) ? FONT_FAMILIES : [style.fontFamily, ...FONT_FAMILIES] as family (family)}
          <option value={family}>{family}</option>
        {/each}
      </select>
    </label>
    <NumberField label={t("inspector.fontSize")} unit="px" min={4} max={1000} value={style.fontSize} onCommit={(fontSize) => setStyle({ fontSize })} />
    <label class="row">
      <span>{t("subtitles.bold")}</span>
      <input type="checkbox" checked={style.isBold} onchange={(event) => setStyle({ isBold: event.currentTarget.checked })} />
    </label>
    <label class="row">
      <span>{t("inspector.textColor")}</span>
      <input
        type="color"
        value={style.textColor.slice(0, 7)}
        onchange={(event) => setStyle({ textColor: withHexAlpha(event.currentTarget.value, style.textColor) })}
      />
    </label>
    <div class="row">
      <span>{t("subtitles.backdrop")}</span>
      <div class="segmented" role="group" aria-label={t("subtitles.backdrop")}>
        {#each BACKDROPS as backdrop (backdrop.value)}
          <button type="button" aria-pressed={style.backdrop === backdrop.value} onclick={() => setStyle({ backdrop: backdrop.value })}>
            {t(backdrop.label)}
          </button>
        {/each}
      </div>
    </div>
    {#if style.backdrop !== "none" && style.backdropColor}
      {@const backdropColor = style.backdropColor}
      <label class="row">
        <span>{style.backdrop === "box" ? t("subtitles.boxColor") : t("subtitles.outlineColor")}</span>
        <input
          type="color"
          value={backdropColor.slice(0, 7)}
          onchange={(event) => setStyle({ backdropColor: withHexAlpha(event.currentTarget.value, backdropColor) })}
        />
      </label>
    {/if}
    {#if style.backdrop === "outline" && style.outlineWidth !== null}
      <NumberField
        label={t("subtitles.outlineWidth")}
        unit="px"
        step={0.5}
        decimals={1}
        min={0.5}
        max={100}
        value={style.outlineWidth}
        onCommit={(outlineWidth) => setStyle({ outlineWidth })}
      />
    {/if}
    <div class="row">
      <span>{t("subtitles.placement")}</span>
      <div class="segmented" role="group" aria-label={t("subtitles.placement")}>
        {#each PLACEMENTS as placement (placement.value)}
          <button type="button" aria-pressed={style.placement === placement.value} onclick={() => setStyle({ placement: placement.value })}>
            {t(placement.label)}
          </button>
        {/each}
      </div>
    </div>
    <NumberField label={t("subtitles.margin")} unit="px" min={0} max={8192} value={style.margin} onCommit={(margin) => setStyle({ margin })} />
    <NumberField
      label={t("subtitles.lineWidth")}
      unit="%"
      min={20}
      max={100}
      value={Math.round(style.maximumLineWidth * 100)}
      onCommit={(percent) => setStyle({ maximumLineWidth: percent / 100 })}
    />
  </section>
{/if}

<section class="section">
  <div class="section-head"><h3>{t("subtitles.file")}</h3></div>
  <Button onclick={() => void exportTrack()}><Icon name="download" size={14} />{t("subtitles.export")}</Button>
</section>

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
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
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
  input[type="checkbox"] {
    justify-self: start;
    width: 16px;
    height: 16px;
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
    padding: 0 4px;
    border: 0;
    border-right: 1px solid var(--color-border);
    background: var(--color-bg-inset);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    cursor: pointer;
  }
  .segmented button:last-child {
    border-right: 0;
  }
  .segmented button[aria-pressed="true"] {
    background: var(--color-accent-soft);
    color: var(--color-text);
  }
</style>
