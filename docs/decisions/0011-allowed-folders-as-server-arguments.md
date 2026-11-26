# 0011. In file mode, the allowed folders are arguments of the server

- Status: Accepted
- Date: 2026-11-26

## Context

Claude chooses the paths the tools read and write. The specification limits file
access to the folders the user allowed in Kadr's settings, forbids replacing a
file without `overwrite: true`, and forbids deleting files. In phase 5 there is no
desktop app yet, so "Kadr's settings" do not exist: file mode needs another way to
receive the allowed folders. Phase 7 adds a "Connect to Claude" screen that
generates the Claude Desktop configuration and manages the allowed folders.

## Options considered

1. **Command-line arguments** (`--allowed-folder <folder>`, repeatable), written in
   the Claude Desktop configuration (`args`) or in `claude mcp add`. Visible next
   to the server command, nothing else to install; the phase 7 screen can write
   them when it generates the configuration.
2. **A settings file** (for example `%APPDATA%\Kadr\settings.json`) read at start.
   Closer to "Kadr's settings", but in phase 5 the user would edit a second file by
   hand, and a mistake there is harder to see.
3. **Environment variable**: hard to read in the JSON configuration, awkward with
   several folders.

## Decision

Option 1. `kadr-mcp --allowed-folder "C:\Users\sam\Videos" --allowed-folder
"D:\Edits"`.

- Each folder is resolved once at start (canonical form, links followed). A path
  Claude gives must be absolute; it is resolved the same way, and its canonical
  form must lie inside an allowed folder, so `..` or a link cannot escape.
- New files (project, export) must go in an existing folder inside an allowed
  folder, and an existing file is only replaced with `"overwrite": true`. Tools
  never delete files (the engine only removes its own temporary `.saving` /
  `.exporting` files).
- A media path stored in a project file is checked too before Kadr reads it, since
  a project can name any path.
- A folder given that does not exist does not stop the server: it is left out,
  logged, and named in error messages and in `get_project_info`, so a typing
  mistake is explained to Claude, and through Claude to the user.
- Without any folder, the server starts and every file tool explains how to add
  one.

## Consequences

- Changing the folders means editing the Claude configuration and restarting
  Claude. Phase 7's settings screen will write these arguments for the user; it
  may add a settings file later, read in addition to the arguments.
- The same checks will apply when `kadr-mcp` talks to the open desktop app.
