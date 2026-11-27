<!--
  The inspector (docs/ux.md, "Adjust a clip"). With nothing selected it shows the
  project; clip settings arrive with the inspector step of phase 6.
-->
<script lang="ts">
  import type { ProjectView } from "../../api/types";
  import { t } from "../../i18n/i18n.svelte";
  import { formatTimecode } from "../../time/ticks";
  import { editor } from "../../state/editor.svelte";
  import ClipInspector from "./ClipInspector.svelte";
  import SubtitleInspector from "./SubtitleInspector.svelte";
  import { aspectRatioName, formatFrameRate } from "./projectFormat";

  interface Props {
    view: ProjectView;
  }

  let { view }: Props = $props();

  const clipCount = $derived(view.tracks.reduce((count, track) => count + track.clips.length, 0));
  const selectedClip = $derived(
    editor.selectedClipIds.length === 1
      ? view.tracks.flatMap((track) => track.clips).find((clip) => clip.id === editor.selectedClipIds[0])
      : undefined,
  );
</script>

<aside class="inspector" aria-label={t("inspector.title")}>
  <div class="header">
    <span class="title">{selectedClip ? t("inspector.clip") : t("inspector.project")}</span>
  </div>
  <div class="body">
    {#if selectedClip}
      {#key selectedClip.id}
        {#if selectedClip.kind === "subtitle"}
          <SubtitleInspector clip={selectedClip} />
        {:else}
          <ClipInspector clip={selectedClip} />
        {/if}
      {/key}
    {:else}
    {#if editor.selectedClipIds.length > 1}
      <section class="section"><p class="hint">{t("inspector.severalClips", { count: editor.selectedClipIds.length })}</p></section>
    {/if}
    <section class="section">
      <h3>{t("inspector.format")}</h3>
      {#if view.adoptsFirstVideoFormat}
        <p class="hint">{t("inspector.formatAutomatic")}</p>
      {/if}
      <dl>
        <dt>{t("inspector.canvas")}</dt>
        <dd class="tabular">
          {aspectRatioName(view.settings.canvasWidth, view.settings.canvasHeight)} · {view.settings.canvasWidth} × {view
            .settings.canvasHeight}
        </dd>
        <dt>{t("inspector.frameRate")}</dt>
        <dd class="tabular">{formatFrameRate(view.settings)}</dd>
        <dt>{t("inspector.length")}</dt>
        <dd class="tabular">{formatTimecode(view.durationTicks, editor.frameRate)}</dd>
        <dt>{t("inspector.media")}</dt>
        <dd class="tabular">{view.media.length}</dd>
        <dt>{t("inspector.clips")}</dt>
        <dd class="tabular">{clipCount}</dd>
      </dl>
    </section>
    {/if}
  </div>
</aside>

<style>
  .inspector {
    grid-area: inspector;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    background: var(--color-bg-panel);
  }
  .header {
    display: flex;
    align-items: center;
    height: var(--panel-header-height);
    padding: 0 var(--space-3);
    border-bottom: 1px solid var(--color-border);
    flex: none;
  }
  .title {
    font-weight: var(--font-weight-semibold);
  }
  .body {
    flex: 1;
    overflow: auto;
  }
  .section {
    padding: var(--space-3);
    border-bottom: 1px solid var(--color-border);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
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
  dl {
    margin: 0;
    display: grid;
    grid-template-columns: 92px 1fr;
    gap: var(--space-2);
    font-size: var(--font-size-sm);
  }
  dt {
    color: var(--color-text-secondary);
  }
  dd {
    margin: 0;
  }
</style>
