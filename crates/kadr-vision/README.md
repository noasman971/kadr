# kadr-vision

Classic computer vision, **without AI**: region tracking for the face blur
(correlation filters, optical flow, template matching, smoothing), the camera's
motion for video stabilisation (Shi and Tomasi points, optical flow, a robust
least-squares similarity, a histogram cut check) and silence detection.

- **Depends on:** nothing inside Kadr (works on plain image and audio buffers).
- **Used by:** `kadr-engine`.
- **Status:** region tracking since phase 4; silence detection; camera motion for
  stabilisation in phase 8d.

See [docs/decisions/0003-face-blur-by-classic-tracking.md](../../docs/decisions/0003-face-blur-by-classic-tracking.md).
