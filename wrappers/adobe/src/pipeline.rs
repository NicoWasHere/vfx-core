//! Builds the core effect chain for a frame's settings, and keeps prepared chains
//! for reuse. Creating pipelines and textures is the expensive part of a render, so a
//! chain is built once per (device, size, stages) and then reused.
//!
//! Each render leases a chain from the pool and returns it when the lease drops. With
//! Multi-Frame Rendering several renders run at once, and each one gets its own chain.
//! The CPU path fills the lease's frames from RAM; the GPU path fills them on the GPU.

use std::sync::Mutex;

use vfx_core::effects::{Blend, ColorAdjust, GaussianBlur, PixelConvert, Vignette};
use vfx_core::{EffectChain, Frame, Gpu, ParamValue};

use crate::params::Settings;

/// Render format. Float keeps HDR values intact. 8- and 16-bit input is converted on the CPU.
pub const FRAME_FORMAT: vfx_core::wgpu::TextureFormat = vfx_core::wgpu::TextureFormat::Rgba32Float;

/// Prepared chains kept for reuse. Enough for a few sizes × concurrent renders.
const POOL_LIMIT: usize = 8;

/// Device key for the plugin's own headless device (the CPU path).
pub const HEADLESS_DEVICE: usize = 0;

static POOL: Mutex<Vec<Prepared>> = Mutex::new(Vec::new());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stages {
    linear: bool,
    color: bool,
    vignette: bool,
    blend: bool,
    blur: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    /// Which wgpu device the chain was built on. Chains can't move between devices.
    device: usize,
    width: u32,
    height: u32,
    stages: Stages,
}

/// Stage indices in the chain, for the stages that have parameters.
#[derive(Default)]
struct Indices {
    color: Option<usize>,
    vignette: Option<usize>,
    blend: Option<usize>,
    blur: Option<usize>,
}

struct Prepared {
    key: Key,
    chain: EffectChain,
    indices: Indices,
    input: Frame,
    layer: Frame,
    output: Frame,
}

/// What a frame needs, independent of where its pixels live. All buffers are
/// `width` × `height` RGBA f32, straight alpha.
pub struct Spec<'a> {
    pub settings: &'a Settings,
    pub width: u32,
    pub height: u32,
    pub has_layer: bool,
    /// The canvas's top-left in layer pixel coordinates, for point parameters.
    pub canvas_origin: [f32; 2],
}

/// A prepared chain borrowed from the pool, configured for one frame.
/// Fill `input` (and `layer`), call [`Lease::render`], then read `output`.
pub struct Lease {
    prepared: Option<Prepared>,
}

impl Lease {
    pub fn new(gpu: &Gpu, device: usize, spec: &Spec) -> Self {
        let s = spec.settings;
        let key = Key {
            device,
            width: spec.width,
            height: spec.height,
            stages: Stages {
                linear: s.linear,
                color: s.color.is_some(),
                vignette: s.vignette.is_some(),
                blend: spec.has_layer,
                blur: s.blur.is_some(),
            },
        };
        let mut prepared = take(gpu, key);
        configure(&mut prepared, spec);
        Self {
            prepared: Some(prepared),
        }
    }

    fn get(&self) -> &Prepared {
        self.prepared
            .as_ref()
            .expect("lease holds a chain until dropped")
    }

    pub fn input(&self) -> &Frame {
        &self.get().input
    }

    pub fn layer(&self) -> &Frame {
        &self.get().layer
    }

    pub fn output(&self) -> &Frame {
        &self.get().output
    }

    pub fn render(&mut self, gpu: &Gpu, time: f64) {
        let p = self
            .prepared
            .as_mut()
            .expect("lease holds a chain until dropped");
        let layers: Vec<&Frame> = if p.key.stages.blend {
            vec![&p.layer]
        } else {
            Vec::new()
        };
        p.chain
            .render_with_layers(gpu, &p.input, &layers, &p.output, time);
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(prepared) = self.prepared.take() {
            give_back(prepared);
        }
    }
}

