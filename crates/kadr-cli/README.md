# kadr-cli

A command-line binary to exercise the Kadr engine without the desktop app.

```powershell
cargo run -p kadr-cli -- --help
cargo run -p kadr-cli -- --version
```

- **Will depend on:** `kadr-engine`.
- **Status:** `--help` and `--version` only (phase 0). `probe` and `render` arrive in
  phase 2, `blur` in phase 4.
