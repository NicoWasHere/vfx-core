//! Real-time GPU video effects on wgpu.
//!
//! The core owns all GPU work. Hosts and wrapper crates only map their own
//! parameters and pixel buffers onto [`Effect`], [`Frame`] and [`EffectChain`].

mod chain;
mod effect;
pub mod effects;
mod frame;
mod gpu;
mod param;
mod shader;

pub use chain::EffectChain;
pub use effect::Effect;
pub use frame::Frame;
pub use gpu::{Gpu, GpuError};
pub use param::{ParamError, ParamKind, ParamSpec, ParamValue, ParamValues};

/// Re-exported so hosts can match the exact wgpu version the core was built against.
pub use wgpu;
