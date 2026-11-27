# Phase 8d: video stabilisation — working plan

**This is a working document**, kept in the repository so the work can be resumed
by anyone. Before the pull request is merged, turn it into `docs/stabilization.md`
(the algorithm, step by step, with the benchmark numbers) and decision record 0053,
then delete this file.

Where the work stands is in the pull request's "Status for resuming" and in
[status.md](status.md). The design below was written before any code existed: where
the code disagrees, follow the code and the "Reports of the finished steps" at the
end, which list every deviation.

**Changed since the design was written:** phase 8e (keyframes) merged first and
took file format 4, so the stabilisation uses **file format 5** (done: migration
chained after 8e's, `vertical_interview_format_4.kadr` kept as a fixture). Phases
8f and 8g may merge before this branch too: check `CURRENT_FORMAT_VERSION` on
`origin/main` again before merging.

## Where the work stands (2026-11-27, 19:40)

Steps 1 to 8 are done, and so is the first half of step 9 (the composer draws
stabilised clips; all engine tests pass). The rest of step 9 and steps 10 to 14 are
left. Nothing is usable from the app, Claude or the command line yet.

### Benchmark results

`scripts/generate-stabilization-benchmark.ps1`, then the `stabilize_video` example
in release, then `scripts/compare-stabilization-with-truth.py`, on the laptop:

| Check | Limit | Measured | |
|---|---|---|---|
| 1. Step error, mean | 0.2 px | 0.17 px | ok |
| 1. Step error, 95th percentile | 0.5 px | 0.72 px | **over** |
| 2. Roll error, 95th percentile | 0.02° | 0.035° | **over** |
| 3. The cut is seen; other frames not measured | 3 | 0 | ok |
| 4. Shake left, RMS (centre and corners) | 0.4 px, 5 % | 0.79 px, 6.3 % | **over** |
| 5. Largest jump of the steadied picture | 1.5 px | 8.6 px (a corner, frame 636) | **over** |
| 6. Enlargement; frames partly corrected | 1.08; 0 | 1.047; 0 | ok |
| 7. Speed, decoding included | 60 frames/s on the desktop PC | 70 frames/s on the laptop | ok |

The camera shakes by 12.4 px RMS in this video. Where the shake that is left is
(high-passed, RMS):

| Part | Centre of the frame | A corner |
|---|---|---|
| Shot B (no moving object) | 0.44 px, largest jump 0.1 px | 0.47 px |
| Shot A | 0.51 px, largest jump 2.9 px (frame 636) | 0.75 px |
| First and last 20 frames of a shot | 0.1 px | |
| While the large object crosses (frames 590 to 700) | 0.56 px | |

So the centre of the picture is steady to about half a pixel, and what is left
there moves slowly (jumps of 0.1 px from one frame to the next). The corners are
worse in shot A: the roll is measured less well than the shift.

### What was found and fixed with the benchmark

- FFmpeg's crop filter rounds the window's corner to the nearest pixel; the truth
  first used `floor`, which alone gave 0.9 px of "error".
- The script's own high-pass counted a steady pan as shake at the ends of a shot;
  it now reflects the positions past the ends.
- The motion fit lost the camera while a large object crossed a side of the
  picture: 54 frames were not measured. Two first guesses are now tried (strict and
  tolerant), a second fit comes before the strict selection, and inside a scene
  the median shift is kept rather than nothing (`camera_motion.rs`, steps 3, 5
  and 6 of its documentation). No frame is left unmeasured apart from the cut.
- The smoothing reflected a run about its end *sample*, so the first and last half
  second of a run kept their shake. It reflects about a line fitted on the run's
  end now (`fitted_end_of` in `camera_path.rs`, step 3 of its documentation). The
  correction is therefore **no longer the identity at the ends of a run**: decision
  8 and the "Properties" list below are out of date on that point.

### Next things to do, in this order

1. **Roll accuracy** (checks 1, 2, 4 and 5 at the corners). The roll's 95th
   percentile is 0.035° and corners keep 0.75 px in shot A. Frame 636 is the worst:
   the median-shift fallback gives no roll there, so the corners jump by the roll
   of that frame. Ideas, each to measure on the benchmark: when the fallback is
   used, keep the previous frame's roll and size change rather than none; lower
   the score threshold that removes background points when a very contrasted
   object is in the frame (`MINIMUM_SCORE_FRACTION_OF_BEST`, 1 % of the best cell:
   only 67 to 76 of 144 cells gave a point while the test pattern was in view);
   more points per cell.
2. Decide with the owner whether the limits of checks 4 and 5 (0.4 px, 1.5 px,
   corners included) must be met exactly before the interface work, or whether
   half a pixel at the centre is good enough to go on: the owner should look at a
   real phone video first (`kadr-cli stabilize`, step 10).
3. The rest of step 9: `clip_geometry.rs` (`clip_source_placement`, for the
   desktop's blur overlay), the export warnings in `export/mod.rs`, and the tests
   listed under "kadr-engine" below (`stabilized_timeline_frames_barely_move`
   first). The tracking benchmark's CSV must also be checked byte-identical before
   and after step 1's refactor (`track_video` example): not done yet.
4. Steps 10 to 14 as designed below.

## The design

I checked the designs' claims in the code and edited no files. Paths are relative to `C:/Users/noahk/Desktop/Project/kadr`.

## Scores

Each criterion is scored 1–5, and higher is always better: for risk, 5 means low risk; for size, 5 means small.

| Design | Correctness | Quality | Fit | Risk | Size | Total | Why |
|---|---|---|---|---|---|---|---|
| 1, minimal | 4 | 3 | 5 | 5 | 5 | **22** | The math is careful and the correction is stateless, like `TrackedBlur::region_at`. But it is rigid only, uses grid points with no detector, has no cut check and no analysis margin, and leaves `visible_source` wrong. |
| 2, quality | 4 | 5 on paper | 2 | 2 | 1 | 14 | The ADMM corridor, RANSAC with a background heuristic, a Locked style and a crop-keyed cache are each a risk, and it needs a stateful `PreviewRenderer::clip_source_placement`. That does not fit, because the desktop calls the stateless `clip_source_placement` from `blur_overlay.rs::clip_overlay_at`. |
| 3, integration | 4 | 4 | 3 | 3 | 2 | 16 | It catches real problems: VFR frames, `has_finished_background_work`, a margin. But smoothing in timeline seconds and a crop-aware cached plan make the correction depend on the clip. Its media trait change collides with the proxies branch and still misses frames the nominal grid skips. |

**Foundation:** design 1. **Grafted in:**
- from designs 2 and 3: Shi–Tomasi points per cell, the similarity model, the cut check, the small-spread rule, the 1 s margin, `visible_source` as the corrected bounding box, the global progress store, and status reporting;
- my own: frames are decoded at their path sample's time.

**Checked against the code:**
- **Design claims confirmed:**
  - Every `LayerPlacement` literal except `FITTED` (`layers.rs:53`), `of_clip_at` (`:77`) and `moved_by` (`transitions.rs:87`) uses `..LayerPlacement::FITTED` or `..self`.
  - `texture_corners` (`layers.rs:293`) has only the GPU as caller (`gpu/mod.rs` `layer_uniforms`). The sampler is a single `ClampToEdge` linear one (`gpu/mod.rs:74-81`).
  - `maps_pixels_one_to_one` (`layers.rs:254`) gates both `pass_through_frame` (`renderer.rs:79`) and `draw_one_to_one` (`cpu_compositor.rs:184`).
  - `VideoFrameSource::frame_at` returns "the last frame starting at or before that time" (`kadr-media/src/backend.rs:61`), and `timestamp_of_frame` rounds up (`time.rs:401`).
  - serde is built with `derive` only (`Cargo.toml:57`).
  - `refine_motion` already rejects windows by the smallest eigenvalue (`optical_flow.rs:302`). The forward-backward loop is at `:125-148`, and `median` is private (`:331`).
  - `validate_effect` gets only `clip_has_picture` (`validation.rs:388`). The undo count is `22` (`editing_scenarios.rs:349`).
  - There is no `set_effect_enabled` tool, but `remove_effect` exists (`content_tools.rs:78`).
  - The "one LUT per clip" rule is enforced in tools (`refuse_second_color_lut`, `content_tools.rs:877`), not in validation.
  - `has_finished_background_work` checks only `blur_tasks` (`toolbox.rs:164`). The "Meanwhile" exemption is by name, `BLUR_STATUS_TOOL_NAME` (`toolbox.rs:182`).
- **Missed by all three designs: three phases run in parallel worktrees on `main`'s head** (`.claude/worktrees/`):
  - **8e keyframes** and **8g subtitles** both already set `CURRENT_FORMAT_VERSION = 4` and add `tests/fixtures/vertical_interview_format_3.kadr`. 8g also adds `ClipContent::Subtitle`.
  - **8f proxies** uses decision record **0055** and 8g uses **0056**.
  - 8f rewrites `FrameComposer::source_picture_of`: it reads from a proxy and scales blur regions to the proxy's size. It also changes `backend.rs` and `video_source.rs`.
- **VFR, design 3's worry, is real and larger than stated.**
  - The analysis decodes at `timestamp_of_frame(i)`, while the composer decodes at `source_time_continued_at(t)`. Whenever a real frame starts in `(T_i, t]`, they decode different frames. Phone VFR with a trim offset or a speed change hits this on about half the frames, and each miss shows as one frame of shake.
  - Decision 11 fixes it with no media change.
- **`visible_source` is used as a bound in source pixels.** `blurTool.svelte.ts:123` calls `movedRegion(…, overlay.visibleSource)`. Design 1's "keep the crop rectangle" would bound the boxes wrongly.
- **The desktop has no range-slider component.** The inspector uses only `NumberField`, so the sliders proposed by designs 1 and 2 are dropped.
- **Design 1's default σ = 1 s is costly.** A pan start needs about 0.4·v·σ px of margin, which is design 1's own figure of 1.14 zoom at 300 px/s. Halving σ halves it.
- **A broken string to avoid copying.** `content_tools.rs:884` (the second-LUT refusal) has 14 spaces inside its message. Fix it in passing, since commit 11 edits that file.

## Decisions

1. **The correction is stateless, computed from the stored raw path.**
   - The correction of a frame is a pure function of four inputs: the effect, the source time, the decoded picture size, and the stored zoom.
   - There is no cache, so `clip_source_placement` stays a free function.
   - *Rejected:* the composer caches of designs 2 and 3. They need invalidation on crop, speed and settings edits, and a stateful overlay API.
2. **The algorithms live in kadr-vision.** New `pub mod camera_motion` and `pub mod camera_path`, documented step by step with the papers.
   - kadr-core stores the data and checks the rules. kadr-engine converts between them.
   - *Rejected:* design 1's Gaussian and zoom fitting in core. Core holds data and rules (`TrackedBlur::region_at` is only an interpolation), and vision's crate rule asks for documented algorithms there.
3. **What is stored:**
   - the **cumulative** measured path, one sample per analysed frame, never simplified;
   - kept as `Arc<[CameraPathSample]>`, which needs serde's `rc` feature;
   - rounded at analysis to 0.01 px, 0.001° and 1e-5 of scale.

   *Rejected:*
   - per-frame motions (design 2): every lookup would have to sum from the run start, which forces a cache;
   - simplification as in `track_samples.rs`: the 1 px tolerance is the shake itself;
   - `Vec`: `ClipChangeRecord` keeps whole clips for undo, and `split_at` clones effects into both parts.
4. **Motion model: similarity** (x, y, roll, scale), both measured and corrected.
   - *Rejected:* rigid only (design 1). Zoom bob and walking stay uncorrected, and a rigid fit under a zoom pushes edge points out as outliers.
   - The fallback, if the benchmark shows scale noise, is one line: ρ = 1.
5. **Points:** in each cell of a 16 × 9 grid, the pixel with the best Shi–Tomasi score.
   - *Rejected:* design 1's fixed grid. Points on flat areas or edges are dropped or ill-posed, and fewer points mean more noise. Decision 0008's lesson: estimation noise becomes jitter.
6. **Fit:**
   1. a median seed;
   2. tolerant inliers;
   3. closed-form least-squares similarity;
   4. re-selection at 1 px;
   5. a refit;
   6. a translation-only fit when the inliers are bunched together (design 2's small-spread rule).

   *Rejected:* RANSAC (designs 2 and 3). It adds a seeded RNG, iteration counts and more code, and gains nothing under 50 % outliers. The background-ring heuristic is a later step, if the benchmark's foreground test fails.
7. **Cut check:** a frame is "not measured" when the fit fails **or** when the 32-bin luma histogram intersection is below 0.5.
   - *Rejected:* fit failure alone (design 1). A missed cut is the worst artefact, a huge correction around the cut, while a false break only weakens smoothing locally.
   - *Rejected:* the thumbnail difference of design 3. It needs a content-dependent threshold.
8. **Smoothing:**
   - a Gaussian over **source** seconds, run by run;
   - run ends extended by **point reflection**, and σ shrunk to run length / 6 for short runs;
   - default σ = **0.5 s**, range 0.1–10 s.

   With these rules, a straight pan stays exactly, and the correction is exactly the identity at every run end, so breaks never jump.

   *Rejected:*
   - Whittaker plus an ADMM corridor (design 2): solver risk, deferred;
   - timeline seconds (design 3): the correction would depend on speed;
   - box-filter approximation (design 3);
   - line reflection: not zero at breaks;
   - a default of σ = 1 s: about twice the zoom at pan starts.
9. **Zoom:**
   - one fixed zoom per effect, computed for the **full picture** over the whole stored path;
   - `zoom = min(1.2, largest per-frame need)`, computed by the engine and **stored** in the effect;
   - a per-frame fraction λ scales a correction down so that no edge ever shows. λ is below 1 only above the cap, or with a stale zoom.

   *Rejected:*
   - a crop-aware zoom (designs 2 and 3): it needs a cache keyed by crop. It is a later step;
   - the 95th percentile (design 2): it clips the shake peaks;
   - zoom changing over time: the picture breathes;
   - a user-set zoom in v1: an owner question.
10. **Analysis range:** the clip's source range plus **1 s on each side**, clamped to the file (design 3).
    - The clip's own edges then sit inside a run and are fully corrected. Transitions that read past a cut, and small later trims, stay analysed.
    - *Rejected:* the exact clip range (design 1). With it, the correction is zero on the clip's first and last frames.
11. **Frame identity:** a stabilised clip's frame is **decoded at its path sample's own time** inside the analysed range: `Stabilization::frame_time_for`. The correction lookup is a step function (the last sample at or before the time).
    - The analysis and the render then call `frame_at` with the same time, so they decode the same frame for any file, VFR included. The cost is a shift of under one frame for VFR files, and no change for constant-rate files.
    - *Rejected:* interpolating the path, which would move repeated frames.
    - *Rejected:* design 3's `shown_frame_start` trait method. It misses frames the nominal grid skips and collides with 8f's `backend.rs` and `video_source.rs`.
12. **Rendering:**
    - A semantic `SourceCorrection` (shift, rotation, scale about the picture centre) becomes a new `LayerPlacement` field.
    - It is folded into `LayerGeometry` as one window-to-source affine map.
    - The GPU warps `texture_corners`; there is no WGSL, uniform or sampler change.
    - The CPU's `source_point_for` returns the warped point, and the CPU clamp covers the whole picture.

    *Rejected:*
    - a raw `[f32; 6]` matrix in the placement (designs 2 and 3): it is harder to validate and to read in tests;
    - pre-warping the pictures, as `hide_regions` does: a second resampling and a CPU cost on the GPU path;
    - a `LayerMotion` field: it moves the whole layer, borders included.
13. **The crop is a window on the steadied picture.** The correction is centred on the picture's centre, never the crop's, so it does not depend on the crop.
14. **At most one stabilisation per clip**, as a rule in `validation.rs` (all three designs). Two stabilisations would compound, and the rule covers races between the user and Claude.
15. **No new commands.** `AddEffect`, `ReplaceEffect`, `SetEffectEnabled` and `RemoveEffect` already give undo, redo and new ids on a split.
16. **Claude's tools:** `stabilize_clip`, `get_stabilization_status` and `cancel_stabilization`.
    - `stabilize_clip` on an already stabilised clip re-tunes **instantly** unless `analyse_again` is set (design 1).
    - *Rejected:* a separate `change_stabilization` tool (design 2), which is one more tool in Claude's context.
    - *Rejected:* an `effect_id` parameter (design 3), redundant with one stabilisation per clip.
17. **Desktop:**
    - its own job slot, as all three designs say: `TrackingJobs` refuses with a blur-worded `TrackingAlreadyRunning`;
    - a global progress store (design 3), so progress survives a change of selection;
    - a `NumberField` for the smoothing and no slider;
    - no timeline badge and no TopBar pill: later steps.
18. **Error type:**
    - its own `StabilizationError`;
    - reuse of `TrackingProgress` and `TrackingCancellation`, which every host already handles;
    - `FrameReader` moved to the crate level.

    *Rejected:* nesting the module under `region_tracking` (design 1). A stabilisation API under `region_tracking::` misleads.
19. **File format:** version **4** with a no-op migration, renumbered at merge time if 8e or 8g merges first (see Data model).
20. **One decision record, 0053:** stored path, smoothing at render time, correction inside the window. 0055 and 0056 are taken by 8f and 8g; check that 8e has not taken 0053 before merging.

## Algorithm

### 1. Motion between two frames (`crates/kadr-vision/src/camera_motion.rs`, `pub mod`)

**Images.**
- Each frame is a `GreyImage` from the engine, shrunk with `shrink_factor_for_tracking`: 1080p becomes 640 × 360.
- `ImagePyramid::new(image, 16)` gives 5 levels at 640 × 360.
- The previous pyramid, its points and its histogram are kept, as `RegionTracker::track` keeps its previous frame.
- A frame of another size returns `CameraMotionError::FrameSizeChanged`.

**Points, detected on each frame and followed from it into the next:**
- **Grid:** 16 cells along the longer side and 9 along the shorter, inside a 16-px border. `GreyImage::sample` smears clamped edges.
- **Score:** in each cell, the pixel with the largest smallest eigenvalue of the 7 × 7 structure tensor, from central-difference gradients on level 0 (Shi and Tomasi 1994).
- **Kept when:** the score per window pixel is at least `MINIMUM_TEXTURE_PER_PIXEL` (0.5, `optical_flow.rs:79`) and at least 1 % of the frame's best cell.
- At most 144 points.

**Following.**
- Extract `pub(crate) fn follow_points_both_ways(previous: &ImagePyramid, current: &ImagePyramid, points: &[Point], expected_motion: (f32, f32), coarsest_level: usize) -> Vec<(Point, Point)>` verbatim from `optical_flow.rs:125-148`. `estimate_region_motion` calls it unchanged, and `median` becomes `pub(crate)`.
- Coarsest level `min(3, levels − 1)`.
- Expected motion: the previous measured shift in tracking pixels, or (0, 0) after an unmeasured pair.
- A point is kept when its round trip is within 1 px (the existing constant).

**Fit**, in tracking pixels about the tracking image's centre c:
1. Not measured if fewer than 8 points survive, or fewer than 25 % of the points tried.
2. Take the median shift m on each axis. The inliers I₀ are the points whose displacement is within 1 px + 0.02·|p − c| of m. That allows about 1° of roll and 2 % of scale per frame.
3. Fit the least-squares similarity on I₀ (Umeyama 1991) with p̃ = p − p̄ and q̃ = q − q̄:
   - α = Σ(p̃·q̃)/Σ|p̃|² and β = Σ(p̃×q̃)/Σ|p̃|²;
   - scale s = hypot(α, β) and angle θ = atan2(β, α), clockwise with y down;
   - shift = q̄ − sR(θ)p̄, re-expressed about c.
4. Take I₁ as the points within 1 px of that model, and refit.
5. **Small spread:** if the RMS distance of I₁ to its centroid is under 15 % of the image diagonal, fit a translation only: the median of I₁'s displacements, θ = 0, s = 1.
6. **Not measured** if |I₁| < 8, |I₁| < 40 % of the survivors, |θ| > 5°, |ln s| > ln 1.05, or the 32-bin luma histogram intersection Σmin(h₁, h₂)/N is below 0.5 (Swain and Ballard 1991).

**API:**
- `CameraMotionEstimator::new()`;
- `push_frame(&mut self, frame: GreyImage) -> Result<Option<SimilarityTransform>, CameraMotionError>`, which gives `None` on the first frame and on frames that cannot be measured;
- the result maps a content point p of the previous frame to c + s·R(θ)(p − c) + shift in the current one.

### 2. Pose (engine, using vision's `SimilarityTransform`)

- **Conversion:** multiply the motion's shift by the shrink factor k, then re-express it about the source centre C = (W/2, H/2), because shrinking drops leftover columns: t_C = t + (I − sR)(k·c − C).
- **Pose:** P_k = M_k ∘ P_{k−1}, an exact composition with P_first = identity, stored as (T_x, T_y, a, σ) about C: P(u) = C + σR(a)(u − C) + T.
- **Unmeasured motion:** P_k = P_{k−1}, with `motion_was_measured = false`.

### 3. Smoothing (`crates/kadr-vision/src/camera_path.rs`, `pub mod`, f64)

- **Channels:** T_x, T_y, a and ln σ are each smoothed on their own, inside a **run**. A sample with `motion_was_measured = false` starts a run, and the first sample always does.
- **Weights:** w = exp(−Δt²/2σ²) over source seconds, window ±3σ, normalised.
- **Run ends:** extended by point reflection about each end sample. The virtual sample at 2t_e − t_j has the value 2p_e − p_j, and the end sample is not reflected onto itself.
- **Short runs:** a run shorter than 6σ uses σ_run = length/6, so the two reflections never both reach a point.
- **Properties:**
  - lines are kept exactly;
  - the smoothed path S equals P at both run ends, so the correction there is exactly the identity;
  - near an end, only a slow drift of at most one shake amplitude remains;
  - the frequency response is exp(−(2πfσ)²/2): at σ = 0.5 s, 1 Hz is cut to 0.7 %, and 0.05 Hz keeps 99.4 %.
- **References:** Matsushita et al. 2006 (Gaussian motion smoothing); vid.stab for the fixed optimal zoom.

### 4. Correction, zoom, partial correction

**Correction.**
- K = S ∘ P⁻¹: ρ = σ_S/σ_P, Δa = a_S − a_P, e = T_S − ρR(Δa)T_P, so K(v) = C + ρR(Δa)(v − C) + e.

**Zoom a frame needs**, for the full picture W × H:
- With e′ = (1/ρ)R(−Δa)e, h_x = |cosΔa|W/2 + |sinΔa|H/2 and h_y = |sinΔa|W/2 + |cosΔa|H/2:
- need = max(1, h_x/(ρ(W/2 − |e′_x|)), h_y/(ρ(H/2 − |e′_y|))), or +∞ when a denominator is ≤ 0.

**Fitted zoom.** `fitted_zoom = min(1.2, max over all samples of need)`.
- It is stored in the effect, recomputed on every analysis and smoothing change, and rounded up to 0.1 %.
- It is independent of the clip's range, so a split or a trim never changes it.

**Partial correction.**
- K_λ = (λe, λΔa, ρ^λ).
- When need(K) > z, λ is the largest value in [0, 1] whose need is ≤ z, found by 16 bisection steps (λ = 0 always fits, since z ≥ 1).

**What the renderer gets:** F = Z ∘ K_λ, so `scale = z·ρ^λ`, `rotation = λΔa`, `shift = z·λ·e × (decoded width / analysed width)`. The last factor keeps 8f's proxies right.

**Outside the analysed range:** the end sample's correction is held. It is the identity, so only the zoom is applied, with no jump.

### 5. Parameters

| Constant | Value |
|---|---|
| Tracking image | `shrink_factor_for_tracking` (≤ 640 px wide) |
| Pyramid smallest side | 16 px |
| Grid | 16 cells along the longer side × 9 along the shorter; 16-px border |
| Shi–Tomasi window, thresholds | 7 × 7; ≥ 0.5 per pixel and ≥ 1 % of the frame's best |
| LK | 15 × 15 window, ≤ 10 steps, 0.03 px, coarsest level 3, forward-backward ≤ 1 px (existing) |
| Minimum followed | 8 points and 25 % of those tried |
| Median inlier tolerance | 1 px + 0.02 × distance to centre |
| Model inlier tolerance | 1 px; ≥ 8 inliers and ≥ 40 % of the survivors |
| Small spread | RMS < 15 % of the diagonal: translation only |
| Plausible step | \|roll\| ≤ 5°, scale within ±5 % per frame |
| Cut | histogram intersection < 0.5 (32 bins) |
| Smoothing σ | default 0.5 s, 0.1–10 s, window ±3σ |
| Zoom cap | 1.2; bisection 16 steps |
| Analysis margin | 1 s on each side |
| Stored rounding | 0.01 px, 0.001°, 1e-5 |

### 6. Expected cost

These are estimates, to be measured with the criterion bench `benches/camera_motion.rs`.

**Analysis, per 1080p frame:**

| Step | Cost |
|---|---|
| Decode | 1–2 ms |
| Shrink | 1.0 ms (measured) |
| Pyramid | 0.3 ms |
| Detector | about 1.5 ms |
| LK both ways, 144 points | about 8 ms (about 60 µs per point) |
| Fit and histogram | < 0.2 ms |
| **Total** | **12–13 ms, about 80 fps** |

**If too slow:**
1. Remove the per-call `Vec`s in `refine_motion` (`optical_flow.rs:282-295`). This changes no behaviour, and the tracking CSV must stay identical.
2. Then drop to 12 × 7 cells.

**At render time:**
- A lookup costs O(window): about 180 samples at σ = 0.5 s and 60 fps, under 10 µs.
- `fitted_zoom` costs O(N·window): under 1 ms per minute at the default, and about 1–2 s for an hour at 60 fps with σ = 10 s.
- A stabilised lone canvas-size video loses the pass-through, so it costs what any scaled layer costs.

## Data model (`crates/kadr-core/src/effect.rs`)

```rust
pub enum EffectKind {
    ColorAdjustment(ColorAdjustment),
    ColorLut(ColorLut),
    TrackedBlur(TrackedBlur),
    /// Steadies a shaky camera: its measured path, smoothed when the clip is drawn.
    Stabilization(Stabilization),
}
// display_name: "stabilisation"; needs_picture: true; JSON "type": "stabilization".
// Module doc order: tracked blurs on the source picture, then the stabilisation
// (inside the layer's window), then colour adjustments, then the LUT.

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stabilization {
    /// Standard deviation of the Gaussian smoothing the camera's path, in seconds of the source file.
    pub smoothing_in_seconds: f32,
    /// How much the steadied picture is enlarged so its moving edges stay outside the frame
    /// (1 = not enlarged, at most MAXIMUM_ZOOM_FACTOR). Derived from the path and the smoothing
    /// by `kadr_engine::stabilization`; the renderer never shows an edge even if it is stale.
    pub zoom_factor: f32,
    /// Size of the pictures the path was measured in (the file's picture size), in pixels.
    pub analysed_picture_size: PictureSize,
    /// The camera's measured path: one sample per analysed frame, strictly increasing
    /// source time, never empty. Shared, not copied, by splits and undo records.
    pub camera_path: Arc<[CameraPathSample]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraPathSample {
    /// Start of the analysed frame in the file: `frame_rate.timestamp_of_frame(i)`.
    pub source_time: Timestamp,
    /// Where the content seen in the path's first frame now is, about the picture centre:
    /// shift right (positive), in source pixels.
    pub shift_x_in_pixels: f32,
    /// Shift down (positive), in source pixels.
    pub shift_y_in_pixels: f32,
    /// Clockwise rotation (y down) since the path's first frame, in degrees.
    pub rotation_in_degrees: f32,
    /// Size relative to the path's first frame (1 = same size).
    pub scale_factor: f32,
    /// Whether the motion since the previous sample was measured. False on the first sample
    /// and after a cut or a frame that could not be followed: the smoothing restarts there.
    pub motion_was_measured: bool,
}

impl Stabilization {
    pub const DEFAULT_SMOOTHING_IN_SECONDS: f32 = 0.5;
    pub const MINIMUM_SMOOTHING_IN_SECONDS: f32 = 0.1;
    pub const MAXIMUM_SMOOTHING_IN_SECONDS: f32 = 10.0;
    pub const MAXIMUM_ZOOM_FACTOR: f32 = 1.2;
    /// The sample whose frame shows at `source_time`: the last at or before it (the
    /// decoder's rule, `partition_point(<=) - 1`), the first before the path. Never interpolated.
    pub fn sample_index_at(&self, source_time: Timestamp) -> usize;
    /// The moment to decode for `source_time`: that sample's own time inside the analysed
    /// range (first sample .. last sample + the last spacing), `source_time` outside.
    pub fn frame_time_for(&self, source_time: Timestamp) -> Timestamp;
    /// First and last sample times.
    pub fn analysed_range(&self) -> (Timestamp, Timestamp);
    /// Whether every frame from `source_start` to `source_end` (excluded) was analysed.
    pub fn covers(&self, source_start: Timestamp, source_end: Timestamp) -> bool;
    /// Stretches whose motion could not be measured (samples after the first with
    /// `motion_was_measured == false`), as `UnsureRange`s with `TrackingConfidence::Lost`.
    pub fn unmeasured_ranges(&self) -> Vec<UnsureRange>;
}
```

**Size.**
- About 160 bytes of JSON per frame, so about 0.3 MB per minute at 30 fps, and 32 bytes in memory.
- Splits, undo records and smoothing changes share one `Arc`. `PartialEq` compares contents.

**Dependency.** The workspace `Cargo.toml` gets `serde = { version = "1.0.229", features = ["derive", "rc"] }`. This adds a feature, not a dependency.

**File format.**
- `CURRENT_FORMAT_VERSION = 4`, with the table row "Stabilisation effects on clips (phase 8d)".
- `Migration { from_version: 3, upgrade_document: allow_stabilization }`, a no-op like `allow_color_luts`.
- Today's snapshot is kept as `tests/fixtures/vertical_interview_format_3.kadr`, with `version_3_files_are_upgraded_without_stabilization`.
- Update the hard-coded `3`s (`project_file.rs:234,287`).
- **If 8e or 8g merges first:** take the next free version, chain the migration after theirs, reuse their `format_3` fixture, and add the previous version's snapshot as the next fixture.

**Validation** (`validation.rs` only, form only):
- In `validate_effect`, for `EffectKind::Stabilization`:
  - refuse it unless the clip is `ClipContent::Media`, with the blur's message pattern. Images are refused by the request builder, not here;
  - then `find_stabilization_problem`, which checks:
    - `first_out_of_range` on smoothing (0.1, 10) and zoom (1, 1.2);
    - the analysed size is non-zero;
    - the path is non-empty, in strictly increasing `source_time`, and every value is finite;
    - `scale_factor` is in [0.25, 4].
- In `validate_clip`: **at most one stabilisation per clip**, reported as `InvalidEffect` on the second one.
- The zoom is never derived here.

**Commands:** none new. Each test uses `apply_and_check_revert`.

| Edit | What happens |
|---|---|
| Analysis | `AddEffect`; re-analysis: `ReplaceEffect` keeping the smoothing |
| Settings | `ReplaceEffect` with the zoom refitted |
| Before/after | `SetEffectEnabled` |
| Remove | `RemoveEffect` |
| Split | `split_at` shares the `Arc` and gives the second part a new effect id; the correction depends only on source time, so the seam is identical |
| Trim, `RemoveClipParts`, speed | Nothing changes |

**Editing scenario.** Add `edit_a_vertical_interview_before_phase_8d`, a hand-built 3-sample stabilisation step, an undo count of 23, and a reviewed snapshot diff. Update `docs/data-model.md`: the effects table, the rules and the version table.

## Engine

**`crates/kadr-engine/src/frame_reading.rs`** (`pub(crate)`, moved from `region_tracking/frame_reading.rs`):
- `FrameReader::open(backend, video_path: &Path, frame_rate, shrink_factor)`, `grey_frame` and `grey_image_of`;
- errors through a small `FrameReadingError { Media(MediaError), SizeChanged }`, with `From` into both error enums;
- region tracking's behaviour does not change.

**`crates/kadr-engine/src/stabilization/mod.rs`:**

```rust
pub struct StabilizationRequest { pub video_path: PathBuf, pub frame_rate: FrameRate,
    pub picture_size: PictureSize, pub source_start: Timestamp, pub source_end: Timestamp } // margin included
impl StabilizationRequest {
    pub const ANALYSIS_MARGIN: TimeSpan; // 1 s
    /// Same checks as `RegionTrackingRequest::for_clip` (exists, Media, video track,
    /// picture_size and frame_rate), range = clip ± margin clamped to [0, duration).
    pub fn for_clip(project: &Project, clip_id: ClipId) -> Result<Self, StabilizationError>;
}
#[derive(Debug, thiserror::Error)]
pub enum StabilizationError { NotAVideoClip { clip_id: ClipId }, VideoSizeChanged, Cancelled,
    Media(#[from] MediaError), Edit(#[from] EditError) }
pub struct CameraPathReport { pub camera_path: Vec<CameraPathSample>,
    pub analysed_picture_size: PictureSize, pub unmeasured_frame_count: usize, pub shrink_factor: usize }
impl CameraPathReport { pub fn stabilization_with(&self, smoothing_in_seconds: f32) -> Stabilization } // fits the zoom
pub fn analyse_camera_path(backend: &dyn MediaBackend, request: &StabilizationRequest,
    on_progress: &mut dyn FnMut(TrackingProgress), cancellation: &TrackingCancellation)
    -> Result<CameraPathReport, StabilizationError>;
```

How `analyse_camera_path` runs:
- one forward pass, opening the source on the calling thread (`VideoFrameSource` is not `Send`);
- frames `frame_index_at(start)` ..= `frame_count_covering(end) − 1`;
- for each frame: check cancellation, `grey_frame`, `push_frame`, compose the pose, round and push the sample, report progress;
- only one pyramid is kept.

**`crates/kadr-engine/src/stabilization/correction.rs`:**

```rust
pub fn stabilization_with_smoothing(stabilization: &Stabilization, smoothing_in_seconds: f32) -> Stabilization;
pub struct FrameCorrection { pub source_correction: SourceCorrection, pub fraction_applied: f32 }
/// Correction of the frame shown at `source_time`, in the pixels of a picture of `picture_size`.
pub fn stabilization_correction_at(stabilization: &Stabilization, source_time: Timestamp,
    picture_size: PictureSize) -> FrameCorrection;
pub struct StabilizationSummary { pub zoom_factor: f32, pub largest_shift_in_pixels: f32,
    pub largest_rotation_in_degrees: f32, pub partly_corrected_frame_count: usize,
    pub unmeasured_frame_count: usize }
pub fn stabilization_summary(stabilization: &Stabilization) -> StabilizationSummary;
pub(crate) fn applied_stabilization_of(clip: &Clip) -> Option<&Stabilization>; // first enabled
```

How `stabilization_correction_at` works:
- step lookup;
- a binary search for the ±3σ window;
- only that window is converted to vision `CameraPose`s;
- then `stabilizing_correction_at`, λ against the stored zoom, and F = Z ∘ K_λ with shifts scaled to `picture_size`.

`lib.rs` adds `pub mod stabilization` and re-exports the public items and `kadr_render::SourceCorrection`.

**Composer** (`export/frame_composer.rs`):
- `source_picture_of` computes `frame_time = applied_stabilization_of(clip).map_or(source_time, |s| s.frame_time_for(source_time))` after `held_inside_file`.
- It decodes **and** applies the blur (`hidden_regions_of(clip, frame_time)`) at `frame_time`, and returns `Option<(Arc<VideoFrame>, Timestamp)>`.
- In `compose_frame_at`, after `of_clip_at(...).moved_by(motion)` (lines 126-127): `placement.source_correction = stabilization_correction_at(s, frame_time, decoded picture size).source_correction`.
- The preview (`PreviewRenderer`) and `still_frames::timeline_pictures_at` inherit this.
- `still_frames::clip_pictures_at` stays **unstabilised**, since Claude's blur rectangles are in those pixels. Document it in its doc and in `get_frame`'s description.

**`clip_geometry.rs` `clip_source_placement`:** for a stabilised clip, it snaps `source_time` with `frame_time_for`, sets the same correction (using the asset's picture size) and returns the snapped `source_time`. The desktop's boxes and pointer mapping then follow the steadied picture.

**`export/mod.rs` `describe_unrendered_parts`:** a `Stabilization` arm with two warnings:
- "clip N shows frames outside its stabilisation's analysed part (from X s to Y s in the file): they keep no correction; analyse it again" (from `covers`);
- "clip N's shake is larger than a 20 % enlargement hides at K frames: they are only partly steadied" (from the summary).

**Example** `crates/kadr-engine/examples/stabilize_video.rs <video> <output.csv> [smoothing_in_seconds]`:
- columns `frame,time_in_seconds,motion_was_measured,path_x,path_y,path_rotation,path_scale,correction_x,correction_y,correction_rotation,correction_scale,fraction_applied,zoom`;
- it prints the frames per second.

## Rendering (`crates/kadr-render`)

**New type** (`src/layers.rs`, re-exported from `lib.rs`):

```rust
/// How the source picture moves inside its layer's window (video stabilisation): source
/// point v is drawn at C + scale_factor·R(rotation)·(v − C) + shift, C the picture's centre,
/// R clockwise with y down, in the decoded picture's pixels. Texts always get IDENTITY.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceCorrection { pub shift_x_in_pixels: f32, pub shift_y_in_pixels: f32,
    pub rotation_in_degrees: f32, pub scale_factor: f32 }
impl SourceCorrection { pub const IDENTITY: Self; pub fn is_identity(&self) -> bool }
pub struct LayerPlacement { /* existing fields */ pub source_correction: SourceCorrection }
```

`FITTED` and `of_clip_at` set `IDENTITY`. `moved_by` keeps the field through `..self`.

**`LayerGeometry`.** It gains `window_to_source: [f32; 6]` (SVG order), `picture_width: f32`, `picture_height: f32` and `has_source_correction: bool`. `new` computes, with s, r, t from the correction and C = (W/2, H/2):
- a = cos r/s, b = −sin r/s, c = sin r/s, d = cos r/s;
- e = C_x − a(C_x + t_x) − c(C_y + t_y);
- f = C_y − b(C_x + t_x) − d(C_y + t_y);
- the source point is then v = (a·w_x + c·w_y + e, b·w_x + d·w_y + f).

`new` also returns `None` for `scale_factor <= 0`.

**CPU** (`source_point_for`, `cpu_compositor.rs`):
- `source_point_for` computes the window point exactly as today, applies the inside-the-crop test **to the window point**, then returns `window_to_source(w)`.
- `VisiblePixels::of(&geometry)` clamps to the **whole picture** (0..W−1, 0..H−1) when `has_source_correction`, which matches the GPU's `ClampToEdge`. Without a correction nothing changes.

**GPU.**
- `texture_corners` maps the four crop corners through `window_to_source`, then divides by (W, H).
- The canvas-to-source map is affine, and w = 1 in `layers.wgsl`, so interpolation over the two triangles reproduces it exactly.
- `layer_uniforms`, `LAYER_UNIFORM_SIZE_IN_BYTES`, the WGSL and the sampler do not change. YUV chroma uses the same normalised coordinates.

**Shortcuts.** `maps_pixels_one_to_one` adds `&& placement.source_correction.is_identity()`. That turns off `pass_through_frame` and `draw_one_to_one`; it is the LUT's bug class. Add it to the CLAUDE.md pitfalls.

**Overlay mapping.**
- `source_to_canvas_mapping` composes today's window-to-canvas matrix with F (source to window).
- F in SVG order: (s cos r, s sin r, −s sin r, s cos r, C_x + t_x − (s cos r)C_x + (s sin r)C_y, C_y + t_y − (s sin r)C_x − (s cos r)C_y).
- `visible_source` becomes the bounding box of window_to_source applied to the four crop corners, clamped to [0, W] × [0, H].

**Order per pixel:**
1. decode, at the sample's time;
2. tracked blurs (`hide_regions`, raw source pixels), so they stay on the face;
3. the stabilisation correction and zoom;
4. crop window, fit, scale, rotation and position keyframes;
5. transition motion (canvas);
6. sampling;
7. colour adjustments, then the LUT;
8. opacity and blending.

Steps 3–5 are one affine map. Document this in `docs/rendering.md` (Effects, and a new "Order" paragraph).

## Surfaces

### Claude (kadr-tools and kadr-mcp)

**New `crates/kadr-tools/src/stabilization_tasks.rs`.** A deliberately simple copy of `BlurTasks`, which stays untouched:
- `StabilizationTasks`;
- `StabilizationTaskState::{Running { progress: TrackingProgress }, Analysed { report: Arc<CameraPathReport>, duration }, Applied { effect_id, summary: StabilizationSummary, duration }, Failed { reason }, Cancelled }`;
- `start(clip_id, project_file: PathBuf, request: StabilizationRequest, backend: Arc<dyn MediaBackend>) -> Result<String, ToolError>`;
- thread `kadr-stabilization`, ids `stabilize-N`, and one running task per clip.

It copies three guards:
- a late cancellation wins (`blur_tasks.rs:366`);
- cancelling drops a result not applied yet (`:231`);
- a result for a project that has been closed is refused (`blur_tools.rs:729`).

**New `tools/stabilization_tools.rs`**, registered in `tools/mod.rs` after `blur_tools`:

```rust
#[derive(Deserialize, JsonSchema)] #[serde(deny_unknown_fields)]
struct StabilizeClipParameters {
    /// The video clip to steady.
    clip_id: ClipId,
    /// How strongly the camera's motion is smoothed, in seconds (0.1 to 10). New stabilisations
    /// default to 0.5; on a stabilised clip, changes it instantly without a new analysis.
    smoothing_in_seconds: Option<f32>,
    /// true to measure the camera's motion again (after the clip was made longer); keeps the smoothing.
    analyse_again: Option<bool>,
}
struct StabilizationStatusParameters { task_id: Option<String>, clip_id: Option<ClipId> }
struct CancelStabilizationParameters { task_id: String }
```

**`stabilize_clip`** (`ChangesProject`) has two modes.

- **Clip already stabilised, `analyse_again` not set: instant.**
  - With `smoothing_in_seconds`: `ReplaceEffect` through `change_project`, using `stabilization_with_smoothing`. The answer gives the effect id, the smoothing, the zoom in percent, and `covers_whole_clip`.
  - Without it: refused, with a message naming the effect and both options.
- **Otherwise: analysis.** It checks everything first:
  - the open project;
  - `StabilizationRequest::for_clip`, which refuses texts, images and sound;
  - `ensure_project_media_readable`;
  - the smoothing range;
  - no task already running for the clip.

  It then starts the task. The answer gives the task id and the estimated seconds (frames ÷ 80), and names `get_stabilization_status`.

**`get_stabilization_status`** (`ReadsOnly`) answers with `message_with_details`. The details are:
- `task_id`, `clip_id`, `state` and `percent_done`;
- once applied: `effect_id`, `zoom_in_percent`, `largest_shift_in_pixels`, `largest_rotation_in_degrees`, `moments_not_stabilised: [{from_second, to_second}]` (timeline seconds) and `covers_whole_clip`;
- if it failed: `reason`.

Its text says to check two or three frames with `get_frame` **without** `clip_id`. If the picture floats or is enlarged too much, it suggests a smaller `smoothing_in_seconds`; if shake is left, a larger one.

**`cancel_stabilization`** (`ChangesProject`, like `cancel_blur`).

**`toolbox.rs` wiring:**
- a `stabilization_tasks` field in `ToolServices`, with `stabilization_tasks()` and `stabilization_tasks_mut()` accessors;
- `apply_finished_background_work` also calls `add_analysed_stabilizations_to_project`: `AddEffect`, or `ReplaceEffect` keeping the smoothing, labelled "Claude: stabilise clip N";
- `has_finished_background_work` checks both registries. `claude_link.rs:140` depends on it;
- the notice exemption becomes `STATUS_TOOL_NAMES: [&str; 2]` (blur and stabilisation status).

**`views.rs` `EffectView::of`** gets a `Stabilization { smoothing_in_seconds, zoom_in_percent, analysed_from_second, analysed_to_second, frames_not_followed }` arm, in file seconds.

**Other kadr-tools edits:**
- the `Stabilization(_)` arms in `blur_tools.rs`, `content_tools.rs` and `picture_tools.rs`;
- the `get_frame` description: clip pictures are shown before stabilisation;
- the `remove_effect` description;
- fix the string at `content_tools.rs:884`.

**kadr-mcp:**
- a `SERVER_INSTRUCTIONS` paragraph (`src/lib.rs:59`): stabilise, wait, check frames, re-tune instantly; blur rectangles stay in clip pictures;
- the three names in `REQUIRED_TOOLS` (`tests/tool_list.rs:14`);
- `docs/mcp-server.md`: the tables, an example and the Testing list.

### kadr-cli

**Command.** `Command::Stabilize`:

```text
kadr-cli stabilize shaky.mp4 -o steady.mp4 [--smoothing 0.5] [--save-project p.kadr] [--software-encoder] [--fast] [--cpu]
```

Fields: `video: PathBuf`, `#[arg(long, short)] output`, `#[arg(long = "smoothing", default_value_t = 0.5)] smoothing_in_seconds: f32`, `save_project`, `software_encoder`, `fast`, `cpu`.

**`src/stabilize.rs`**, following `blur.rs`:
1. refuse an output that would replace the input;
2. probe the file and assemble the project on an `even_size` canvas;
3. `analyse_camera_path` with `ProgressLine::update_tracking("Measuring the camera's motion", …)`;
4. `AddEffect` in an `EditSession`, with optional `--save-project`;
5. print a summary through `reports::describe_stabilization`;
6. `export_project`.

There is one `ctrlc::set_handler`. Add the dispatch arm and the header example in `main.rs`, plus parse tests.

### Desktop back end (`apps/desktop/src-tauri/src`)

**Job: `stabilization_job.rs`.**
- `StabilizationJobs { running: Arc<Mutex<Option<TrackingCancellation>>> }`, a slot of its own;
- `start(&self, backend, request: StabilizationRequest, report: impl Fn(StabilizationEvent) + Send + 'static, finish: impl FnOnce(CameraPathReport) -> StabilizationEvent + Send + 'static) -> CommandResult<()>` and `cancel`;
- progress throttled to 150 ms, as `PROGRESS_INTERVAL` does;
- `StabilizationEvent` uses `tag = "type"` in camelCase: `Progress { analysed_frame_count, total_frame_count }`, `Finished { view: Box<ProjectView>, clip_id, effect_id, is_new, zoom_factor, unmeasured_frame_count }`, `Failed { code, details }`, `Cancelled`.

**Results: `stabilization_results.rs`.** `add_analysis_to_project(open_project: &mut OpenProject, clip_id, report: &CameraPathReport) -> CommandResult<StabilizationEvent>`.
- It checks the project identity, as `commands/blur_commands.rs:116` does.
- Undo keys: `"stabilizeClip"` (add) and `"analyseStabilizationAgain"` (replace, keeping the smoothing).

**Commands: `commands/stabilization_commands.rs`.**
- `start_clip_stabilization(app, state, clip_id: ClipId, on_event: Channel<StabilizationEvent>) -> CommandResult<()>` and `cancel_clip_stabilization(state)`;
- registered in `lib.rs`, with `AppState.stabilization: StabilizationJobs`.

**Edit requests.** In `editing.rs` plus a new `editing/stabilization_settings.rs`, applied to the project as it is at that moment:

| Request | Undo key(s) |
|---|---|
| `SetStabilizationSettings { clip_id, changes: StabilizationChanges { smoothing_in_seconds: Option<f32> } }` (`deny_unknown_fields`) | `"changeStabilization"` |
| `SetStabilizationEnabled { clip_id, is_enabled }` | `"showStabilization"` / `"hideStabilization"` |
| `RemoveStabilization { clip_id }` | `"removeStabilization"` |

**View: `project_view.rs`.**
- `ClipView.stabilization: Option<StabilizationView { effect_id, is_enabled, smoothing_in_seconds, zoom_factor, covers_clip, unmeasured_ranges: Vec<UnsureRangeView> }>`. It never carries the path.
- `unsure_ranges_on_timeline` (`:553`) takes `ranges: &[UnsureRange]` instead of `&TrackedBlur`.
- Add the `Stabilization(_)` arms in `project_view.rs`, `editing/blur_settings.rs` and `editing/clip_settings.rs`.

**Error codes: `command_error.rs`.**
- New codes: `StabilizationClipUnsuitable`, `StabilizationAlreadyRunning`, `StabilizationFailed` and `StabilizationProjectClosed`, with `From<StabilizationError>`.
- Each needs `error.*` texts in both message files, or `errors.ts` crashes.

### Desktop front end (`apps/desktop/src/lib`)

**API:** `api/types.ts` mirrors the event, the view, the `EditRequest` variants (with `Partial` changes) and the error codes. `api/backend.ts` gets `startClipStabilization(clipId, onEvent)` and `cancelClipStabilization()`.

**Store:** `state/stabilization.svelte.ts` is a global store: `clipId`, `percentDone`, `secondsLeft`, `start`, `stop` and `handleEvent`. `handleEvent` calls `editor.acceptView(event.view)` and shows a toast with Undo in group `"stabilization"`.

**Section:** `components/editor/StabilizationInspector.svelte`, mounted inside `{#if clip.kind === "video"}` next to `BlurInspector` (`ClipInspector.svelte:327`).
- It keeps `clipId = untrack(() => clip.id)` and reads settings from `editor.view`. Do **not** copy `BlurInspector`, which reads its props inside handlers.
- What it shows in each state:

| State | Shown |
|---|---|
| Not stabilised | A hint ("Steadies a shaky camera; analysing takes about N s") and a **Stabilise** button |
| Analysing | "42 % · 8 s left" and **Stop** |
| Stabilised | A header with show/hide and remove; a Smoothing `NumberField` (unit "s", 0.1–10, commits on change); a read-only "Enlarged by 6 %"; **Analyse again** |
| Hints | When `covers_clip` is false; when there are unmeasured moments |

**Texts:** `stabilize.*`, `undo.<keys>` and `error.<codes>` in `i18n/messages.en.ts` and `messages.fr.ts`.

**Docs:** `docs/desktop-app.md` and `docs/ux.md`.

## Tests and benchmark

**kadr-vision**

*Refactor check:*
- `tracking_scenarios.rs` passes unchanged;
- `track_video` on the tracking benchmark gives a byte-identical CSV before and after commit 1.

*`camera_motion.rs` unit tests:*
- `corner_points_are_spread_and_avoid_the_border`
- `flat_cells_give_no_point`
- `the_similarity_fit_recovers_a_known_transform`
- `the_fit_ignores_a_third_of_outliers`
- `bunched_inliers_give_a_translation_only`

*Synthetic scene:* `synthetic_scenes.rs` gets `pub struct ShakenCameraFrame { width, height, background_seed, camera: SimilarityTransform, moving_object: Option<SceneObject>, noise_amplitude, noise_seed }` with `render()`, built on `textured_value`. `SimilarityTransform` is public in `geometry.rs`.

*`tests/camera_motion_scenarios.rs`* (the `tracking_scenarios.rs` style). Thresholds: shift error ≤ 0.05 tracking px, roll ≤ 0.01°, scale ≤ 5e-4.
- `a_pure_shift_is_measured`
- `a_clockwise_roll_is_measured_with_its_sign`
- `a_change_of_size_is_measured`
- `shift_roll_and_size_together`
- `a_moving_object_covering_a_third_is_ignored`
- `a_uniform_frame_is_not_measured`
- `a_cut_is_not_measured`
- `a_fast_pan_is_followed_through_the_prediction` (40 px per frame)
- `noise_does_not_bias_the_motion`

*`camera_path.rs` unit tests:*
- `composing_motions_gives_the_pose`
- `a_straight_pan_needs_no_correction`
- `a_shake_on_a_pan_is_removed`
- `the_correction_is_identity_at_both_ends_of_a_run`
- `a_run_break_never_makes_the_correction_jump`
- `short_runs_shrink_the_smoothing`
- `a_constant_offset_of_the_path_changes_nothing`
- `the_fitted_zoom_covers_every_frame_and_respects_the_cap`
- `a_partly_applied_correction_fits_inside_the_picture`

*Bench* `benches/camera_motion.rs`: `push_frame` at 640 × 360, `fitted_zoom` on 18 000 samples, and one lookup.

**kadr-core**
- *Effect lookups:* `stabilization_lookup_is_a_step` (exact at `timestamp_of_frame(i)`, and one tick before), `frame_time_snaps_only_inside_the_analysed_range`, `unmeasured_ranges_skip_the_first_sample`.
- *Validation:* `a_stabilization_on_a_title_is_refused`, `a_second_stabilization_is_refused`, `stabilization_settings_out_of_range_are_refused`, `an_empty_or_unsorted_camera_path_is_refused`, `non_finite_path_values_are_refused`.
- *Commands* (`effect_commands.rs`): `adding_replacing_hiding_and_removing_a_stabilization_revert_exactly`, using `apply_and_check_revert`.
- *Split* (`clip.rs`): `splitting_a_stabilized_clip_shares_its_camera_path` (`Arc::ptr_eq`, new id).
- *File format:* `version_3_files_are_upgraded_without_stabilization`.
- *Scenario:* the scenario step and the snapshot.

**kadr-render**
- *`layers.rs` unit tests:*
  - `an_identity_correction_changes_no_geometry`;
  - `interpolated_texture_corners_agree_with_source_point_for` (bilinear interpolation of the corners at pixel centres within 1e-3 px);
  - `a_correction_stops_the_one_to_one_shortcut`;
  - extend `the_public_mapping_agrees_with_the_renderers_geometry` with a correction and the bounding-box `visible_source`.
- *`cpu_compositor.rs`:* `a_stabilized_canvas_sized_video_is_not_passed_through`.
- *`tests/gpu_matches_cpu.rs`:*
  - `stabilized_video`: YUV checkerboard 1920 × 1080, shift (7.3, −4.1), 0.8°, scale 1.06;
  - `stabilized_cropped_scaled_and_rotated_video`;
  - `stabilized_gradient_with_a_lut_matches_strictly`: canvas-size RGBA, maximum difference ≤ 2.
  - Rerun with `--test-threads=1` if WARP crashes on the owner's machines.

**kadr-engine**

`tests/stabilization.rs` encodes a `ShakenCameraFrame` sequence to H.264, as `region_tracking.rs` does.
- `the_camera_path_follows_a_known_shake` (frame-to-frame error ≤ 0.3 source px)
- `the_analysis_covers_the_clip_and_a_second_around_it`
- `cancelling_stops_the_analysis`
- `images_and_titles_are_refused`
- `stabilized_timeline_frames_barely_move` (re-measured with `CameraMotionEstimator`, high-pass ≤ 0.5 px)
- `a_split_leaves_the_frames_on_both_sides_unchanged` (CPU, byte-identical to the unsplit clip)
- `half_speed_repeats_each_frames_correction`
- `frames_are_decoded_at_their_sample_time` (a clip starting half a frame off the grid)
- `clip_source_placement_matches_the_rendered_picture`
- `the_export_warns_about_frames_outside_the_analysed_part`
- `cpu_and_gpu_exports_of_a_stabilized_clip_agree` (PSNR ≥ 42 dB; skipped without a GPU)

**kadr-mcp** (`tests/stabilization.rs`, modelled on `tests/blur.rs`)
- `stabilize_clip_runs_in_the_background_and_adds_one_undo_step`
- `mistakes_are_refused_before_anything_runs` (title, image, unknown clip, smoothing out of range, second run for the same clip)
- `changing_the_smoothing_is_instant`
- `analyse_again_replaces_the_path_and_keeps_the_smoothing`
- `cancelling_a_stabilization_adds_nothing`
- `status_answers_have_no_background_notice`
- `finished_stabilizations_count_as_background_work`
- plus `tool_list.rs`

**kadr-cli:** `stabilize_parses_its_options` and `stabilize_uses_the_default_smoothing`.

**Desktop**
- *Rust:* `changing_the_smoothing_keeps_the_path_and_refits_the_zoom`, `hiding_and_removing_the_stabilization`, `a_result_for_another_project_is_refused`, `a_second_analysis_replaces_the_first_and_keeps_its_smoothing`, `the_clip_view_shows_the_stabilization_without_its_path`.
- *Front end:* `npm run check`, `npm test`, and a scripted check through the DevTools port.

**Benchmark** (local only; fixtures go to the git-ignored `tests/fixtures/generated/stabilization/`)

*`scripts/generate-stabilization-benchmark.ps1`* follows `generate-tracking-benchmark.ps1`.

1. **Stills:** two `mandelbrot=s=2560x1440` stills, with different `start_x`, taken with `-frames:v 1`.
2. **Moving objects, overlaid on the still before the camera** (outliers):
   - a 320 × 240 `testsrc2` box on x = 1100 + 600 sin(2πt/9), y = 600 + 250 sin(2πt/6);
   - a 640 × 720 box crossing the frame during 20–24 s.
3. **Camera roll:** `rotate=a='(0.5*sin(2*PI*1.3*t)+0.25*sin(2*PI*3.7*t+1))*PI/180':ow=iw:oh=ih:bilinear=1`.
4. **Camera path:** `crop=1920:1080:exact=1`, with:
   - x = 320 + 200 sin(2πt/20) + 10 sin(2π·2.3t) + 6 sin(2π·5.1t+1) + 3 sin(2π·9.7t+2);
   - y = 180 + 8 sin(2π·1.7t+0.5) + 5 sin(2π·4.3t) + 2 sin(2π·8.9t+1).
5. **Encoding:** `noise=alls=6:allf=t`, libx264 crf 18, yuv420p, 30 fps.
6. **Shots:** shot A lasts 40 s and shot B 10 s, on the other still, concatenated, so there is a hard cut at 40 s.

*`scripts/compare-stabilization-with-truth.py`* (stdlib only, like the tracking script):
- It rebuilds the truth from the same formulas: a clockwise rotation about (1280, 720), then the floored crop offsets, re-expressed about (960, 540).
- It reads the example's CSV.
- It measures at the frame centre and at the four corners.

**Acceptance.** Every line must hold at the default smoothing of 0.5 s, on the desktop PC, in release:

| # | Check | Threshold |
|---|---|---|
| 1 | Frame-to-frame motion error at 1080p (centre and corners) | mean ≤ 0.2 px; 95th percentile ≤ 0.5 px |
| 2 | Roll error | 95th percentile ≤ 0.02° |
| 3 | Frames not measured | the cut within ±1 frame, at most 3 others |
| 4 | Residual jitter: the truth composed with the applied correction, high-pass = minus its own 0.25 s Gaussian, at centre and corners | RMS ≤ 0.4 px, and ≤ 5 % of the input's |
| 5 | Largest frame-to-frame jump of that high-passed path, including the second around the cut and both ends | ≤ 1.5 px |
| 6 | Zoom | ≤ 1.08, and `fraction_applied` = 1 on every frame |
| 7 | Analysis speed, decode included | ≥ 60 fps (the laptop's figure is recorded, not gated) |
| 8 | End to end: `kadr-cli stabilize --cpu` against the GPU, `ffmpeg -i a -i b -lavfi psnr -f null -` | ≥ 42 dB |
| 9 | Re-analysing the CPU export with the example | high-passed jitter RMS ≤ 0.6 px |

Check 9 catches sign errors in the renderer. If the first run misses a line, investigate rather than loosen it, and record the numbers in `docs/stabilization.md`.

## Implementation steps

Each commit leaves CI green. Rough size: about 5 000 lines including tests.

1. **`refactor(vision): follow any list of points both ways`.** `crates/kadr-vision/src/optical_flow.rs`. Then check that the tracking CSV is identical.
2. **`feat(vision): measure the camera's motion between frames`.**
   - `src/geometry.rs` (`SimilarityTransform`), `src/camera_motion.rs` (new) and `src/lib.rs`;
   - `src/synthetic_scenes.rs` and `tests/camera_motion_scenarios.rs` (new);
   - `benches/camera_motion.rs` (new) and `Cargo.toml` (`[[bench]]` and the description);
   - `README.md` and the crate doc.
3. **`feat(vision): smooth a camera path and size the zoom`.** `src/camera_path.rs` (new), `src/lib.rs` and the bench.
4. **`feat(core): stabilisation effect (file format 4)`.**
   - Core: `effect.rs`, `validation.rs`, `project_file.rs`, `clip.rs` tests, `commands/effect_commands.rs` tests.
   - Core tests: `tests/fixtures/vertical_interview_format_3.kadr`, `tests/editing_scenarios.rs` and its `.snap`.
   - The workspace `Cargo.toml` (serde `rc`) and `docs/data-model.md`.
   - `Stabilization(_)` arms in every exhaustive match, or the workspace does not compile:
     - engine: `export/frame_composer.rs`, `color_grading.rs` and `export/mod.rs`;
     - tools: `blur_tools.rs`, `content_tools.rs`, `picture_tools.rs` and `views.rs`;
     - CLI: `finishing_touches.rs`;
     - desktop: `project_view.rs`, `editing/blur_settings.rs` and `editing/clip_settings.rs`;
     - and any others the compiler finds.
5. **`feat(render): move the source picture inside its layer's window`.** `kadr-render/src/layers.rs`, `cpu_compositor.rs`, `lib.rs`, `tests/gpu_matches_cpu.rs` and `docs/rendering.md`.
6. **`refactor(engine): share frame reading between analyses`.** `src/frame_reading.rs` (moved), `src/region_tracking/mod.rs` and `src/lib.rs`.
7. **`feat(engine): analyse a clip's camera path`.** `src/stabilization/mod.rs` and `correction.rs` (new), `src/lib.rs`, the first half of `tests/stabilization.rs`, and `examples/stabilize_video.rs`.
8. **`test(scripts): stabilisation benchmark`.** `scripts/generate-stabilization-benchmark.ps1` and `scripts/compare-stabilization-with-truth.py`, with the numbers in `docs/stabilization.md`. **This is the quality gate: checks 1–7 before any surface work.**
9. **`feat(engine): render stabilised clips`.** `export/frame_composer.rs`, `clip_geometry.rs`, `export/mod.rs`, the `still_frames.rs` doc, the rest of `tests/stabilization.rs`, and `docs/media-and-export.md`.
10. **`feat(cli): kadr-cli stabilize`.** `kadr-cli/src/arguments.rs`, `main.rs`, `stabilize.rs` (new) and `reports.rs`. Then run checks 8 and 9.
11. **`feat(tools): stabilise clips from Claude`.**
    - kadr-tools: `stabilization_tasks.rs` (new), `tools/stabilization_tools.rs` (new), `tools/mod.rs`, `toolbox.rs`, `views.rs`, `tools/picture_tools.rs`, `tools/content_tools.rs` and `lib.rs`;
    - kadr-mcp: `src/lib.rs`, `tests/stabilization.rs` (new) and `tests/tool_list.rs`;
    - `docs/mcp-server.md`.
12. **`feat(desktop): stabilisation in the back end`.** `stabilization_job.rs`, `stabilization_results.rs`, `commands/stabilization_commands.rs`, `commands/mod.rs`, `editing.rs`, `editing/stabilization_settings.rs`, `project_view.rs`, `command_error.rs`, `app_state.rs` and `lib.rs`.
13. **`feat(desktop): stabilisation section in the inspector`.** `api/types.ts`, `api/backend.ts`, `state/stabilization.svelte.ts`, `components/editor/StabilizationInspector.svelte`, `ClipInspector.svelte`, `i18n/messages.en.ts`, `i18n/messages.fr.ts`, `docs/desktop-app.md` and `docs/ux.md`.
14. **`docs: stabilisation`.**
    - `docs/decisions/0053-video-stabilization-from-a-measured-camera-path.md` and `docs/decisions/README.md`;
    - `docs/stabilization.md` (algorithm step by step with the papers);
    - `docs/architecture.md`;
    - `CLAUDE.md` pitfalls:
      - `SourceCorrection` in `maps_pixels_one_to_one`;
      - decode at the sample's time;
      - never simplify or interpolate the path;
      - the zoom is derived, so use `stabilization_with_smoothing`;
    - `CHANGELOG.md` and `docs/status.md`.

**Later steps, cut from this PR:**
- a crop-aware zoom, which gives vertical crops of landscape footage 1–3 points less zoom;
- a tripod ("Locked") mode with an L1 or corridor path (Grundmann et al. 2011, or design 2's ADMM);
- a zoom or maximum zoom set by the user;
- RANSAC with a background preference, for a foreground that covers most of the frame;
- rolling-shutter correction;
- a debug video;
- a timeline badge, a TopBar progress pill, and unmeasured moments shown on the timeline;
- following points in parallel with rayon;
- a compact path encoding, which is a format change;
- a 60 fps benchmark variant;
- a "hold to compare" preview;
- `kadr-cli assemble --stabilize`.

## Risks

- **Merge conflicts with the parallel phases:**
  - 8e and 8g also take format 4: whoever merges later renumbers;
  - 8f rewrites `source_picture_of`: our frame-time snapping and correction must sit after its `picture_preferably_from_proxy`, using the proxy picture's size;
  - 8g adds `ClipContent::Subtitle` to the matches in `of_clip_at`, the validation and the composer;
  - decision record numbers: 0055 and 0056 are taken.
- **Signs and conventions:** clockwise with y down, the centre, inverse against forward. They are guarded by the unit tests, the GPU-vs-CPU scenes and benchmark check 9.
- **Estimation noise becomes jitter** (decision 0008's lesson). It is measured on the encoded benchmark, and the owner should check a real phone clip.
- **Classic limits:**
  - the median follows the majority, so a subject filling more than half the frame becomes the reference;
  - rolling-shutter wobble, parallax and motion blur stay.

  Document these in `docs/stabilization.md`.
- **Scale correction** could add breathing on noisy footage. The fallback is ρ = 1, a one-line change.
- **VFR files:** decoding at the sample's time can show a frame up to one frame earlier than without stabilisation. This cannot be seen, but turning the effect on or off may swap a frame.
- **File size and speed:**
  - about 0.3 MB per minute in the `.kadr`, rewritten on every change in MCP file mode (decision 0012);
  - zoom refitting takes about 1–2 s for an hour at 60 fps with σ = 10 s, while the edit holds the project lock.
- **Export and preview speed:** a stabilised lone canvas-size video loses the pass-through, so CPU exports and CPU previews are slower.
- **Analysis speed** is unmeasured. The laptop varies 2–3×, and the fallbacks are in Algorithm §6.
- **GPU tests:** WARP crashes in parallel GPU tests on the owner's machines; rerun single-threaded.

## Questions for the owner

1. **Defaults.** I propose a smoothing of 0.5 s (adjustable from 0.1 to 10 s) and an automatic enlargement capped at 20 %, with no zoom setting in this version. *Recommendation:* yes; add a zoom setting later if exports look too cropped.
2. **Tripod mode.** I propose to ship without a "tripod" mode (camera held completely still) and add it with a better path solver later. *Recommendation:* yes; a Gaussian-only tripod mode would often hit the zoom cap and look worse.
3. **Project size.** About 0.3 MB of project file per minute of stabilised video, saved with the project. *Recommendation:* accept, rather than a separate cache file that could get lost or go stale.

## Reports of the finished steps

Written by the agents that built each step, right after their commits.

### Step 4: kadr-core (effect, validation, file format)

**Commits:**

- b84f26e feat(core): stabilisation effect (file format 4)

**Checked:**

- Disk had 59 GB free at the start and about 44 GB at the end, so above the 15 GB limit throughout.
- cargo test -p kadr-core (run once with INSTA_UPDATE='always', then again without it and with CI=true): 117 unit, 7 integration and 3 doc tests pass. A filtered run confirmed every new test runs.
- New kadr-core tests: stabilization_lookup_is_a_step, frame_time_snaps_only_inside_the_analysed_range, a_path_covers_its_frames_up_to_the_end_of_the_last_one, unmeasured_ranges_skip_the_first_sample, a_stabilization_reads_back_from_json_with_its_path, a_stabilization_on_a_title_is_refused, a_second_stabilization_is_refused, stabilization_settings_out_of_range_are_refused, an_empty_or_unsorted_camera_path_is_refused, non_finite_path_values_are_refused, adding_replacing_hiding_and_removing_a_stabilization_revert_exactly (apply_and_check_revert, plus Arc::ptr_eq after ReplaceEffect), splitting_a_stabilized_clip_shares_its_camera_path (Arc::ptr_eq on both parts, new effect id on the second), version_3_files_are_upgraded_without_stabilization and a_format_3_file_is_upgraded_to_the_same_project.
- I read the snapshot diff: format_version goes from 3 to 4, a stabilization effect with id 14 appears on the first interview clip (3 samples at 2.0, 2.0333 and 2.0667 s, i.e. 1411200000, 1434720000 and 1458240000 ticks), and next_raw_id goes from 14 to 15. Nothing else changed.
- The format-3 fixture is today's snapshot without insta's 4-line header. It is byte-identical (cmp) to the format_3 fixture that the 8e and 8g branches already added.
- RUSTDOCFLAGS='-D warnings' cargo doc -p kadr-core --no-deps: no warnings.
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- cargo build --workspace --all-targets: succeeds with no warnings (33 min).
- cargo test -p kadr-tools: 30 passed. cargo test -p kadr-desktop: 51 passed. cargo test -p kadr-engine: all suites pass (54 unit tests plus 10, 3, 5, 6, 2, 2 and 4 integration tests), with no WARP crash.
- cargo fmt --all --check: clean on the committed state. Working tree clean after the commit.

**Deviations from the design:**

- There is no arm in kadr-cli/src/finishing_touches.rs. The plan lists it, but that match uses `_ => None` and compiles unchanged. The desktop's editing.rs matches also use wildcards and needed no change.
- Added public constants the plan does not list: Stabilization::MINIMUM_ZOOM_FACTOR (1.0), MINIMUM_PATH_SCALE_FACTOR (0.25) and MAXIMUM_PATH_SCALE_FACTOR (4.0). The validation range checks use them instead of literals, and the engine can clamp to them.
- covers() allows one tick of tolerance at the end (a private constant FRAME_START_ROUNDING). timestamp_of_frame rounds up, so for rates whose frames do not start on whole ticks (for example 2997/100), the last sample plus the last spacing can fall one tick before the next frame start. The test confirms this happens on about half the frames at 2997/100. frame_time_for() uses the exact end, without the tolerance. NTSC 30000/1001 frames are whole ticks, so the plan's example rate does not show the issue.
- Edge cases of a short path: a one-sample path has its analysed end at the sample itself, so frame_time_for() returns its input and covers() is true only up to one tick after the sample. An empty path, which validation refuses, gives sample_index_at = 0, analysed_range = (0, 0), covers = false and frame_time_for = identity, and never panics.
- unmeasured_ranges() uses the unmeasured samples' own times, consecutive ones joined. A single cut gives a range whose first and last times are equal, like the blur's single-sample unsure ranges.
- The validation message for non-media clips is 'a stabilisation only applies to a video'. I did not copy the blur's 'video or an image', because images are refused later by the request builder. Audio clips get the generic 'a stabilisation needs a picture, and this clip only has sound' first.
- The rule refusing a second stabilisation runs in validate_clip, after the per-effect loop, as validate_at_most_one_stabilization. Its reason reads 'a clip has at most one stabilisation, and effect N already steadies this one'.
- Besides the plan's version_3_files_are_upgraded_without_stabilization in project_file.rs, I added a_format_3_file_is_upgraded_to_the_same_project in editing_scenarios.rs. It follows the format 1 and 2 pattern and has the same name as the 8e branch's test.
- The command test does not check that a second stabilisation is refused. Commands do not validate; the session does, so apply_expecting_error cannot see the refusal. validation.rs tests that rule through the session.
- Added a test-only helper effect::test_samples::stabilization_of_frames(first_frame, last_frame) (pub(crate), cfg(test)), used by the effect, validation, command and clip tests.
- docs/data-model.md: I also fixed the mermaid diagram's effects node, which still lacked the colour LUT, and added a paragraph describing the lookups.

**Open problems:**

- I did not run the kadr-mcp and kadr-cli tests, since I did not touch those crates. Only kadr-tools' EffectView arm can reach Claude's answers, and no existing test creates a stabilisation.
- Engine risk for step 7 or 9, not in this commit: if the fitted zoom is capped first and then rounded up to 0.1 %, 1.2 becomes 1.201 in f32 ((1.2f32 * 1000).ceil() = 1201), which validation refuses. Round up first, then take min(MAXIMUM_ZOOM_FACTOR).

**Notes for the next steps:**

["New public API in kadr_core::effect:", "- `EffectKind::Stabilization(Stabilization)`. Its display_name is \"stabilisation\", needs_picture is true, and the JSON is `{\"type\": \"stabilization\", ...}`.", "- `pub struct Stabilization { pub smoothing_in_seconds: f32, pub zoom_factor: f32, pub analysed_picture_size: PictureSize, pub camera_path: Arc<[CameraPathSample]> }`, deriving Debug, Clone, PartialEq, Serialize and Deserialize.", "- Consts on Stabilization: DEFAULT_SMOOTHING_IN_SECONDS = 0.5, MINIMUM_SMOOTHING_IN_SECONDS = 0.1, MAXIMUM_SMOOTHING_IN_SECONDS = 10.0, MINIMUM_ZOOM_FACTOR = 1.0, MAXIMUM_ZOOM_FACTOR = 1.2, MINIMUM_PATH_SCALE_FACTOR = 0.25, MAXIMUM_PATH_SCALE_FACTOR = 4.0.", "- `pub fn sample_index_at(&self, source_time: Timestamp) -> usize`, computed as partition_point(<=) - 1, saturating at 0.", "- `pub fn frame_time_for(&self, source_time: Timestamp) -> Timestamp`: snaps to the sample's time inside [first sample, last sample + last spacing), and returns its input outside.", "- `pub fn analysed_range(&self) -> (Timestamp, Timestamp)`: the first and last sample times.", "- `pub fn covers(&self, source_start: Timestamp, source_end: Timestamp) -> bool`: the end is excluded, with one tick of tolerance.", "- `pub fn unmeasured_ranges(&self) -> Vec<UnsureRange>`, each with confidence TrackingConfidence::Lost.", "- `pub struct CameraPathSample { pub source_time: Timestamp, pub shift_x_in_pixels: f32, pub shift_y_in_pixels: f32, pub rotation_in_degrees: f32, pub scale_factor: f32, pub motion_was_measured: bool }`, deriving Debug, Clone, Copy, PartialEq, Serialize and Deserialize.", "Build a Stabilization with `camera_path: vec.into()` (Vec<CameraPathSample> into Arc<[_]>). To change only the smoothing, use `Stabilization { smoothing_in_seconds, zoom_factor, ..existing.clone() }`, which shares the Arc.", "Validation (form only) refuses a stabilisation on a non-media clip, smoothing outside 0.1–10 or NaN, a zoom outside 1–1.2, a zero analysed side, an empty path, a path not in strictly increasing source_time, a non-finite value, a scale outside 0.25–4, and a second stabilisation on the same clip (InvalidEffect on the second). The engine must produce paths that pass these checks.", "File format: CURRENT_FORMAT_VERSION = 4, with Migration { from_version: 3, upgrade_document: allow_stabilization }. 8e and 8g also take version 4 with an identical format_3 fixture, so whoever merges later renumbers to 5, chains after them, and adds the version 4 snapshot as the next fixture.", "Editing scenario: edit_a_vertical_interview now calls edit_a_vertical_interview_before_phase_8d and then adds one stabilisation (3 samples, frames 60–62 at 30 fps, zoom 1.05) to tracks()[0].clips()[0]. The undo count is 23.", "Behaviour-neutral arms that later steps must fill:", "- crates/kadr-engine/src/color_grading.rs: applied_color_lut_of and ColorTableLibrary's used-files filter → None. Final.", "- crates/kadr-engine/src/export/frame_composer.rs: enabled_color_adjustments → None and hidden_regions_of → None. Final; step 9 adds the frame_time and correction logic elsewhere in that file.", "- crates/kadr-engine/src/export/mod.rs, describe_unrendered_parts: `EffectKind::TrackedBlur(_) | EffectKind::Stabilization(_) => {}` with the comment '// Stabilisations are not rendered yet.'. Step 9 replaces it with the two warnings (covers, partly corrected frames).", "- crates/kadr-tools/src/views.rs, EffectView::of: `EffectKind::Stabilization(_) => (\"stabilization\", serde_json::json!({}))` with the comment '// Its settings are not described to Claude yet.'. Step 11 fills it with smoothing_in_seconds, zoom_in_percent, analysed_from_second, analysed_to_second and frames_not_followed.", "- crates/kadr-tools/src/tools/blur_tools.rs blur_views_of_clip → None; content_tools.rs existing_color_adjustment and existing_color_lut → not_an_effect_of_kind error; picture_tools.rs enabled_color_luts → None. All final.", "- apps/desktop/src-tauri/src/editing/blur_settings.rs tracked_blur_of → 'it is not a tracked blur' error; editing/clip_settings.rs color_lut_of → None; project_view.rs tracked_blurs_of, first_color_adjustment_of and first_color_lut_of → None. All final; step 12 adds ClipView.stabilization separately.", "The content_tools.rs refuse_second_color_lut string with 14 inner spaces is still unfixed; it is left for step 11.", "`cargo test -p kadr-engine` alone recompiles serde, kadr-core and the engine, because features unify differently than in the workspace build. Expect about 10 extra minutes."]

### Steps 1-3: kadr-vision (camera motion, camera path)

**Commits:**

- 0ea96cc refactor(vision): follow any list of points both ways
- 3c59d5c feat(vision): measure the camera's motion between frames
- d5d6bf1 feat(vision): smooth a camera path and size the zoom

**Checked:**

- Branch stabilization-vision was created from 6740063 in worktree C:/Users/noahk/Desktop/Project/kadr/.claude/worktrees/wf_87e6e4c5-93d-1. It has 3 commits, the tree is clean and nothing was pushed.
- Step 1 (refactor): ran `cargo test -p kadr-vision -- --nocapture --test-threads=1` before and after, and diffed the outputs with timings removed. They are identical: 44 unit and 8 tracking_scenarios tests, and every reported tracking error the same. Clippy is clean.
- After step 2, the tracking_scenarios eprintln output is still identical to the baseline. The only difference is the doctest count, because of the new doctest. This shows that moving SceneFrame's object drawing into SceneObject::drawn_over changed nothing.
- Step 2: unit tests corner_points_are_spread_and_avoid_the_border, flat_cells_give_no_point, the_similarity_fit_recovers_a_known_transform, the_fit_ignores_a_third_of_outliers, bunched_inliers_give_a_translation_only pass, plus extra tests too_few_or_disagreeing_points_give_no_motion and a_change_of_brightness_everywhere_is_a_cut, and 3 SimilarityTransform tests and 1 ShakenCameraFrame test.
- Step 2: all 9 tests in tests/camera_motion_scenarios.rs pass at 640x360 (shift error <= 0.05 tracking px, roll <= 0.01°, scale <= 5e-4). Worst errors measured: pure shift 0.0064 px; roll 0.00019°; change of size scale 8e-6; combined 0.0048 px / 0.00044° / 1.1e-5; moving object covering 33.6% of the frame 0.0143 px / 0.0025°; fast pan 0.0085 px; noise ±6 grey levels 0.0113 px, mean bias (-0.0005, 0.0005) px. Uniform frames and cuts give None.
- Step 3: all 9 planned camera_path unit tests pass, plus the_whole_path_and_single_frames_agree and the_zoom_needed_matches_the_turned_frame. Shake left inside a run at sigma 0.5 s: 0.0003 px in x, 0.0006 px in y, 0.00014° of roll, 5e-7 in log scale. A straight pan gets corrections below 1e-6 and a zoom of exactly 1.0. Run ends give exactly IDENTITY. The largest correction step is 2.849 px both with and without the break, and 0 across the break. Fitted zoom test: largest need 1.02894 gives zoom 1.029, and every frame has fraction 1 even with the zoom passed through f32.
- Final run of `cargo test -p kadr-vision`: 66 unit tests, 9 camera_motion_scenarios, 8 tracking_scenarios and 3 doctests all pass. `cargo clippy -p kadr-vision --all-targets -- -D warnings` is clean, `cargo fmt --all --check` is clean, and `cargo doc -p kadr-vision --no-deps` with RUSTDOCFLAGS=-D warnings is clean.
- Release bench `cargo bench -p kadr-vision --bench camera_motion` on this laptop, with 14 cargo/rustc processes from other sessions running: push_frame_640x360 117 ms per frame (8.5 fps); fitted_zoom_10_minutes (18 000 samples, sigma 0.5) 147 ms; correction_of_one_frame 18.7 µs. For calibration, track_region follow_visible_region ran here at 20.5 ms per frame against the documented 3.3 ms on the desktop, so this machine was about 6x slow. Free disk was 60 GB, then 42 GB; this worktree's target is 0.8 GB.

**Deviations from the design:**

- Skipped as instructed: the byte-identical tracking CSV check for step 1 (track_video on the tracking benchmark). It needs kadr-engine and the generated benchmark, so the engine agent must run it. As a substitute I diffed the tracking_scenarios output before and after, and it is identical.
- Vision API shape: instead of the engine binary-searching a ±3σ window and converting it to CameraPoses, vision reads the path through a trait `CameraPath { fn sample_count(&self) -> usize; fn sample_at(&self, index) -> CameraPathPoint }`. Reason: the 'σ shrunk to run length / 6' rule needs to know the run's length, which a ±3σ window cannot always tell. Vision scans at most 6σ each side to find whether the run is short, and computes weights only within 3σ. The engine implements the trait on a wrapper over &[CameraPathSample] and converts one sample at a time, with no allocation.
- Fit step 3: the median-seed inlier set I0 must also hold at least 8 points, otherwise the motion is not measured. The plan put no count check on I0. Fewer than 8 points agreeing with the median means the median itself cannot be trusted.
- Smoothing window boundary: samples within 1e-9 (relative) of 3σ are kept on both sides (REACH_TOLERANCE). Without this, at a constant frame rate the samples at exactly ±3σ were kept on one side only because of floating rounding. A straight pan then got a 0.13 px correction, and the shake test left 0.09 px. The kernel is otherwise exactly the plan's truncated Gaussian.
- Zoom tolerances: the fitted zoom is computed as ceil(need·(1−1e-6)·1000)/1000, so a straight path costs no 0.1 % zoom. A correction gets λ = 1 if need ≤ z·(1+1e-5), which covers the zoom being stored as f32. When λ < 1, the 16-step bisection compares strictly against z, so a partial correction never shows an edge. fitted_zoom_factor stops early once the 1.2 cap is reached.
- Run ends: an end sample returns its own pose directly. This equals the reflected average mathematically, and it makes the identity correction at run ends bit-exact.
- Robustness additions: smoothed_pose_at returns Option (None outside the path). stabilizing_correction_at returns IDENTITY for an index outside the path, a smoothing that is not a positive finite number, or a result that is not finite.
- Test a_fast_pan_is_followed_through_the_prediction speeds the pan up to 40 px per frame (-8, -16, -24, -32, then -40 four times) instead of starting at 40 px from rest. A sudden 40 px jump from (0,0) is not measured on the synthetic texture: with 14 px features, pyramid level 3 has almost no texture left. Every frame after that is measured, with a worst error of 0.0085 px.
- Test a_constant_offset_of_the_path_changes_nothing checks two cases. (a) A constant offset of roll (+7°) and size (x1.3) on a fully shaky path. (b) A constant shift offset (+250, -120) on a path with constant roll and size. Smoothing each channel separately cannot be invariant to a shift offset when the correction rotates or scales: e changes by (I − ρR(Δa))q. So re-analysing from another start frame changes the corrections slightly; this is worth writing in docs/stabilization.md.
- Doc figure: at σ = 0.5 s a 0.05 Hz move keeps exp(−(2π·0.05·0.5)²/2) = 98.8 %, not the plan's 99.4 %. The 1 Hz figure of 0.7 % is correct.
- Small additions outside the planned file list: `pub(crate) fn values(&self) -> &[f32]` on GreyImage (grey_image.rs), and `MINIMUM_TEXTURE_PER_PIXEL` made pub(crate) in optical_flow.rs. Both are in commit 2.
- Module visibility: camera_motion and camera_path are `pub mod` with no root re-exports, like silence_detection. SimilarityTransform is re-exported at the crate root (kadr_vision::SimilarityTransform). ShakenCameraFrame adds uniform noise in ±noise_amplitude, built from the lattice hash.

**Open problems:**

- Analysis speed is the main risk for gate #7 (≥ 60 fps). push_frame measured 117 ms per frame on the loaded laptop. Dividing by the 6x calibration factor gives roughly 19 ms per frame on the desktop, about 50 fps for motion measurement alone, before decode and shrink. The plan estimated 12–13 ms in total. The cost is Lucas-Kanade on 144 points: per point, about 4 levels x 2 directions x (225-pixel window + up to 10 x 225 bilinear samples). The plan's fallbacks apply. First, remove the per-call Vecs in optical_flow.rs refine_motion; the behaviour stays the same, but the tracking CSV must be re-checked. Second, use a 12 x 7 grid by changing CELLS_ALONG_LONGER_SIDE and CELLS_ALONG_SHORTER_SIDE in camera_motion.rs. Measure on the desktop before deciding.
- fitted_zoom_factor took 147 ms for 18 000 samples on the laptop, about 25 ms on the desktop by calibration, which is about 2.5 ms per minute of 30 fps video at σ = 0.5. An hour at 60 fps with σ = 10 s scales to several seconds, against the plan's 1–2 s. A cheap speed-up that changes no result: compute ln(scale) per sample once per run in visit_corrections. Today PoseChannels::of computes it for every window the sample falls in.
- A sudden fast pan from rest (40 tracking px in the first frame) is not measured on the fine synthetic texture, because coarse pyramid levels lose their texture. Real footage has coarse structure, but check this on the benchmark video.

**Notes for the next steps:**

Branch to merge: stabilization-vision (head d5d6bf1, 3 commits on 6740063). New public API in kadr-vision:

1) `kadr_vision::SimilarityTransform { pub center: Point, pub shift_x_in_pixels: f32, pub shift_y_in_pixels: f32, pub rotation_in_degrees: f32, pub scale_factor: f32 }`. It maps p to center + s·R(θ)(p − center) + shift, with R clockwise (y down). Methods: `identity_about(center: Point) -> Self`, `apply_to(&self, Point) -> Point`, `inverse(&self) -> Self`, `followed_by(&self, next: &SimilarityTransform) -> Self` (self, then next, about self.center), `about(&self, center: Point) -> Self`, `in_full_size_image(&self, shrink_factor: usize) -> Self` (multiplies the centre and the shift).

2) `kadr_vision::camera_motion::{CameraMotionEstimator, CameraMotionError}`. `CameraMotionEstimator::new()` (also Default). `push_frame(&mut self, frame: GreyImage) -> Result<Option<SimilarityTransform>, CameraMotionError>`: None on the first frame and on unmeasured frames. The result is in tracking pixels, about the tracking image's centre (w/2, h/2). Error: `CameraMotionError::FrameSizeChanged { width, height, expected_width, expected_height }`.

3) `kadr_vision::synthetic_scenes::ShakenCameraFrame { width: usize, height: usize, background_seed: u32, camera: SimilarityTransform, moving_object: Option<SceneObject>, noise_amplitude: f32, noise_seed: u32 }` with `render() -> GreyImage`. The background point p is seen at camera.apply_to(p). The true motion between two frames is `previous.camera.inverse().followed_by(&current.camera)`. For engine H.264 tests: background feature size is 14 px; ramp fast pans up gradually.

4) `kadr_vision::camera_path`:
- `pub const MAXIMUM_ZOOM_FACTOR: f64 = 1.2`; keep core's `Stabilization::MAXIMUM_ZOOM_FACTOR` equal to it and assert this in a test.
- `CameraPose { pub shift_x_in_pixels: f64, pub shift_y_in_pixels: f64, pub rotation_in_degrees: f64, pub scale_factor: f64 }` with `IDENTITY` and `followed_by(&self, motion: &SimilarityTransform, picture_center: Point) -> CameraPose`, which re-expresses the motion about picture_center first.
- `CameraPathPoint { pub time_in_seconds: f64, pub pose: CameraPose, pub motion_was_measured: bool }`.
- `pub trait CameraPath { fn sample_count(&self) -> usize; fn sample_at(&self, index: usize) -> CameraPathPoint; }`, implemented for [CameraPathPoint] and Vec<CameraPathPoint>.
- `StabilizingCorrection { pub shift_x_in_pixels: f64, pub shift_y_in_pixels: f64, pub rotation_in_degrees: f64, pub scale_factor: f64 }` (K; source point v is drawn at C + s·R(r)(v − C) + shift) with `IDENTITY`, `between(measured: &CameraPose, smoothed: &CameraPose) -> Self`, `zoom_needed(&self, picture_width_in_pixels: f64, picture_height_in_pixels: f64) -> f64`, `partly_applied(&self, fraction: f64) -> Self`, `enlarged_by(&self, zoom_factor: f64) -> Self`, `fitted_inside_zoom(&self, zoom_factor: f64, w: f64, h: f64) -> FittedCorrection`.
- `FittedCorrection { pub correction: StabilizingCorrection /* F = Z∘K_λ, analysed pixels */, pub fraction_applied: f64 }`.
- Functions: `smoothed_pose_at<P: CameraPath + ?Sized>(path: &P, index: usize, smoothing_in_seconds: f64) -> Option<CameraPose>`, `stabilizing_correction_at<P>(path: &P, index: usize, smoothing_in_seconds: f64) -> StabilizingCorrection` (K before zoom), `stabilizing_corrections<P>(path: &P, smoothing_in_seconds: f64) -> Vec<StabilizingCorrection>` (bit-identical to the per-index results), `fitted_zoom_factor<P>(path: &P, smoothing_in_seconds: f64, w: f64, h: f64) -> f64` (rounded up to 0.001, clamped to [1, 1.2]).

Engine recipe:
- Analysis loop: `let center = Point { x: W as f32 / 2.0, y: H as f32 / 2.0 }; let motion = estimator.push_frame(grey)?; if let Some(m) = motion { pose = pose.followed_by(&m.in_full_size_image(shrink_factor), center); }` and `motion_was_measured = motion.is_some()`. The first frame gives None, hence false. Store rounded f32 values but keep composing the unrounded f64 pose.
- Correction: wrap &[CameraPathSample] in a struct implementing CameraPath, converting source_time to f64 seconds and the f32 fields to f64. Then `stabilizing_correction_at(&wrapper, stabilization.sample_index_at(t), smoothing as f64).fitted_inside_zoom(zoom as f64, analysed_w, analysed_h)` gives F. SourceCorrection = (F.shift·decoded_w/analysed_w, F.rotation, F.scale) as f32. No window search is needed in the engine: vision reads at most 6σ around the frame.
- Zoom: `fitted_zoom_factor(&wrapper, σ, W, H) as f32`; the tolerances inside vision handle f32 storage.
- Summary: call `stabilizing_corrections` once, then `fitted_inside_zoom` per frame to count frames with λ < 1. Largest shift and rotation come from K.

Facts for docs/stabilization.md:
- At σ = 0.5 a 0.05 Hz move keeps 98.8 %, and 1 Hz keeps 0.7 %.
- Smoothing each channel separately is not invariant to re-analysing from another start frame, when there is roll or scale.
- The correction's shift e includes the term ρR(Δa)T_P, so long pans with roll shake give large shifts that cancel. Keep the path in f64 or f32 with 0.01 px rounding: this is fine.

Speed numbers and fallbacks are under Problems. Gate #7 must be measured on the desktop.

### Step 5: kadr-render (source correction)

**Commits:**

- 2f69a8b feat(render): move the source picture inside its layer's window

**Checked:**

- Disk had 36.6 GB free at the start and 19.3 GB at the end (other sessions were building too), so it stayed above 15 GB.
- cargo fmt --all --check: clean on the committed state. The working tree is clean after the commit.
- cargo clippy -p kadr-render --all-targets -- -D warnings: clean. cargo clippy --workspace --all-targets -- -D warnings: clean.
- RUSTDOCFLAGS='-D warnings' cargo doc -p kadr-render --no-deps: no warnings.
- cargo test -p kadr-render with KADR_REQUIRE_GPU=1 on the laptop's real GPU: 55 unit tests and 13 gpu_matches_cpu tests pass. A filtered run confirmed that every new test runs.
- The same gpu_matches_cpu suite with KADR_GPU_ADAPTER=software (WARP) and KADR_REQUIRE_GPU=1 crashed with 0xc0000005 in the parallel run, which is the known CLAUDE.md pitfall. With --test-threads=1, all 13 pass.
- Differences measured on the real GPU (Y/U/V mean difference, no values off by more than 10): stabilized_video 0.028/0.399/0.244, and stabilized_cropped_scaled_and_rotated_video about 0.055/0.073/0.071 before I raised its shift_y from 9 to 18, so that the window also reaches past the picture's edge. stabilized_gradient_with_a_lut_matches_strictly: largest difference 1 (limit 2). I removed the temporary measurement prints before committing.
- Mutation check 1: texture_corners left unwarped, so the GPU ignored the correction. All three new GPU tests failed (stabilized video Y mean 67.7; cropped scene Y mean 5.1 with 2.45 % of values off by more than 10; strict largest difference 22), and so did the unit test interpolated_texture_corners_agree_with_source_point_for.
- Mutation check 2: the CPU's VisiblePixels kept the crop clamp. The unit test a_corrected_layer_shows_pixels_beyond_its_crop failed. The tolerant GPU-vs-CPU scenes did not catch it: the mean only rose from 0.055 to 0.078. I restored both mutations byte for byte from copies.
- cargo build --workspace --all-targets: succeeds with no warnings (6 min 46 s, incremental from render up through desktop and cli).
- cargo test --workspace: exit 0, every suite passes (engine, tools, mcp, cli, desktop, core, vision, render).
- New kadr-render tests. layers.rs: an_identity_correction_changes_no_geometry, interpolated_texture_corners_agree_with_source_point_for (within 1e-3 px at more than 300 pixel centres), a_correction_stops_the_one_to_one_shortcut, a_correction_turns_clockwise_and_enlarges_around_the_picture_centre, a_correction_shrinking_the_picture_to_nothing_shows_nothing, and the extended the_public_mapping_agrees_with_the_renderers_geometry. cpu_compositor.rs: a_stabilized_canvas_sized_video_is_not_passed_through and a_corrected_layer_shows_pixels_beyond_its_crop. gpu_matches_cpu.rs: stabilized_video, stabilized_cropped_scaled_and_rotated_video and stabilized_gradient_with_a_lut_matches_strictly.

**Deviations from the design:**

- The only full LayerPlacement literals in the workspace are FITTED and of_clip_at. Every other literal uses `..LayerPlacement::FITTED` or `..self`, so I had no literal to fix outside kadr-render.
- LayerGeometry::texture_corners lost its (picture_width, picture_height) parameters, because the geometry now stores the picture's size. Its only caller, crates/kadr-render/src/gpu/mod.rs layer_uniforms, needed a one-line change. That file is not in the plan's step 5 list. I changed no WGSL, uniforms, sampler or LAYER_UNIFORM_SIZE_IN_BYTES.
- The correction's matrices (window_to_source and source_to_window) are computed in f64 and stored as f32. An identity correction short-circuits to an exact identity matrix, so placements without a correction give bit-identical results to before.
- LayerGeometry::new also returns None when source_correction.scale_factor <= 0, as the plan says. A NaN scale is not refused, which matches how the existing layer scale is checked.
- Added tests beyond the plan: a_correction_turns_clockwise_and_enlarges_around_the_picture_centre (sign and centre convention), a_correction_shrinking_the_picture_to_nothing_shows_nothing, and a_corrected_layer_shows_pixels_beyond_its_crop. The last one is the only test that catches a CPU clamp to the crop instead of the whole picture.
- Small refactor in tests/gpu_matches_cpu.rs: the strict LUT test's gradient, its red/blue-swapped LUT and its largest-difference computation became helpers (small_rgba_gradient, red_and_blue_swapped, largest_difference_between_renderers), shared with the new strict stabilisation test. The existing test behaves the same.
- I also updated crates/kadr-render/README.md (one bullet on SourceCorrection) and the module docs of cpu_compositor.rs and lib.rs, neither listed in the plan. In docs/rendering.md I also updated the pass-through bullet, the GPU step 2 and CPU paragraphs, and the GPU-matches-CPU paragraph, besides the new '### Order' and '### Stabilisation: the source correction' sections.

**Open problems:**

- WARP (KADR_GPU_ADAPTER=software) crashed with 0xc0000005 in the parallel gpu_matches_cpu run on this laptop, the known pitfall. The single-threaded run passes.
- The tolerant GPU-vs-CPU threshold (mean < 1.5, under 2 % off by more than 10) cannot tell a CPU clamp to the crop from a clamp to the whole picture. The CPU unit test a_corrected_layer_shows_pixels_beyond_its_crop guards that instead.
- docs/rendering.md's Order step 1 already says a stabilised clip's frame is decoded at its camera path sample's time. The engine only does that from step 9; until then, nothing sets a non-identity correction.

**Notes for the next steps:**

New public API in kadr_render (crates/kadr-render/src/layers.rs, re-exported from lib.rs):
- `pub struct SourceCorrection { pub shift_x_in_pixels: f32, pub shift_y_in_pixels: f32, pub rotation_in_degrees: f32, pub scale_factor: f32 }`, deriving Debug, Clone, Copy and PartialEq. Source point v is drawn at C + s·R(r)·(v − C) + t: C is the decoded picture's centre (W/2, H/2), R turns clockwise with y down, and everything is in the decoded picture's pixels. This is exactly the plan's F = Z ∘ K_λ: `scale_factor = z·ρ^λ`, `rotation_in_degrees = λΔa`, `shift = z·λ·e × (decoded width / analysed width)`.
- `SourceCorrection::IDENTITY` (shift 0, rotation 0, scale 1) and `pub fn is_identity(&self) -> bool`, an exact compare where -0.0 counts as 0.
- `LayerPlacement` has a new field `pub source_correction: SourceCorrection`. FITTED and `of_clip_at` set IDENTITY, and `moved_by` keeps it (`..self`). Step 9 sets it after `of_clip_at(...).moved_by(motion)` in frame_composer.rs and in clip_geometry.rs's clip_source_placement, with the decoded picture's size for the shift. Texts must keep IDENTITY.
- `LayerPlacement::source_to_canvas_mapping(canvas_size, picture_size)` now composes the correction: the matrix maps raw source pixels to canvas pixels. `visible_source` is the crop without a correction, and with one the bounding box of the crop corners mapped back through the correction, clamped to [0, W] × [0, H]. Pass the same picture size the correction's shift was scaled to.
- `LayerGeometry::new` returns None when `source_correction.scale_factor <= 0`.
- A non-identity correction makes `maps_pixels_one_to_one` false, which turns off both the pass-through and the CPU one-to-one copy. Add this to the CLAUDE.md pitfalls in step 14, as the plan says.
- Engine and consumer code needs no other change for unstabilised clips: behaviour is bit-identical with IDENTITY, and the workspace tests all pass.
- Build note: `cargo build --workspace --all-targets` after this commit took about 7 minutes (incremental). `cargo test --workspace` then reuses those binaries without recompiling.
