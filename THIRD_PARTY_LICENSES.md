# Third-party licenses

Every third-party component Kadr uses, ships or downloads is listed here with its
license. Kadr is released under GPL-3.0-or-later, so every component must be
compatible with it and redistributable, including for commercial use by Kadr's users.

**Rule:** a pull request that adds a dependency adds its line here.

## Rust crates

None yet. Phase 0 has no external dependency.

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
