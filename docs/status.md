# Status, and how to resume

Last updated: **2026-11-27**, on the laptop, after phase 7's last pull requests
(7d, 7e) and phase 8b were merged.

This page says where every phase stands and how to pick the work up on another
computer. Each open pull request starts with a **"Status for resuming"** section,
which is more detailed and is the reference for its phase.

## Resuming on another computer

1. **Install the tools.** In PowerShell, from a copy of the repository:

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts/setup-windows.ps1
   ```

   It checks everything and prints the `winget` command for what is missing:
   - Visual Studio Build Tools;
   - Rust (rustup);
   - Git;
   - the FFmpeg command line (Gyan);
   - LLVM (libclang);
   - the FFmpeg 9 "shared" development files (`winget install BtbN.FFmpeg.GPL.Shared.9.0`),
     with the user variable `FFMPEG_DIR` pointing to the folder that holds `include\`
     and `lib\`, and that folder's `bin\` on the `PATH` (see `README.md`, "FFmpeg
     development files");
   - Node.js 20.19 or newer, for the desktop app.

   The Android tools are only needed for phase 9 (see its pull request). If the
   Claude app was open while the tools were installed, restart it, or its sessions
   keep the old `PATH` (CLAUDE.md, "Pitfalls").
2. **Get the latest work.** In an existing copy: `git switch main`, `git pull`, then
   `git fetch --prune --tags`. Otherwise clone it, preferably outside another Git
   repository and outside OneDrive:

   ```powershell
   git clone https://github.com/noasman971/kadr.git
   ```

3. **Check the build:**
   1. `cargo build --workspace`;
   2. `powershell -ExecutionPolicy Bypass -File scripts/generate-test-fixtures.ps1`;
   3. `cargo test --workspace`;
   4. for the app, from `apps/desktop`: `npm ci`, then `npm run tauri dev`.

   The first build takes several minutes.
4. **Open the folder in Claude Code.** `CLAUDE.md` is read automatically. Start one
   session per phase to continue, with the prompt given in that phase's section
   below.
5. **Keep two or three sessions at most at the same time**, and limit each build:
   `$env:CARGO_BUILD_JOBS=4` in PowerShell, or `export CARGO_BUILD_JOBS=4` in Git
   Bash. Each session's build folder grows to 10–30 GB: watch the free disk space.

## Phases at a glance

| Phase | State | Branch / pull request | Next step |
|---|---|---|---|
| 0–3 | Done, validated, tagged `v0.1.0`–`v0.4.0` | `main` | — |
| 4. Tracking and blur | Done, tagged `v0.5.0` | `main` (#7, #8) | — (owner's real-video test still to do) |
| 5. MCP server | Done, tagged `v0.6.0` | `main` (#9) | — (owner's Claude Desktop test still to do) |
| 6. Desktop app | Done, tagged `v0.7.0` | `main` (#6, #10, #14, #15, #16; fix #18 after the tag) | — (owner's edit-and-export test still to do) |
| 7. Blur and Claude in the app | 7a–7e merged (#17, #19, #20, #21, #24); blur inspector fix in #25 | `main`; `fix/blur-inspector-edit-race`, [#25](https://github.com/noasman971/kadr/pull/25) | Merge #25 once green, then the owner's end test |
| 8. Features | 8a and 8b merged (#11, #23) | `main` | Owner: which feature next |
| 9. Android | **Paused** by the owner. 9a done, 9b started (draft #12) | `feature/phase-9a-android-build`, [#12](https://github.com/noasman971/kadr/pull/12) | Owner: resume or not |
| 10. iOS | Needs a Mac | — | — |

Versions are tagged in phase order, each after the owner validates the phase:
`v0.8.0` for phase 7, `v0.9.0` for phase 8, and so on. `v0.5.0` and `v0.6.0` point to
the same commit (the merge of #8): phase 5 was finished first, but tags follow the
phase order. `v0.7.0` points to the merge of #16.

## Phases 4, 5 and 6: done

Tagged on 2026-11-27 at the owner's request. The real tests the specification
describes have **not** been done by the owner yet; they are the first thing to try
on the laptop:

- **Phase 4:** `kadr-cli blur <video with a face> --at <seconds> --rect x,y,w,h -o out.mp4 --debug`,
  then watch the debug video. On the 60 s benchmark the mean centre error is 2 px
  (`docs/tracking-and-blur.md`).
- **Phase 5:** build `kadr-mcp` in release, add it to Claude Desktop (README,
  "Editing with Claude"), and ask for a simple edit and an export.
- **Phase 6:** in the app (`npm run tauri dev`), a real edit: import, cut, titles,
  inspector, export.

Fixes found after the tags, both merged: inspector changes lost when it closes (#16,
in `v0.7.0`), quick inspector changes undoing each other (#18, after it).

## Phase 7: blur and Claude in the app

- **Plan**, one branch and pull request each, merged in order:
  1. **7a, blur tools for Claude** (merged, #17): MCP tools `blur_region`
     (background analysis; with an `effect_id` it corrects a blur and re-tracks from
     there), `get_blur_status` (ranges to check), `cancel_blur`.
  2. **7b, blur in the app** (merged, #19): draw a box on the preview, tracking with
     progress and cancel, a Blur section in the inspector.
  3. **7c, checking and correcting** (merged, #20): unsure ranges on the timeline, go
     to the next one, correct the box and re-track.
  4. **7d, live link** (merged, #21): decision record 0030; `kadr-mcp` passes
     Claude's calls to the open app, which runs them on its own project; *"Claude a
     modifié le projet"* notice; file mode when the app is closed.
  5. **7e, Connect to Claude** (merged, #24): decision record 0031; Settings →
     Claude writes the Claude Desktop configuration and manages the allowed folders,
     read by `kadr-mcp --kadr-settings` before every call. Docs in the README,
     `docs/mcp-server.md`, `docs/ux.md` and `docs/desktop-app.md`.
- **Fix in review:** #25 (`fix/blur-inspector-edit-race`): the inspector's Blur section
  had the race #18 fixed for the other settings (two quick changes, the second
  undoing the first). Blur requests now name only what changes. Merge once green.
- **Next step: the owner's end test.** Its steps, in French, are below. After it, the
  owner validates phase 7, and it is tagged `v0.8.0`.
- **Not done yet (not blocking):** the installer does not ship `kadr-mcp` next to the
  app (Tauri `externalBin`); the Connect screen's native pickers were not clicked by
  a person; macOS / Linux configuration paths are untested.

### End test of phase 7 (for the owner, in French)

1. Construire l'app et le serveur côte à côte, depuis le dossier du projet :
   `cargo build --release -p kadr-mcp`, puis depuis `apps/desktop` :
   `npx tauri build --no-bundle`. Les deux programmes sont alors dans
   `target\release\`.
2. Lancer `target\release\kadr-desktop.exe`, ouvrir **Paramètres → Claude →
   Connecter à Claude…**, ajouter le dossier qui contient `interview.mp4` (une vidéo
   avec deux personnes) et celui des exports, puis **Connecter Kadr à Claude
   Desktop**.
3. Quitter complètement Claude Desktop (aussi depuis l'icône près de l'horloge), le
   rouvrir, et dans une nouvelle conversation, avec l'app Kadr ouverte : *« floute le
   visage de la personne à gauche dans interview.mp4 et exporte pour TikTok »*.
4. Vérifier : Claude regarde une image, donne le rectangle, le suivi tourne dans
   l'app (le flou apparaît, avec *« Claude a modifié le projet »*), Claude vérifie
   les moments incertains, et la vidéo exportée (1080 × 1920) garde le visage flouté
   du début à la fin.
- **Done when** the owner says in Claude Desktop *« floute le visage de la personne
  à gauche dans interview.mp4 et exporte pour TikTok »* and it works.
- Decision records 0030–0039 are reserved for phase 7.
- **Resume with:** *"Resume phase 7 of Kadr: read CLAUDE.md, docs/status.md and the
  'Status for resuming' section of the latest phase 7 pull request, then continue
  with its next step."*

## Phase 8: features

- One pull request per feature. The owner is asked in one line before each one
  starts (the specification requires it).
- **8a, transitions and text animations** (merged, #11): entry and exit transitions
  (fade, slide, zoom) and text animations (fade, slide, pop, type-on, word by word),
  file format 2 with a migration, decision record 0050.
- **8b, silence removal** (merged, #23): the core command removing parts of a clip and
  closing the gaps, the engine's plan, the MCP tool `remove_silences`, `kadr-cli
  remove-silences` and the desktop app part (decision record 0051). Merging it with
  phases 7d and 7e needed one fix: tools now receive a `ToolCall`, not the
  `KadrToolbox`.
- **Next step:** the owner chooses the next feature.
- **Left, proposed in the specification's order:** LUT `.cube` files for colour grading,
  stabilisation (`kadr-vision`'s optical flow and smoothing), editable keyframes in
  the interface, automatic proxies for heavy 4K videos, SRT subtitles. Also waiting:
  Claude's `add_transition` still makes fades from opacity keyframes, and the
  inspector has no transition controls yet.
- Decision records 0050–0059.
- **Resume with:** *"Resume phase 8 of Kadr: read CLAUDE.md, docs/status.md and the
  'Status for resuming' section of the open phase 8 pull request, then continue with
  its next step."*

## Phase 9: Android (paused)

- **Paused** at the owner's request (2026-11-26).
- Both are in draft PR #12:
  - 9a, the engine building for Android without FFmpeg, is done and checked
    locally. A CI job builds for `aarch64-linux-android`; it is not a required
    check.
  - 9b, the MediaCodec backend, is written but has never been compiled.
- The computer needs the Android SDK, the NDK r30, `cargo-ndk` and the
  `aarch64-linux-android` target, plus an emulator image for the device tests.
  PR #12 gives the versions and how they were installed.
- **Resume with:** *"Resume phase 9 of Kadr: read CLAUDE.md, docs/status.md and the
  'Status for resuming' section of PR #12, then continue with its next step."*
