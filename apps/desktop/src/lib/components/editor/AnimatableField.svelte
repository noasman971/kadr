<!--
  An inspector setting that can be animated (docs/ux.md, "Animate a clip"): its
  number field shows the value at the playhead, the diamond next to it adds or
  removes a keyframe there, and once animated a lane shows its keyframes along
  the clip, with the curve of the stretch the playhead is in. Changing the value
  of an animated setting sets its keyframe at the playhead. Every finished
  gesture is one request, so one undo step.
-->
<script lang="ts">
  import { untrack } from "svelte";

  import type { ClipValueName, ClipView, KeyframeCurve, PropertyView } from "../../api/types";
  import { t } from "../../i18n/i18n.svelte";
  import {
    bezierHandlesOf,
    keyframeAt,
    nextKeyframeOffset,
    PRESET_CURVES,
    presetCurve,
    previousKeyframeOffset,
    stretchStartAt,
    valueAt,
    type BezierHandles,
    type CurveName,
  } from "../../keyframes/keyframes";
  import { editor } from "../../state/editor.svelte";
  import { playback } from "../../state/playback.svelte";
  import Icon from "../../ui/Icon.svelte";
  import IconButton from "../../ui/IconButton.svelte";
  import NumberField from "../../ui/NumberField.svelte";
  import CurveEditor from "./CurveEditor.svelte";
  import KeyframeLane from "./KeyframeLane.svelte";

  interface Props {
    clip: ClipView;
    value: ClipValueName;
    label: string;
    /** From the value's own unit to the field's: 100 shows a multiplier as a percentage. */
    fieldFactor?: number;
    unit?: string;
    step?: number;
    min?: number;
    max?: number;
    decimals?: number;
  }

  let { clip, value, label, fieldFactor = 1, unit, step, min, max, decimals }: Props = $props();

  // Like the rest of the inspector, requests name the clip and the value by what
  // they were when this field was created, and read the clip from the project
  // when they are sent: `clip` may already be another one while the field closes.
  const clipId = untrack(() => clip.id);
  const valueName = untrack(() => value);

  const CURVE_CHOICES: CurveName[] = [...PRESET_CURVES, "bezier"];
  const CURVE_LABELS: Record<CurveName, string> = $derived({
    linear: t("keyframes.curveLinear"),
    easeIn: t("keyframes.curveEaseIn"),
    easeOut: t("keyframes.curveEaseOut"),
    easeInOut: t("keyframes.curveEaseInOut"),
    hold: t("keyframes.curveHold"),
    bezier: t("keyframes.curveBezier"),
  });

  function propertyOf(view: ClipView): PropertyView {
    return view[valueName];
  }

  /** This clip as the project has it now, or `undefined` once it is gone. */
  function currentClip(): ClipView | undefined {
    return editor.view?.tracks.flatMap((track) => track.clips).find((candidate) => candidate.id === clipId);
  }

  const property = $derived(propertyOf(clip));
  const isAnimated = $derived(property.keyframes.length > 0);
  const playheadOffset = $derived(editor.playheadTicks - clip.startTicks);
  const isPlayheadOnClip = $derived(playheadOffset >= 0 && playheadOffset < clip.durationTicks);
  const shownValue = $derived(
    valueAt(property, Math.min(Math.max(playheadOffset, 0), clip.durationTicks)) * fieldFactor,
  );
  const keyframeUnderPlayhead = $derived(isPlayheadOnClip ? keyframeAt(property, playheadOffset) : undefined);
  const stretchStart = $derived(isPlayheadOnClip ? stretchStartAt(property, playheadOffset) : undefined);
  const offsets = $derived(property.keyframes.map((keyframe) => keyframe.offsetTicks));
  const previousOffset = $derived(previousKeyframeOffset(offsets, playheadOffset));
  const nextOffset = $derived(nextKeyframeOffset(offsets, playheadOffset));
  const toggleLabel = $derived(
    !isPlayheadOnClip
      ? t("keyframes.playheadOffClip")
      : keyframeUnderPlayhead
        ? t("keyframes.remove", { name: label })
        : isAnimated
          ? t("keyframes.add", { name: label })
          : t("keyframes.animate", { name: label }),
  );

  /** The playhead, from the clip's start as the project has it now. */
  function playheadOffsetNow(): { clip: ClipView; offsetTicks: number } | undefined {
    const current = currentClip();
    return current ? { clip: current, offsetTicks: editor.playheadTicks - current.startTicks } : undefined;
  }

  function toggleKeyframe(): void {
    const now = playheadOffsetNow();
    if (!now) {
      return;
    }
    if (keyframeAt(propertyOf(now.clip), now.offsetTicks)) {
      void editor.edit({ type: "removeKeyframe", clipId, values: [valueName], offsetTicks: now.offsetTicks });
    } else {
      void editor.edit({ type: "addKeyframe", clipId, value: valueName, timeTicks: editor.playheadTicks });
    }
  }

  function commitValue(fieldValue: number): void {
    if (currentClip()) {
      void editor.edit({
        type: "setClipValue",
        clipId,
        value: valueName,
        amount: fieldValue / fieldFactor,
        timeTicks: editor.playheadTicks,
      });
    }
  }

  function seekToOffset(offsetTicks: number): void {
    const current = currentClip();
    if (current) {
      playback.seek(current.startTicks + offsetTicks);
    }
  }

  function moveKeyframe(fromOffsetTicks: number, toOffsetTicks: number): void {
    void editor
      .edit({ type: "moveKeyframe", clipId, values: [valueName], fromOffsetTicks, toOffsetTicks })
      .then(() => seekToOffset(toOffsetTicks));
  }

  function removeKeyframe(offsetTicks: number): void {
    void editor.edit({ type: "removeKeyframe", clipId, values: [valueName], offsetTicks });
  }

  function setCurve(curve: KeyframeCurve): void {
    const now = playheadOffsetNow();
    const start = now ? stretchStartAt(propertyOf(now.clip), now.offsetTicks) : undefined;
    if (start) {
      void editor.edit({ type: "setKeyframeCurve", clipId, value: valueName, offsetTicks: start.offsetTicks, curve });
    }
  }

  function chooseCurve(name: CurveName): void {
    if (name === "bezier") {
      // Start from the shape the stretch has, so nothing moves until a handle does.
      const current = stretchStart ? bezierHandlesOf(stretchStart.curve) : null;
      const handles = current ?? bezierHandlesOf({ type: "linear" });
      if (handles) {
        setCurve({ type: "bezier", ...handles });
      }
    } else {
      setCurve(presetCurve(name));
    }
  }

  function changeHandles(handles: BezierHandles): void {
    setCurve({ type: "bezier", ...handles });
  }
