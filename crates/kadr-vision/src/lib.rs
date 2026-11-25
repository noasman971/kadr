//! # kadr-vision — classic computer vision, without AI
//!
//! This crate contains the deterministic algorithms behind Kadr's analysis
//! features:
//!
//! - **region tracking** for the face blur: the user draws a box around a face,
//!   and a correlation-filter tracker (MOSSE, then KCF) follows it frame after
//!   frame, with scale estimation, a confidence score (peak-to-sidelobe ratio),
//!   recovery by template matching, Lucas-Kanade optical flow for fast motion and
//!   temporal smoothing;
//! - **silence detection** on audio samples.
//!
//! ## Rules for this crate
//!
//! - **No neural network, no pre-trained model, no model file.** Every algorithm
//!   is classic, explainable, and documented step by step with a reference to
//!   its original paper.
//! - Algorithms work on plain buffers (grey-level images, audio samples) so they
//!   can be tested on synthetic data.
//!
//! Implemented in phase 4. See `docs/architecture.md` and
//! `docs/decisions/0003-face-blur-by-classic-tracking.md`.
