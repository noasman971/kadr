# 0050. Transitions at clip edges, using the neighbouring clip's handles

- Status: Accepted
- Date: 2026-11-26

## Context

Phase 8 adds transitions (fade, slide, zoom) and text animations. A transition
between two clips needs **both pictures at once** for a moment, but on one track
clips never overlap: that rule is checked after every command and many commands
rely on it. Claude must also be able to add a transition with one simple call, and
undo must keep working one command at a time.

## Options considered

1. **Overlap the clips** during the transition (Shotcut, and CapCut's way of making
   the edit shorter). Breaks the no-overlap rule that validation, splitting, moving
   and the renderer rely on; every command would need a special case.
2. **Separate transition objects** stored on the track, pointing at two clip
   identifiers (OpenShot, Kdenlive compositions). Every move, trim, split or removal
   of either clip must update or delete them, and undo must restore them: many
   commands change at once.
3. **A transition on each clip edge** (`entry_transition`, `exit_transition`), drawn
   inside the clip's own time range. When another clip on the same track touches
   that edge, its picture **continues underneath** during the transition, reading the
   source file past its trimmed range ("handles", like Premiere Pro, Final Cut and
   DaVinci Resolve) and holding its first or last frame where the file ends.

## Decision

Option 3.

- A transition belongs to one clip edge; no other object refers to it, so moving,
  trimming, splitting or deleting clips needs no special case. A split keeps the
  entry on the first part and the exit on the second.
- The timeline length never changes and clips never overlap.
- The same setting gives a fade from black (nothing before the clip), a dissolve
  (a clip touching before it) or a push (slide) from one clip to the next.
- Durations are stored as set. If a clip becomes shorter than its two transitions
  together, both are shortened in proportion when drawn, so a trim or split is never
  refused because of a transition.
- Transitions apply to videos and images. Text clips get **text animations**
  instead (fade, slide, pop, type-on, word by word), stored on the text itself,
  because they act on the letters (type-on) and move by a distance proportional to
  the font size rather than to the canvas.
- Sound is not changed by transitions; volume keyframes already fade it.

## Consequences

- A cross transition shows a little of the source beyond the neighbour's cut. Where
  the file has no more pictures, its last (or first) frame is held, as other editors
  do.
- The file format goes to version 2 (`entry_transition`, `exit_transition` on clips,
  `entry_animation`, `exit_animation` on texts), with a migration from version 1.
- The renderers need no new shader: a transition only changes a layer's opacity,
  position and scale, and type-on is drawn by the text rasterizer.
