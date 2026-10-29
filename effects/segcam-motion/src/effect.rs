//! segcam Motion as a vfx-core [`CpuEffect`]: its controls, and a render at whatever depth
//! the host works in. Hosts fetch (or keep) the frames before the current one that
//! [`CpuEffect::frames_before`] asks for.

use vfx_core::cpu::{FrameSource, ImageMut};
use vfx_core::{Context, CpuEffect, ParamSpec, ParamValues, Unit};

use crate::pixel::{Depth, ViewMut};
use crate::{Advance, Color, Incremental, MotionSettings, OutputMode, RenderSettings};

/// The controls, with segcam's units (percentages are 0-100), ranges and defaults
/// (those of [`RenderSettings::default`]).
pub const PARAMS: &[ParamSpec] = &[
    ParamSpec::choice("output", &OutputMode::NAMES, 0).label("Output"),
    ParamSpec::float("sensitivity", 0.0, 100.0, 86.4)
        .label("Sensitivity")
        .group("Motion")
        .unit(Unit::Percent),
    ParamSpec::float("adaptation", 0.1, 50.0, 6.0)
        .label("Adaptation")
        .group("Motion")
        .unit(Unit::Percent)
        .slider(0.1, 25.0),
    ParamSpec::int("background", 1, 120, 16)
        .label("Background")
        .group("Motion")
        .slider(1.0, 60.0),
    ParamSpec::float("min_size", 0.0, 25.0, 0.25)
        .label("Minimum size")
        .group("Motion")
        .unit(Unit::Percent)
        .slider(0.0, 5.0),
    ParamSpec::int("max_segments", 1, 256, 16)
        .label("Max segments")
        .group("Motion")
        .slider(1.0, 64.0),
    ParamSpec::int("grid", 32, 1024, 192)
        .label("Grid width")
        .group("Motion")
        .slider(64.0, 384.0),
    ParamSpec::toggle("mirror", false).label("Mirror"),
    ParamSpec::toggle("boxes", true)
        .label("Boxes")
        .group("Overlay"),
    ParamSpec::toggle("labels", true)
        .label("Labels")
        .group("Overlay"),
    ParamSpec::color("box_color", [0.25, 1.0, 0.4, 1.0])
        .label("Box color")
        .group("Overlay"),
    ParamSpec::int("trail", 1, 120, 24)
        .label("Trail frames")
        .group("Swarm")
        .slider(1.0, 60.0),
    ParamSpec::int("max_tiles", 1, 1000, 100)
        .label("Swarm max tiles")
        .group("Swarm")
        .slider(1.0, 300.0),
];

/// One frame's settings. `premultiplied` says how the host's frames store colour.
pub fn settings(p: &ParamValues, premultiplied: bool) -> RenderSettings {
    let [r, g, b, _] = p.color("box_color");
    RenderSettings {
        mode: OutputMode::from_index(p.int("output") + 1).unwrap_or(OutputMode::Overlay),
        motion: MotionSettings {
            sensitivity: f64::from(p.float("sensitivity")) / 100.0,
            adaptation: p.float("adaptation") / 100.0,
            min_area_fraction: f64::from(p.float("min_size")) / 100.0,
            max_segments: p.int("max_segments"),
        },
        background_frames: p.int("background"),
        grid_width: p.int("grid"),
        mirror: p.bool("mirror"),
        boxes: p.bool("boxes"),
        labels: p.bool("labels"),
        straight_alpha: !premultiplied,
        box_color: Color { r, g, b },
        trail_frames: p.int("trail"),
        max_tiles: p.int("max_tiles"),
    }
}

/// segcam Motion. It keeps what it works out for each frame (see [`Incremental`]), so a host
/// that keeps the instance reads one new frame per render.
#[derive(Default)]
pub struct SegcamMotion {
    state: Incremental,
}

impl CpuEffect for SegcamMotion {
    fn params(&self) -> &[ParamSpec] {
        PARAMS
    }

    fn keeps_state(&self) -> bool {
        true
    }

    fn frames_before(&self, params: &ParamValues) -> usize {
        crate::previous_frames_needed(&settings(params, false)) as usize
    }

    fn render(
        &mut self,
        frames: &mut dyn FrameSource,
        count: usize,
        output: ImageMut<'_>,
        params: &ParamValues,
        ctx: &Context,
    ) {
        let s = settings(params, ctx.premultiplied);
        let (state, n) = (&mut self.state, ctx.frame);
        match output {
            ImageMut::U8(mut out) => advance(state, &s, frames, count, n, &mut out),
            ImageMut::U16(mut out) => advance(state, &s, frames, count, n, &mut out),
            ImageMut::F32(mut out) => advance(state, &s, frames, count, n, &mut out),
        }
    }
}

/// Renders through what is kept, and again from cold if the source changed.
fn advance<C: Depth>(
    state: &mut Incremental,
    s: &RenderSettings,
    frames: &mut dyn FrameSource,
    count: usize,
    number: i64,
    out: &mut ViewMut<C>,
) {
    if state.advance(s, frames, count, number, out) == Advance::Discontinuity {
        let _ = state.advance(s, frames, count, number, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vfx_core::cpu::{Argb, History, ViewMut};

    #[test]
    fn defaults_are_the_cores() {
        let mut s = settings(&ParamValues::new(PARAMS), false);
        let core = RenderSettings {
            straight_alpha: true,
            ..RenderSettings::default()
        };
        // Parameters are f32, so 86.4% arrives as 0.86400002: the same threshold.
        assert_eq!(s.motion.threshold(), core.motion.threshold());
        s.motion.sensitivity = core.motion.sensitivity;
        assert_eq!(s, core);
    }

    #[test]
    fn labels_fit_ffgl() {
        for p in PARAMS {
            assert!(p.display_label().len() < 16, "{}", p.display_label());
        }
    }

    /// Driven the way a streaming host drives it: one frame at a time, kept in a history.
    #[test]
    fn finds_motion_through_the_effect_interface() {
        let (w, h) = (160, 90);
        let grey = |v: u8| Argb {
            alpha: 255,
            red: v,
            green: v,
            blue: v,
        };
        let mut effect = SegcamMotion::default();
        let mut params = ParamValues::new(PARAMS);
        params
            .set("output", vfx_core::ParamValue::Int(1))
            .expect("output is an int");
        let mut history = History::<u8>::default();
        history.resize(w, h);
        let mut out = vec![Argb::<u8>::default(); (w * h) as usize];
        for f in 0..8 {
            let frame = history.next(effect.frames_before(&params));
            frame.fill(grey(80));
            for y in 20..44 {
                for x in 10 + f * 12..34 + f * 12 {
                    frame[(y * w + x) as usize] = grey(250);
                }
            }
            let ctx = Context {
                frame: history.frame_number(),
                ..Context::at(f64::from(f) / 30.0)
            };
            let count = history.len();
            let view = ViewMut::from_pixels(&mut out, w, h);
            effect.render(&mut history, count, ImageMut::U8(view), &params, &ctx);
        }
        let at = |x: i32, y: i32| out[(y * w + x) as usize].alpha;
        assert_eq!(at(2, 2), 0, "still background is cut out");
        assert_eq!(at(10 + 7 * 12 + 12, 32), 255, "the square is kept");
    }
}