/// One frame of CPU-path work: pixels come from and go back to RAM.
pub struct Job<'a> {
    pub settings: &'a Settings,
    pub width: u32,
    pub height: u32,
    pub input: &'a [f32],
    pub layer: Option<&'a [f32]>,
    /// The canvas's top-left in layer pixel coordinates, for point parameters.
    pub canvas_origin: [f32; 2],
    pub time: f64,
}

pub fn run(gpu: &Gpu, job: &Job) -> Vec<f32> {
    let spec = Spec {
        settings: job.settings,
        width: job.width,
        height: job.height,
        has_layer: job.layer.is_some(),
        canvas_origin: job.canvas_origin,
    };
    let mut lease = Lease::new(gpu, HEADLESS_DEVICE, &spec);
    lease.input().write(gpu, bytemuck::cast_slice(job.input));
    if let Some(layer) = job.layer {
        lease.layer().write(gpu, bytemuck::cast_slice(layer));
    }
    lease.render(gpu, job.time);
    bytemuck::pod_collect_to_vec(&lease.output().read(gpu))
}

/// Copies this frame's values into the chain's stages.
fn configure(prepared: &mut Prepared, spec: &Spec) {
    let s = spec.settings;
    let chain = &mut prepared.chain;
    let mut set = |stage: Option<usize>, name: &str, value: ParamValue| {
        if let Some(stage) = stage {
            chain
                .params_mut(stage)
                .set(name, value)
                .expect("wrapper and core agree on parameter names and types");
        }
    };
    if let Some([brightness, contrast, saturation]) = s.color {
        set(
            prepared.indices.color,
            "brightness",
            ParamValue::Float(brightness),
        );
        set(
            prepared.indices.color,
            "contrast",
            ParamValue::Float(contrast),
        );
        set(
            prepared.indices.color,
            "saturation",
            ParamValue::Float(saturation),
        );
    }
    if let Some(v) = s.vignette {
        // The core wants the centre in normalised canvas coordinates.
        let center = [
            (v.center_px[0] - spec.canvas_origin[0]) / spec.width as f32,
            (v.center_px[1] - spec.canvas_origin[1]) / spec.height as f32,
        ];
        set(
            prepared.indices.vignette,
            "center",
            ParamValue::Point(center),
        );
        set(
            prepared.indices.vignette,
            "radius",
            ParamValue::Float(v.radius),
        );
        set(
            prepared.indices.vignette,
            "softness",
            ParamValue::Float(v.softness),
        );
        set(
            prepared.indices.vignette,
            "color",
            ParamValue::Color(v.color),
        );
        set(
            prepared.indices.vignette,
            "amount",
            ParamValue::Float(v.amount),
        );
    }
    set(
        prepared.indices.blend,
        "mode",
        ParamValue::Int(s.blend_mode),
    );
    set(
        prepared.indices.blend,
        "amount",
        ParamValue::Float(s.blend_amount),
    );
    set(
        prepared.indices.blend,
        "linearize_layer",
        ParamValue::Bool(s.linear),
    );
    if let Some(b) = s.blur {
        set(prepared.indices.blur, "radius", ParamValue::Float(b.radius));
        set(
            prepared.indices.blur,
            "direction",
            ParamValue::Int(b.direction),
        );
        set(
            prepared.indices.blur,
            "vertical_scale",
            ParamValue::Float(b.vertical_scale),
        );
    }
}

fn take(gpu: &Gpu, key: Key) -> Prepared {
    let found = {
        let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
        pool.iter()
            .position(|p| p.key == key)
            .map(|i| pool.swap_remove(i))
    };
    found.unwrap_or_else(|| build(gpu, key))
}

fn give_back(prepared: Prepared) {
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    if pool.len() >= POOL_LIMIT {
        pool.remove(0);
    }
    pool.push(prepared);
}

