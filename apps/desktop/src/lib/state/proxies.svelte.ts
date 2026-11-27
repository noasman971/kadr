// Proxies of heavy videos (decision record 0055, docs/ux.md "Proxies"): the back
// end makes them in the background and sends each video's state; this module
// keeps them by file path for the media library, and holds the setting.

import { listen } from "@tauri-apps/api/event";

import { backend } from "../api/backend";
import type { ProxyOverview, ProxyState, ProxyStatus } from "../api/types";
import { t } from "../i18n/i18n.svelte";
import { formatByteSize } from "../media/proxyBadge";
import { describeError } from "./errors";
import { toasts } from "./toasts.svelte";

/** The event the back end sends (`lib.rs`, `proxy_jobs.rs`). */
const PROXY_CHANGED_EVENT = "proxy-changed";

class Proxies {
  /** Heavy videos get proxies automatically, and the preview uses them. */
  isAutomatic = $state(true);
  /** The space proxies take on this computer, when last asked. */
  cacheSizeInBytes = $state(0);
  /** Each video's proxy, by `MediaView.filePath`. */
  states = $state<Record<string, ProxyState>>({});
  isClearing = $state(false);

  /** Starts listening to the back end and reads the current states; returns how to stop. */
  async listen(): Promise<() => void> {
    const unlisten = await listen<ProxyStatus>(PROXY_CHANGED_EVENT, (event) => this.accept(event.payload));
    await this.refresh();
    return unlisten;
  }

  /** The proxy of the video at `filePath`, or `null`. */
  stateOf(filePath: string): ProxyState | null {
    return this.states[filePath] ?? null;
  }

  accept(status: ProxyStatus): void {
    if (status.proxy) {
      this.states[status.filePath] = status.proxy;
    } else {
      delete this.states[status.filePath];
    }
  }

  /** Reads the setting, the cache's size and every state again. */
  async refresh(): Promise<void> {
    try {
      this.applyOverview(await backend.getProxyOverview());
    } catch (error) {
      this.showError(error);
    }
  }

  async stop(mediaId: number): Promise<void> {
    try {
      await backend.stopProxy(mediaId);
    } catch (error) {
      this.showError(error);
    }
  }

  async makeAgain(mediaId: number): Promise<void> {
    try {
      await backend.makeProxyAgain(mediaId);
    } catch (error) {
      this.showError(error);
    }
  }

  async setAutomatic(isAutomatic: boolean): Promise<void> {
    try {
      this.applyOverview(await backend.setAutomaticProxies(isAutomatic));
    } catch (error) {
      this.showError(error);
    }
  }

  /** Deletes every proxy file, then says how much space it freed. */
  async clearCache(): Promise<void> {
    if (this.isClearing) {
      return;
    }
    this.isClearing = true;
    try {
      const cleared = await backend.clearProxyCache();
      this.applyOverview(cleared.overview);
      toasts.show({
        tone: "success",
        message: t("settings.proxies.cleared", { size: formatByteSize(cleared.freedBytes) }),
      });
    } catch (error) {
      this.showError(error);
    } finally {
      this.isClearing = false;
    }
  }

  private applyOverview(overview: ProxyOverview): void {
    this.isAutomatic = overview.isAutomatic;
    this.cacheSizeInBytes = overview.cacheSizeInBytes;
    const states: Record<string, ProxyState> = {};
    for (const status of overview.statuses) {
      if (status.proxy) {
        states[status.filePath] = status.proxy;
      }
    }
    this.states = states;
  }

  private showError(error: unknown): void {
    const described = describeError(error);
    toasts.show({ tone: "error", message: described.message, details: described.details });
  }
}

export const proxies = new Proxies();
