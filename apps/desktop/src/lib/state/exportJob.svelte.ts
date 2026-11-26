// The export running in the background (docs/ux.md, "Export"), shared by the
// export dialog and the progress pill in the top bar: the dialog can be folded
// away while the export goes on ("Keep editing") and opened again.

import { ask } from "@tauri-apps/plugin-dialog";

import { backend } from "../api/backend";
import { isCommandError, type ExportEvent, type ExportSettings } from "../api/types";
import { quoted, t } from "../i18n/i18n.svelte";
import { describeError } from "./errors";
import { toasts } from "./toasts.svelte";

export type ExportPhase = "setup" | "running" | "finished" | "failed";
export type FinishedExport = Extract<ExportEvent, { type: "finished" }>;

export interface ExportProgressView {
  encodedFrameCount: number;
  totalFrameCount: number;
  elapsedSeconds: number;
}

/** The file name of a path, for messages. */
export function fileNameOf(filePath: string): string {
  return filePath.split(/[\\/]/).pop() ?? filePath;
}

/** The folder of a path. */
export function folderOf(filePath: string): string {
  const index = Math.max(filePath.lastIndexOf("\\"), filePath.lastIndexOf("/"));
  return index > 0 ? filePath.slice(0, index) : filePath;
}

class ExportJob {
  phase = $state<ExportPhase>("setup");
  isDialogOpen = $state(false);
  /** The file being written, or written. */
  filePath = $state("");
  progress = $state<ExportProgressView>({ encodedFrameCount: 0, totalFrameCount: 0, elapsedSeconds: 0 });
  finished = $state<FinishedExport | null>(null);
  failure = $state<{ message: string; details: string } | null>(null);

  /** The settings of the last start, for *Try again*. */
  private lastSettings: ExportSettings | null = null;

  readonly percentDone = $derived(
    this.progress.totalFrameCount > 0
      ? Math.min(100, Math.floor((100 * this.progress.encodedFrameCount) / this.progress.totalFrameCount))
      : 0,
  );

  /** Seconds left, from the pace so far; `null` until there is a pace to go by. */
  readonly secondsLeft = $derived.by(() => {
    const { encodedFrameCount, totalFrameCount, elapsedSeconds } = this.progress;
    if (encodedFrameCount < 10 || elapsedSeconds <= 0) {
      return null;
    }
    return ((totalFrameCount - encodedFrameCount) * elapsedSeconds) / encodedFrameCount;
  });

  openDialog(): void {
    this.isDialogOpen = true;
  }

  /** Closes the dialog; a finished or failed export goes back to the settings. */
  closeDialog(): void {
    this.isDialogOpen = false;
    if (this.phase === "finished" || this.phase === "failed") {
      this.phase = "setup";
    }
  }

  /** *Keep editing*: the export goes on, shown as a pill in the top bar. */
  keepEditing(): void {
    this.isDialogOpen = false;
  }

  async start(settings: ExportSettings): Promise<void> {
    this.lastSettings = settings;
    this.filePath = settings.filePath;
    this.progress = { encodedFrameCount: 0, totalFrameCount: 0, elapsedSeconds: 0 };
    this.finished = null;
    this.failure = null;
    this.phase = "running";
    try {
      await backend.startExport(settings, (event) => this.handleEvent(event));
    } catch (error) {
      if (isCommandError(error) && error.code === "exportFileExists" && !settings.replacesExistingFile) {
        // Nothing runs while the question is asked: the settings stay on screen.
        this.phase = "setup";
        await this.askToReplace(settings);
        return;
      }
      this.fail(describeError(error));
    }
  }

  async tryAgain(): Promise<void> {
    if (this.lastSettings) {
      await this.start(this.lastSettings);
    }
  }

  cancel(): void {
    void backend.cancelExport();
  }

  private async askToReplace(settings: ExportSettings): Promise<void> {
    const name = quoted(fileNameOf(settings.filePath));
    const replaces = await ask(t("export.replace.body", { name }), {
      title: t("export.replace.title"),
      kind: "warning",
      okLabel: t("export.replace.confirm"),
      cancelLabel: t("common.cancel"),
    });
    if (replaces) {
      await this.start({ ...settings, replacesExistingFile: true });
    } else {
      this.phase = "setup";
    }
  }

  private handleEvent(event: ExportEvent): void {
    switch (event.type) {
      case "progress":
        this.progress = {
          encodedFrameCount: event.encodedFrameCount,
          totalFrameCount: event.totalFrameCount,
          elapsedSeconds: event.elapsedSeconds,
        };
        break;
      case "finished":
        this.finished = event;
        this.phase = "finished";
        if (!this.isDialogOpen) {
          // The notice tells the result: the next Export starts a new one.
          this.phase = "setup";
          toasts.show({
            tone: "success",
            message: t("export.finishedToast", { name: quoted(fileNameOf(event.filePath)) }),
            action: { label: t("export.showInFolder"), run: () => void backend.revealExportedFile(event.filePath) },
          });
        }
        break;
      case "failed":
        this.fail({ message: t("export.failed.body", { name: quoted(fileNameOf(this.filePath)) }), details: event.details });
        break;
      case "cancelled":
        this.phase = "setup";
        toasts.show({ tone: "info", message: t("export.cancelled") });
        break;
    }
  }

  private fail(failure: { message: string; details: string }): void {
    this.failure = failure;
    this.phase = "failed";
    if (!this.isDialogOpen) {
      toasts.show({ tone: "error", message: t("export.failed.title"), details: failure.details, action: {
        label: t("export.open"),
        run: () => this.openDialog(),
      } });
    }
  }
}

export const exportJob = new ExportJob();
