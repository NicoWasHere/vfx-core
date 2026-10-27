//! Sample effects. Each one covers a case the core has to handle:
//! - [`ColorAdjust`]: one pass, no state
//! - [`GaussianBlur`]: two passes with an internal scratch frame
//! - [`FeedbackTrail`]: persistent state carried between frames
//!
//! - [`Vignette`]: point, colour and float parameters together
//! - [`Blend`]: reads a second layer supplied by the host
//!
//! [`PixelConvert`] is plumbing rather than a look: it converts alpha form (straight or
//! premultiplied) or transfer curve (sRGB or linear), so each effect can run in the
//! space it is correct in.

mod blend;
mod blur;
mod color_adjust;
mod convert;
mod feedback;
mod vignette;

pub use blend::{BLEND_MODES, Blend};
pub use blur::{BLUR_DIRECTIONS, GaussianBlur};
pub use color_adjust::ColorAdjust;
pub use convert::PixelConvert;
pub use feedback::FeedbackTrail;
pub use vignette::Vignette;
