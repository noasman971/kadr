# Architecture decision records

Each important technical decision gets a short, numbered record. Records are never
rewritten: if a decision changes, a new record supersedes the old one.

| # | Decision | Status |
|---|---|---|
| [0001](0001-cargo-workspace-of-small-crates.md) | A Cargo workspace of small crates, interfaces only talk to the engine | Accepted |
| [0002](0002-gpl-3-license.md) | GPL-3.0-or-later license | Accepted |
| [0003](0003-face-blur-by-classic-tracking.md) | Face blur by classic region tracking, without AI | Accepted |
| [0004](0004-claude-through-local-mcp-only.md) | Claude only through a local MCP server, no Claude API | Accepted |
| [0005](0005-ffmpeg-through-ffmpeg-next.md) | FFmpeg through `ffmpeg-next`, behind backend traits | Accepted |
| [0006](0006-gpu-rendering-yuv-end-to-end.md) | GPU rendering with `wgpu`, YUV from end to end, CPU as reference | Accepted |
| [0007](0007-scale-from-point-tracking.md) | The tracked region's size comes from point tracking (Median Flow) | Accepted |
| [0008](0008-anchor-tracking-to-the-selection.md) | A slow-learning filter, anchored to the selection's appearance | Accepted |
| [0010](0010-mcp-server-with-rmcp.md) | The MCP server is built on `rmcp`, with tools defined in `kadr-tools` | Accepted |
| [0011](0011-allowed-folders-as-server-arguments.md) | In file mode, the allowed folders are arguments of the server | Accepted |
| [0012](0012-file-mode-saves-every-change.md) | File mode saves the project after every change | Accepted |
| [0020](0020-svelte-typescript-front-end.md) | Svelte 5 and TypeScript for the desktop front end | Accepted |
| [0021](0021-desktop-app-layout.md) | The desktop app in `apps/desktop/`, its Rust crate in the workspace | Accepted |
| [0022](0022-front-end-and-engine-communication.md) | The back end owns the project; preview frames and sound rendered by the engine, sent as binary | Accepted |
| [0030](0030-claude-edits-the-open-app-through-a-local-pipe.md) | Claude edits the project open in the app, through a local pipe; file mode when the app is closed | Accepted |
| [0031](0031-claude-access-settings-file.md) | The folders Claude may use are kept in Kadr's settings, read by `kadr-mcp` before every call | Accepted |
| [0050](0050-transitions-at-clip-edges.md) | Transitions at clip edges, using the neighbouring clip's handles | Accepted |
| [0051](0051-silence-removal.md) | Silences shortened to a pause, cut by one command on the clip's track | Accepted |
| [0052](0052-colour-luts-from-cube-files.md) | Colour LUTs: a path to a `.cube` file, read by the engine, applied after the colour adjustments | Accepted |
| [0054](0054-keyframe-curves-and-editing.md) | Keyframe curves: named eases and a bounded cubic Bézier, edited at the playhead | Accepted |
| [0055](0055-automatic-proxies-for-heavy-videos.md) | Automatic proxies for heavy videos: 1080p H.264 for quick jumps, in a cache outside projects, for the preview only | Accepted |

## Template

```markdown
# NNNN. Title

- Status: Proposed | Accepted | Superseded by NNNN
- Date: YYYY-MM-DD

## Context
What problem are we solving, and what constraints apply?

## Options considered
1. Option A — pros, cons.
2. Option B — pros, cons.

## Decision
What we chose.

## Consequences
What becomes easier, what becomes harder, what we must watch.
```
