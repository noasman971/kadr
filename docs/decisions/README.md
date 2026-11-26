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
| [0040](0040-media-backend-features.md) | The media backend is chosen with Cargo features (FFmpeg by default, none on Android) | Accepted |
| [0041](0041-minimum-android-api-level.md) | Android 9 (API level 28) is the oldest supported Android | Accepted |
| [0042](0042-mediacodec-through-the-ndk.md) | MediaCodec through the NDK media API, with the `ndk` crate | Accepted |

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
