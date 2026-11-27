<!--
  A small editor for the curve between two keyframes (docs/ux.md, "Animate a
  clip"): time goes right, the share of the change goes up, from the first
  keyframe (bottom left) to the next one (top right). Two handles shape it, as
  in CSS `cubic-bezier()`, and stay inside the square so the value never goes
  past either keyframe. Dragging a handle shows the curve at once and sends it
  on release; the arrow keys move the focused handle (Shift: finer). Each
  finished gesture is one change, so one undo step.
-->
<script lang="ts">
  import { t } from "../../i18n/i18n.svelte";
  import { clampedHandles, type BezierHandles } from "../../keyframes/keyframes";
  import { pointerOffsetX, pointerOffsetY } from "../../ui/pointer";

  interface Props {
    handles: BezierHandles;
    onChange: (handles: BezierHandles) => void;
  }

  let { handles, onChange }: Props = $props();

  /** The drawing's side and its margin, in CSS pixels. */
  const SIZE = 128;
  const MARGIN = 12;
  const SQUARE = SIZE - 2 * MARGIN;
  const KEY_STEP = 0.05;
  const FINE_KEY_STEP = 0.01;
  const HANDLES = [1, 2] as const;

  let surface: SVGSVGElement | undefined = $state();
  /** The handles while one is dragged, before they are sent. */
  let draft = $state<BezierHandles | null>(null);

  const shown = $derived(draft ?? handles);

  const x = (fraction: number) => MARGIN + fraction * SQUARE;
  const y = (fraction: number) => MARGIN + (1 - fraction) * SQUARE;

  const curvePath = $derived(
    `M ${x(0)} ${y(0)} C ${x(shown.x1)} ${y(shown.y1)}, ${x(shown.x2)} ${y(shown.y2)}, ${x(1)} ${y(1)}`,
  );

  function fractionsAtPointer(event: PointerEvent): { time: number; progress: number } {
    if (!surface) {
      return { time: 0, progress: 0 };
    }
    return {
      time: (pointerOffsetX(event.clientX, surface) - MARGIN) / SQUARE,
      progress: 1 - (pointerOffsetY(event.clientY, surface) - MARGIN) / SQUARE,
    };
  }

  function moved(start: BezierHandles, handle: 1 | 2, time: number, progress: number): BezierHandles {
    return clampedHandles(handle === 1 ? { ...start, x1: time, y1: progress } : { ...start, x2: time, y2: progress });
  }

  function startDrag(event: PointerEvent, handle: 1 | 2): void {
    if (event.button !== 0) {
      return;
    }
    event.preventDefault();
    const target = event.currentTarget as SVGElement;
    target.setPointerCapture(event.pointerId);
    const start = handles;
    const move = (moveEvent: PointerEvent) => {
      const { time, progress } = fractionsAtPointer(moveEvent);
      draft = moved(start, handle, time, progress);
    };
    const end = () => {
      target.removeEventListener("pointermove", move);
      const finished = draft;
      draft = null;
      if (finished && !sameHandles(finished, start)) {
        onChange(finished);
      }
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", end, { once: true });
    target.addEventListener("pointercancel", () => {
      target.removeEventListener("pointermove", move);
      draft = null;
    }, { once: true });
  }

  function sameHandles(first: BezierHandles, second: BezierHandles): boolean {
    return first.x1 === second.x1 && first.y1 === second.y1 && first.x2 === second.x2 && first.y2 === second.y2;
  }

  function handleKeydown(event: KeyboardEvent, handle: 1 | 2): void {
    const step = event.shiftKey ? FINE_KEY_STEP : KEY_STEP;
    const moves: Record<string, [number, number]> = {
      ArrowLeft: [-step, 0],
      ArrowRight: [step, 0],
      ArrowDown: [0, -step],
      ArrowUp: [0, step],
    };
    const delta = moves[event.key];
    if (!delta) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    const time = (handle === 1 ? handles.x1 : handles.x2) + delta[0];
    const progress = (handle === 1 ? handles.y1 : handles.y2) + delta[1];
    const next = moved(handles, handle, time, progress);
    if (!sameHandles(next, handles)) {
      onChange(next);
    }
  }

  function handleLabel(handle: 1 | 2): string {
    const time = handle === 1 ? shown.x1 : shown.x2;
    const progress = handle === 1 ? shown.y1 : shown.y2;
    return t(handle === 1 ? "keyframes.firstHandle" : "keyframes.secondHandle", {
      time: Math.round(time * 100),
      progress: Math.round(progress * 100),
    });
  }
</script>

<svg
  bind:this={surface}
  class="curve-editor"
  width={SIZE}
  height={SIZE}
  viewBox="0 0 {SIZE} {SIZE}"
  role="group"
  aria-label={t("keyframes.curveEditor")}
>
  <rect class="square" x={MARGIN} y={MARGIN} width={SQUARE} height={SQUARE} />
  <line class="diagonal" x1={x(0)} y1={y(0)} x2={x(1)} y2={y(1)} />
  <line class="arm" x1={x(0)} y1={y(0)} x2={x(shown.x1)} y2={y(shown.y1)} />
  <line class="arm" x1={x(1)} y1={y(1)} x2={x(shown.x2)} y2={y(shown.y2)} />
  <path class="curve" d={curvePath} />
  <circle class="end" cx={x(0)} cy={y(0)} r="3" />
  <circle class="end" cx={x(1)} cy={y(1)} r="3" />
  {#each HANDLES as handle (handle)}
    <!-- A two-axis handle: the arrow keys move it, its label says where it is. -->
    <circle
      class="handle"
      cx={x(handle === 1 ? shown.x1 : shown.x2)}
      cy={y(handle === 1 ? shown.y1 : shown.y2)}
      r="6"
      tabindex="0"
      role="button"
      aria-label={handleLabel(handle)}
      onpointerdown={(event) => startDrag(event, handle)}
      onkeydown={(event) => handleKeydown(event, handle)}
    ><title>{handleLabel(handle)}</title></circle>
  {/each}
</svg>

<style>
  .curve-editor {
    display: block;
    flex: none;
    touch-action: none;
    user-select: none;
  }
  .square {
    fill: var(--color-bg-inset);
    stroke: var(--color-border-strong);
    stroke-width: 1;
  }
  .diagonal {
    stroke: var(--color-border-strong);
    stroke-width: 1;
    stroke-dasharray: 3 3;
  }
  .arm {
    stroke: var(--color-text-muted);
    stroke-width: 1;
  }
  .curve {
    fill: none;
    stroke: var(--color-accent-text);
    stroke-width: 2;
  }
  .end {
    fill: var(--color-text-secondary);
  }
  .handle {
    fill: var(--color-bg-panel);
    stroke: var(--color-accent-text);
    stroke-width: 2;
    cursor: grab;
  }
  .handle:hover {
    fill: var(--color-accent-soft);
  }
  .handle:focus-visible {
    outline: none;
    stroke: var(--color-focus-ring);
    stroke-width: 3;
  }
</style>
