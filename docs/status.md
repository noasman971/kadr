# Status, and how to resume

Last updated: **2026-11-27**, on the laptop, after phase 7's last pull requests
(7d, 7e, the blur inspector fix) and phases 8b and 8c were merged.

This page says where every phase stands and how to pick the work up on another
computer. Each open pull request starts with a **"Status for resuming"** section,
which is more detailed and is the reference for its phase.

## Work in progress on 2026-11-27 (read this first)

Work stopped on the laptop when the owner's usage limit came near. Everything is
pushed; nothing lives only on the laptop.

| Work | Where | State |
|---|---|---|
| 8d, stabilisation | Branch `feature/phase-8d-stabilization` and its draft pull request (#30) | Steps 1–8 of 14 are done (motion measurement, path smoothing, the effect in file format 5, the renderer, the analysis, the benchmark) and the composer draws stabilised clips. On the benchmark the centre of the picture is steady to half a pixel; the roll is measured less well. [stabilization-plan.md](stabilization-plan.md), "Where the work stands", gives the numbers and the next things to do |
| 8f, automatic proxies | Branch `claude/phase-8f-proxies-98d76d` and its pull request | Proxies are made, used by the preview and shown in the app. Docs and the end checks were left: see its "Status for resuming" |
| 8g, SRT subtitles | Pull request #29 | Code and docs written. One CI job (Ubuntu) failed when work stopped: see its "Status for resuming" |
| Automatic subtitles | Not started | Approved by the owner, as the step after 8g (below) |

To resume 8d: *"Resume phase 8d of Kadr (stabilisation): read CLAUDE.md,
docs/status.md, docs/stabilization-plan.md and the 'Status for resuming' section of
the draft pull request of `feature/phase-8d-stabilization`, then continue with its
next step."*

**The owner's decisions of 2026-11-27 that are written nowhere else:**

- **Work faster.** Keep the tests, the quality benchmarks, CI and the end-of-phase
  checklist. Write the code directly rather than through large multi-agent
  workflows, review once before the pull request, and move non-essential polish to
  "Later steps".
- **Automatic subtitles are approved**, as their own pull request after 8g: speech
  recognition on the computer with whisper.cpp through a Rust binding, the model
  downloaded on demand and never shipped, nothing sent anywhere. 8g was asked to keep
  one function from (start, end, text) cues to the subtitle track and one command
  that inserts a batch of cues. Building whisper.cpp needs CMake on the `PATH`:
  `winget install Kitware.CMake` (Visual Studio Build Tools ship one, but not on the
  `PATH`); add it to `scripts/setup-windows.ps1`. Use the next free decision record
  number (0057).
- **Phase 9 (Android) resumes only after all of phase 8**, automatic subtitles
  included. It starts with a half-day trial: a minimal Tauri 2 Android app showing
  the engine's preview frames, measured on the owner's phone. If it is not smooth,
  the mobile interface moves to Flutter with `flutter_rust_bridge` (or Kotlin) on the
  same engine.
- **A friend of the owner with a Mac may contribute to phase 10 (iOS).** The owner
  has the prompt to give them. Decision records 0060–0069 are reserved for phase 10.
  The owner reviews and merges that contributor's pull requests; Claude never does.
  Open question for the owner: Kadr is GPL, which is widely considered incompatible
  with the App Store, so outside contributions may need a contributor agreement
  first.
- **Auto-merge is allowed on the repository** (`gh pr merge --auto --merge`), for
  the pull requests Claude opened with the owner.

**The laptop's disk is nearly full** (about 8 GB free on 2026-11-27). The build
folders take about 150 GB: `target/` of the main checkout (68 GB) and of each
worktree under `.claude/worktrees/`. Those of finished sessions (8e, then 8f and 8g
once merged) can be deleted with the owner's OK; nothing builds until then.

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
| 7. Blur and Claude in the app | 7a–7e merged (#17, #19, #20, #21, #24), blur inspector fix merged (#25) | `main` | The owner's end test |
| 8. Features | 8a, 8b and 8c merged (#11, #23, #27), 8e in #28 | `main` | Owner: which feature next |
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
- **Fix merged:** #25: the inspector's Blur section had the race #18 fixed for the
  other settings (two quick changes, the second undoing the first). Blur requests now
  name only what changes.
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
2. Lancer `target\release\kadr.exe`, ouvrir **Paramètres → Claude →
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
- **8c, colour grading with `.cube` LUTs** (merged, #27):
  - the `color_lut` effect (file format 3);
  - the parser and the CPU/GPU application in `kadr-render`;
  - reading and caching in the engine (a LUT that cannot be read is skipped with an
    export warning);
  - Claude's `add_color_lut`;
  - the inspector's LUT row;
  - `kadr-cli assemble --lut`;
  - decision record 0052.

  Not tried by a person yet: the inspector's native picker, and a real LUT from
  Resolve on a real video (the tests use small generated tables). Owner's check:
  `kadr-cli assemble -o graded.kadr --lut <a .cube> <a video>`, then
  `kadr-cli render graded.kadr graded.mp4`, or the inspector's Colour section.
- **8e, editable keyframes with curves** (#28, decision record 0054, file format 4):
  - core: the curves ease in, ease out and cubic Bézier (handles kept from 0 to 1),
    `MoveKeyframe`;
  - the app: a diamond per setting in the inspector adds or removes a keyframe at
    the playhead, the field sets the value at the playhead, a lane per animated
    value moves keyframes, the *Curve* menu and a small curve editor change the
    curve; diamonds on the timeline clip;
  - Claude's `set_keyframes`.

  Owner's check, in French: *sélectionner un titre, mettre la tête de lecture à son
  début, cliquer sur ◇ devant Position X et Opacité, mettre l'opacité à 0 et la
  position X à −600 ; avancer d'une seconde, remettre l'opacité à 100 % et la
  position X à 0 ; choisir « Arrivée en douceur » dans Courbe ; lire, annuler
  (Ctrl+Z), rétablir, enregistrer, rouvrir, exporter.*
- **Next step:** the owner chooses the next feature (8d stabilisation, 8f proxies
  and 8g subtitles are being built in parallel).
- **Left, proposed in the specification's order:** stabilisation (`kadr-vision`'s
  optical flow and smoothing), automatic proxies for heavy 4K videos, SRT
  subtitles. Also waiting:
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
