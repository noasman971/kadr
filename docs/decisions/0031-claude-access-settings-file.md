# 0031. The folders Claude may use are kept in Kadr's settings, read at every call

- Status: Accepted
- Date: 2026-11-27

## Context

Decision [0011](0011-allowed-folders-as-server-arguments.md) put the folders Claude may
use in `kadr-mcp`'s arguments (`--allowed-folder`), written by hand in Claude Desktop's
configuration, and announced that phase 7's *Connect to Claude* screen would write them.
The specification asks for that screen "in the settings": it **generates Claude
Desktop's configuration in one click and manages the allowed folders**.

If the folders stay arguments, every change made on the screen means rewriting Claude
Desktop's configuration and quitting and reopening Claude Desktop, which also stops its
conversations.

## Options considered

1. **Arguments only** (as in 0011): the screen rewrites `args` at each change. Nothing
   new to read, but every change needs a restart of Claude Desktop.
2. **A settings file Kadr owns**, `claude-access.json` in the app's configuration
   folder (`%APPDATA%\org.kadr.desktop` on Windows), listing the folders. The
   configuration the screen writes passes its path to `kadr-mcp`
   (`--kadr-settings <file>`), which reads it **before every call**; the app reads it
   too. A change applies to the next call, without restarting anything.
3. **Both sides ask the app** for the folders: does not work when the app is closed.

## Decision

Option 2. Folders given with `--allowed-folder` still count (configurations written by
hand keep working); the folders allowed are the union of both.

The screen:

- finds `kadr-mcp` next to the app's own program (the installer puts them together);
- finds Claude Desktop's configuration file (the usual places on each system, and the
  per-installation folders some Windows installations use), or lets the person choose
  it;
- **Connect** adds or replaces only the `kadr` entry of `mcpServers` (the command and
  `--kadr-settings <file>`, no `--allowed-folder`: a folder removed on the screen must
  stop being allowed), keeps everything else, keeps a backup of the previous file next
  to it (`claude_desktop_config.json.kadr-backup`), and writes atomically. A file that
  is not valid JSON is never overwritten: the screen says so;
- adds and removes allowed folders in `claude-access.json` at once;
- shows the `claude mcp add` command for Claude Code, ready to copy.

## Consequences

- Changing a folder takes effect at Claude's next call; only the first connection
  needs Claude Desktop to restart.
- Anything able to write the user's application data can widen the folders, exactly as
  it could edit Claude Desktop's configuration before: no new exposure.
- Claude Desktop's configuration is re-written with its keys in alphabetical order
  (JSON meaning unchanged); the backup keeps the original text.
