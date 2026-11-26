# 0021. Layout of the desktop app in the repository

- Status: Accepted
- Date: 2026-11-26

## Context

The desktop app has a web front end (Svelte, see
[0020](0020-svelte-typescript-front-end.md)) and a Rust back end (Tauri). The
specification reserves `apps/desktop/` for it and `apps/mobile/` for the Android
build of the same app. The engine crates must not depend on Tauri
([0001](0001-cargo-workspace-of-small-crates.md)).

## Options considered

1. **The Tauri crate inside the Cargo workspace.** One `Cargo.lock`, one `target/`
   folder, the workspace lints, and `cargo build/clippy/test --workspace` cover the
   app like any crate. Building the workspace on Linux then needs the webview's
   development packages (WebKitGTK).
2. **The Tauri crate in its own workspace.** The engine workspace stays free of
   Tauri, but the app compiles the engine a second time, has its own lock file whose
   versions can drift, and escapes the shared lints unless they are copied.

## Decision

Option 1:

```text
apps/desktop/
├── package.json, package-lock.json   front-end tooling (Vite, Svelte, Tauri CLI)
├── index.html, vite.config.ts, tsconfig.json
├── src/                              Svelte front end
│   ├── main.ts, App.svelte
│   └── lib/
│       ├── api/          typed calls to the Rust back end
│       ├── state/        app state (project view, selection, playback)
│       ├── components/   screens and panels
│       ├── ui/           reusable components (buttons, fields, dialogs…)
│       ├── i18n/         French and English dictionaries
│       └── styles/       design tokens and base styles
└── src-tauri/                        crate `kadr-desktop`, a workspace member
    ├── Cargo.toml, build.rs, tauri.conf.json, capabilities/, icons/
    └── src/              Tauri commands: thin calls to kadr-engine and kadr-core
```

- The workspace `members` gain `apps/desktop/src-tauri`. The crate is named
  `kadr-desktop` and produces the `kadr` executable.
- The Rust back end holds the editing rules the interface needs (add a clip at the
  end, split every clip under the playhead…) as small functions over `kadr-core`
  commands, tested like the rest of the workspace. It holds no drawing code.
- Picture and sound work the app needs and the engine lacked (preview frames,
  thumbnails, waveforms, playback sound) goes into **new `kadr-engine` modules**, so
  the Android app and the MCP server can reuse it.

## Consequences

- `cargo build --workspace` builds the app too. Without the front end built, a debug
  build loads the front end from the Vite development server (`npm run tauri dev`);
  `npm run tauri build` embeds it for release.
- CI: the Linux job installs the WebKitGTK development packages; a new job builds the
  front end (type check, tests) and the app.
- `apps/mobile/` (phase 9) will reuse `apps/desktop/src/` rather than copy it.
