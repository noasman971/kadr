//! Decoded pictures.
//!
//! Every picture Kadr handles internally is 8-bit RGBA, top row first: the
//! format the compositor works in. Decoders convert to it; encoders convert
//! from it.

/// One decoded picture: 8-bit red, green, blue, alpha, top row first.
pub type VideoFrame = image::RgbaImage;
