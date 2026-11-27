//! # kadr-vision — classic computer vision, without AI
//!
//! This crate contains the deterministic algorithms behind Kadr's analysis
//! features. Today: **region tracking** for the face blur. The user draws a box
//! around a face (or a licence plate, a screen…) and [`RegionTracker`] follows it
//! frame after frame:
//!
//! - a MOSSE correlation filter finds the region in each frame
//!   (`correlation_filter.rs` explains it step by step);
//! - Lucas-Kanade optical flow on points of the region predicts fast motions and
//!   measures how much the region grows or shrinks when it comes closer or moves
//!   away (Median Flow);
//! - the peak-to-sidelobe ratio says how sure each answer is: confident,
//!   uncertain or lost;
//! - a lost region is searched again by normalised cross-correlation with its
//!   memorised appearance;
//! - [`smooth_track`] steadies the result without lag (One Euro filter, forward
//!   and backward).
//!
//! `docs/tracking-and-blur.md` explains the whole pipeline, its settings and its
//! limits.
//!
//! The crate also finds the **silent parts of a sound** ([`silence_detection`]),
//! for cutting silences: its loudness is measured window by window.
//!
//! For **video stabilisation**, [`camera_motion`] measures how a shaky camera
//! moved from each frame to the next (its shift, roll and change of size): Shi and
//! Tomasi points spread over the frame, followed by the same optical flow, and a
//! least-squares similarity that ignores what moves on its own.
//!
//! ## Rules for this crate
//!
//! - **No neural network, no pre-trained model, no model file.** Every algorithm
//!   is classic, explainable, and documented step by step with a reference to its
//!   original paper.
//! - Algorithms work on plain buffers (grey-level images, audio samples) and depend on no other
//!   Kadr crate, so they can be tested on synthetic data ([`synthetic_scenes`]).
//!
//! See also `docs/decisions/0003-face-blur-by-classic-tracking.md`.

pub mod camera_motion;
mod correlation_filter;
mod fourier;
mod geometry;
mod grey_image;
mod optical_flow;
pub mod silence_detection;
mod smoothing;
pub mod synthetic_scenes;
mod template_matching;
mod tracker;

pub use geometry::{Point, Region, SimilarityTransform};
pub use grey_image::GreyImage;
pub use smoothing::smooth_track;
pub use tracker::{
    CONFIDENT_PEAK_TO_SIDELOBE_RATIO, LOST_PEAK_TO_SIDELOBE_RATIO,
    MAXIMUM_UNCERTAIN_FRAMES_IN_A_ROW, MINIMUM_REGION_SIDE_IN_PIXELS, RegionTracker,
    TRACKING_IMAGE_MAXIMUM_WIDTH_IN_PIXELS, TrackedRegion, TrackingError, TrackingState,
    shrink_factor_for_tracking,
};
