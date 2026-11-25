# 0004. Claude only through a local MCP server, no Claude API

- Status: Accepted
- Date: 2026-11-25

## Context

Users should be able to describe an edit in a Claude conversation and have Kadr do it.
The project owner wants this to work with a **regular Claude subscription**, without
any API key or pay-per-use cost.

## Options considered

1. **Kadr calls the Claude API** with its own assistant panel. Works anywhere,
   including mobile, but needs an API key and costs money per request.
2. **Kadr exposes a local MCP server.** The user adds it to Claude Desktop or Claude
   Code; Claude, in the user's own conversation and subscription, calls Kadr's tools.

## Decision

Option 2 only. `kadr-mcp` runs locally over stdio and exposes the tools of
`kadr-tools`. Kadr never sends a request to Anthropic and contains no API key.

- Claude "sees" the video through tools that return images (a frame, a contact sheet).
- Videos stay on the computer. A video attached to a Claude conversation is **not**
  passed to an MCP server, so the user points Claude to files by name or folder, and
  Claude finds them with a search tool limited to folders the user allowed.
- When the desktop app is open, `kadr-mcp` connects to it (named pipe on Windows, Unix
  socket elsewhere) so changes appear live; otherwise it edits the project file.

## Consequences

- Zero extra cost for users; no secret to store.
- File access must be sandboxed to allowed folders, and destructive actions need an
  explicit flag.
- On Android, the Claude app cannot connect to a local MCP server. The Android app
  ships without Claude at first; alternatives without a paid API (for example the
  phone driving the desktop Kadr) will be studied later.
