# kadr-mcp

The local MCP server (stdio) that lets Claude Desktop and Claude Code edit videos in
Kadr. No Claude API key: a regular Claude subscription is enough.

- **Depends on:** `kadr-tools`, `kadr-engine`, `rmcp` (the official Rust MCP SDK).
- **Used by:** Claude Desktop / Claude Code, which launch it as a local MCP server.

```powershell
cargo build --release -p kadr-mcp
.\target\release\kadr-mcp.exe --help
```

Claude starts it with `--allowed-folder <folder>` (repeatable): the only folders
Kadr may read and write in. It speaks MCP on stdin / stdout and logs to stderr
(`KADR_LOG=debug` for more detail). Setup: [README](../../README.md#editing-with-claude).
Tools and rules: [docs/mcp-server.md](../../docs/mcp-server.md).

In phase 5 the server edits a `.kadr` file directly; in phase 7 it connects to the
open desktop app so changes appear live.
