# 0030. Claude edits the project open in the app, through a local pipe

- Status: Accepted
- Date: 2026-11-27

## Context

Claude Desktop starts `kadr-mcp` itself and talks to it over stdin / stdout. In phase 5
the server edits a `.kadr` file directly and refuses to touch it when another program
changed it ([0012](0012-file-mode-saves-every-change.md)). The desktop app, meanwhile,
**owns** the project it shows: one `EditSession`, one undo history, a project view
rebuilt after every change ([0022](0022-front-end-and-engine-communication.md)).

The specification asks that the app can be open while Claude works, that Claude's
changes show **live** with a *"Claude a modifié le projet"* notice, the changed clips
highlighted and *"Claude : …"* entries in the history, and that `kadr-mcp` keeps
working on the file when the app is closed. It suggests a named pipe on Windows and a
Unix socket elsewhere.

## Options considered

1. **Relay: the app runs Claude's calls.** When the app is open, `kadr-mcp` passes each
   tool call (its name and JSON arguments) to the app over a local pipe. The app runs
   the same `kadr-tools` code on its own open project and sends the answer back. When
   no app answers, `kadr-mcp` runs the call itself, in file mode. One project, one undo
   history, no conflict between the user's gestures and Claude's calls; the app knows
   exactly what changed. More code: the tools must work on the app's project, and a
   small protocol.
2. **Shared file, watched.** `kadr-mcp` stays in file mode; the app watches the `.kadr`
   file and reloads it when Claude saves. Little code, but two histories that ignore
   each other: a gesture in the app between two of Claude's calls makes Claude's next
   call fail (*"changed by another program"*) or forces a reload that loses Claude's
   undo; reloads arrive late and can race with the app's own saves.
3. **The app is the MCP server.** Claude Desktop starts a stdio program; a window app
   cannot be that program, so a stdio relay is needed anyway: this is option 1.

## Decision

Option 1, as the specification proposes. No question for the owner: it is the
specification's own mechanism, and the other options lose work or cannot be built.

**Transport.** A local pipe only the user can open, one JSON message per line:

- Windows: the named pipe `\\.\pipe\kadr-app-<user name>`, created by the app with the
  default security (only the user, administrators and the system may write to it),
  remote clients refused, `first_pipe_instance` so that no other program can take the
  name first.
- macOS and Linux: a Unix socket `kadr-app.sock` in the user's runtime folder
  (`XDG_RUNTIME_DIR`, else the temporary folder with the user name), mode `0600`.
- The app serves with `tokio` (already used by Tauri); `kadr-mcp` connects with plain
  blocking I/O from its tool thread.
- The protocol lives in a small crate, `kadr-app-link`: `hello` (protocol version), then
  `call_tool { name, arguments, allowed_folders }` → the tool's text and pictures (base64)
  and whether it failed. A version mismatch or any link failure sends `kadr-mcp` back to
  file mode, with a sentence for Claude.

**Who runs the tools.** The app, with the same `KadrToolbox`. The tools no longer reach
the project through the toolbox's own file: they reach it through a **project host**,
a small trait with what they need (read the project and its history, change it in one
undo step, undo, redo, save, open, create). File mode's host is phase 5's `.kadr` file
session; the app's host is its `OpenProject`. Tools that only read get a **snapshot**
of the project, so a slow picture (a contact sheet) never blocks the window; tools that
change it hold the app's lock for the call, so a user gesture and a Claude call never
interleave.

**What the user sees.** After each change by Claude, the back end sends the new view
with the step's label and the clips that changed. The front end shows *"Claude a
modifié le projet : …"* with **Undo**, outlines the changed clips for a moment, and the
undo history shows *"Claude : …"*. `open_project` and `create_project` open the project
in the app (*"Claude a ouvert « … »"*). Without a project open in the app, the tools say
so and Claude opens or creates one.

**Undo.** There is one history. Claude's `undo` undoes the last step **only if Claude
made it**; when the user changed something since, it refuses and says what the user
did, so Claude never undoes the user's work. The user's Ctrl+Z undoes anything.

**Allowed folders.** Each call carries the folders `kadr-mcp` was started with, and the
app's toolbox applies them exactly like file mode does
([0011](0011-allowed-folders-as-server-arguments.md)); phase 7e adds the folders
allowed in Kadr's own settings.

**Background work.** Analyses started by `blur_region` run in the app's toolbox; the app
adds them to its project as soon as they end (a one-second timer), with the same notice.

**Switching.** `kadr-mcp` tries the pipe before every call (a missing pipe fails at
once). When the app appears, calls go to it and the first answer says so (*"The Kadr app
is open: tools now work on the project open in it"*). When it goes away, `kadr-mcp`
reopens the app's last project file in file mode and says so.

## Consequences

- Claude and the user edit the same project at the same time, with one undo history;
  nothing is reloaded, nothing is lost.
- `kadr-tools` stays independent of the transport and of Tauri; `kadr-mcp` gains a
  relay and keeps file mode for when the app is closed.
- Any program running as the same user can talk to the pipe, as it could already read
  the user's files: the pipe gives no right the user's programs do not already have.
  Programs of other users, and other computers, cannot.
- Exports started by Claude run in the app's toolbox, apart from the export dialog's.
- Android and iOS (phases 9, 10) build the app without the pipe: Claude cannot reach a
  phone this way.
