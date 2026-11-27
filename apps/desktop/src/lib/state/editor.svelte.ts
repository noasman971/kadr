// The state of the editor screen: the project view sent by the back end, the
// playhead, the selection, and files being imported.
//
// The back end owns the project (decision record 0022). This module never
// changes the view itself: it sends requests and replaces the view with the
// answer. A failure becomes a notification in plain words.

import { backend } from "../api/backend";
import type {
  EditNotice,
  EditRequest,
  EditResponse,
  ImportFailure,
  ProjectFormatChoice,
  ProjectView,
  TitleStyle,
} from "../api/types";
import { quoted, t, undoStepDisplayName } from "../i18n/i18n.svelte";
import { snapToFrame, type FrameRate } from "../time/ticks";
import { describeError } from "./errors";
import { toasts } from "./toasts.svelte";

/** A file shown as "Reading…" in the library while it is probed. */
export interface PendingImport {
  filePath: string;
  fileName: string;
}

/** Undo and redo notices replace each other instead of piling up. */
const HISTORY_TOAST_GROUP = "history";

function fileNameOf(filePath: string): string {
  return filePath.split(/[\\/]/).pop() ?? filePath;
}

class EditorState {
  view = $state<ProjectView | null>(null);
  playheadTicks = $state(0);
  selectedClipIds = $state<number[]>([]);
  pendingImports = $state<PendingImport[]>([]);
  importFailures = $state<ImportFailure[]>([]);
  /** A title just added: the inspector focuses its words, then clears this. */
  clipToFocusForText = $state<number | null>(null);

  frameRate: FrameRate = $derived({
    numerator: this.view?.settings.frameRateNumerator ?? 30,
    denominator: this.view?.settings.frameRateDenominator ?? 1,
  });

  /** Listeners told about every new view (playback restarts its sound). */
  private viewListeners: Array<(view: ProjectView) => void> = [];

  /** The last change (edit, undo or redo) sent or waiting to be sent. */
  private lastChange: Promise<unknown> = Promise.resolve();

  onViewChanged(listener: (view: ProjectView) => void): void {
    this.viewListeners.push(listener);
  }

  /** Shows a view from the back end, ignoring answers older than the current one. */
  acceptView(view: ProjectView): void {
    if (this.view && view.filePath === this.view.filePath && view.revision < this.view.revision) {
      return;
    }
    this.view = view;
    const existingClipIds = new Set(view.tracks.flatMap((track) => track.clips.map((clip) => clip.id)));
    this.selectedClipIds = this.selectedClipIds.filter((clipId) => existingClipIds.has(clipId));
    this.playheadTicks = Math.min(this.playheadTicks, view.durationTicks);
    for (const listener of this.viewListeners) {
      listener(view);
    }
  }

  /** Opens a project view (after creating or opening a project). */
  showProject(view: ProjectView): void {
    this.view = null;
    this.playheadTicks = 0;
    this.selectedClipIds = [];
    this.pendingImports = [];
    this.importFailures = [];
    this.acceptView(view);
  }

  async createProject(format: ProjectFormatChoice): Promise<boolean> {
    return this.runAndShowProject(() => backend.createProject(t("home.defaultProjectName"), format));
  }

  async openProject(filePath: string): Promise<boolean> {
    return this.runAndShowProject(() => backend.openProject(filePath));
  }

  async closeProject(): Promise<void> {
    await backend.closeProject().catch(() => undefined);
    this.view = null;
  }

  async renameProject(newName: string): Promise<void> {
    await this.runRequest(() => backend.renameProject(newName));
  }

  /** Applies one edit (one undo step); returns the created clips. */
  async edit(request: EditRequest): Promise<number[]> {
    return this.applyChange(() => backend.applyEdit(request));
  }

  /**
   * Gives a clip the look of a `.cube` file (one undo step). The back end reads
   * the file first; an unreadable one is explained and changes nothing.
   */
  async chooseLutFile(clipId: number, filePath: string): Promise<void> {
    await this.applyChange(() => backend.chooseClipLutFile(clipId, filePath));
  }

  /**
   * Imports an `.srt` file as a new subtitle track (one undo step). The back end
   * reads it first: a broken file is explained, with its line, and changes
   * nothing. The first cue is selected.
   */
  async importSubtitleFile(filePath: string): Promise<void> {
    const createdClipIds = await this.applyChange(() => backend.importSubtitleFile(filePath));
    if (createdClipIds.length > 0) {
      this.selectedClipIds = createdClipIds;
    }
  }

  /** Writes a subtitle track to an `.srt` file, after the changes asked before it. */
  async exportSubtitleFile(trackId: number, filePath: string): Promise<void> {
    await this.inOrder(async () => {
      try {
        const cueCount = await backend.exportSubtitleFile(trackId, filePath);
        toasts.show({
          tone: "success",
          message: t("toast.subtitlesExported", { count: cueCount, name: quoted(fileNameOf(filePath)) }),
        });
      } catch (error) {
        this.showError(error);
      }
    });
  }

  /** Adds a subtitle cue at the playhead, selected, its words focused. */
  async addSubtitleCue(trackId: number | null): Promise<void> {
    const startTicks = snapToFrame(this.playheadTicks, this.frameRate);
    const createdClipIds = await this.edit({
      type: "addSubtitleCue",
      startTicks,
      trackId,
      text: t("subtitles.newCueText"),
    });
    if (createdClipIds.length > 0) {
      this.selectedClipIds = createdClipIds;
      this.clipToFocusForText = createdClipIds[0] ?? null;
    }
  }

