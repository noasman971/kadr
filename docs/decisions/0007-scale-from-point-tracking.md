# 0007. The region's size comes from point tracking (Median Flow)

- Status: Accepted
- Date: 2026-11-26

## Context

A face coming closer to the camera grows; one moving away shrinks. The blur must
follow, or it leaves part of the face visible. [0003](0003-face-blur-by-classic-tracking.md)
planned "scale estimation with a small pyramid around the current size", and the
specification suggests trying 3 to 5 scales around the current size with the
correlation filter.

Phase 4a measured each option on synthetic scenes, with a textured object growing
from 40 to 72 pixels or shrinking from 64 to 38 pixels over 80 frames.

## Options considered

1. **MOSSE at three scales** (1/1.04, 1, 1.04), keeping the scale with the highest
   peak, with a small penalty for changing. Final size: 52.6 for 72 (−27 %), 64 for
   38.4 (no shrinking at all). Without the penalty, with a parabola between the three
   peaks: 64 for 72, 47 for 38.4. The filter learns the region at the slightly wrong
   size every frame, and that size becomes the new reference: the error accumulates.
2. **DSST** (Danelljan et al. 2014): a separate one-dimensional filter across 33 scales,
   learning slowly. On grey levels, even a pure 4 % zoom of the whole image was measured
   as 0.5–1 %. The cells of a grey-level window change too smoothly from one scale to the
   next: a nearly linear signal that shifts only changes its mean, which the
   normalisation removes. DSST relies on gradient histogram (HOG) features, which change
   sharply as edges cross cells.
3. **Median Flow** (Kalal et al. 2010): the points that Lucas-Kanade optical flow already
   follows give the change of size directly, as the median change of distance between
   pairs of points. Nothing is learnt from frame to frame. Final size: 70.7 for 72
   (−1.8 %) and 39.1 for 38.4 (+1.8 %).

## Decision

Option 3. The correlation filter finds the position; the optical flow points, which
the tracker follows anyway to predict fast motion, give the size. The change is limited
to 5 % per frame, and only applied on confident frames: on uncertain frames the points
may be following an obstacle.

## Consequences

- One mechanism serves two needs: the flow predicts fast motion and measures the size.
- The size is measured, not learnt: errors do not build up, they only add up randomly
  (under 2 % over 80 frames in the tests).
- The size needs texture inside the region for the points to be followed. On a uniform
  region the size stays as it is, which is safe with the blur's margin.
- If real videos show a need, DSST with gradient features remains possible later.
