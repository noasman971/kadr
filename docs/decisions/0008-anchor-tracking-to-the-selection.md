# 0008. A slow-learning filter, anchored to the selection's appearance

- Status: Accepted
- Date: 2026-11-26

## Context

Phase 4b measured the tracker on a real, encoded video for the first time
(`scripts/generate-tracking-benchmark.ps1`): a 200 × 240 test pattern moving for 60
seconds over a detailed, still background, in 1080p, on a known path. Every synthetic
scenario passed with errors under a pixel, yet on this video:

- every frame was "confident", with PSRs of 15 to 500;
- the region slid off the pattern by 2 to 4 pixels per second (mean centre error 197
  px, 880 px at worst), always in the same direction, and ended on the background;
- its size drifted by 24 % on average (55 % at worst).

A high PSR only says that the correlation filter found what it learnt. The filter
learns every confident frame, so a small error of position becomes the next frame's
centre, and nothing pulls it back. Median Flow ([0007](0007-scale-from-point-tracking.md))
has the same weakness over a minute: each frame's size is measured from the previous
one.

## Options considered

Each was measured on the benchmark (mean centre error; size error).

1. **Other window settings.** A context of 1.5 region sizes instead of 2: 160 px;
   giving the background a weight of 0.2 in the window: 154 px. They do not touch the
   cause.
2. **Not learning at all** (rate 0): the pattern changes (a clock, moving shapes), so
   the filter stops recognising it: lost on 1 084 frames of 1 800.
3. **Learning slowly.** 0.05: 17 px; 0.035: 8 px; **0.02: 4.4 px**; 0.01: 5.8 px with
   some lost frames. KCF and DSST use 0.02 to 0.025. The drift is much slower, but
   still there (about 0.2 px per second with the right size), and the size still
   drifts by 17 %.
4. **Stricter Median Flow** (pairs of points must agree within 0.5 % instead of 1 %):
   size error 2.6 %, but it is still a measure relative to the previous frame.
5. **Anchoring to the selection.** The user's selection is the one reference that
   cannot drift. On every confident frame, its picture (the same coarse template
   already used to find a lost region) is searched a few cells around the filter's
   answer, at three sizes, and refined between cells. Where it matches clearly, the
   region is pulled part of the way towards it. With learning at 0.02: **2.05 px**
   (max 8.3 px), size error **1.7 %** (max 4.9 %). Even with the old rate of 0.125 the
   anchor alone brings the error to 5.6 px.
6. **Drift detection alone**, as in TLD (Kalal et al. 2012): when the region no longer
   looks like the selection although the PSR is high, report it and search for the
   selection further away. On the drifting tracker it flagged 95 % of the frames that
   were more than half a region off, and none of those within a quarter; re-anchoring
   brought the mean error from 197 to 44 px. It catches large drifts, not small ones.

## Decision

Options 3, 5 and 6 together:

- the filter learns at **0.02**;
- every confident frame is **anchored**: the selection's picture is searched up to 3
  cells around the answer at sizes × 1/1.04, 1 and 1.04; where the similarity is at
  least 0.7, the region moves 35 % of the way towards it (position and size);
- below 0.5, the frame is **uncertain** (the filter may have drifted onto something
  else) and the selection is searched up to 3 region sizes away; found at 0.85 or
  more, the region jumps there and the filter is trained afresh;
- between 0.5 and 0.7 (a changed appearance, a head turning), the filter's answer
  stands.

## Consequences

- On the benchmark: mean centre error 197 → 2 px, size error 24 % → 1.7 %; the
  synthetic scenarios improve too (mean errors 0.1–0.3 px instead of 0.1–2 px).
- The anchor costs 0.05 ms per frame (3 sizes × 49 offsets of a 16-cell template).
  The wider search costs about 2 ms, and only runs on suspect frames.
- The anchor only helps while the object still looks like its selection. A face that
  turns away for long is followed by the filter alone, which drifts slowly at 0.02;
  when it turns back, the anchor pulls the region back if it is still within about a
  fifth of its size, and the wide search finds it if it matches at 0.85 or more.
- Frames that do not look like the selection at all are now reported as uncertain,
  even when the filter is sure: on real faces some head turns may show up as ranges to
  check. That is the honest answer; the thresholds should be checked on real faces.
- Two very similar objects within 3 region sizes (two faces) could make a re-anchoring
  jump to the wrong one when the tracked one turns away; the 0.85 threshold makes it
  unlikely, and the frame is reported as uncertain.
