# Status, and how to resume

Last updated: **2026-11-26**, when the owner moved the work from the laptop to the
desktop PC.

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

   The Android tools are only needed for phase 9 (see its pull request).
2. **Clone the repository**, preferably not inside another Git repository:

   ```powershell
   git clone https://github.com/noasman971/kadr.git
   ```

3. **Check the build:**
   1. `cargo build --workspace`;
   2. `powershell -ExecutionPolicy Bypass -File scripts/generate-test-fixtures.ps1`;
   3. `cargo test --workspace`.

   The first build takes several minutes.
4. **Open the folder in Claude Code.** `CLAUDE.md` is read automatically. Start one
   session per phase to continue, with the prompt given in that phase's section
   below.
5. **Keep two or three sessions at most at the same time**, and limit each build:
   `$env:CARGO_BUILD_JOBS=4` in PowerShell, or `export CARGO_BUILD_JOBS=4` in Git
   Bash. Five sessions building at once saturated the laptop's 32 GB of memory, and
   everything ran 10 to 20 times slower.

## Phases at a glance

| Phase | State | Branch / pull request | Next step |
|---|---|---|---|
| 0–3 | Done, validated, tagged `v0.1.0`–`v0.4.0` | `main` | — |
| 4. Tracking and blur | 4a merged (#7). 4b (#8): drift on a real video fixed (mean error 197 → 2 px) | `feature/phase-4b-blur-analysis`, [#8](https://github.com/noasman971/kadr/pull/8) | Owner: validate phase 4, then tag `v0.5.0` |
| 5. MCP server | Draft (#9): 34 of 35 tools tested | `feature/phase-5-mcp-server`, [#9](https://github.com/noasman971/kadr/pull/9) | Run the tests after the merge of `main`, then merge |
| 6. Desktop app | 6a (#6), 6b (#10) and 6c (#14) merged. 6d in PR #15, tried by hand | `feature/phase-6d-inspector-export`, [#15](https://github.com/noasman971/kadr/pull/15) | Merge 6d, then the owner's edit-and-export test |
| 7. Blur and Claude in the app | Started 2026-11-26: 7a (blur tools for Claude) in PR | `feature/phase-7a-blur-tools` | 7b: blur in the app |
| 8. Features, engine side | **Paused** by the owner. 8a in draft (#11) | `feature/phase-8a-transitions-text-animations`, [#11](https://github.com/noasman971/kadr/pull/11) | Owner: resume or not |
| 9. Android | **Paused** by the owner. 9a done, 9b started (draft #12) | `feature/phase-9a-android-build`, [#12](https://github.com/noasman971/kadr/pull/12) | Owner: resume or not |
| 10. iOS | Needs a Mac | — | — |

Versions are tagged in phase order, each only after the owner validates the phase:
`v0.5.0` for phase 4, `v0.6.0` for phase 5, `v0.7.0` for phase 6, and so on.

## Phase 4: tracking and blur

- **Done:**
  - 4a, the tracker in `kadr-vision` (merged, PR #7);
  - 4b in PR #8: the analysis in `kadr-engine` (`region_tracking`: forwards and
    backwards, samples stored in the `TrackedBlur`), the debug video,
    `kadr-cli blur`, and the 16 fixes from a code review.
- **The drift on a real video is fixed** (decision record 0008):
  - `scripts/generate-tracking-benchmark.ps1` makes a 60 s 1080p video where a
    test pattern moves over a detailed, still background on a known path; the
    `track_video` example and `scripts/compare-tracking-with-truth.py` measure the
    tracker on it.
  - The region slid off the object by 2–4 px per second while every frame stayed
    "confident". The cause was the correlation filter's learning rate (0.125):
    every small error was learnt as the new centre.
  - Now the filter learns at 0.02, every confident frame is anchored to the
    selection's appearance, and a confident frame that does not look like the
    selection at all is reported as uncertain and re-anchored. Mean centre error
    197 → 2.05 px, size error 24 % → 1.7 %, 182 frames per second decoding
    included. `docs/tracking-and-blur.md` has the details.
- **Next:** the owner validates phase 4 (ideally on a real video with faces, with
  `kadr-cli blur … --debug`), then `v0.5.0` is tagged.
- **Resume with:** *"Resume phase 4 of Kadr: read CLAUDE.md, docs/status.md and the
  'Status for resuming' section of PR #8, then continue with its next step."*

## Phase 5: MCP server

- **Done:**
  - `kadr-tools`;
  - the `kadr-mcp` server (stdio, `rmcp`);
  - 34 of the 35 tools of the specification, tested, including over stdio with the
    real binary;
  - frames and contact sheets;
  - silence detection.
- **Not verified yet:** the merge of `main` into the branch and the 35th tool
  (`detect_silences`). Next, run `cargo test -p kadr-mcp`, `cargo test -p kadr-engine
  --lib` and `cargo doc --workspace --no-deps`, merge once CI is green, then the real
  test with the owner in Claude Desktop.
- `blur_region` and `get_blur_status` came with phase 7a (see below).
- **Resume with:** *"Resume phase 5 of Kadr: read CLAUDE.md, docs/status.md and the
  'Status for resuming' section of PR #9, then continue with its next step."*

## Phase 6: desktop app

- **Done:**
  - 6a (screens, design system, mockups, decision records 0020–0022), merged (#6).
    The owner validated the mockups and the technical choices: Svelte 5 and
    TypeScript, Tauri 2.
  - 6b (engine preview API, app skeleton), tried by hand with screenshots and
    merged (#10).
  - 6c (editing on the timeline: drag from the library, move, trim, split, delete,
    snapping, tracks), tried by hand and merged (#14).
  - 6d (inspector, titles, export dialog with presets and progress), tried by hand,
    in PR #15.
- **Next:** merge #15, then the owner's real edit-and-export test, and the phase 6
  validation (no tag before it).
- **First step on a new computer**, from `apps/desktop`: `npm ci`, then
  `npm run tauri dev`.
- **Resume with:** *"Resume phase 6 of Kadr: read CLAUDE.md, docs/status.md and the
  'Status for resuming' section of PR #15, then continue with its next step."*

## Phase 7: blur and Claude in the app

- **Plan**, one branch and pull request each, merged in order:
  1. **7a, blur tools for Claude** (`feature/phase-7a-blur-tools`): MCP tools
     `blur_region` (background analysis; with an `effect_id` it corrects a blur and
     re-tracks from there), `get_blur_status` (ranges to check), `cancel_blur`;
     re-tracking from a correction in `kadr-engine`.
  2. **7b, blur in the app** (`feature/phase-7b-blur-in-app`): draw a box on the
     preview, tracking with progress and cancel, a Blur section in the inspector.
  3. **7c, checking and correcting** (`feature/phase-7c-blur-correction`): unsure
     ranges on the timeline, go to the next one, correct the box and re-track.
  4. **7d, live link** (`feature/phase-7d-live-link`): decision record 0030;
     `kadr-mcp` passes Claude's calls to the open app, which runs them on its own
     project; *"Claude a modifié le projet"* notice; file mode when the app is
     closed.
  5. **7e, Connect to Claude** (`feature/phase-7e-connect-to-claude`): a settings
     screen that writes the Claude Desktop configuration and manages the allowed
     folders.
- **Done when** the owner says in Claude Desktop *« floute le visage de la personne
  à gauche dans interview.mp4 et exporte pour TikTok »* and it works.
- Decision records 0030–0039 are reserved for phase 7.
- **Resume with:** *"Resume phase 7 of Kadr: read CLAUDE.md, docs/status.md and the
  'Status for resuming' section of the latest phase 7 pull request, then continue
  with its next step."*

## Phase 8: features, engine side (paused)

- **Paused** at the owner's request (2026-11-26) until phases 4, 5 and 6 are merged.
- 8a (transitions and text animations) is in draft PR #11. Everything passes except
  one GPU-vs-CPU test (push transition, chroma differences). The PR explains the
  likely cause and the fix to try.
- **Decision for the owner:** resume phase 8 or not, and in which order after 8a:
  LUTs, automatic proxies, SRT subtitles, stabilisation, silence removal.
- **Resume with:** *"Resume phase 8 of Kadr: read CLAUDE.md, docs/status.md and the
  'Status for resuming' section of PR #11, then continue with its next step."*

## Phase 9: Android (paused)

- **Paused** at the owner's request (2026-11-26) until phases 4, 5 and 6 are merged.
- Both are in draft PR #12:
  - 9a, the engine building for Android without FFmpeg, is done and checked
    locally. A new CI job builds for `aarch64-linux-android`; it is not a
    required check.
  - 9b, the MediaCodec backend, is written but has never been compiled.
- The new computer needs the Android SDK, the NDK r30, `cargo-ndk` and the
  `aarch64-linux-android` target, plus an emulator image for the device tests.
  PR #12 gives the versions and how they were installed.
- **Resume with:** *"Resume phase 9 of Kadr: read CLAUDE.md, docs/status.md and the
  'Status for resuming' section of PR #12, then continue with its next step."*
