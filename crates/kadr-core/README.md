# kadr-core

The Kadr data model: exact time, project, timeline, tracks, clips, effects, keyframes,
the undoable commands that change them, the validation rules, and the `.kadr` file
format.

- **Depends on:** `serde`, `serde_json`, `thiserror`. Nothing inside Kadr.
- **Used by:** every other crate.
- **Must never depend on:** video decoding, GPU, user interface or platform-specific code.
- **Status:** implemented (phase 1).

## Example

```rust
use kadr_core::commands::{AddClip, AddTrack, CreatedObjectsExt, TrackPosition};
use kadr_core::history::EditSession;
use kadr_core::project::{Project, ProjectSettings};
use kadr_core::project_file::save_project_to_json;
use kadr_core::time::{TimeSpan, Timestamp};
use kadr_core::timeline::TrackKind;

let mut session = EditSession::new(Project::new(ProjectSettings::vertical_1080x1920()))?;
let track_id = session
    .execute(AddTrack::new(TrackKind::Video, TrackPosition::Top))?
    .first_track_id()
    .expect("a track was created");
session.execute(AddClip::text(track_id, "Hello", TimeSpan::from_whole_seconds(3), Timestamp::ZERO))?;
session.undo()?;
let file_text = save_project_to_json(session.project())?;
```

See [docs/data-model.md](../../docs/data-model.md) for a guided tour.
