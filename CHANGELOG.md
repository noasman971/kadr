# Changelog

All notable changes to Kadr are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project uses [Semantic Versioning](https://semver.org/). Each validated phase is tagged
(`v0.1.0` for phase 0, `v0.2.0` for phase 1, and so on).

## [Unreleased]

## [0.1.0] — Phase 0: foundations

### Added

- Cargo workspace with eight documented crates: `kadr-core`, `kadr-media`,
  `kadr-render`, `kadr-vision`, `kadr-engine`, `kadr-tools`, `kadr-mcp`, `kadr-cli`.
- Workspace-wide lints: missing documentation, `unsafe`, `unwrap` and `expect` are
  reported; CI fails on any warning.
- `kadr-cli` with `--help` and `--version`, and its tests.
- Continuous integration on Windows, Linux and macOS: formatting, build, clippy, tests,
  documentation, and test video generation.
- `scripts/setup-windows.ps1`: read-only check of the Windows development environment.
- `scripts/generate-test-fixtures.ps1`: synthesises the test videos with FFmpeg.
- Documentation: README, architecture, contribution guide, first architecture decision
  records, third-party licenses.
- GPL-3.0-or-later license.
