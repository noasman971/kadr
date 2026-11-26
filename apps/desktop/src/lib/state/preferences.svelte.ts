// The user's interface preferences: language, theme, interface size, layout.
// They are per computer and remembered in the webview's local storage; they never
// go into a project file. Reading or writing storage can fail (private mode,
// cleared data): the app then simply uses the defaults.

import { translations, systemLanguage, type LanguageChoice } from "../i18n/i18n.svelte";

export type ThemeChoice = "dark" | "light" | "system";
export type PreviewQuality = "auto" | "full" | "half" | "quarter";

const STORAGE_KEY = "kadr.preferences";
export const SMALLEST_INTERFACE_SCALE = 0.8;
export const LARGEST_INTERFACE_SCALE = 1.5;

interface StoredPreferences {
  language: LanguageChoice;
  theme: ThemeChoice;
  interfaceScale: number;
  previewQuality: PreviewQuality;
  isSnappingOn: boolean;
}

const DEFAULT_PREFERENCES: StoredPreferences = {
  language: "automatic",
  theme: "dark",
  interfaceScale: 1,
  previewQuality: "auto",
  isSnappingOn: true,
};

function readStoredPreferences(): StoredPreferences {
  try {
    const text = localStorage.getItem(STORAGE_KEY);
    return text ? { ...DEFAULT_PREFERENCES, ...(JSON.parse(text) as Partial<StoredPreferences>) } : DEFAULT_PREFERENCES;
  } catch {
    return DEFAULT_PREFERENCES;
  }
}

class Preferences {
  language = $state<LanguageChoice>(DEFAULT_PREFERENCES.language);
  theme = $state<ThemeChoice>(DEFAULT_PREFERENCES.theme);
  interfaceScale = $state(DEFAULT_PREFERENCES.interfaceScale);
  previewQuality = $state<PreviewQuality>(DEFAULT_PREFERENCES.previewQuality);
  isSnappingOn = $state(DEFAULT_PREFERENCES.isSnappingOn);

  /** Reads the stored preferences and applies them to the page. Call once at start. */
  load(): void {
    const stored = readStoredPreferences();
    this.language = stored.language;
    this.theme = stored.theme;
    this.interfaceScale = stored.interfaceScale;
    this.previewQuality = stored.previewQuality;
    this.isSnappingOn = stored.isSnappingOn;
    this.apply();
    window
      .matchMedia("(prefers-color-scheme: dark)")
      .addEventListener("change", () => this.apply());
  }

  /** Changes preferences, applies and remembers them. */
  update(changes: Partial<StoredPreferences>): void {
    Object.assign(this, changes);
    this.apply();
    this.save();
  }

  private apply(): void {
    translations.language = this.language === "automatic" ? systemLanguage() : this.language;
    const root = document.documentElement;
    root.lang = translations.language;
    const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
    const theme = this.theme === "system" ? (prefersDark ? "dark" : "light") : this.theme;
    root.dataset.theme = theme;
    const scale = Math.min(LARGEST_INTERFACE_SCALE, Math.max(SMALLEST_INTERFACE_SCALE, this.interfaceScale));
    root.style.setProperty("--interface-scale", String(scale));
  }

  private save(): void {
    const stored: StoredPreferences = {
      language: this.language,
      theme: this.theme,
      interfaceScale: this.interfaceScale,
      previewQuality: this.previewQuality,
      isSnappingOn: this.isSnappingOn,
    };
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(stored));
    } catch {
      // Storage unavailable: the preference lasts until the app closes.
    }
  }
}

export const preferences = new Preferences();
