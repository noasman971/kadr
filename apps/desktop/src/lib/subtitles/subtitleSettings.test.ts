import { describe, expect, it } from "vitest";

import { isSubtitleFilePath, subtitleFileNameFor, withHexAlpha } from "./subtitleSettings";

describe("subtitle settings", () => {
  it("keeps the opacity of the current colour when a new one is picked", () => {
    expect(withHexAlpha("#ff8800", "#000000B3")).toBe("#FF8800B3");
    expect(withHexAlpha("#ffffff", "#000000")).toBe("#FFFFFFFF");
  });

  it("offers the track's name as the exported file's name", () => {
    expect(subtitleFileNameFor("interview")).toBe("interview.srt");
    expect(subtitleFileNameFor('a/b: "c"')).toBe("a b   c.srt");
    expect(subtitleFileNameFor("  ")).toBe("subtitles.srt");
  });

  it("recognises subtitle files by their extension", () => {
    expect(isSubtitleFilePath("C:\\Videos\\Interview.SRT")).toBe(true);
    expect(isSubtitleFilePath("/videos/clip.mp4")).toBe(false);
    expect(isSubtitleFilePath("/videos/srt")).toBe(false);
  });
});
