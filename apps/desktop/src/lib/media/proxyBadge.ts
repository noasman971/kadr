// What the media library shows about a video's proxy (docs/ux.md "Proxies"):
// a small badge on its thumbnail, its tooltip, a progress bar while it is made,
// and the one action that fits (stop it, or make it again).

import type { MediaView, ProxyReason, ProxyState } from "../api/types";
import { t, translations } from "../i18n/i18n.svelte";
import type { MessageKey } from "../i18n/messages.en";

export type ProxyBadgeTone = "waiting" | "ready" | "warning";
export type ProxyBadgeAction = "stop" | "makeAgain";

export interface ProxyBadge {
  label: string;
  /** The tooltip: what it means, and that the export uses the original. */
  hint: string;
  tone: ProxyBadgeTone;
  /** From 0 to 1 while the proxy is made, otherwise `null`. */
  fraction: number | null;
  action: ProxyBadgeAction | null;
}

const REASON_KEYS: Record<ProxyReason, MessageKey> = {
  largePicture: "library.proxy.reason.largePicture",
  slowToDecode: "library.proxy.reason.slowToDecode",
  highBitRate: "library.proxy.reason.highBitRate",
};

function reasonText(reason: ProxyReason | null): string {
  return t(reason ? REASON_KEYS[reason] : "library.proxy.reason.asked");
}

/** The badge of `media`'s proxy, or `null` when it has none to show. */
export function proxyBadgeOf(proxy: ProxyState | null, media: MediaView): ProxyBadge | null {
  if (!proxy || media.kind !== "video") {
    return null;
  }
  const size = { width: media.width ?? 0, height: media.height ?? 0 };
  switch (proxy.state) {
    case "queued":
      return {
        label: t("library.proxy.queued"),
        hint: t("library.proxy.queuedHint", { reason: reasonText(proxy.reason), ...size }),
        tone: "waiting",
        fraction: null,
        action: "stop",
      };
    case "making":
      return {
        label: t("library.proxy.making", { percent: Math.floor(proxy.fraction * 100) }),
        hint: t("library.proxy.makingHint", { reason: reasonText(proxy.reason), ...size }),
        tone: "waiting",
        fraction: Math.min(1, Math.max(0, proxy.fraction)),
        action: "stop",
      };
    case "ready":
      return {
        label: t("library.proxy.ready"),
        hint: t("library.proxy.readyHint", { proxyWidth: proxy.width, proxyHeight: proxy.height, ...size }),
        tone: "ready",
        fraction: null,
        action: null,
      };
    case "stopped":
      return {
        label: t("library.proxy.stopped"),
        hint: t("library.proxy.stoppedHint"),
        tone: "waiting",
        fraction: null,
        action: "makeAgain",
      };
    case "failed":
      return {
        label: t("library.proxy.failed"),
        hint: t("library.proxy.failedHint", { details: proxy.details }),
        tone: "warning",
        fraction: null,
        action: "makeAgain",
      };
  }
}

/** A size on disk for people: "850 MB", "1.2 GB" (decimal units, like the system). */
export function formatByteSize(bytes: number): string {
  const megabytes = bytes / 1_000_000;
  if (megabytes < 1_000) {
    return t("common.megabytes", { value: translations.formatNumber(megabytes, megabytes < 10 ? 1 : 0) });
  }
  return t("common.gigabytes", { value: translations.formatNumber(megabytes / 1_000, 1) });
}
