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

> **Status: phase 3 — GPU rendering.** Kadr can read video, audio and image files,
> build a project, and export it to MP4 from the command line, rendering on the GPU
> with text, colour adjustment and tracked blurs. The desktop app (phase 6) is being
> built: it already imports media, plays the edit with its sound and saves projects.
> See the [roadmap](#roadmap) and [Trying Kadr today](#trying-kadr-today).

---

## Roadmap

| Phase | Content | Status |
|---|---|---|
| 0 | Foundations: workspace, CI, scripts, documentation | ✅ Done |
| 1 | Data model: timeline, clips, effects, undo/redo, project file | ✅ Done |
| 2 | Decoding, encoding and export from the command line | ✅ Done |
| 3 | GPU rendering (`wgpu`): compositing, text, effects | 🔍 In review |
| 4 | Face blur by classic region tracking | |
| 5 | Local MCP server for Claude | 🔍 In review |
| 6 | Desktop app (Tauri 2) — MVP | 🚧 In progress |
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

4. **LLVM** (`bindgen` uses `libclang` to compile the FFmpeg bindings):

   ```powershell
   winget install LLVM.LLVM
   ```

5. <a id="ffmpeg-development-files"></a>**FFmpeg development files.** The command-line
   FFmpeg above does not contain the headers and libraries Rust links against. Kadr
   needs a *shared* build of **FFmpeg 7 or newer** (FFmpeg 9 is used in CI). Install
   one and point `FFMPEG_DIR` to it; winget also adds its `bin` folder (the DLLs) to
   your `PATH`:

   ```powershell
   winget install BtbN.FFmpeg.GPL.Shared.9.0
   ```

   ```powershell
   $folder = (Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\BtbN.FFmpeg.GPL.Shared.9.0*\ffmpeg-*" -Directory | Select-Object -First 1).FullName; [Environment]::SetEnvironmentVariable('FFMPEG_DIR', $folder, 'User')
   ```

   Open a new terminal afterwards. On Linux, download the matching `linux64-gpl-shared`
   build from [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds/releases), set
   `FFMPEG_DIR` and `LD_LIBRARY_PATH`; on macOS, `brew install pkgconf ffmpeg` is enough.

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

---

## Trying Kadr today

### The desktop app (in progress)

With [Node.js](#setting-up-windows) installed, from `apps/desktop/`:

```powershell
npm ci
```

```powershell
npm run tauri dev
```

Create a project, drop videos, photos or music on the window, click **+** on a
thumbnail to put it on the timeline, and play it (Space, J/K/L). Projects are saved
automatically in `Documents/Kadr/`. Editing on the timeline, the inspector and export
arrive in the next steps of phase 6. See [apps/desktop/README.md](apps/desktop/README.md).

### The command line

Build the command-line tool once (optimised):

```powershell
cargo build --release -p kadr-cli
```

Describe a file:

```powershell
.\target\release\kadr-cli probe my-video.mp4
```

Put files end to end in a new project (videos and images on a video track, music on
an audio track; `--format vertical` for TikTok / Reels / Shorts):

```powershell
.\target\release\kadr-cli assemble --output my-edit.kadr --format vertical clip1.mp4 clip2.mp4 music.mp3
```

Add a transition at every cut and an animated title (see
[docs/transitions-and-text-animations.md](docs/transitions-and-text-animations.md)):

```powershell
.\target\release\kadr-cli assemble --output demo.kadr --transition slide-left --title "Hello" --title-animation type-on clip1.mp4 clip2.mp4
```

Export it to MP4 (rendered on the GPU and encoded in hardware when available;
`--cpu` to render on the CPU, `--software-encoder` to force libx264, `--fast` for quick
previews, Ctrl+C to cancel cleanly):

```powershell
.\target\release\kadr-cli render my-edit.kadr my-edit.mp4
```

Shorten the silences of a video to short pauses and export the result (`--threshold
-50` keeps quieter sounds, `-30` also removes background noise; `--kept-pause 0.3`
leaves longer pauses):

```powershell
.\target\release\kadr-cli remove-silences interview.mp4 -o interview-short.mp4
```

`kadr-cli blur` blurs a face from one rectangle (see
[docs/tracking-and-blur.md](docs/tracking-and-blur.md)). Current limit: phone videos
with a rotation flag are not rotated yet.

---

## Editing with Claude

Kadr includes a local MCP server, `kadr-mcp`. Once it is added to Claude Desktop or
Claude Code, you describe the edit in a normal conversation and Claude does it with
Kadr, on your computer. **No API key and no extra cost: your regular Claude
subscription is enough.** Kadr never sends anything to Anthropic or anywhere else.

**Your videos stay on your computer.** A video attached to the conversation is *not*
passed to Kadr. Tell Claude the name of the file or the folder it is in; Claude finds
it with Kadr's search tool, which only looks in the folders you allowed.

### 1. Build the server

```powershell
cargo build --release -p kadr-mcp
```

The program is `target\release\kadr-mcp.exe`. It needs the FFmpeg DLLs on your
`PATH` (the winget installation of [Setting up Windows](#setting-up-windows) does
it; restart Claude after installing FFmpeg).

### 2. Choose the folders Kadr may use

Kadr only reads and writes inside the folders you give it with `--allowed-folder`
(repeat it for several folders): your videos, where projects are saved, where
exports go. It never replaces a file unless you agree, and never deletes files.

### 3a. Claude Desktop

1. Open Claude Desktop, then **Settings → Developer → Edit Config**. This shows
   `claude_desktop_config.json` wherever your installation keeps it (usually
   `%APPDATA%\Claude\` on Windows; some installations keep it under
   `%LOCALAPPDATA%`).
2. Add Kadr (use your own paths; in JSON, every `\` is written `\\`):

   ```json
   {
     "mcpServers": {
       "kadr": {
         "command": "C:\\Users\\sam\\Projects\\kadr\\target\\release\\kadr-mcp.exe",
         "args": [
           "--allowed-folder", "C:\\Users\\sam\\Videos",
           "--allowed-folder", "D:\\Edits"
         ]
       }
     }
   }
   ```

   Keep what the file already contains: add `"mcpServers"` next to the existing
   entries, separated by a comma, or, if it already has an `mcpServers` section, add
   the `"kadr"` entry inside it.
3. Quit Claude Desktop completely (also from the system tray) and start it again.
   Kadr's tools appear in the tools menu of a new conversation.

If Kadr does not appear, Claude Desktop's log `mcp-server-kadr.log`, in the `logs`
folder next to `claude_desktop_config.json`, says why.

### 3b. Claude Code

```powershell
claude mcp add kadr --scope user -- C:\Users\sam\Projects\kadr\target\release\kadr-mcp.exe --allowed-folder C:\Users\sam\Videos
```

Then `claude mcp list` shows `kadr`, and `/mcp` inside Claude Code shows its tools.

### 4. Ask

Five requests to try:

1. *"Create a Kadr project called holiday in my Videos folder, find my beach videos
   and put them one after another."*
2. *"Make a 20-second vertical video of the best moment of beach.mp4 with the title
   'Summer 2026' at the top, and export it for TikTok."*
3. *"Look at the interview and cut everything between 1:12 and 1:20, then fade out the
   end."*
4. *"Lower the music to 30 %, make the second clip a bit warmer and brighter, and show
   me a frame so I can check."*
5. *"Undo that last change, then export the project in 1080p to D:\Edits\final.mp4."*

And for the face blur: *"Blur the face of the person on the left in interview.mp4 and
export it for TikTok."* Claude looks at a frame, gives the rectangle around the face,
Kadr follows it through the clip, and Claude checks the moments where the tracker was
unsure.

Claude looks at the video through pictures Kadr sends back (a frame, or thumbnails
across a clip), so it can check its own work. Every request it carries out is one
step in the undo history. The project file is saved after every change.

The list of tools and their rules is in [docs/mcp-server.md](docs/mcp-server.md).

---

## Project layout

```
crates/
  kadr-core     data model: project, timeline, clips, effects, undo/redo
  kadr-media    decoding and encoding (FFmpeg on desktop, MediaCodec on Android)
  kadr-render   GPU (wgpu) and CPU renderers shared by preview and export
  kadr-vision   classic computer vision: face tracking for blur, silence detection
  kadr-engine   single API used by every interface; background tasks
  kadr-tools    editing tools exposed to Claude
  kadr-mcp      local MCP server for Claude Desktop / Claude Code
  kadr-cli      command-line binary for testing
apps/
  desktop       desktop app: Svelte front end, Tauri back end (crate kadr-desktop)
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
