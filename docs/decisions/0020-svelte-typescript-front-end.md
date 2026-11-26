# 0020. Svelte 5 and TypeScript for the desktop front end

- Status: Accepted
- Date: 2026-11-26

## Context

The desktop app is a Tauri 2 application: a Rust back end that calls the engine, and
a web front end shown in the system webview (WebView2 on Windows). The specification
proposes Svelte or React with TypeScript. Tauri 2 also builds for Android, so the same
front end will become the mobile interface in phase 9.

The front end is mostly custom drawing and dense interaction: a timeline with
thousands of elements while zooming and dragging, a preview canvas, sliders that
update the picture while they move. Ready-made widget kits help little here.

## Options considered

1. **Svelte 5 + TypeScript + Vite.** Components compile to direct DOM updates, with
   fine-grained reactivity (*runes*) and no virtual DOM. Little code per component,
   a small bundle, an official Tauri template. Smaller ecosystem than React.
2. **React + TypeScript + Vite.** The largest ecosystem and hiring pool. More code for
   the same component, and re-render cost to manage by hand (memoisation) in a
   timeline that changes 60 times per second while dragging.
3. **A Rust front end (Leptos, Yew, Dioxus) compiled to WebAssembly.** One language
   everywhere, but slower to iterate, fewer tools, and a less mature path on Android
   webviews.

## Decision

Option 1, validated by the owner on 2026-11-26:

- **Svelte 5** with runes, **TypeScript** in strict mode, built by **Vite**.
- **No component library**: Kadr's own components, built on the tokens of
  [design-system.md](../design-system.md).
- Icons: **Lucide** (`@lucide/svelte`, ISC). Font: **Inter**
  (`@fontsource-variable/inter`, SIL Open Font License), bundled, never fetched.
- Translations: a small in-house module, French and English dictionaries typed from
  one shape so a missing key fails the type check. No i18n dependency.
- Checks: `svelte-check` (types), `vitest` for pure logic (time ↔ pixels, snapping,
  timecodes).
- Tooling is local to `apps/desktop/` (`npm ci`): nothing installed globally, the
  Tauri CLI included (`@tauri-apps/cli`).

## Consequences

- Contributors need Node.js (20.19 or newer) on top of the Rust toolchain to build
  the app; the engine crates still build with Cargo alone.
- Interface logic that is not drawing (editing rules, file handling) stays in Rust,
  where it is tested with the rest of the engine; the front end stays a view.
- JavaScript dependencies are pinned (`package-lock.json`) and listed in
  `THIRD_PARTY_LICENSES.md` like the Rust ones.
