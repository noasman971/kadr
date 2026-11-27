# Transitions and text animations

How videos and images enter and leave the picture, and how texts appear and
disappear. Code: `kadr-core` (`transition.rs`, `text_animation.rs`, the
`SetClipTransition` and `SetTextAnimation` commands), `kadr-render`
(`transitions.rs`, `text.rs`), `kadr-engine` (`export/clip_layers.rs`). Why it works
this way: [decision record 0050](decisions/0050-transitions-at-clip-edges.md).

## Transitions belong to clip edges

Every video or image clip has an optional **entry transition**, played over its
first moments, and an optional **exit transition**, played over its last moments.
They stay inside the clip's own time range: adding one never moves a clip nor
changes the length of the timeline.

What a transition does depends on what touches that edge **on the same track**:

| Touching the edge | Entry transition | Exit transition |
|---|---|---|
| Nothing | The clip appears over what is below it (lower tracks or the background colour): a fade from black, a slide in… | The clip disappears the same way |
| Another clip | That clip keeps showing **underneath** while this one comes in: a fade becomes a dissolve, a slide pushes it out | The next clip shows underneath while this one leaves |

So "a dissolve from clip 4 to clip 7" is an entry fade on clip 7 (or an exit fade on
clip 4).

### The styles

| Style | Entering picture | Leaving picture | Touching clip |
|---|---|---|---|
| `fade` | Fades in | Fades out | Stays put: a dissolve |
| `slide` (`left`, `right`, `up`, `down`) | Comes in from one canvas away, moving that way | Goes out one canvas away, same way | Pushed along: the two pictures stay edge to edge |
| `zoom` `in` | Grows from half size while fading in | Grows to twice its size while fading out | Grows too, without fading |
| `zoom` `out` | Shrinks from twice its size while fading in | Shrinks to half size while fading out | Shrinks too, without fading |

Positions and sizes follow an ease-in-out curve; opacity changes linearly. A
transition multiplies the clip's own (possibly keyframed) opacity and scale and adds
to its position, so it combines with any animation.

### The touching clip continues past its cut

During the transition, the touching clip is drawn as it looks at its own edge (its
keyframes held there), but its **picture keeps playing**: after the end of a clip,
the source file continues beyond the trimmed range; before the start of a clip, the
file is read from a little earlier. Professional editors call this spare material
"handles". Where the file has no more pictures, its last (or first) frame is held.
Tracked blurs still apply there, using the source time.

### Durations

- A transition lasts more than zero and at most 60 seconds.
- If a clip is shorter than its two transitions together, both are **shortened in
  proportion** when drawn, so that trimming or splitting a clip is never refused
  because of its transitions.
- Splitting a clip keeps the entry transition on the first part and the exit
  transition on the second part.
- Sound is not changed by transitions; volume keyframes fade it.

## Text animations

Text clips take **text animations** instead of transitions, with the same entry and
exit timing rules. They act on the letters themselves and move by a distance that
follows the font size.

| Style | Entry | Exit |
|---|---|---|
| `fade` | Fades in | Fades out |
| `slide` (`left`, `right`, `up`, `down`) | Glides into place over 1.5 font sizes while fading in | Glides on the same way while fading out |
| `pop` | Grows from nothing, overshoots by about 10 % and settles | Shrinks away the same way, reversed |
| `type_on` | Letters appear one after another, as if typed | Letters are erased from the last one |
| `word_by_word` | Words appear one after another | Words disappear from the last one |

The text is always laid out whole, so letters already visible never move while
others appear. The letter (or word) being revealed fades in rather than popping.
Line breaks start a new word.

## Commands

| Command | Does |
|---|---|
| `SetClipTransition::new(clip_id, ClipEdge::Start or ClipEdge::End, Some(transition))` | Sets or replaces a transition (`None` removes it) |
| `SetTextAnimation::new(clip_id, ClipEdge::Start or ClipEdge::End, Some(animation))` | Sets or replaces a text animation (`None` removes it) |

Both are undoable like every command. Validation refuses transitions on text clips
(use a text animation) and on sound-only clips, and durations of zero or over 60 s.

## In the file

Format version 2 (version 1 files are upgraded when opened):

```json
"entry_transition": { "style": { "type": "slide", "direction": "left" }, "duration": 282240000 },
"exit_transition": null,
```

and, inside a text's content:

```json
"entry_animation": { "style": { "type": "type_on" }, "duration": 564480000 },
"exit_animation": null
```

Durations are in ticks (705 600 000 per second).

## How it is drawn

1. For each video track, `kadr-engine` finds the clip under the playhead and, during
   one of its transitions, the clip touching that edge (`plan_track_layers`).
2. `kadr-render` turns where each clip is in its entry or exit into a `LayerMotion`
   (opacity, offset, scale) — `transition_motion` and `text_animation_frame` — and
   moves the layer's placement with it.
3. Type-on and word-by-word texts are rasterised with only part of their letters
   (`TextRasterizer::rasterize_revealed`); the frame composer re-rasterises them
   only while the reveal changes.
4. Both renderers draw the moved layers like any others: no shader is specific to
   transitions, and `gpu_matches_cpu.rs` checks dissolves, pushes, zooms and
   animated texts on both.

Exports, the desktop app's preview and Claude's frames (`get_frame`) all go through
the same frame composer, so they show transitions and text animations the same way.

## Trying it from the command line

```powershell
.\target\release\kadr-cli assemble --output demo.kadr --transition fade --title "Hello" --title-animation type-on clip1.mp4 clip2.mp4
```

```powershell
.\target\release\kadr-cli render demo.kadr demo.mp4
```

`--transition` puts the same entry transition on every video and image of the bottom
track (`fade`, `slide-left`, `slide-right`, `slide-up`, `slide-down`, `zoom-in`,
`zoom-out`; `--transition-seconds`, 0.5 by default). `--title` adds a title on a new
top track (`--title-seconds`, 3 by default) that appears and disappears with
`--title-animation` (`fade`, `slide-up`, `pop`, `type-on`, `word-by-word`;
`--title-animation-seconds`, 0.8 by default).

## Known limits

- Only one transition plays on a track at a time: an exit and an entry at the same
  cut play one after the other (which makes a "dip to background").
- The transitioning clip is always drawn above the touching one.
- Custom curves inside transitions are future work. A clip's own keyframes, which
  transitions combine with, have curves and an editor in the app since phase 8e
  (decision record 0054).
- Claude's `add_transition` tool still makes fades from opacity keyframes, and the
  desktop app's inspector has no transition controls yet: for now, transitions are
  set from the command line or the `kadr-core` commands.
