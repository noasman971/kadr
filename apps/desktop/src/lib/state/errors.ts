// Turning back-end errors into sentences for people (docs/ux.md: "what happened,
// why, what to do"; never a raw code).

import { isCommandError } from "../api/types";
import { quoted, t } from "../i18n/i18n.svelte";
import type { MessageKey } from "../i18n/messages.en";

export interface DescribedError {
  /** The sentence to show. */
  message: string;
  /** The engine's own words, for *Copy the details*. */
  details: string;
}

export function describeError(error: unknown): DescribedError {
  if (isCommandError(error)) {
    const key = `error.${error.code}` as MessageKey;
    return {
      message: t(key, { name: error.fileName ? quoted(error.fileName) : "" }).replace(/\s+/g, " ").trim(),
      details: error.details,
    };
  }
  const details = error instanceof Error ? error.message : String(error);
  return { message: t("error.internal"), details };
}

/** Whether the error only means "a newer request replaced this one". */
export function isSupersededError(error: unknown): boolean {
  return isCommandError(error) && error.code === "superseded";
}