/// Stage order: decode to linear light (optional), straight-alpha looks, then the blur
/// on premultiplied alpha so transparent pixels don't bleed colour into soft edges,
/// then back to straight alpha and the original transfer curve.
fn build(gpu: &Gpu, key: Key) -> Prepared {
    let st = key.stages;
    let mut chain = EffectChain::new();
    let mut indices = Indices::default();
    if st.linear {
        chain.push(Box::new(PixelConvert::srgb_to_linear()));
    }
    if st.color {
        indices.color = Some(chain.push(Box::new(ColorAdjust::default())));
    }
    if st.vignette {
        indices.vignette = Some(chain.push(Box::new(Vignette::default())));
    }
    if st.blend {
        indices.blend = Some(chain.push(Box::new(Blend::default())));
    }
    if st.blur {
        chain.push(Box::new(PixelConvert::premultiply()));
        indices.blur = Some(chain.push(Box::new(GaussianBlur::default())));
        chain.push(Box::new(PixelConvert::unpremultiply()));
    }
    if st.linear {
        chain.push(Box::new(PixelConvert::linear_to_srgb()));
    }
    let size = (key.width, key.height);
    chain.prepare(gpu, FRAME_FORMAT, size);
    Prepared {
        key,
        chain,
        indices,
        input: Frame::new(gpu, size.0, size.1, FRAME_FORMAT),
        layer: Frame::new(gpu, size.0, size.1, FRAME_FORMAT),
        output: Frame::new(gpu, size.0, size.1, FRAME_FORMAT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{Blur, Settings};

    fn gpu() -> Gpu {
        pollster::block_on(Gpu::headless()).expect("no GPU adapter available for tests")
    }

    fn settings() -> Settings {
        Settings {
            linear: false,
            color: None,
            vignette: None,
            blend_mode: 0,
            blend_amount: 1.0,
            blur: None,
        }
    }

    fn job<'a>(s: &'a Settings, w: u32, h: u32, input: &'a [f32]) -> Job<'a> {
        Job {
            settings: s,
            width: w,
            height: h,
            input,
            layer: None,
            canvas_origin: [0.0, 0.0],
            time: 0.0,
        }
    }

    #[test]
    fn no_stages_passes_through() {
        let gpu = gpu();
        let s = settings();
        let input = [0.1, 0.2, 0.3, 0.4, 2.0, 0.0, -0.5, 1.0];
        assert_eq!(run(&gpu, &job(&s, 2, 1, &input)), input.to_vec());
    }

    #[test]
    fn blur_does_not_bleed_transparent_colour() {
        // Opaque red pixel between fully transparent green ones. Blurring straight alpha
        // would tint the red pixel green. Blurring premultiplied alpha keeps it pure red.
        let gpu = gpu();
        let mut s = settings();
        s.blur = Some(Blur {
            radius: 1.0,
            vertical_scale: 1.0,
            direction: 1,
            grow: false,
        });
        let input = [
            0.0, 1.0, 0.0, 0.0, // transparent green
            1.0, 0.0, 0.0, 1.0, // opaque red
            0.0, 1.0, 0.0, 0.0, // transparent green
        ];
        let out = run(&gpu, &job(&s, 3, 1, &input));
        let red = &out[4..8];
        assert!((red[0] - 1.0).abs() < 1e-5, "red stays 1.0: {red:?}");
        assert!(red[1].abs() < 1e-6, "no green bleed: {red:?}");
        assert!(red[3] < 1.0 && red[3] > 0.0, "alpha softens: {red:?}");
    }

    #[test]
    fn repeated_runs_give_identical_results() {
        let gpu = gpu();
        let mut s = settings();
        s.color = Some([0.25, 1.0, 1.0]);
        let input = [0.5, 0.5, 0.5, 1.0];
        let first = run(&gpu, &job(&s, 1, 1, &input));
        let second = run(&gpu, &job(&s, 1, 1, &input));
        assert_eq!(first, second);
        assert!((first[0] - 0.75).abs() < 1e-6, "{first:?}");
    }

    #[test]
    fn linear_light_blend_decodes_the_layer() {
        // Normal blend at full strength, in linear light: the output is the layer, round
        // tripped through linear, so it must come back unchanged (not double-encoded).
        let gpu = gpu();
        let mut s = settings();
        s.linear = true;
        let base = [0.2, 0.2, 0.2, 1.0];
        let layer = [0.5, 0.25, 0.75, 1.0];
        let mut j = job(&s, 1, 1, &base);
        j.layer = Some(&layer);
        let out = run(&gpu, &j);
        for (got, want) in out.iter().zip(layer) {
            assert!((got - want).abs() < 1e-4, "{out:?}");
        }
    }
}
