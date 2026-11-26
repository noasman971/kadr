// Typed calls to the Rust back end (`src-tauri/src/commands/`). Every function
// here is a thin wrapper around one Tauri command; failures throw a
// `CommandError` (see `types.ts`).

import { invoke } from "@tauri-apps/api/core";

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
  EditRequest,
  EditResponse,
  ImportResponse,
  ProjectFormatChoice,
  ProjectView,
  RecentProject,
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
