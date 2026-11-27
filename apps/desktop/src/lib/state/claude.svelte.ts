// Claude working on the open project (decision record 0030, docs/ux.md
// "Claude in the app"): the back end runs Claude's calls and sends each change;
// this module shows it at once, says so, and outlines the clips that changed.

import { listen } from "@tauri-apps/api/event";

import type { ClaudeChange } from "../api/types";
import { quoted, t } from "../i18n/i18n.svelte";
import { editor } from "./editor.svelte";
import { toasts } from "./toasts.svelte";

/** The event the back end sends (`claude_link.rs`). */
const CLAUDE_CHANGED_EVENT = "claude-changed-project";
/** How long changed clips stay outlined. */
const HIGHLIGHT_DURATION_IN_MILLISECONDS = 2_500;
/** Claude's notices replace each other instead of piling up. */
const CLAUDE_TOAST_GROUP = "claude";

class ClaudeActivity {
  /** Clips Claude just added or changed, outlined on the timeline for a moment. */
  highlightedClipIds = $state<number[]>([]);
  private highlightTimer: number | undefined;

  /** Starts listening to the back end; returns how to stop. */
  async listen(): Promise<() => void> {
    return listen<ClaudeChange>(CLAUDE_CHANGED_EVENT, (event) => this.show(event.payload));
  }

  /** Shows one change by Claude. */
  show(change: ClaudeChange): void {
    if (change.openedProject) {
      editor.showProject(change.view);
      toasts.show({
        tone: "info",
        group: CLAUDE_TOAST_GROUP,
        message: t("claude.openedProject", { name: quoted(change.view.name) }),
      });
      return;
    }
    editor.acceptView(change.view);
    this.highlight(change.changedClipIds);
    const isUndoable = change.stepLabel !== null && change.view.undoLabel?.startsWith("Claude: ");
    toasts.show({
      tone: "info",
      group: CLAUDE_TOAST_GROUP,
      message: change.stepLabel ? t("claude.changedStep", { step: change.stepLabel }) : t("claude.changed"),
      action: isUndoable ? { label: t("toast.undo"), run: () => void editor.undo() } : undefined,
    });
  }

  private highlight(clipIds: number[]): void {
    window.clearTimeout(this.highlightTimer);
    this.highlightedClipIds = clipIds;
    this.highlightTimer = window.setTimeout(() => {
      this.highlightedClipIds = [];
    }, HIGHLIGHT_DURATION_IN_MILLISECONDS);
  }
}

export const claudeActivity = new ClaudeActivity();
