<!--
  The export dialog (docs/ux.md, "Export"): presets, file and quality, then the
  progress, then the finished file or a human explanation of what went wrong.
  The export itself lives in `state/exportJob.svelte.ts`, so the dialog can be
  folded away while it runs.
-->
<script lang="ts">
  import { join, videoDir } from "@tauri-apps/api/path";
  import { open } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";

  import { backend } from "../../api/backend";
  import type { ExportSize, ProjectView } from "../../api/types";
  import {
    PRESET_SIZES,
    estimatedFileSizeInBytes,
    haveSameShape,
    pictureSizeOf,
    shapeOf,
    type PictureSize,
  } from "../../export/exportSizes";
  import { t, translations } from "../../i18n/i18n.svelte";
  import type { MessageKey } from "../../i18n/messages.en";
  import { editor } from "../../state/editor.svelte";
  import { exportJob, fileNameOf, folderOf } from "../../state/exportJob.svelte";
  import { preferences } from "../../state/preferences.svelte";
  import { TICKS_PER_SECOND, formatShortDuration } from "../../time/ticks";
  import Button from "../../ui/Button.svelte";
  import Dialog from "../../ui/Dialog.svelte";
  import Icon from "../../ui/Icon.svelte";
  import type { IconName } from "../../ui/icons";
  import KeyCap from "../../ui/KeyCap.svelte";
  import { formatFrameRate } from "../editor/projectFormat";

  interface Props {
    view: ProjectView;
  }

  let { view }: Props = $props();

  const PRESET_LABELS: Record<ExportSize, MessageKey> = {
    project: "export.preset.project",
    youtube1080p: "export.preset.youtube1080p",
    youtube4k: "export.preset.youtube4k",
    verticalReelsShorts: "export.preset.verticalReelsShorts",
    square1080: "export.preset.square1080",
  };
  const SHAPE_ICONS: Record<ReturnType<typeof shapeOf>, IconName> = {
    horizontal: "monitor",
    vertical: "smartphone",
    square: "square",
  };
  /** Characters Windows refuses in file names. */
  const FORBIDDEN_FILE_NAME_CHARACTERS = /[\\/:*?"<>|]/g;

  const canvas: PictureSize = $derived({ width: view.settings.canvasWidth, height: view.settings.canvasHeight });
  const framesPerSecond = $derived(view.settings.frameRateNumerator / view.settings.frameRateDenominator);
  const durationSeconds = $derived(view.durationTicks / TICKS_PER_SECOND);

  let size = $state<ExportSize>("project");
  let fileName = $state("");
  let folder = $state("");
  let quality = $state<"high" | "fast">("high");
  let usesHardwareEncoder = $state(true);

  const picture = $derived(pictureSizeOf(size, canvas));
  const changesShape = $derived(!haveSameShape(picture, canvas));
  const cleanFileName = $derived(fileName.replace(FORBIDDEN_FILE_NAME_CHARACTERS, "").trim());
  const canStart = $derived(cleanFileName !== "" && folder !== "" && view.durationTicks > 0);

  onMount(async () => {
    fileName = view.name;
    folder = preferences.lastExportFolder || (await videoDir());
  });

  const SHAPE_NAMES: Record<ReturnType<typeof shapeOf>, MessageKey> = {
    horizontal: "export.shape.horizontal",
    vertical: "export.shape.vertical",
    square: "export.shape.square",
  };

  function shapeName(shape: ReturnType<typeof shapeOf>): string {
    return t(SHAPE_NAMES[shape]);
  }

  function formatMegabytes(bytes: number): string {
    return t("export.megabytes", { value: translations.formatNumber(bytes / 1_000_000, bytes < 10_000_000 ? 1 : 0) });
  }

  function formatSeconds(seconds: number): string {
    return formatShortDuration(Math.max(0, seconds) * TICKS_PER_SECOND);
  }

  async function chooseFolder(): Promise<void> {
    const chosen = await open({ directory: true, multiple: false, defaultPath: folder, title: t("export.folder") });
    if (typeof chosen === "string") {
      folder = chosen;
    }
  }

  async function startExport(): Promise<void> {
    if (!canStart) {
      return;
    }
    if (changesShape) {
      // Another shape: the project itself switches (one undo step), then exports at its new size.
      await editor.edit({ type: "setCanvasSize", width: picture.width, height: picture.height });
    }
    preferences.update({ lastExportFolder: folder });
    const name = cleanFileName.toLowerCase().endsWith(".mp4") ? cleanFileName : `${cleanFileName}.mp4`;
    await exportJob.start({
      filePath: await join(folder, name),
      size: changesShape ? "project" : size,
      quality,
      usesHardwareEncoder,
      replacesExistingFile: false,
    });
  }

  function copyDetails(): void {
    void navigator.clipboard.writeText(exportJob.failure?.details ?? "");
  }

  function handleKeydown(event: KeyboardEvent): void {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "e" && exportJob.phase === "setup") {
      event.preventDefault();
      void startExport();
    }
  }
