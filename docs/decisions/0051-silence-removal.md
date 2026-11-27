# 0051. Silence removal: silences shortened to a pause, cut by one command on the clip's track

- Status: Accepted
- Date: 2026-11-27

## Context

Phase 8's first feature in the specification is the automatic removal of silences,
by measuring the volume, without AI. Phase 5 already finds silences
(`kadr_vision::silence_detection`, `kadr_engine::silence_analysis`, Claude's
`detect_silences`); removing them was left to Claude as a recipe: split at both ends
of each silence, remove the middle with `close_gap`, from the last silence to the
first. A ten-minute interview has a hundred silences: three hundred tool calls, three
hundred undo steps, and many chances to get a time wrong.

Four questions had to be answered: what a removal leaves of each silence, where
cuts land, what moves, and how it is undone.

## Options considered

1. **Remove each silence entirely.** Speech then runs words together and sounds
   rushed, and a silence whose edges are measured a few milliseconds late eats the
   start of the next word.
2. **Shorten each silence to a short pause**, half of it kept next to the sound on
   each side (Descript's "shorten word gaps", the padding of auto-editors).
3. **A loop of existing commands** (`SplitClip`, `RemoveClip`, `MoveClip`) in one
   transaction: one undo step, but quadratic moves (every later piece moves once
   per silence before it), a long history entry, and new identifiers for pieces
   that are thrown away.
4. **One command, `RemoveClipParts`**, that replaces the clip by its kept pieces end
   to end and moves the later clips of its track once.
5. **Close the gaps on every track** (a true ripple of the whole timeline), or
   **only on the clip's track** (what `remove_clip` with `close_gap` already does).

## Decision

- Each silence is **shortened to a pause** (option 2), 0.2 s by default, half on
  each side; a silence at the clip's start or end keeps only the half next to the
  sound. Silences shorter than the pause are left alone.
- Cut edges are moved onto **frame starts**, inwards (a cut starts at the next frame
  start and ends at the previous one): every piece lasts whole frames, and rounding
  only ever keeps a little more sound, never less.
- The cut is **one command**, `RemoveClipParts` (option 4), so **one undo step**
  however many silences. The first piece keeps the clip's identifier (selection,
  inspector and Claude's references stay on it) and its entry transition; the last
  keeps the exit transition; keyframes stay on their moments of the content.
- Only the clip's **own track** closes up (option 5, second part): later clips of that
  track move earlier by the time removed; other tracks do not move.
- Finding the cuts (`plan_silence_removal`) and making them are separate, so the
  interface shows what will go before anything changes, and Claude's
  `remove_silences` reports the cuts it made.

## Consequences

- Claude removes every silence of a clip in one call (`remove_silences`); the app
  has a dialog with three settings and a strip of what will go; the CLI has
  `remove-silences`.
- Titles or music placed over the clip on other tracks keep their times: after a
  removal they may no longer line up with the speech. Moving them would mean
  guessing what they belong to; clips have no links between tracks yet. The tool
  tells Claude to check them, and the dialog says other tracks do not move.
- Breath sounds, clicks and room noise louder than the threshold count as sound:
  the threshold is the user's control (−50 keeps quieter sounds, −30 also cuts
  background noise).
- `RemoveClipParts` is generic: any future "cut these ranges out" (filler words,
  bad takes) can reuse it.
