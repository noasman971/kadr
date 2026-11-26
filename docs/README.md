# Kadr documentation

| Document | What you will find |
|---|---|
| [architecture.md](architecture.md) | The crates, how they depend on each other, how a frame travels from the source file to the export |
| [data-model.md](data-model.md) | Time, identifiers, project, timeline, clips, animation, effects, commands and undo, validation rules, the `.kadr` file |
| [contributing.md](contributing.md) | Environment, naming and code conventions, branches, commits, pull requests |
| [decisions/](decisions/) | Architecture decision records: why things are the way they are |
| [media-and-export.md](media-and-export.md) | How files are read, how sound and pictures stay in sync, how an export works |
| [rendering.md](rendering.md) | YUV from end to end, colour spaces, the GPU and CPU renderers, how they are compared, benchmarks |
| [tracking-and-blur.md](tracking-and-blur.md) | The region tracker behind the face blur, step by step: settings, accuracy, speed, limits |
| [transitions-and-text-animations.md](transitions-and-text-animations.md) | Fade, slide and zoom transitions between clips, text animations (type-on…), how they are drawn |

Documents added in later phases:

| Document | Phase |
|---|---|
| `gpu-rendering.md` — `wgpu` pipeline, shaders, adding an effect | 3 |
| `mcp.md` — every MCP tool, Claude Desktop / Claude Code setup, example conversations | 5 |
| `design-system.md` and `user-journeys.md` — interface tokens, components, user journeys | 6 |

A feature is not finished until its documentation is up to date, in the same pull
request as the code.
