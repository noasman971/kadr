# Changelog

All notable changes to Kadr are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project uses [Semantic Versioning](https://semver.org/). Each validated phase is tagged
(`v0.1.0` for phase 0, `v0.2.0` for phase 1, and so on).

## [Unreleased]

## [0.2.0] — Phase 1: data model

### Added

- `kadr-core` data model:
  - exact time in integer ticks (flicks) with `Timestamp`, `TimeSpan` and exact
    `FrameRate` fractions;
  - one identifier counter for media, tracks, clips and effects, with a distinct type
    per kind;
  - project settings with landscape, vertical and square presets;
  - media assets (video, audio, image), tracks stacked bottom to top, media and text
    clips with exact playback speeds;
  - volume, opacity, position, scale and rotation animatable by keyframes (linear,
    ease-in-out, hold), and crop;
  - colour adjustment and tracked blur effects, the blur regions indexed by source time.
- 22 undoable commands covering media, tracks, clips, keyframes, effects and settings.
- `EditSession` with undo, redo, a history limit, and transactions that group several
  commands into one undo step, all or nothing.
- `validate_project`: every rule of a valid project in one place, run after every
  command and on every loaded file. Error messages name the objects involved and list
  what exists when something is missing.
- Versioned `.kadr` JSON file format with a migration mechanism, refusing files from
  newer versions.
- Tests: unit tests for every module and command (apply, revert and redo must give
  identical projects), an end-to-end editing scenario, 400 pseudo-random edits fully
  undone, and a snapshot of a complete project file.
- `docs/data-model.md`.

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

### Changed

- CI uses `actions/checkout` v7, which runs on Node 24.
