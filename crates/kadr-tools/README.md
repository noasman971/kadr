# kadr-tools

The editing tools Claude can call, each defined once: name, description for a
language model, JSON parameter schema (generated from a Rust struct), and the code
that turns a call into Kadr commands. No MCP dependency: `kadr-mcp` serves these
tools over MCP, and the desktop app can call them directly.

- **Depends on:** `kadr-core`, `kadr-engine`.
- **Used by:** `kadr-mcp`.

```rust
use std::sync::Arc;
use kadr_engine::FfmpegBackend;
use kadr_tools::{AllowedFolders, KadrToolbox};

let allowed_folders = AllowedFolders::new(["C:\\Users\\sam\\Videos".into()]);
let mut toolbox = KadrToolbox::new(allowed_folders, Arc::new(FfmpegBackend::new()?));
let output = toolbox.call_tool(
    "create_project",
    serde_json::json!({ "file_path": "C:\\Users\\sam\\Videos\\holiday.kadr" }),
);
assert!(!output.is_error);
```

| Module | Role |
|---|---|
| `toolbox` | `KadrToolbox`: open project, allowed folders, exports; `call_tool` |
| `catalog` | `ToolDefinition`, schema generation, argument reading |
| `tools/` | The tools, by subject, each with its description and parameter struct |
| `project_file_session` | The open project, saved after every change, one undo step per call |
| `file_access` | `AllowedFolders`: the only files tools may touch |
| `export_tasks` | Exports in the background, followed by task id |
| `pictures_for_claude` | Pictures scaled to 1024 pixels at most, JPEG |
| `views` | The project described in seconds and top-left canvas pixels |

Every tool and its rules: [docs/mcp-server.md](../../docs/mcp-server.md).
