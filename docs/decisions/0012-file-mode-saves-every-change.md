# 0012. File mode saves the project after every change

- Status: Accepted
- Date: 2026-11-26

## Context

In phase 5 the MCP server works directly on a `.kadr` file, without the desktop
app. Claude Desktop starts the server when it starts and keeps it running; a
conversation can end at any moment, and Claude may forget to call `save_project`.
The specification also asks that one tool call be one undo group, and phase 7
will let the desktop app have the same file open.

## Options considered

1. **Save only on `save_project`**: the classic editor model, but work is lost when
   the conversation or Claude Desktop ends, and the file on disk does not show
   what Claude did until it remembers to save.
2. **Save after every successful change** (including undo and redo): the file is
   always up to date; `save_project` remains for "save as".

## Decision

Option 2.

- The open project lives in an `EditSession` (undo history in memory, up to 200
  steps, for as long as the server runs). Each tool call that changes the project
  runs as **one transaction labelled "Claude: …"**, then the file is written
  atomically (`kadr_engine::project_files`).
- If writing fails, the change is undone, so the project in memory and the file
  never disagree.
- Before each change, the file on disk is compared with what Kadr last read or
  wrote. If another program changed it (for example the desktop app), the call is
  refused and Claude is told to reopen the project, rather than overwriting the
  other program's work.
- `save_project` with a new path saves a copy and continues in it ("save as").

## Consequences

- No work is lost when a conversation ends; the undo history is (it is not stored
  in the file).
- A file is written after every call: small JSON, atomic rename, negligible cost.
- The comparison with the file on disk protects against two editors at once until
  phase 7 replaces it with a live link to the app.
