// Translations: French and English, chosen from the system language unless the
// user picks one in the settings. `t()` reads the current language from a rune,
// so every text in the interface updates when the language changes.

import { englishMessages, type MessageKey, type Messages } from "./messages.en";
import { frenchMessages } from "./messages.fr";

export type Language = "fr" | "en";
export type LanguageChoice = Language | "automatic";

const MESSAGES: Record<Language, Messages> = { en: englishMessages, fr: frenchMessages };

/** French if the system speaks French, English otherwise. */
export function systemLanguage(): Language {
  const tags = typeof navigator === "undefined" ? [] : navigator.languages ?? [navigator.language];
  return tags.some((tag) => tag.toLowerCase().startsWith("fr")) ? "fr" : "en";
}

class Translations {
  language = $state<Language>(systemLanguage());

  /** The text for `key` in the current language, with `{name}` placeholders filled. */
  translate(key: MessageKey, parameters: Record<string, string | number> = {}): string {
    const template = MESSAGES[this.language][key] ?? englishMessages[key];
    return template.replace(/\{(\w+)\}/g, (placeholder, name: string) =>
      name in parameters ? String(parameters[name]) : placeholder,
    );
  }

  /** Numbers in the language's style: `17,6` / `17.6`. */
  formatNumber(value: number, maximumFractionDigits = 1): string {
    return new Intl.NumberFormat(this.language, { maximumFractionDigits }).format(value);
  }

  /** A date and time in the language's style. */
  formatDateTime(milliseconds: number): string {
    return new Intl.DateTimeFormat(this.language, { dateStyle: "medium", timeStyle: "short" }).format(
      new Date(milliseconds),
    );
  }
}

export const translations = new Translations();

/** Shorthand for `translations.translate`. */
export function t(key: MessageKey, parameters?: Record<string, string | number>): string {
  return translations.translate(key, parameters);
}

/** Whether `key` is a known message (for labels coming from the back end). */
export function isMessageKey(key: string): key is MessageKey {
  return key in englishMessages;
}

/** A track name for display: default names ("Video 2") are translated. */
export function trackDisplayName(name: string): string {
  const match = /^(Video|Audio) (\d+)$/.exec(name);
  if (!match) {
    return name;
  }
  const key: MessageKey = match[1] === "Video" ? "track.video" : "track.audio";
  return t(key, { number: match[2] ?? "" });
}

/** An undo label for display: keys sent by Kadr are translated, others kept. */
export function undoStepDisplayName(label: string): string {
  const key = `undo.${label}`;
  return isMessageKey(key) ? t(key) : label;
}

/** A file or clip name in the language's quotation marks. */
export function quoted(text: string): string {
  return translations.language === "fr" ? `« ${text} »` : `“${text}”`;
}
