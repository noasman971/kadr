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
| `ffmpeg-next`, `ffmpeg-sys-next` | Rust bindings to FFmpeg | WTFPL |
| `bindgen`, `clang-sys` and their dependencies *(build)* | Generate the FFmpeg bindings | BSD-3-Clause / Apache-2.0 and MIT |
| `image` | RGBA picture buffers (no file codecs enabled) | MIT OR Apache-2.0 |
| `rayon` (+ `crossbeam-*`, `either`) | Parallel rendering | MIT OR Apache-2.0 |
| `clap` and its dependencies | Command-line arguments of `kadr-cli` | MIT OR Apache-2.0 |
| `ctrlc` | Clean cancellation with Ctrl+C in `kadr-cli` | MIT OR Apache-2.0 |
| `wgpu`, `naga` and their dependencies (`ash`, `glow`, `gpu-allocator`, `half`…) | GPU rendering | MIT OR Apache-2.0 (some also Zlib, BSD, ISC, Apache-2.0 only) |
| `pollster` | Waiting for GPU operations | MIT OR Apache-2.0 |
| `rustfft` (+ `num-complex`, `num-traits`, `num-integer`, `primal-check`, `strength_reduce`, `transpose`) | Fourier transforms of the region tracker | MIT OR Apache-2.0 |
| `cosmic-text` and its dependencies (`swash`, `harfrust`, `skrifa`, `fontdb`, `ttf-parser`, `unicode-*`…) | Text layout and rasterisation | MIT OR Apache-2.0 / MIT / Apache-2.0 / Zlib; `self_cell` is used under Apache-2.0 |

## Rust crates used only by tests (not shipped)

| Crate | Role | License |
|---|---|---|
| `insta` | Snapshot tests of the project file | Apache-2.0 |
| `tempfile` | Temporary folders in tests | MIT OR Apache-2.0 |
| `criterion` and its dependencies | Benchmarks | Apache-2.0 OR MIT |
| `similar`, `console`, `tempfile` and their dependencies | Used by `insta` | Apache-2.0 / MIT / MIT OR Apache-2.0 |

## Native libraries

| Library | Role | License |
|---|---|---|
| FFmpeg 7+ (`libavcodec`, `libavformat`, `libavfilter`, `libavutil`, `libswscale`, `libswresample`, `libavdevice`) | Decoding, encoding, filters | LGPL-2.1-or-later; **GPL** builds (with libx264), as used in development and CI, are GPL-2.0-or-later, compatible with Kadr's GPL-3.0-or-later |
| libx264 (inside GPL FFmpeg builds) | Software H.264 encoder | GPL-2.0-or-later |

FFmpeg is linked dynamically: users can replace the libraries with their own build.

## Fonts

| Font | Role | License |
|---|---|---|
| Inter 4.1 (`InterVariable.ttf`), by The Inter Project Authors | Default text font, bundled in `crates/kadr-render/assets/fonts/` | SIL Open Font License 1.1 (full text next to the font, `Inter-LICENSE.txt`) |

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