  /** Sends a change once the ones before it are answered, and shows its result. */
  private applyChange(send: () => Promise<EditResponse>): Promise<number[]> {
    return this.inOrder(async () => {
      try {
        const response = await send();
        this.acceptView(response.view);
        this.announceNotices(response.outcome.notices);
        return response.outcome.createdClipIds;
      } catch (error) {
        this.showError(error);
        return [];
      }
    });
  }

  /** Adds a title at the playhead (T, the toolbar, the Text tab), selected, its words focused. */
  async addTitle(style: TitleStyle): Promise<void> {
    const text = t(style === "subtitle" ? "title.subtitleText" : style === "bigWord" ? "title.bigWordText" : "title.titleText");
    const startTicks = snapToFrame(this.playheadTicks, this.frameRate);
    const createdClipIds = await this.edit({ type: "addTitle", startTicks, text, style });
    if (createdClipIds.length > 0) {
      this.selectedClipIds = createdClipIds;
      this.clipToFocusForText = createdClipIds[0] ?? null;
    }
  }

  /** Undoes the last step, once the changes made before have been answered. */
  async undo(): Promise<void> {
    await this.inOrder(async () => {
      const step = this.view?.undoLabel;
      if (step && (await this.runRequest(() => backend.undo()))) {
        toasts.show({ tone: "info", group: HISTORY_TOAST_GROUP, message: t("toast.undone", { step: undoStepDisplayName(step) }) });
      }
    });
  }

  /** Redoes the last undone step, once the changes made before have been answered. */
  async redo(): Promise<void> {
    await this.inOrder(async () => {
      const step = this.view?.redoLabel;
      if (step && (await this.runRequest(() => backend.redo()))) {
        toasts.show({ tone: "info", group: HISTORY_TOAST_GROUP, message: t("toast.redone", { step: undoStepDisplayName(step) }) });
      }
    });
  }

  /** Imports files: they show as "Reading…" tiles until the back end answers. */
  async importFiles(filePaths: string[]): Promise<number[]> {
    const newPaths = filePaths.filter(
      (filePath) => !this.pendingImports.some((pending) => pending.filePath === filePath),
    );
    if (newPaths.length === 0) {
      return [];
    }
    this.pendingImports = [
      ...this.pendingImports,
      ...newPaths.map((filePath) => ({ filePath, fileName: fileNameOf(filePath) })),
    ];
    this.importFailures = this.importFailures.filter((failure) => !newPaths.includes(failure.filePath));
    try {
      const response = await backend.importMedia(newPaths);
      this.acceptView(response.view);
      this.importFailures = [...this.importFailures, ...response.failures];
      for (const failure of response.failures) {
        toasts.show({
          tone: "error",
          message: t("toast.importFailed", { name: quoted(failure.fileName) }),
          details: failure.details,
        });
      }
      return response.mediaIds;
    } catch (error) {
      this.showError(error);
      return [];
    } finally {
      this.pendingImports = this.pendingImports.filter((pending) => !newPaths.includes(pending.filePath));
    }
  }

  dismissImportFailure(filePath: string): void {
    this.importFailures = this.importFailures.filter((failure) => failure.filePath !== filePath);
  }

  /** Moves the playhead, kept on a frame start and inside the edit. */
  seek(ticks: number): void {
    const duration = this.view?.durationTicks ?? 0;
    this.playheadTicks = snapToFrame(Math.min(Math.max(0, ticks), duration), this.frameRate);
  }

  showError(error: unknown): void {
    const described = describeError(error);
    if (described.message === "") {
      return;
    }
    toasts.show({ tone: "error", message: described.message, details: described.details });
  }

  private announceNotices(notices: EditNotice[]): void {
    for (const notice of notices) {
      if (notice.type === "formatAdopted") {
        toasts.show({
          tone: "info",
          message: t("toast.formatAdopted", {
            width: notice.width,
            height: notice.height,
            name: quoted(notice.mediaName),
          }),
          action: { label: t("toast.undo"), run: () => void this.undo() },
        });
      } else if (notice.type === "subtitlesImported") {
        toasts.show({
          tone: "success",
          message: t("toast.subtitlesImported", { count: notice.cueCount }),
          details: notice.warnings.length > 0 ? notice.warnings.join("\n") : undefined,
          action: { label: t("toast.undo"), run: () => void this.undo() },
        });
      }
    }
  }

  /**
   * Runs `change` once every change asked before it has been answered.
   *
   * Tauri runs commands on a pool of threads, so two requests sent at once can
   * reach the project in either order: left then right alignment could end up
   * left, and an undo could overtake the edit it was meant to undo. Changes
   * therefore go one at a time, in the order the user made them. A change that
   * fails does not hold back the next ones.
   */
  private inOrder<Result>(change: () => Promise<Result>): Promise<Result> {
    const result = this.lastChange.then(change);
    this.lastChange = result.catch(() => undefined);
    return result;
  }

  private async runAndShowProject(request: () => Promise<ProjectView>): Promise<boolean> {
    try {
      this.showProject(await request());
      return true;
    } catch (error) {
      this.showError(error);
      return false;
    }
  }

  private async runRequest(request: () => Promise<ProjectView>): Promise<boolean> {
    try {
      this.acceptView(await request());
      return true;
    } catch (error) {
      this.showError(error);
      return false;
    }
  }
}

export const editor = new EditorState();
