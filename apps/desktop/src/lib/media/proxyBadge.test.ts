import { beforeEach, describe, expect, it } from "vitest";

import type { MediaView } from "../api/types";
import { translations } from "../i18n/i18n.svelte";
import { formatByteSize, proxyBadgeOf } from "./proxyBadge";

const video: MediaView = {
  id: 3,
  name: "drone.mp4",
  filePath: "C:/videos/drone.mp4",
  kind: "video",
  durationTicks: 705_600_000 * 20,
  width: 3840,
  height: 2160,
  hasPicture: true,
  hasSound: true,
  useCount: 1,
  isMissing: false,
};

describe("proxy badges", () => {
  beforeEach(() => {
    translations.language = "en";
  });

  it("shows nothing without a proxy, or for files that are not videos", () => {
    expect(proxyBadgeOf(null, video)).toBeNull();
    expect(proxyBadgeOf({ state: "ready", width: 1920, height: 1080 }, { ...video, kind: "image" })).toBeNull();
  });

  it("shows the progress of a proxy being made, which can be stopped", () => {
    const badge = proxyBadgeOf({ state: "making", reason: "largePicture", fraction: 0.426 }, video);
    expect(badge).toMatchObject({ label: "Proxy 42 %", tone: "waiting", fraction: 0.426, action: "stop" });
    expect(badge?.hint).toContain("Larger than 1080p (3840 × 2160)");
    expect(badge?.hint).toContain("export always uses the original");
  });

  it("lets a waiting proxy be skipped", () => {
    const badge = proxyBadgeOf({ state: "queued", reason: null }, video);
    expect(badge).toMatchObject({ label: "Proxy…", fraction: null, action: "stop" });
    expect(badge?.hint).toContain("Asked for");
  });

  it("says which copy the preview shows once ready", () => {
    const badge = proxyBadgeOf({ state: "ready", width: 1920, height: 1080 }, video);
    expect(badge).toMatchObject({ label: "Proxy", tone: "ready", action: null });
    expect(badge?.hint).toContain("1920 × 1080 copy of this 3840 × 2160 video");
  });

  it("offers to make a stopped or failed proxy again", () => {
    expect(proxyBadgeOf({ state: "stopped" }, video)).toMatchObject({ label: "No proxy", action: "makeAgain" });
    const failed = proxyBadgeOf({ state: "failed", details: "disk full" }, video);
    expect(failed).toMatchObject({ label: "Proxy failed", tone: "warning", action: "makeAgain" });
    expect(failed?.hint).toContain("disk full");
  });

  it("speaks French too", () => {
    translations.language = "fr";
    expect(proxyBadgeOf({ state: "stopped" }, video)?.label).toBe("Sans proxy");
  });
});

describe("sizes on disk", () => {
  beforeEach(() => {
    translations.language = "en";
  });

  it("uses megabytes, then gigabytes", () => {
    expect(formatByteSize(0)).toBe("0 MB");
    expect(formatByteSize(4_250_000)).toBe("4.3 MB");
    expect(formatByteSize(850_000_000)).toBe("850 MB");
    expect(formatByteSize(1_234_000_000)).toBe("1.2 GB");
    translations.language = "fr";
    expect(formatByteSize(1_234_000_000)).toBe("1,2 Go");
  });
});
