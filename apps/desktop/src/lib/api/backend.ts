// Typed calls to the Rust back end (`src-tauri/src/commands/`). Every function
// here is a thin wrapper around one Tauri command; failures throw a
// `CommandError` (see `types.ts`).

import { Channel, invoke } from "@tauri-apps/api/core";

import {
  decodeRgbaPicture,
  decodeSound,
  decodeWaveform,
  decodeYuvFrame,
  type RgbaPicture,
  type Waveform,
  type YuvFrame,
} from "./binaryMessages";
import type {
  BlurShapeChoice,
  ClaudeSetupView,
  ClipOverlay,
  EditRequest,
  EditResponse,
  ExportEvent,
  ExportSettings,
  ExportStarted,
  ImportResponse,
  ProjectFormatChoice,
  ProjectView,
  RecentProject,
  SilencePlan,
  SilenceSearchSettings,
  SourceRegion,
  TrackingEvent,
} from "./types";

export const backend = {
  listRecentProjects: () => invoke<RecentProject[]>("list_recent_projects"),
  forgetRecentProject: (filePath: string) => invoke<void>("forget_recent", { filePath }),
  createProject: (name: string, format: ProjectFormatChoice) =>
    invoke<ProjectView>("create_project", { name, format }),
  openProject: (filePath: string) => invoke<ProjectView>("open_project", { filePath }),
  getProjectView: () => invoke<ProjectView | null>("get_project_view"),
  closeProject: () => invoke<void>("close_project"),
  renameProject: (newName: string) => invoke<ProjectView>("rename_project", { newName }),
  saveProjectAs: (filePath: string) => invoke<ProjectView>("save_project_as", { filePath }),
  saveProject: () => invoke<ProjectView>("save_project"),

  applyEdit: (request: EditRequest) => invoke<EditResponse>("apply_edit", { request }),
  undo: () => invoke<ProjectView>("undo"),
  redo: () => invoke<ProjectView>("redo"),
  importMedia: (filePaths: string[]) => invoke<ImportResponse>("import_media", { filePaths }),
  /** Reads a `.cube` file, then gives the clip its look (one undo step). */
  chooseClipLutFile: (clipId: number, filePath: string) =>
    invoke<EditResponse>("choose_clip_lut_file", { clipId, filePath }),

  async renderPreviewFrame(timeTicks: number, maxWidth: number, maxHeight: number): Promise<YuvFrame> {
    const buffer = await invoke<ArrayBuffer>("render_preview_frame", {
      timeTicks: Math.max(0, Math.round(timeTicks)),
      maxWidth: Math.round(maxWidth),
      maxHeight: Math.round(maxHeight),
    });
    return decodeYuvFrame(buffer);
  },

  async getThumbnail(
    mediaId: number,
    sourceTimeTicks: number,
    maxWidth: number,
    maxHeight: number,
  ): Promise<RgbaPicture> {
    const buffer = await invoke<ArrayBuffer>("get_thumbnail", {
      mediaId,
      sourceTimeTicks: Math.max(0, Math.round(sourceTimeTicks)),
      maxWidth: Math.round(maxWidth),
      maxHeight: Math.round(maxHeight),
    });
    return decodeRgbaPicture(buffer);
  },

  /** Starts an export; `onEvent` receives its progress, then how it ended. */
  startExport(settings: ExportSettings, onEvent: (event: ExportEvent) => void): Promise<ExportStarted> {
    const channel = new Channel<ExportEvent>();
    channel.onmessage = onEvent;
    return invoke<ExportStarted>("start_export", { settings, onEvent: channel });
  },
  cancelExport: () => invoke<void>("cancel_export"),

  /**
   * What removing a clip's silences would cut; changes nothing. `onProgress`
   * receives the fraction analysed (0 to 1). A newer search, or
   * `stopSilenceSearch`, ends this one with a `superseded` error.
   */
  findClipSilences(
    clipId: number,
    settings: SilenceSearchSettings,
    onProgress: (fraction: number) => void,
  ): Promise<SilencePlan> {
    const channel = new Channel<number>();
    channel.onmessage = onProgress;
    return invoke<SilencePlan>("find_clip_silences", { clipId, settings, onProgress: channel });
  },
  stopSilenceSearch: () => invoke<void>("stop_silence_search"),
  revealExportedFile: (filePath: string) => invoke<void>("reveal_exported_file", { filePath }),
  openExportedFile: (filePath: string) => invoke<void>("open_exported_file", { filePath }),

  /** Where a clip's picture is on the canvas at `timeTicks`, and its blurs' boxes; `null` without a picture then. */
  getClipOverlay: (clipId: number, timeTicks: number) =>
    invoke<ClipOverlay | null>("get_clip_overlay", { clipId, timeTicks: Math.max(0, Math.round(timeTicks)) }),

  /**
   * Follows `region` (source pixels) through the clip, for a new blur, or to correct
   * blur `effectId`; `onEvent` receives the progress, then the result.
   */
  startRegionTracking(
    clipId: number,
    timeTicks: number,
    region: SourceRegion,
    shape: BlurShapeChoice,
    effectId: number | null,
    onEvent: (event: TrackingEvent) => void,
  ): Promise<{ clipId: number }> {
    const channel = new Channel<TrackingEvent>();
    channel.onmessage = onEvent;
    return invoke<{ clipId: number }>("start_region_tracking", {
      clipId,
      timeTicks: Math.max(0, Math.round(timeTicks)),
      region,
      shape,
      effectId,
      onEvent: channel,
    });
  },
  cancelRegionTracking: () => invoke<void>("cancel_region_tracking"),

  getClaudeSetup: (configPath: string | null) => invoke<ClaudeSetupView>("get_claude_setup", { configPath }),
  addClaudeFolder: (folder: string, configPath: string | null) =>
    invoke<ClaudeSetupView>("add_claude_folder", { folder, configPath }),
  removeClaudeFolder: (folder: string, configPath: string | null) =>
    invoke<ClaudeSetupView>("remove_claude_folder", { folder, configPath }),
  connectClaudeDesktop: (configPath: string | null) =>
    invoke<ClaudeSetupView>("connect_claude_desktop", { configPath }),

  async getWaveform(mediaId: number): Promise<Waveform> {
    return decodeWaveform(await invoke<ArrayBuffer>("get_waveform", { mediaId }));
  },

  async getTimelineSound(firstSample: number, sampleCount: number): Promise<Float32Array> {
    const buffer = await invoke<ArrayBuffer>("get_timeline_sound", {
      firstSample: Math.max(0, Math.floor(firstSample)),
      sampleCount: Math.max(0, Math.floor(sampleCount)),
    });
    return decodeSound(buffer);
  },
};