</script>

<Dialog title={t("export.title")} widthInPixels={760} onClose={() => exportJob.closeDialog()}>
  {#if exportJob.phase === "setup"}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="setup" onkeydown={handleKeydown}>
      <fieldset class="presets">
        <legend class="visually-hidden">{t("export.presets")}</legend>
        {#each [{ size: "project" as ExportSize, picture: canvas }, ...PRESET_SIZES] as preset (preset.size)}
          <label class="preset" class:selected={size === preset.size}>
            <input type="radio" name="preset" value={preset.size} bind:group={size} />
            <span class="preset-icon"><Icon name={SHAPE_ICONS[shapeOf(preset.picture)]} size={18} /></span>
            <span class="preset-text">
              <strong>{t(PRESET_LABELS[preset.size])}</strong>
              <small class="tabular">
                {preset.picture.width} × {preset.picture.height}{preset.size === "project"
                  ? ` · ${formatFrameRate(view.settings)} ${t("export.fps")}`
                  : ""}{!haveSameShape(preset.picture, canvas) ? ` · ${shapeName(shapeOf(preset.picture))}` : ""}
              </small>
            </span>
            {#if preset.size === "project"}<span class="suggested">{t("export.suggested")}</span>{/if}
          </label>
        {/each}
      </fieldset>

      <div class="settings">
        {#if changesShape}
          <p class="notice">
            <Icon name="info" size={16} />
            {t("export.shapeChange", {
              presetShape: shapeName(shapeOf(picture)),
              projectShape: shapeName(shapeOf(canvas)),
            })}
          </p>
        {/if}
        <label class="field">
          <span>{t("export.fileName")}</span>
          <span class="file-name"><input bind:value={fileName} spellcheck="false" /><span class="extension">.mp4</span></span>
        </label>
        <div class="field">
          <span>{t("export.folder")}</span>
          <span class="folder">
            <span class="folder-path" title={folder}><Icon name="folder" size={14} />{folder}</span>
            <Button onclick={chooseFolder}>{t("export.choose")}</Button>
          </span>
        </div>
        <fieldset class="field">
          <legend>{t("export.quality")}</legend>
          <div class="choices">
            <label><input type="radio" value="high" bind:group={quality} />{t("export.quality.high")}</label>
            <label><input type="radio" value="fast" bind:group={quality} />{t("export.quality.fast")}</label>
          </div>
        </fieldset>
        <details>
          <summary>{t("export.advanced")}</summary>
          <div class="choices">
            <label><input type="radio" value={true} bind:group={usesHardwareEncoder} />{t("export.encoder.hardware")}</label>
            <label><input type="radio" value={false} bind:group={usesHardwareEncoder} />{t("export.encoder.software")}</label>
          </div>
        </details>
        <dl class="summary">
          <div><dt>{t("export.length")}</dt><dd class="tabular">{formatShortDuration(view.durationTicks)}</dd></div>
          <div>
            <dt>{t("export.estimatedSize")}</dt>
            <dd class="tabular">≈ {formatMegabytes(estimatedFileSizeInBytes(picture, framesPerSecond, durationSeconds, quality))}</dd>
          </div>
          <div><dt></dt><dd class="muted">MP4 · H.264 · AAC</dd></div>
        </dl>
      </div>
    </div>
  {:else if exportJob.phase === "running"}
    <div class="running" aria-live="polite">
      <div class="running-head">
        <strong>{t("export.running")}</strong>
        <span class="percent tabular">{exportJob.percentDone} %</span>
      </div>
      <div class="bar" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={exportJob.percentDone}>
        <span style:width="{exportJob.percentDone}%"></span>
      </div>
      <div class="running-numbers tabular">
        <span>
          {t("export.frames", {
            current: exportJob.progress.encodedFrameCount,
            total: exportJob.progress.totalFrameCount,
          })}
        </span>
        <span>{t("export.elapsed", { time: formatSeconds(exportJob.progress.elapsedSeconds) })}</span>
        {#if exportJob.secondsLeft !== null}
          <span>{t("export.timeLeft", { time: formatSeconds(exportJob.secondsLeft) })}</span>
        {/if}
      </div>
      <p class="muted">{t("export.keepEditingHint")}</p>
      <p class="file tabular"><Icon name="film" size={14} />{exportJob.filePath}</p>
    </div>
  {:else if exportJob.phase === "finished" && exportJob.finished}
    {@const finished = exportJob.finished}
    <div class="result">
      <span class="result-icon done"><Icon name="circleCheck" size={32} /></span>
      <div>
        <strong class="result-title">{t("export.done.ready")}</strong>
        <p class="result-file">{fileNameOf(finished.filePath)}</p>
        <p class="muted tabular">
          {t("export.done.summary", {
            folder: folderOf(finished.filePath),
            length: formatSeconds(finished.durationSeconds),
            size: formatMegabytes(finished.fileSizeInBytes),
            time: formatSeconds(finished.elapsedSeconds),
          })}
        </p>
        <p class="muted small">{finished.videoEncoderName} · {finished.rendererDescription}</p>
        {#if finished.warnings.length > 0}
          <div class="warnings">
            <strong>{t("export.warnings")}</strong>
            <ul>{#each finished.warnings as warning (warning)}<li>{warning}</li>{/each}</ul>
          </div>
        {/if}
      </div>
    </div>
  {:else if exportJob.phase === "failed" && exportJob.failure}
    <div class="result">
      <span class="result-icon failed"><Icon name="circleAlert" size={32} /></span>
      <div>
        <strong class="result-title">{t("export.failed.title")}</strong>
        <p>{exportJob.failure.message}</p>
        <details>
          <summary>{t("error.details")}</summary>
          <pre>{exportJob.failure.details}</pre>
        </details>
      </div>
    </div>
  {/if}

  {#snippet footer()}
    {#if exportJob.phase === "setup"}
      <span class="hint"><KeyCap text="Ctrl" /> <KeyCap text="E" /></span>
      <span class="spacer"></span>
      <Button onclick={() => exportJob.closeDialog()}>{t("common.cancel")}</Button>
      <Button variant="primary" disabled={!canStart} onclick={startExport}><Icon name="download" />{t("export.start")}</Button>
    {:else if exportJob.phase === "running"}
      <span class="spacer"></span>
      <Button variant="danger" onclick={() => exportJob.cancel()}>{t("export.cancel")}</Button>
      <Button variant="primary" onclick={() => exportJob.keepEditing()}>{t("export.keepEditing")}</Button>
    {:else if exportJob.phase === "finished" && exportJob.finished}
      {@const filePath = exportJob.finished.filePath}
      <Button onclick={() => void backend.revealExportedFile(filePath)}><Icon name="folderOpen" />{t("export.showInFolder")}</Button>
      <Button onclick={() => void backend.openExportedFile(filePath)}><Icon name="play" />{t("export.play")}</Button>
      <span class="spacer"></span>
      <Button variant="primary" onclick={() => exportJob.closeDialog()}>{t("common.close")}</Button>
    {:else}
      <Button onclick={copyDetails}><Icon name="copy" />{t("export.copyDetails")}</Button>
      <span class="spacer"></span>
      <Button onclick={() => exportJob.closeDialog()}>{t("common.close")}</Button>
      <Button variant="primary" onclick={() => void exportJob.tryAgain()}>{t("export.tryAgain")}</Button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .setup {
    display: grid;
    grid-template-columns: 260px minmax(0, 1fr);
    gap: var(--space-5);
  }
  fieldset {
    margin: 0;
    padding: 0;
    border: 0;
    min-width: 0;
  }
  .presets {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .preset {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    cursor: pointer;
  }
  .preset:hover {
    background: var(--color-bg-raised);
  }
  .preset.selected {
    border-color: var(--color-accent);
    background: var(--color-accent-soft);
  }
  .preset input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }
  .preset:focus-within {
    outline: 2px solid var(--color-focus-ring);
    outline-offset: 2px;
  }
  .preset-icon {
    color: var(--color-text-secondary);
    display: grid;
    place-items: center;
  }
  .preset-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .preset-text small {
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }
  .suggested {
    margin-left: auto;
    padding: 2px 6px;
    border-radius: var(--radius-sm);
    background: var(--color-bg-active);
    color: var(--color-accent-text);
    font-size: var(--font-size-xs);
  }
  .settings {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
  }
  .notice {
    display: flex;
    gap: var(--space-2);
    margin: 0;
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: var(--color-accent-soft);
    font-size: var(--font-size-sm);
    line-height: 1.5;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .field > span:first-child,
  legend {
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    padding: 0;
  }
  .file-name {
    display: flex;
    align-items: center;
    border: 1px solid var(--color-border-strong);
    border-radius: var(--radius-md);
    background: var(--color-bg-inset);
  }
  .file-name input {
    flex: 1;
    min-width: 0;
    height: var(--control-height);
    padding: 0 var(--space-2);
    border: 0;
    background: transparent;
  }
  .extension {
    padding: 0 var(--space-2);
    color: var(--color-text-muted);
  }
  .folder {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .folder-path {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    color: var(--color-text-secondary);
  }
  .choices {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2) var(--space-4);
  }
  .choices label {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  details summary {
    cursor: pointer;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
    margin-bottom: var(--space-2);
  }
  .summary {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin: 0;
    padding-top: var(--space-3);
    border-top: 1px solid var(--color-border);
  }
  .summary div {
    display: flex;
    justify-content: space-between;
  }
  .summary dt {
    color: var(--color-text-secondary);
  }
  .summary dd {
    margin: 0;
  }
  .muted {
    color: var(--color-text-muted);
  }
  .small {
    font-size: var(--font-size-xs);
  }
  .running {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .running-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
  }
  .percent {
    font-size: 28px;
    font-weight: var(--font-weight-semibold);
  }
  .bar {
    height: 8px;
    border-radius: 4px;
    background: var(--color-bg-inset);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--color-accent);
    transition: width var(--duration-fast) linear;
  }
  .running-numbers {
    display: flex;
    gap: var(--space-5);
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }
  .running p,
  .result p {
    margin: 0;
  }
  .file {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
    overflow-wrap: anywhere;
  }
  .result {
    display: flex;
    gap: var(--space-4);
    align-items: flex-start;
  }
  .result > div {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }
  .result-icon.done {
    color: var(--color-success);
  }
  .result-icon.failed {
    color: var(--color-danger);
  }
  .result-title {
    font-size: var(--font-size-lg);
  }
  .result-file {
    font-weight: var(--font-weight-medium);
    overflow-wrap: anywhere;
  }
  .warnings ul {
    margin: 4px 0 0;
    padding-left: var(--space-5);
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }
  pre {
    margin: 0;
    padding: var(--space-2);
    border-radius: var(--radius-md);
    background: var(--color-bg-inset);
    white-space: pre-wrap;
    font-size: var(--font-size-xs);
  }
  .hint {
    color: var(--color-text-muted);
  }
  .spacer {
    flex: 1;
  }
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }
</style>
