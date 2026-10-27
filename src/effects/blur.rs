use crate::shader::{self, Uniform};
use crate::{Effect, Frame, Gpu, ParamKind, ParamSpec, ParamValues};

pub const BLUR_DIRECTIONS: &[&str] = &["Both", "Horizontal", "Vertical"];

const PARAMS: &[ParamSpec] = &[
    ParamSpec {
        name: "radius",
        kind: ParamKind::Float {
            min: 0.0,
            max: 256.0,
            default: 4.0,
        },
    },
    ParamSpec {
        name: "direction",
        kind: ParamKind::Int {
            min: 0,
            max: 2,
            default: 0,
            options: BLUR_DIRECTIONS,
        },
    },
    // Multiplies the vertical radius. Hosts use it when the horizontal and vertical
    // pixel scales differ, such as a preview downsampled unevenly.
    ParamSpec {
        name: "vertical_scale",
        kind: ParamKind::Float {
            min: 0.0,
            max: 4.0,
            default: 1.0,
        },
    },
];

// Gaussian with sigma = radius / 2, normalised per pass. Edge taps clamp to the frame.
const WGSL: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(2) var<uniform> u: vec4<f32>; // xy: unit step along one axis, z: radius in pixels

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let last = vec2<i32>(textureDimensions(src)) - vec2<i32>(1);
    let center = vec2<i32>(pos.xy);
    let dir = vec2<i32>(u.xy);
    let radius = i32(ceil(u.z));
    let sigma = max(u.z * 0.5, 0.001);
    var sum = vec4<f32>(0.0);
    var weight_sum = 0.0;
    for (var i = -radius; i <= radius; i++) {
        let x = f32(i);
        let w = exp(-(x * x) / (2.0 * sigma * sigma));
        let p = clamp(center + dir * i, vec2<i32>(0), last);
        sum += w * textureLoad(src, p, 0);
        weight_sum += w;
    }
    return sum / weight_sum;
}
"#;

#[derive(Default)]
pub struct GaussianBlur {
    built: Option<Built>,
}

struct Built {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    // One uniform per pass, because queued writes all land before the submit.
    uniforms: [Uniform; 2],
    scratch: Frame,
}

impl Effect for GaussianBlur {
    fn params(&self) -> &[ParamSpec] {
        PARAMS
    }

    fn prepare(&mut self, gpu: &Gpu, format: wgpu::TextureFormat, size: (u32, u32)) {
        let layout = shader::bind_layout(&gpu.device, 1);
        let pipeline = shader::pipeline(&gpu.device, "gaussian blur", WGSL, format, &layout);
        self.built = Some(Built {
            pipeline,
            layout,
            uniforms: [Uniform::new(&gpu.device), Uniform::new(&gpu.device)],
            scratch: Frame::new(gpu, size.0, size.1, format),
        });
    }

    fn render(
        &mut self,
        gpu: &Gpu,
        input: &Frame,
        output: &Frame,
        params: &ParamValues,
        _time: f64,
    ) {
        let built = self.built.as_ref().expect("prepare before render");
        let radius = params.float("radius");
        let direction = params.int("direction");
        // A zero radius makes a pass a straight copy, which switches that axis off.
        let horizontal = if direction == 2 { 0.0 } else { radius };
        let vertical = if direction == 1 {
            0.0
        } else {
            radius * params.float("vertical_scale")
        };
        let passes = [
            (input, &built.scratch, [1.0, 0.0], horizontal),
            (&built.scratch, output, [0.0, 1.0], vertical),
        ];

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        for (uniform, (src, dst, dir, radius)) in built.uniforms.iter().zip(passes) {
            uniform.write(gpu, [dir[0], dir[1], radius, 0.0]);
            let bind = shader::bind_group(&gpu.device, &built.layout, &[&src.view], uniform);
            shader::fullscreen_pass(&mut encoder, &built.pipeline, &bind, &dst.view);
        }
        gpu.queue.submit(Some(encoder.finish()));
    }
}
