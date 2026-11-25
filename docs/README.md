# Kadr documentation

| Document | What you will find |
|---|---|
| [architecture.md](architecture.md) | The crates, how they depend on each other, how a frame travels from the source file to the export |
| [contributing.md](contributing.md) | Environment, naming and code conventions, branches, commits, pull requests |
| [decisions/](decisions/) | Architecture decision records: why things are the way they are |

Documents added in later phases:

| Document | Phase |
|---|---|
| `data-model.md` — project, timeline, clips, effects, commands, `.kadr` file format | 1 |
| `gpu-rendering.md` — `wgpu` pipeline, shaders, adding an effect | 3 |
| `tracking-and-blur.md` — the region tracker, its parameters and limits | 4 |
| `mcp.md` — every MCP tool, Claude Desktop / Claude Code setup, example conversations | 5 |
| `design-system.md` and `user-journeys.md` — interface tokens, components, user journeys | 6 |

A feature is not finished until its documentation is up to date, in the same pull
request as the code.
