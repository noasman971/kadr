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

/** Why a video gets a proxy (`kadr_engine::proxies::proxy_need_of`, decision record 0055). */
export type ProxyReason = "largePicture" | "slowToDecode" | "highBitRate";

/** A video's proxy, as `proxy_jobs.rs` sends it; `reason` is `null` when the user asked for it. */
export type ProxyState =
  | { state: "queued"; reason: ProxyReason | null }
  | { state: "making"; reason: ProxyReason | null; fraction: number }
  | { state: "ready"; width: number; height: number }
  | { state: "stopped" }
  | { state: "failed"; details: string };

/** The `proxy-changed` event: one video's proxy, `null` when it has none. */
export interface ProxyStatus {
  /** The video, as `MediaView.filePath` names it. */
  filePath: string;
  proxy: ProxyState | null;
}

export interface ProxyOverview {
  /** Heavy videos get proxies automatically, and the preview uses them. */
  isAutomatic: boolean;
  cacheSizeInBytes: number;
  cacheFolder: string;
  statuses: ProxyStatus[];
}

export interface ProxyCacheCleared {
  freedBytes: number;
  overview: ProxyOverview;
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

/** The first colour LUT of a clip (`project_view.rs`). */
export interface ColorLutView {
  effectId: number;
  isEnabled: boolean;
  /** The `.cube` file. */
  filePath: string;
  fileName: string;
  /** 0 (no look) to 1 (the full look). */
  strength: number;
  /** The file has gone: the clip is drawn without its look. */
  isMissing: boolean;
}

export type BlurStyleChoice = "blur" | "pixels" | "mask";
export type BlurShapeChoice = "ellipse" | "rectangle";
export type Confidence = "confident" | "uncertain" | "lost";

/** What the inspector shows and changes about a tracked blur (`editing/blur_settings.rs`). */
export interface BlurSettings {
  style: BlurStyleChoice;
  /** Blur radius or square size, in source pixels. */
  strength: number;
  /** `#RRGGBB`, for the mask. */
  maskColor: string;
  shape: BlurShapeChoice;
  marginInPercent: number;
}

/** A stretch where a blur's tracker was not sure, on the timeline. */
export interface UnsureRangeView {
  startTicks: number;
  /** Excluded. */
  endTicks: number;
  confidence: Exclude<Confidence, "confident">;
}

export interface TrackedBlurView {
  effectId: number;
  isEnabled: boolean;
  settings: BlurSettings;
  correctionCount: number;
  unsureRanges: UnsureRangeView[];
}

/** A box in the source video's pixels. */
export interface SourceRegion {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface BlurBoxView {
  effectId: number;
  isEnabled: boolean;
  shape: BlurShapeChoice;
  region: SourceRegion;
  hiddenArea: SourceRegion;
  confidence: Confidence;
}

/** What the preview draws on top of a clip at one moment (`blur_overlay.rs`). */
export interface ClipOverlay {
  clipId: number;
  sourceWidth: number;
  sourceHeight: number;
  sourceTimeTicks: number;
  /** Source pixels to canvas pixels, in SVG's `matrix(a, b, c, d, e, f)` order. */
  sourceToCanvas: [number, number, number, number, number, number];
  /** Left, top, right, bottom of the visible part of the source. */
  visibleSource: [number, number, number, number];
  /** The shortest side a box may have to be followed, in source pixels. */
  minimumRegionSide: number;
  blurBoxes: BlurBoxView[];
}

/** What a running analysis reports (`tracking_job.rs`). */
export type TrackingEvent =
  | { type: "progress"; analysedFrameCount: number; totalFrameCount: number; elapsedSeconds: number }
  | {
      type: "finished";
      view: ProjectView;
      clipId: number;
      effectId: number;
      isCorrection: boolean;
      uncertainRangeCount: number;
      lostRangeCount: number;
    }
  | { type: "failed"; code: ErrorCode; details: string }
  | { type: "cancelled" };

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
  colorLut: ColorLutView | null;
  trackedBlurs: TrackedBlurView[];
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

/** What Claude did to the project, sent by the back end (`claude_link.rs`). */
export interface ClaudeChange {
  view: ProjectView;
  /** Claude opened or created this project: the window shows it now. */
  openedProject: boolean;
  /** The step Claude added to the history, without "Claude: ". */
  stepLabel: string | null;
  /** Clips Claude added or changed. */
  changedClipIds: number[];
}

/** The Connect to Claude screen (`claude_setup.rs`). */
export interface ClaudeSetupView {
  /** `kadr-mcp` next to the app; `null` when missing. */
  kadrMcpPath: string | null;
  settingsFile: string;
  allowedFolders: Array<{ path: string; exists: boolean }>;
  claudeDesktopConfig: string;
  claudeDesktopConfigExists: boolean;
  connection: "connected" | "notConnected" | "otherKadr" | "unreadable";
  claudeCodeCommand: string | null;
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
  | { type: "removeTrack"; trackId: number }
  /** An export preset of another shape changes the project first (one undo step). */
  | { type: "setCanvasSize"; width: number; height: number }
  /** Units: volume and scale are multipliers, opacity 0–1, positions pixels from the centre, rotation degrees. */
  | { type: "setClipValue"; clipId: number; value: ClipValueName; amount: number }
  /**
   * Crop, colour and text requests name only the settings they change: the back
   * end keeps the others as the project has them when it applies the request.
   * The view here can be behind (an earlier change not answered yet), so sending
   * every setting from it would put old values back.
   */
  | { type: "setClipCrop"; clipId: number; changes: Partial<CropView> }
  | { type: "resetClipPicture"; clipId: number }
  | { type: "setClipSpeed"; clipId: number; speed: number }
  | { type: "setClipColor"; clipId: number; changes: Partial<ColorSettings> }
  /** Removes the colour adjustment and the colour LUT. */
  | { type: "resetClipColor"; clipId: number }
  /** Only the strength: a file is chosen with `backend.chooseClipLutFile`, which reads it first. */
  | { type: "setClipLut"; clipId: number; changes: { strength?: number } }
  | { type: "removeClipLut"; clipId: number }
  | { type: "setClipText"; clipId: number; changes: Partial<TextSettings> }
  | { type: "addTitle"; startTicks: number; text: string; style: TitleStyle }
  | { type: "setBlurSettings"; clipId: number; effectId: number; changes: Partial<BlurSettings> }
  | { type: "setBlurEnabled"; clipId: number; effectId: number; isEnabled: boolean }
  | { type: "removeBlur"; clipId: number; effectId: number }
  /** Cuts the parts the silence dialog found and closes the gaps on the clip's track. */
  | { type: "removeSilences"; clipId: number; cuts: TickRange[] };

/** A stretch of the timeline, in ticks: `startTicks` included, `endTicks` excluded. */
export interface TickRange {
  startTicks: number;
  endTicks: number;
}

/** What the silence dialog looks for (`silence_search.rs`). */
export interface SilenceSearchSettings {
  /** Quieter than this many dBFS is silence (−40 by default). */
  thresholdInDecibels: number;
  /** Only silences at least this long are shortened. */
  minimumSilenceInSeconds: number;
  /** How long each silence lasts once shortened. */
  keptPauseInSeconds: number;
}

/** What removing a clip's silences would cut. */
export interface SilencePlan {
  /** On the timeline as the clip is now, in order. */
  cuts: TickRange[];
  /** How much shorter the clip gets. */
  removedTicks: number;
  /** The clip is silent from start to end: nothing would be left, so nothing is cut. */
  wholeClipIsSilent: boolean;
}

export type ClipValueName = "volume" | "opacity" | "positionX" | "positionY" | "scale" | "rotation";

export interface ColorSettings {
  /** Photographic stops. */
  exposure: number;
  /** Multiplier, 1 unchanged. */
  contrast: number;
  /** Multiplier, 1 unchanged. */
  saturation: number;
  /** −1 cooler to 1 warmer. */
  temperature: number;
}

export interface TextSettings {
  text: string;
  fontFamily: string;
  fontSize: number;
  /** `#RRGGBB` or `#RRGGBBAA`. */
  color: string;
  alignment: TextAlignment;
}

export type TitleStyle = "title" | "subtitle" | "bigWord";

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
  | "lutUnreadable"
  | "editRefused"
  | "nothingToUndo"
  | "nothingToRedo"
  | "previewFailed"
  | "superseded"
  | "exportFailed"
  | "exportCancelled"
  | "exportAlreadyRunning"
  | "exportFileExists"
  | "exportOtherShape"
  | "blurRegionTooSmall"
  | "blurRegionOutsidePicture"
  | "blurRegionWithoutDetail"
  | "blurClipNotShown"
  | "trackingAlreadyRunning"
  | "trackingFailed"
  | "trackingProjectClosed"
  | "claudeFolderUnusable"
  | "claudeSettingsNotSaved"
  | "kadrMcpMissing"
  | "claudeConfigUnreadable"
  | "claudeConfigNotSaved"
  | "internal";

/** The size of an exported video (`export_job.rs`). */
export type ExportSize = "project" | "youtube1080p" | "youtube4k" | "verticalReelsShorts" | "square1080";

/** What the export dialog asks the back end for. */
export interface ExportSettings {
  filePath: string;
  size: ExportSize;
  quality: "high" | "fast";
  usesHardwareEncoder: boolean;
  replacesExistingFile: boolean;
}

/** What `start_export` answers at once. */
export interface ExportStarted {
  filePath: string;
  width: number;
  height: number;
}

/** What a running export reports. */
export type ExportEvent =
  | { type: "progress"; encodedFrameCount: number; totalFrameCount: number; elapsedSeconds: number }
  | {
      type: "finished";
      filePath: string;
      fileSizeInBytes: number;
      durationSeconds: number;
      frameCount: number;
      elapsedSeconds: number;
      videoEncoderName: string;
      rendererDescription: string;
      warnings: string[];
    }
  | { type: "failed"; details: string }
  | { type: "cancelled" };

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
