// The shapes the Rust back end sends and accepts. They mirror
// `src-tauri/src/project_view.rs`, `editing.rs` and `command_error.rs`: change
// both sides together.
//
// Times are integer ticks: 705 600 000 per second (see `lib/time/ticks.ts`).

export type MediaKind = "video" | "audio" | "image";
export type TrackKind = "video" | "audio";
export type ClipKind = "video" | "audio" | "image" | "text";
export type TextAlignment = "left" | "center" | "right";

export interface SettingsView {
  canvasWidth: number;
  canvasHeight: number;
  frameRateNumerator: number;
  frameRateDenominator: number;
  audioSampleRate: number;
  /** `#RRGGBBAA`. */
  backgroundColor: string;
}

export interface MediaView {
  id: number;
  name: string;
  filePath: string;
  kind: MediaKind;
  durationTicks: number | null;
  width: number | null;
  height: number | null;
  hasPicture: boolean;
  hasSound: boolean;
  useCount: number;
  isMissing: boolean;
}

export interface PropertyView {
  value: number;
  keyframeCount: number;
}

export interface CropView {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export interface TextView {
  text: string;
  fontFamily: string;
  fontSize: number;
  color: string;
  alignment: TextAlignment;
}

export interface ColorAdjustmentView {
  effectId: number;
  isEnabled: boolean;
  exposure: number;
  contrast: number;
  saturation: number;
  temperature: number;
}

export interface ClipView {
  id: number;
  trackId: number;
  kind: ClipKind;
  name: string;
  startTicks: number;
  durationTicks: number;
  mediaId: number | null;
  sourceStartTicks: number | null;
  sourceEndTicks: number | null;
  sourceDurationTicks: number | null;
  speed: number;
  hasPicture: boolean;
  hasSound: boolean;
  volume: PropertyView;
  opacity: PropertyView;
  positionX: PropertyView;
  positionY: PropertyView;
  scale: PropertyView;
  rotation: PropertyView;
  crop: CropView;
  text: TextView | null;
  colorAdjustment: ColorAdjustmentView | null;
  effectCount: number;
}

export interface TrackView {
  id: number;
  kind: TrackKind;
  name: string;
  isMuted: boolean;
  isHidden: boolean;
  clips: ClipView[];
}

export interface ProjectView {
  revision: number;
  name: string;
  filePath: string;
  saveFailure: string | null;
  settings: SettingsView;
  adoptsFirstVideoFormat: boolean;
  durationTicks: number;
  media: MediaView[];
  /** Video tracks from the top layer down, then audio tracks. */
  tracks: TrackView[];
  undoLabel: string | null;
  redoLabel: string | null;
}

export interface RecentProject {
  filePath: string;
  name: string;
  lastOpenedInMilliseconds: number;
  exists: boolean;
}

export type ProjectFormatChoice = "automatic" | "landscape" | "vertical" | "square";

/** A change asked of the back end; one request is one undo step. */
/** One user gesture, applied by the back end as one undo step (`editing.rs`). */
export type EditRequest =
  | { type: "addMediaAtEnd"; mediaId: number }
  /** `trackId: null` puts the clip on a new track on top. */
  | { type: "addMediaAt"; mediaId: number; trackId: number | null; startTicks: number }
  | { type: "moveClip"; clipId: number; trackId: number | null; startTicks: number }
  | { type: "trimClip"; clipId: number; edge: "start" | "end"; timeTicks: number }
  /** An empty `clipIds` cuts every clip under `timeTicks`. */
  | { type: "splitClips"; clipIds: number[]; timeTicks: number }
  | { type: "removeClips"; clipIds: number[] }
  | { type: "setTrackMuted"; trackId: number; isMuted: boolean }
  | { type: "setTrackHidden"; trackId: number; isHidden: boolean }
  | { type: "removeTrack"; trackId: number };

export type EditNotice = {
  type: "formatAdopted";
  mediaName: string;
  width: number;
  height: number;
};

export interface EditOutcome {
  createdClipIds: number[];
  notices: EditNotice[];
}

export interface EditResponse {
  view: ProjectView;
  outcome: EditOutcome;
}

export interface ImportFailure {
  filePath: string;
  fileName: string;
  details: string;
}

export interface ImportResponse {
  view: ProjectView;
  mediaIds: number[];
  failures: ImportFailure[];
}

export type ErrorCode =
  | "noProjectOpen"
  | "projectFileUnreadable"
  | "projectFileNotSaved"
  | "projectNameTaken"
  | "mediaUnreadable"
  | "editRefused"
  | "nothingToUndo"
  | "nothingToRedo"
  | "previewFailed"
  | "superseded"
  | "exportFailed"
  | "exportCancelled"
  | "exportAlreadyRunning"
  | "internal";

/** What a failed command throws. */
export interface CommandError {
  code: ErrorCode;
  details: string;
  fileName: string | null;
}

export function isCommandError(value: unknown): value is CommandError {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    "details" in value
  );
}
