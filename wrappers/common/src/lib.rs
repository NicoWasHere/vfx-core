//! Host-neutral parts of the vfx-core plugin wrappers: per-frame settings and the
//! pooled effect chain that renders them. Each host wrapper (After Effects, OpenFX)
//! reads its own parameters into [`Settings`] and moves pixels in and out.

pub mod diag;
pub mod pipeline;
mod settings;

pub use settings::{Blur, Settings, Vignette};
