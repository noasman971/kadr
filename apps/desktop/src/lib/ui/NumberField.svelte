<!--
  A number setting (docs/ux.md, "Adjust a clip"): type a value, use ↑ / ↓ (Shift
  for bigger steps), or drag sideways on the label. Each finished gesture (Enter,
  leaving the field, one arrow press, releasing the drag) calls `onCommit` once:
  one undo step. A value animated with keyframes is shown, not edited.
-->
<script lang="ts">
  import { t, translations } from "../i18n/i18n.svelte";
  import { formatEditableNumber, parseEditableNumber } from "./numberText";

  interface Props {
    label: string;
    /** The value in the field's own unit (%, px, °…). */
    value: number;
    unit?: string;
    step?: number;
    min?: number;
    max?: number;
    decimals?: number;
    /** Keyframes on this value: it cannot be set to a single value here yet. */
    keyframeCount?: number;
    onCommit: (value: number) => void;
  }

  let { label, value, unit = "", step = 1, min = -Infinity, max = Infinity, decimals = 0, keyframeCount = 0, onCommit }: Props =
    $props();

  /** Pixels of drag per step. */
  const PIXELS_PER_STEP = 3;
  const BIG_STEP_FACTOR = 10;

  let text = $state("");
  let isEditing = $state(false);
  /** The value shown while dragging, before it is committed. */
  let draggedValue = $state<number | null>(null);
  // The value when the field got the focus. Leaving the field compares the
  // typed text with it, never with `value`: the field can be left while the
  // component owning it closes (WebView2 blurs a focused field as it is removed),
  // and by then `value` may read an item that is gone, or another one.
  let valueAtFocus = 0;

  const shown = $derived(draggedValue ?? value);

  $effect(() => {
    if (!isEditing) {
      text = formatEditableNumber(shown, decimals, translations.language);
    }
  });

  function clamp(candidate: number): number {
    const factor = 10 ** decimals;
    return Math.min(max, Math.max(min, Math.round(candidate * factor) / factor));
  }

  /** Commits `candidate` if it differs from `baseline`, the value it replaces. */
  function commit(candidate: number, baseline: number): void {
    const clamped = clamp(candidate);
    if (clamped !== clamp(baseline)) {
      onCommit(clamped);
    }
  }

  function startEditing(): void {
    isEditing = true;
    valueAtFocus = value;
  }

  function commitText(): void {
    if (!isEditing) {
      return;
    }
    isEditing = false;
    const parsed = parseEditableNumber(text);
    if (Number.isFinite(parsed)) {
      commit(parsed, valueAtFocus);
    } else {
      text = formatEditableNumber(valueAtFocus, decimals, translations.language);
    }
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (event.key === "Enter") {
      (event.currentTarget as HTMLInputElement).blur();
    } else if (event.key === "Escape") {
      isEditing = false;
      text = formatEditableNumber(value, decimals, translations.language);
      (event.currentTarget as HTMLInputElement).blur();
      event.stopPropagation();
    } else if (event.key === "ArrowUp" || event.key === "ArrowDown") {
      event.preventDefault();
      const direction = event.key === "ArrowUp" ? 1 : -1;
      const stepped = clamp(value + direction * step * (event.shiftKey ? BIG_STEP_FACTOR : 1));
      commit(stepped, value);
      // The field shows the new value, so leaving it does not commit the old text back.
      valueAtFocus = stepped;
      text = formatEditableNumber(stepped, decimals, translations.language);
    }
    // Keys typed here are not editor shortcuts.
    event.stopPropagation();
  }

  function startDrag(event: PointerEvent): void {
    if (event.button !== 0 || keyframeCount > 0) {
      return;
    }
    const startX = event.clientX;
    const startValue = value;
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture(event.pointerId);
    const move = (moveEvent: PointerEvent) => {
      const steps = Math.round((moveEvent.clientX - startX) / PIXELS_PER_STEP);
      draggedValue = clamp(startValue + steps * step * (moveEvent.shiftKey ? BIG_STEP_FACTOR : 1));
    };
    const end = () => {
      target.removeEventListener("pointermove", move);
      if (draggedValue !== null) {
        commit(draggedValue, startValue);
      }
      draggedValue = null;
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", end, { once: true });
    target.addEventListener("pointercancel", end, { once: true });
  }
</script>

<div class="number-field">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <span class="label" class:draggable={keyframeCount === 0} onpointerdown={startDrag} title={label}>{label}</span>
  {#if keyframeCount > 0}
    <span class="animated">{t("inspector.animated", { count: keyframeCount })}</span>
  {:else}
    <span class="input">
      <input
        type="text"
        inputmode="decimal"
        aria-label={label}
        bind:value={text}
        onfocus={startEditing}
        onblur={commitText}
        onkeydown={handleKeydown}
        spellcheck="false"
      />
      {#if unit}<span class="unit">{unit}</span>{/if}
    </span>
  {/if}
</div>

<style>
  .number-field {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 96px;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--font-size-sm);
  }
  .label {
    color: var(--color-text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    user-select: none;
  }
  .label.draggable {
    cursor: ew-resize;
  }
  .input {
    display: flex;
    align-items: center;
    height: 28px;
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    background: var(--color-bg-inset);
  }
  .input:focus-within {
    border-color: var(--color-accent);
  }
  input {
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0 6px;
    border: 0;
    background: transparent;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .unit {
    padding-right: 6px;
    color: var(--color-text-muted);
  }
  .animated {
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
    text-align: right;
  }
</style>
