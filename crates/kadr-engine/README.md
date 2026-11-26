# kadr-engine

The single public API used by every user interface:

- `project_files`: save (atomically) and open `.kadr` files;
- `export`: render a project to MP4, with progress and cancellation; a failed or
  cancelled export never leaves a broken file;
- `quick_assembly`: build a project placing files end to end.

- **Depends on:** `kadr-core`, `kadr-media`, `kadr-render`.
- **Used by:** `kadr-cli`; later `kadr-tools`, the desktop app and the Android app.
- **Status:** phase 2. Preview frames and background task management grow with the
  desktop app (phase 6).

See [docs/media-and-export.md](../../docs/media-and-export.md).
