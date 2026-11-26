# Kadr

**A free and open source video editor written in Rust, that you can drive from a
Claude conversation.**

- **Free, for real.** No premium tier, no locked feature, no account, no telemetry.
- **Edit by talking to Claude.** Add Kadr to Claude Desktop as a local MCP server, then
  ask: *"cut the silences, add a title at the start and export for TikTok"*. Claude
  uses Kadr's tools on your computer. **No API key: a regular Claude subscription is
  enough.**
- **Blur a face in one selection.** Draw a box around a face; a classic tracking
  algorithm (no AI) follows it through the whole clip and blurs it.
- **Everything stays on your machine.** Your videos are never uploaded anywhere.
- **Desktop first, Android next.** Windows, macOS and Linux, then an Android app built
  on the same engine.

> **Status: phase 1 — data model.** The project model, undo/redo and the `.kadr` file
> format exist as a library. There is no video decoding or interface yet. See the
> [roadmap](#roadmap).

---

## Roadmap

| Phase | Content | Status |
|---|---|---|
| 0 | Foundations: workspace, CI, scripts, documentation | ✅ Done |
| 1 | Data model: timeline, clips, effects, undo/redo, project file | 🔍 In review |
| 2 | Decoding, encoding and export from the command line | ⏳ Next |
| 3 | GPU rendering (`wgpu`) | |
| 4 | Face blur by classic region tracking | |
| 5 | Local MCP server for Claude | |
| 6 | Desktop app (Tauri 2) — MVP | |
| 7 | Face blur and Claude inside the app | |
| 8 | Silence removal, transitions, colour, stabilisation, keyframes, proxies | |
| 9 | Android app | |
| 10 | iOS app | |

The full specification (in French) is in [PROMPT_CLAUDE_CODE.md](PROMPT_CLAUDE_CODE.md).

---

## Setting up Windows

You need, in this order:

1. **Visual Studio 2022 Build Tools** with the *Desktop development with C++* workload
   (the Rust compiler uses its linker):

   ```powershell
   winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
   ```

2. **Rust** (through `rustup`). Open a **new** terminal afterwards:

   ```powershell
   winget install Rustlang.Rustup
   ```

   The exact toolchain (stable, with `rustfmt` and `clippy`) is selected automatically
   by [rust-toolchain.toml](rust-toolchain.toml).

3. **Git** and the **FFmpeg command line** (used to generate the test videos):

   ```powershell
   winget install Git.Git
   ```

   ```powershell
   winget install Gyan.FFmpeg
   ```

4. **LLVM** (needed from phase 2: `bindgen` uses `libclang` to compile the FFmpeg
   bindings):

   ```powershell
   winget install LLVM.LLVM
   ```

5. **FFmpeg development files** (needed from phase 2). The command-line FFmpeg above does
   not contain the headers and libraries Rust links against. Phase 2 will pick the
   exact FFmpeg version supported by the Rust bindings and document here how to install
   a *shared* build and set `FFMPEG_DIR`.

6. **Node.js LTS** (needed from phase 6, for the desktop interface):

   ```powershell
   winget install OpenJS.NodeJS.LTS
   ```

Then check everything at once. The script only reads, it installs nothing:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/setup-windows.ps1
```

---

## Building and testing

```powershell
cargo build --workspace
```

```powershell
cargo test --workspace
```

```powershell
cargo clippy --workspace --all-targets -- -D warnings
```

```powershell
cargo fmt --all
```

```powershell
cargo doc --workspace --no-deps --open
```

Generate the test videos (written to `tests/fixtures/generated/`, ignored by Git):

```powershell
powershell -ExecutionPolicy Bypass -File scripts/generate-test-fixtures.ps1
```

Try the command-line tool:

```powershell
cargo run -p kadr-cli -- --help
```

---

## Project layout

```
crates/
  kadr-core     data model: project, timeline, clips, effects, undo/redo
  kadr-media    decoding and encoding (FFmpeg on desktop, MediaCodec on Android)
  kadr-render   GPU compositor shared by preview and export
  kadr-vision   classic computer vision: face tracking for blur, silence detection
  kadr-engine   single API used by every interface; background tasks
  kadr-tools    editing tools exposed to Claude
  kadr-mcp      local MCP server for Claude Desktop / Claude Code
  kadr-cli      command-line binary for testing
docs/           architecture, decisions, guides
scripts/        environment check and test video generation
tests/fixtures/ test video descriptions (videos are generated, not committed)
```

Read [docs/architecture.md](docs/architecture.md) to understand how the pieces fit, and
[docs/contributing.md](docs/contributing.md) before opening a pull request.

---

## License

Kadr is free software, released under the
[GNU General Public License v3.0 or later](LICENSE). Third-party components and their
licenses are listed in [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).
