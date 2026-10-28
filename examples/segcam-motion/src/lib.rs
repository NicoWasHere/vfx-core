//! segcam's **motion** segmenter as an After Effects effect: it finds whatever is moving in
//! a layer, then draws it the segcam way (boxes with ID chips, mattes, or the swarm collage).
//!
//! A Rust port of segcam's C++ effect (`segcam/ae`). The core here is host-free and generic
//! over 8, 16 and 32 bpc pixels; `ae` is the After Effects glue, and `src/bin` runs the same
//! render path on raw frames from the command line.

#[rustfmt::skip]
mod chrome;
pub mod font;
pub mod motion;
pub mod pixel;
pub mod render;
pub mod swarm;

// The lints are about code the after-effects crate's `define_effect!` writes.
#[cfg(any(target_os = "macos", target_os = "windows"))]
#[allow(clippy::drop_non_drop, clippy::question_mark)]
mod ae;

pub use motion::{Blob, LumaGrid, Motion, MotionSettings, NormRect};
pub use pixel::{Argb, Channel, Color, View, ViewMut};
pub use render::{
    FrameSource, OutputMode, RenderSettings, previous_frames_needed, render, segment_label,
};
