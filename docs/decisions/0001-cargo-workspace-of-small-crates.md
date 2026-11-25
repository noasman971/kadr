# 0001. A Cargo workspace of small crates

- Status: Accepted
- Date: 2026-11-25

## Context

Kadr must run on Windows, macOS and Linux first, then on Android (and later iOS),
without rewriting the engine. It must also be driven by several interfaces: a desktop
app, a command-line tool and an MCP server for Claude. Contributors new to the project
need to find their way quickly.

## Options considered

1. **One big crate** with modules. Simple to start, but nothing prevents desktop-only
   or UI code from leaking into the core, and compile times grow with every change.
2. **A workspace of small crates with one job each**, and a single engine API used by
   every interface. More files to set up, but dependency rules are enforced by Cargo
   itself and each crate can be tested alone.

## Decision

Option 2. Crates: `kadr-core`, `kadr-media`, `kadr-render`, `kadr-vision`,
`kadr-engine`, `kadr-tools`, `kadr-mcp`, `kadr-cli`; apps in `apps/`. Interfaces only
depend on `kadr-engine` (and `kadr-mcp` on `kadr-tools`). See
[architecture.md](../architecture.md).

Shared metadata, dependency versions and lints live in the workspace `Cargo.toml`.

## Consequences

- The Android port only has to provide a new media backend and a touch interface.
- A crate that pulls in a platform-specific dependency is visible in review.
- Internal dependencies are declared only when used, because Cargo warns about unused
  dependencies and CI fails on warnings.
