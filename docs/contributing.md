# Contributing to Kadr

Thank you for helping. This guide is the reference for how code is written,
reviewed and merged. The goal: **someone who discovers the project can understand
any file without asking anyone.**

## 1. Set up your machine

Follow [README.md — Setting up Windows](../README.md#setting-up-windows), then check:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/setup-windows.ps1
```

On Linux and macOS: install Rust with [rustup](https://rustup.rs/) and FFmpeg with
your package manager.

## 2. Git workflow

- **`main` always builds and passes its tests.** Do not work on it directly.
- **One branch per feature or fix**, created from an up-to-date `main`:

  | Prefix | For | Example |
  |---|---|---|
  | `feature/` | A new feature | `feature/timeline-undo-redo` |
  | `fix/` | A bug fix | `fix/audio-offset-after-seek` |
  | `docs/` | Documentation only | `docs/mcp-setup-guide` |
  | `refactor/` | Restructuring without behaviour change | `refactor/split-render-pipeline` |
  | `test/` | Tests only | `test/tracker-occlusion-cases` |
  | `chore/` | Tooling, CI, dependencies | `chore/update-wgpu` |

- **Small, frequent commits.** One commit = one logical change that compiles. No
  "wip", "fix" or "update" commits.
- **Conventional Commits**: `type(scope): short summary in the imperative`, then a blank
  line and a body explaining **why** when it is not obvious.

  ```text
  feat(core): add command grouping for undo

  A single request from Claude can change ten things; the user expects one
  Ctrl+Z to revert all of them.
  ```

  Types: `feat`, `fix`, `docs`, `test`, `refactor`, `perf`, `chore`, `ci`.
  Scopes: crate names without the `kadr-` prefix (`core`, `media`, `render`, `vision`,
  `engine`, `tools`, `mcp`, `cli`), or `desktop`, `mobile`, `docs`, `ci`.
- **Before every commit**: `cargo fmt --all`, `cargo clippy` and the relevant tests.
  Never commit secrets, heavy generated files or large videos.
- **Push often**: at least when a feature is done and at the end of every work session.
- **Pull request** to `main` when the feature is done: what changes, why, how to test,
  screenshots (dark and light theme) for anything visible. CI must be green.
  Pull requests opened by the owner or by Claude working with the owner are merged
  as soon as CI is green; pull requests from other contributors are merged by the
  owner after review.
- Each validated phase gets a version tag (`v0.1.0`, `v0.2.0`…) and a
  [CHANGELOG.md](../CHANGELOG.md) entry.

## 3. Readable code

Readability comes before cleverness.

### Naming

Names must say what a thing is or does, without reading its code. No obscure
abbreviations.

| Kind | Good | Bad |
|---|---|---|
| Function: verb + precise object | `split_clip_at_time`, `compute_peak_to_sidelobe_ratio`, `find_clip_by_id`, `export_timeline_to_mp4` | `process`, `handle`, `do_it`, `calc`, `run2` |
| Type: a domain concept | `TimelineClip`, `TrackedBlurEffect`, `CorrelationFilterTracker`, `ExportPreset` | `Data`, `Manager`, `Helper`, `Thing`, `Obj` |
| Variable | `clip_start_time`, `frame_width_in_pixels`, `tracking_confidence`, `remaining_frames` | `t`, `tmp`, `x2`, `val`, `res`, `d` |
| Boolean: a question | `is_playing`, `has_audio_track`, `should_loop` | `playing`, `flag`, `audio` |
| Ambiguous quantity: unit in the name | `duration_in_seconds`, `offset_in_frames`, `blur_radius_in_pixels` | `duration`, `offset`, `radius` |

- One-letter names only for very short loop indices or standard mathematical notation,
  with a comment.
- Rust conventions: `snake_case` for functions and variables, `PascalCase` for types,
  `SCREAMING_SNAKE_CASE` for constants.
- Everything is written in **English**: identifiers, comments, documentation, commits.

### Structure

- **Short functions doing one thing** (aim for under ~40 lines). Split long ones into
  well-named helpers.
- **Modules and files organised by concept.** No file of thousands of lines.
- **No magic numbers**: named, commented constants.

  ```rust
  /// Below this peak-to-sidelobe ratio, the tracker is no longer sure of the target.
  /// Empirical value from the MOSSE paper; see docs/tracking-and-blur.md.
  const MINIMUM_PSR_FOR_CONFIDENT_TRACKING: f32 = 7.0;
  ```

- **No duplication**: factor out a block as soon as it appears twice.
- **Explicit over clever**: a clear loop beats an unreadable iterator chain.
- **Strong types over bare primitives**: `ClipId` and `TimelinePosition`, not `u64`
  everywhere.
- Timeline time is exact (integers or rationals), never `f64`.

### Errors and safety

- Libraries return errors (`thiserror`); binaries may use `anyhow`.
- No `unwrap()` / `expect()` outside tests, unless an invariant is proven and commented.
  Clippy warns and CI fails on warnings.
- `unsafe` is denied workspace-wide. A crate that needs FFI opts out in its own
  `Cargo.toml`, and every `unsafe` block carries a `// SAFETY:` comment.
- Error messages are written for people: what happened, why, and what to do.

### Comments and documentation

- `///` on **every public item**: what it is for, parameters, return value, possible
  errors, and an example when useful. The `missing_docs` lint enforces it.
- `//!` at the top of every module: its role and how it fits in the whole.
- Comments explain **why** (choices, trade-offs, references), not what the code says.
- Algorithms (MOSSE, KCF, Lucas-Kanade, One Euro filter…) are explained step by step,
  with a reference to the original paper.
- `cargo doc --workspace --no-deps` must build without warnings.

## 4. Tests

- Unit tests next to the code (`#[cfg(test)] mod tests`), integration tests in each
  crate's `tests/` folder.
- Test names describe the behaviour: `split_clip_at_time_keeps_total_duration`.
- Media tests use the generated fixtures (see [tests/fixtures/README.md](../tests/fixtures/README.md)).
- Every new command is tested with `commands::test_support::apply_and_check_revert`,
  which checks that apply, revert and redo give identical projects.
- Snapshot tests use `insta`. After an intended change, regenerate the snapshots and
  review the diff before committing:

  ```powershell
  $env:INSTA_UPDATE='always'; cargo test -p kadr-core
  ```
- Performance claims are backed by `criterion` benchmarks, with numbers in the pull
  request.

## 5. Documentation

A feature is not finished until its documentation is up to date, in the same pull
request: the relevant file in [docs/](README.md), the crate `README.md`, the
[CHANGELOG.md](../CHANGELOG.md), and a [decision record](decisions/) for any important
technical choice.

## 6. Pull request checklist

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cargo doc --workspace --no-deps` without warnings
- [ ] Documentation and CHANGELOG updated
- [ ] New dependencies listed in [THIRD_PARTY_LICENSES.md](../THIRD_PARTY_LICENSES.md)
- [ ] For visible changes: screenshots in dark and light theme, keyboard navigation
      tested, empty / loading / error states checked, small window (1280×720) checked
