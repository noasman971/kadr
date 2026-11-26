# Third-party licenses

Every third-party component Kadr uses, ships or downloads is listed here with its
license. Kadr is released under GPL-3.0-or-later, so every component must be
compatible with it and redistributable, including for commercial use by Kadr's users.

**Rule:** a pull request that adds a dependency adds its line here. To list the
dependencies and their licenses:

```powershell
cargo tree -e normal,build --prefix none
```

## Rust crates built into Kadr

Used by `kadr-core`, directly or through another crate. Crates marked *(build)* only
run while compiling (procedural macros) and are not part of the program.

| Crate | Role | License |
|---|---|---|
| `serde` (+ `serde_core`) | Serialization framework | MIT OR Apache-2.0 |
| `serde_derive` *(build)* | Generates serialization code | MIT OR Apache-2.0 |
| `serde_json` | Reads and writes the `.kadr` JSON file | MIT OR Apache-2.0 |
| `itoa`, `zmij` | Fast number formatting for `serde_json` | MIT OR Apache-2.0 / MIT |
| `memchr` | Fast byte search for `serde_json` | Unlicense OR MIT |
| `thiserror` | Error type definitions | MIT OR Apache-2.0 |
| `thiserror-impl` *(build)* | Generates error code | MIT OR Apache-2.0 |
| `proc-macro2`, `quote`, `syn` *(build)* | Procedural macro support | MIT OR Apache-2.0 |
| `unicode-ident` *(build)* | Identifier rules for `syn` | (MIT OR Apache-2.0) AND Unicode-3.0 |

## Rust crates used only by tests (not shipped)

| Crate | Role | License |
|---|---|---|
| `insta` | Snapshot tests of the project file | Apache-2.0 |
| `similar`, `console`, `tempfile` and their dependencies | Used by `insta` | Apache-2.0 / MIT / MIT OR Apache-2.0 |

## Native libraries

None linked yet. FFmpeg arrives in phase 2: its exact build and license (LGPL or GPL
depending on the enabled codecs) will be recorded here.

## Models and data files

None, and none planned: the face blur uses classic tracking algorithms, without any
pre-trained model. See
[docs/decisions/0003-face-blur-by-classic-tracking.md](docs/decisions/0003-face-blur-by-classic-tracking.md).

## Development tools (not shipped)

| Tool | Use | License |
|---|---|---|
| FFmpeg command line | Generating test videos | LGPL-2.1+ / GPL-2.0+ depending on the build |
| GitHub Action `actions/checkout` | CI | MIT |
| GitHub Action `dtolnay/rust-toolchain` | CI | MIT |
| GitHub Action `Swatinem/rust-cache` | CI | LGPL-3.0 |
