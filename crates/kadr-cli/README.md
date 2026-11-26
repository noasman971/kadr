# kadr-cli

Kadr from the command line, to use the engine without the desktop app.

```powershell
cargo run --release -p kadr-cli -- probe my-video.mp4
cargo run --release -p kadr-cli -- assemble --output my-edit.kadr --format vertical clip1.mp4 clip2.mp4 music.mp3
cargo run --release -p kadr-cli -- render my-edit.kadr my-edit.mp4
```

| Command | What it does |
|---|---|
| `probe FILE [--json]` | Describes a media file |
| `assemble --output PROJECT [--format landscape\|vertical\|square] [--image-duration SECONDS] FILES…` | Creates a project with the files end to end |
| `render PROJECT OUTPUT [--software-encoder] [--fast]` | Exports to MP4; Ctrl+C cancels cleanly |

- **Depends on:** `kadr-engine`, `kadr-media`, `kadr-core`, `clap`, `ctrlc`.
- **Status:** phase 2. A `blur` command arrives in phase 4.
