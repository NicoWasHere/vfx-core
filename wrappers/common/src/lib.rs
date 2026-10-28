//! Host-neutral parts of the vfx-core plugin wrappers: per-frame settings and the
//! pooled effect chain that renders them. Each host wrapper (After Effects, OpenFX)
//! reads its own parameters into [`Settings`] and moves pixels in and out.

pub mod diag;
pub mod pipeline;
mod settings;

pub use settings::{Blur, Settings, Vignette};

/// A value from plugin.env as a C string, at compile time. The wrapper's build.rs sets
/// the variables (`VFX_PLUGIN_NAME` and so on) by including `plugin_env.rs`.
#[macro_export]
macro_rules! plugin_cstr {
    ($var:literal) => {
        match ::std::ffi::CStr::from_bytes_with_nul(concat!(env!($var), "\0").as_bytes()) {
            Ok(s) => s,
            Err(_) => panic!(concat!($var, " contains a NUL byte")),
        }
    };
}
