# 0054. Keyframe curves: named eases and a bounded cubic Bézier, edited at the playhead

- Status: Accepted
- Date: 2026-11-27

## Context

Since phase 1, any of a clip's six animatable values (volume, opacity, position X and
Y, scale, rotation) can hold keyframes, with three curves between them: linear,
ease-in-out (smoothstep) and hold. Claude's `add_transition` made fades from them, but
the app only showed *"Animated (3 keyframes)"* and could not change them. The
specification's phase 8 asks for "editable keyframes in the interface (curves)".

Five questions had to be answered:

- which curves the model offers, and how a curve is described;
- how far a curve may bend: a value that overshoots could leave its allowed range
  (an opacity above 1, a scale below 0.01) between two valid keyframes;
- how the preview and the export stay identical;
- which gestures the app offers, and which requests and commands they become;
- what Claude's tools offer.

## Options considered

1. **Curves.**
   - a. Named presets only (linear, ease in, ease out, ease in-out, hold).
   - b. Presets, plus a cubic Bézier timing curve with two handles, as CSS
     `cubic-bezier(x1, y1, x2, y2)`, After Effects and Resolve do.
   - c. Bézier handles on the value graph itself (tangents in value units), as
     After Effects' graph editor: more powerful, much harder to show in a 300-pixel
     inspector and to explain to Claude.
2. **Overshoot.**
   - a. Let handles go past 0 and 1 (CSS allows `y` outside), and clamp the value to
     its property's range when drawing.
   - b. Keep every handle coordinate from 0 to 1. Then x never goes back (the curve is
     a function of time) and, by the convex hull property, y stays from 0 to 1: the
     value stays between its two keyframes, so the existing rule on keyframe values
     covers every moment.
3. **Ease in and ease out formulas**: CSS's (`0.42, 0, 1, 1`), or the quadratic `p²`
   and `1 − (1 − p)²`, which belong to the same family as the existing smoothstep:
   all three are cubic Bézier curves with handles at a third and two thirds of the
   time.
4. **Moving a keyframe**: a remove and an add in one transaction, or a
   `MoveKeyframe` command.
5. **Changing a value in the app**: a separate value field per keyframe, or the
   setting's own field, which shows and sets the value **at the playhead**.

## Decision

- **Model** (1b, 2b, 3 quadratic): `Interpolation` gains `EaseIn`, `EaseOut` and
  `CubicBezier(BezierCurve { start_handle, end_handle })`, each `BezierHandle` a
  `time_fraction` and a `progress_fraction`.
  - The rule (in `validation.rs`): every handle coordinate finite, from 0 to 1.
  - Saved as `"ease_in"`, `"ease_out"` and
    `{"cubic_bezier": {"start_handle": {…}, "end_handle": {…}}}`. Linear, ease-in-out
    and hold keep their names and shapes.
  - **File format 4**, with a migration that changes nothing: an older Kadr then
    says "newer version" instead of "damaged file". The format 3 snapshot is kept
    as `crates/kadr-core/tests/fixtures/vertical_interview_format_3.kadr`.
  - `Interpolation::as_bezier_curve` gives each preset's exact Bézier shape
    (handles at ⅓ and ⅔), so the curve editor starts from it without changing
    anything.
- **Solving a Bézier curve**: for the time fraction p, find the parameter t where
  x(t) = p, then return y(t).
  - Newton's method first (8 steps at most), then bisection (64 steps) if it does
    not settle, to within 10⁻⁹. With x controls from 0 to 1, x never decreases, so
    bisection always finds it.
  - All in `f64`. Everything that draws a frame or mixes sound calls
    `AnimatedValue::value_at`: the preview frames come from the same engine as the
    export, so both are identical.
  - The front end repeats the arithmetic (`src/lib/keyframes/keyframes.ts`) only
    to show the number in the inspector at the playhead and to draw curves. Its
    tests use the same numbers as the engine's.
- **Commands** (4): `MoveKeyframe` moves one keyframe, keeping its value and curve.
  It may pass other keyframes, not land on one (`KeyframeAlreadyThere`).
  `RemoveKeyframe::keeping_the_value_when_last()` keeps the last keyframe's value
  instead of going back to the value from before the animation: what the person
  sees does not jump. Both are tested with `apply_and_check_revert`.
- **App** (5): editing happens at the playhead.
  - The setting's diamond adds a keyframe there, holding the current value, or
    removes the one there.
  - Typing, dragging or stepping an animated value sets the keyframe at the
    playhead, adding one if needed.
  - A lane under each animated setting shows its keyframes. A click moves the
    playhead there; a drag moves the keyframe, snapped to frames.
  - The **Curve** menu and, for a custom curve, a small editor with two handles
    change the curve of the stretch the playhead is in.
  - Diamonds on the timeline clip stand for every keyframe at that moment: a click
    goes there, a drag moves them all.
  - Requests carry only what changes (`addKeyframe`, `moveKeyframe`,
    `setKeyframeCurve`, `removeKeyframe`, `setClipValue` with the playhead). The
    back end reads everything else from the project when it applies them.
- **Claude**: one tool, `set_keyframes`, replaces one property's keyframes with a
  list.
  - Each keyframe has a time from the clip's start, rounded to the nearest frame, a
    value, and a curve (`linear`, `ease_in`, `ease_out`, `ease_in_out`, `hold`, or
    `bezier` with `bezier_handles`).
  - Positions are in canvas pixels from the top-left corner, as in the other tools.
  - Moving or changing a keyframe means sending the list again. An empty list
    removes the animation and keeps the last value, as in the app.
  - `get_clip` now shows positions in the same coordinates (it showed them from the
    centre under the name `center_x_in_pixels`), with each keyframe's `curve` and
    `bezier_handles`.

## Consequences

- Every curve stays between its two keyframes: no clamping anywhere, and no
  overshoot ("back") easing. A bouncing title stays the job of the text
  animations (`pop`), which have their own curve.
- Splitting or trimming a clip in the middle of an eased or Bézier stretch draws
  the same kind of curve again over each shorter part, so that stretch changes
  slightly, as ease-in-out already did. Cutting the Bézier exactly (de Casteljau)
  is possible later, but the cut curves' handles can leave the unit square, so
  they would need another rule.
- The front end and the engine both solve Bézier curves. Only the engine's result
  reaches pictures and sound; the front end's is only a number shown at the
  playhead, so a difference there would be cosmetic.
- Keyframes are identified by their moment, not by an identifier. A request built
  from a view that is behind (a keyframe moved meanwhile by Claude) is refused
  with "no keyframe at …" and changes nothing.
- Claude's `add_transition` still makes fades from opacity keyframes, which the
  person now sees and can change in the app.
