# Kadr desktop app

The Kadr desktop app: a [Tauri 2](https://tauri.app) application with a Svelte 5 +
TypeScript front end (`src/`) and a Rust back end, the crate `kadr-desktop`
(`src-tauri/`), which runs the engine.

- How it is built: [docs/desktop-app.md](../../docs/desktop-app.md).
- How it looks and behaves: [docs/ux.md](../../docs/ux.md),
  [docs/design-system.md](../../docs/design-system.md).

## Run it

Set up Rust and FFmpeg as the [main README](../../README.md#setting-up-windows)
explains, install Node.js 20.19 or newer, then from this folder:

```powershell
npm ci
```

```powershell
npm run tauri dev
```

## Check it

```powershell
npm run check
```

```powershell
npm test
```

The Rust side is checked with the rest of the workspace (`cargo clippy --workspace
--all-targets -- -D warnings`, `cargo test --workspace`).

## Package it

```powershell
npm run tauri build
```

The installer goes to `target/release/bundle/` at the repository root. FFmpeg's DLLs
must sit next to `kadr.exe` (or on the `PATH`) for the app to start.