</script>

<div class="animatable">
  <div class="field-row">
    <button
      type="button"
      class="key-toggle"
      class:on={keyframeUnderPlayhead !== undefined}
      class:animated={isAnimated}
      disabled={!isPlayheadOnClip}
      title={toggleLabel}
      aria-label={toggleLabel}
      aria-pressed={keyframeUnderPlayhead !== undefined}
      onclick={toggleKeyframe}
    >
      <Icon name="diamond" size={12} />
    </button>
    <div class="field">
      <NumberField
        {label}
        {unit}
        {step}
        {min}
        {max}
        {decimals}
        value={shownValue}
        {isAnimated}
        disabled={isAnimated && !isPlayheadOnClip}
        disabledReason={t("keyframes.animatedOffClip")}
        onCommit={commitValue}
      />
    </div>
  </div>
  {#if isAnimated}
    <div class="keyframe-row">
      <IconButton size="small" label={t("keyframes.previous")} disabled={previousOffset === undefined} onclick={() => previousOffset !== undefined && seekToOffset(previousOffset)}>
        <Icon name="chevronLeft" size={14} />
      </IconButton>
      <KeyframeLane
        {label}
        keyframes={property.keyframes}
        clipStartTicks={clip.startTicks}
        clipDurationTicks={clip.durationTicks}
        playheadOffsetTicks={playheadOffset}
        stretchStartTicks={stretchStart?.offsetTicks}
        frameRate={editor.frameRate}
        onSeek={seekToOffset}
        onMove={moveKeyframe}
        onRemove={removeKeyframe}
      />
      <IconButton size="small" label={t("keyframes.next")} disabled={nextOffset === undefined} onclick={() => nextOffset !== undefined && seekToOffset(nextOffset)}>
        <Icon name="chevronRight" size={14} />
      </IconButton>
    </div>
    {#if stretchStart}
      {@const curveType = stretchStart.curve.type}
      <label class="curve-row">
        <span>{t("keyframes.curve")}</span>
        <select value={curveType} onchange={(event) => chooseCurve(event.currentTarget.value as CurveName)}>
          {#each CURVE_CHOICES as name (name)}
            <option value={name}>{CURVE_LABELS[name]}</option>
          {/each}
        </select>
      </label>
      {#if stretchStart.curve.type === "bezier"}
        {@const handles = bezierHandlesOf(stretchStart.curve)}
        {#if handles}
          <div class="curve-editor-row">
            <CurveEditor {handles} onChange={changeHandles} />
            <p class="hint">{t("keyframes.curveHint")}</p>
          </div>
        {/if}
      {/if}
    {/if}
  {/if}
</div>

<style>
  .animatable {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .field-row {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  .field {
    flex: 1;
    min-width: 0;
  }
  .key-toggle {
    display: grid;
    place-items: center;
    flex: none;
    width: 20px;
    height: 20px;
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
  }
  .key-toggle:hover:not(:disabled) {
    background: var(--color-bg-hover);
    color: var(--color-text);
  }
  .key-toggle.animated {
    color: var(--color-accent-text);
  }
  /* On a keyframe: a filled diamond (never colour alone: the shape changes). */
  .key-toggle.on :global(svg) {
    fill: currentColor;
  }
  .key-toggle:disabled {
    cursor: not-allowed;
    opacity: 0.45;
  }
  .key-toggle:focus-visible {
    outline: 2px solid var(--color-focus-ring);
    outline-offset: 1px;
  }
  .keyframe-row {
    display: flex;
    align-items: center;
    gap: 2px;
    padding-left: 22px;
  }
  .curve-row {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    align-items: center;
    gap: var(--space-2);
    padding-left: 24px;
    font-size: var(--font-size-sm);
  }
  .curve-row > span {
    color: var(--color-text-secondary);
  }
  select {
    height: 28px;
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    background: var(--color-bg-inset);
    padding: 0 6px;
  }
  .curve-editor-row {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    align-items: center;
    padding-left: 24px;
  }
  .hint {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
    line-height: 1.45;
  }
</style>
