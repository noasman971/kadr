# 0003. Face blur by classic region tracking, without AI

- Status: Accepted
- Date: 2026-11-25

## Context

Kadr's signature feature is blurring one chosen face through a whole clip. The common
approach uses neural networks for face detection and recognition, but the project
owner wants **no AI** for this feature: deterministic, explainable algorithms only.
Many popular pre-trained face models also have non-commercial licenses.

## Options considered

1. **Neural face detection + recognition** (ONNX models). Finds faces automatically and
   re-identifies a person after a long absence, but needs model files, has licensing
   risks, and is not what the owner wants.
2. **User selection + classic region tracking.** The user draws a box around the face
   (or clicks, and a skin-colour region-growing step proposes a box). A correlation
   filter tracker follows the region frame after frame.

## Decision

Option 2, implemented in `kadr-vision`:

- **Correlation filter tracker**: MOSSE first, then KCF if quality requires it,
  computed in the Fourier domain.
- **Scale estimation** with a small pyramid around the current size.
- **Confidence** from the peak-to-sidelobe ratio (PSR): low → "uncertain", long low →
  "lost" and the model stops updating to avoid drifting.
- **Recovery** by normalised cross-correlation template matching around the last
  position.
- **Pyramidal Lucas-Kanade optical flow** to predict fast motion.
- **One Euro filter** smoothing, and a wider margin when in doubt: blurring too much is
  better than revealing a face.
- Tracking runs **forwards and backwards** from the selected frame.

The same tool blurs anything: licence plates, logos, screens.

## Consequences

- No model file to download, no model license to track.
- Kadr cannot find a face by itself after a long disappearance: the user (or Claude,
  looking at a frame through MCP) selects it again. The interface must therefore make
  uncertain and lost ranges very visible on the timeline, and correcting them fast.
- Tracking is fast enough to analyse a minute of 1080p well under a minute on a laptop
  (to be measured in phase 4).
