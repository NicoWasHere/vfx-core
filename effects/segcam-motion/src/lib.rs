//! segcam's **motion** segmenter as a vfx-core effect: it finds whatever is moving in a
//! layer, then draws it the segcam way (boxes with ID chips, mattes, or the swarm collage).
//!
//! A Rust port of segcam's C++ effect (`segcam/ae`). It is a [`vfx_core::CpuEffect`]
//! ([`effect::SegcamMotion`]): it works on frames in RAM at the host's own depth, and reads
//! the frames before the current one, which hosts fetch or keep for it. The host targets
//! (After Effects, OpenFX, FFGL, the preview app) are vfx-core's; `src/bin` runs the render
//! path on raw frames from the command line.

#[rustfmt::skip]
mod chrome;
pub mod effect;
pub mod font;
pub mod motion;
pub mod pixel;
pub mod render;
pub mod swarm;

pub use effect::SegcamMotion;

vfx_plugin::export!(cpu: SegcamMotion);
pub use motion::{Blob, LumaGrid, Motion, MotionSettings, NormRect};
pub use pixel::{Argb, Channel, Color, View, ViewMut};
pub use render::{OutputMode, RenderSettings, previous_frames_needed, render, segment_label};
