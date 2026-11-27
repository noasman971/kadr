// Small helpers of the subtitle inspector and import (decision record 0056).

/** Characters Windows refuses in file names. */
const FORBIDDEN_FILE_NAME_CHARACTERS = /[<>:"/\\|?*]/g;

/** Extensions of subtitle files the app imports. */
export const SUBTITLE_FILE_EXTENSIONS = ["srt"];

/**
 * A colour picked as `#rrggbb` (colour inputs have no opacity) with the opacity
 * of the current `#RRGGBBAA` colour, so a half-transparent box stays so.
 */
export function withHexAlpha(pickedColor: string, currentColor: string): string {
  const alpha = currentColor.length === 9 ? currentColor.slice(7, 9) : "FF";
  return `${pickedColor.slice(0, 7).toUpperCase()}${alpha.toUpperCase()}`;
}

/** The file name offered when a subtitle track is exported: its name, `.srt`. */
export function subtitleFileNameFor(trackName: string): string {
  const cleaned = trackName.replace(FORBIDDEN_FILE_NAME_CHARACTERS, " ").trim();
  return `${cleaned === "" ? "subtitles" : cleaned}.srt`;
}

/** Whether a path names a subtitle file (`.srt`, any case). */
export function isSubtitleFilePath(filePath: string): boolean {
  const extension = /\.([^./\\]+)$/.exec(filePath)?.[1]?.toLowerCase();
  return extension !== undefined && SUBTITLE_FILE_EXTENSIONS.includes(extension);
}
