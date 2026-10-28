//! Everything on the effects side of the preview, all on the GPU:
//! a test-pattern generator (or an image someone dropped on the window), the shared
//! plugin pipeline, a feedback trail after it, and a pass that turns the float result
//! into an 8-bit texture egui can draw.

use vfx_core::effects::FeedbackTrail;
use vfx_core::{EffectChain, Frame, Gpu, ParamValue};
use vfx_wrapper_common::pipeline::{FRAME_FORMAT, HEADLESS_DEVICE, Lease, Spec};

use crate::controls::Controls;

/// Generated patterns render at this size; dropped images keep their own.
pub const PATTERN_SIZE: (u32, u32) = (1280, 720);
/// Dropped images are scaled down to fit this, to keep the demo interactive.
const MAX_IMAGE_SIDE: u32 = 1920;
/// egui samples "normal" (not sRGB-aware) textures holding gamma-encoded colour,
/// which is what the pipeline produces.
const DISPLAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

pub const PATTERNS: &[&str] = &[
    "Moving disc",
    "Colour bars",
    "Checker",
    "Soft shapes (alpha)",
];

const GENERATOR_WGSL: &str = r#"
struct U { p: vec4<f32> } // x: pattern, y: time, zw: size
@group(0) @binding(0) var<uniform> u: U;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i & 1u) * 4.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn disc(p: vec2<f32>, c: vec2<f32>, r: f32) -> f32 {
    return clamp(r - distance(p, c), 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let size = u.p.zw;
    let t = u.p.y;
    let uv = pos.xy / size;
    let pattern = i32(u.p.x);
    if pattern == 1 {
        // Eight colour bars with a white line sweeping across.
        let bars = array<vec3<f32>, 8>(
            vec3(0.75), vec3(0.75, 0.75, 0.0), vec3(0.0, 0.75, 0.75), vec3(0.0, 0.75, 0.0),
            vec3(0.75, 0.0, 0.75), vec3(0.75, 0.0, 0.0), vec3(0.0, 0.0, 0.75), vec3(0.05));
        var c = bars[min(i32(uv.x * 8.0), 7)];
        let line = fract(t * 0.25) * size.x;
        if abs(pos.x - line) < 3.0 { c = vec3(1.0); }
        return vec4<f32>(c, 1.0);
    }
    if pattern == 2 {
        // Scrolling 64 px checker.
        let q = floor((pos.xy + vec2(t * 60.0, t * 25.0)) / 64.0);
        let on = (i32(q.x) + i32(q.y)) % 2 == 0;
        return select(vec4(0.08, 0.08, 0.1, 1.0), vec4(0.92, 0.9, 0.85, 1.0), on);
    }
    if pattern == 3 {
        // Soft coloured discs on a transparent background.
        let a = disc(pos.xy, size * vec2(0.3 + 0.1 * sin(t), 0.45), 180.0);
        let b = disc(pos.xy, size * vec2(0.65, 0.5 + 0.1 * cos(t * 0.7)), 220.0);
        let soft_a = smoothstep(0.0, 1.0, a / 60.0);
        let soft_b = smoothstep(0.0, 1.0, b / 60.0) * 0.6;
        let alpha = max(soft_a, soft_b);
        let c = select(vec3(0.2, 0.5, 1.0), vec3(1.0, 0.45, 0.15), soft_a >= soft_b);
        return vec4<f32>(c, alpha);
    }
    // Gradient with a bright disc moving around it.
    let c = size * vec2(0.5 + 0.35 * cos(t * 0.8), 0.5 + 0.35 * sin(t * 0.6));
    let d = disc(pos.xy, c, 60.0);
    let base = vec3(uv.x, uv.y, 0.15);
    return vec4<f32>(mix(base, vec3(1.0, 0.95, 0.8), clamp(d, 0.0, 1.0)), 1.0);
}
"#;

const DISPLAY_WGSL: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i & 1u) * 4.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(src, vec2<i32>(pos.xy), 0);
    // Show transparency over a dark checkerboard. The pipeline's output is straight
    // alpha and already gamma-encoded, as egui expects.
    let q = vec2<i32>(pos.xy) / 16;
    let bg = select(vec3(0.16), vec3(0.22), (q.x + q.y) % 2 == 0);
    let a = clamp(c.a, 0.0, 1.0);
    return vec4<f32>(mix(bg, clamp(c.rgb, vec3(0.0), vec3(1.0)), a), 1.0);
}
"#;

/// A fullscreen-triangle pass with one bind group.
struct Pass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

impl Pass {
    fn new(
        device: &wgpu::Device,
        wgsl: &str,
        entry: wgpu::BindingType,
        target: wgpu::TextureFormat,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: entry,
                count: None,
            }],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, layout }
    }

    fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        bind: &wgpu::BindGroup,
        target: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Size-dependent frames, rebuilt when the source size changes.
struct Frames {
    size: (u32, u32),
    source: Frame,
    layer: Frame,
    feedback_out: Frame,
    /// Keeps its texture alive.
    display_view: wgpu::TextureView,
}

impl Frames {
    fn new(gpu: &Gpu, size: (u32, u32)) -> Self {
        let display = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("preview display"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DISPLAY_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let display_view = display.create_view(&Default::default());
        Self {
            size,
            source: Frame::new(gpu, size.0, size.1, FRAME_FORMAT),
            layer: Frame::new(gpu, size.0, size.1, FRAME_FORMAT),
            feedback_out: Frame::new(gpu, size.0, size.1, FRAME_FORMAT),
            display_view,
        }
    }
}

pub struct Effects {
    frames: Frames,
    /// A dropped image, uploaded once, used instead of the generated pattern.
    image: Option<(String, Frame)>,
    generator: Pass,
    display_pass: Pass,
    feedback: EffectChain,
}

impl Effects {
    pub fn new(gpu: &Gpu) -> Self {
        let generator = Pass::new(
            &gpu.device,
            GENERATOR_WGSL,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            FRAME_FORMAT,
        );
        let display_pass = Pass::new(
            &gpu.device,
            DISPLAY_WGSL,
            wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            DISPLAY_FORMAT,
        );
        let mut effects = Self {
            frames: Frames::new(gpu, PATTERN_SIZE),
            image: None,
            generator,
            display_pass,
            feedback: EffectChain::new(),
        };
        effects.rebuild_feedback(gpu);
        effects
    }

    fn rebuild_feedback(&mut self, gpu: &Gpu) {
        self.feedback = EffectChain::new();
        self.feedback.push(Box::new(FeedbackTrail::default()));
        self.feedback.prepare(gpu, FRAME_FORMAT, self.frames.size);
    }

    fn set_size(&mut self, gpu: &Gpu, size: (u32, u32)) {
        if self.frames.size != size {
            self.frames = Frames::new(gpu, size);
            self.rebuild_feedback(gpu);
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.frames.size
    }

    /// The texture egui shows. Changes identity when the size changes.
    pub fn display_view(&self) -> &wgpu::TextureView {
        &self.frames.display_view
    }

    pub fn image_name(&self) -> Option<&str> {
        self.image.as_ref().map(|(name, _)| name.as_str())
    }

    /// Loads a PNG or JPEG as the source, scaled to fit MAX_IMAGE_SIDE.
    pub fn load_image(&mut self, gpu: &Gpu, path: &std::path::Path) -> Result<(), String> {
        let img = image::open(path).map_err(|e| e.to_string())?;
        let img = if img.width().max(img.height()) > MAX_IMAGE_SIDE {
            img.resize(
                MAX_IMAGE_SIDE,
                MAX_IMAGE_SIDE,
                image::imageops::FilterType::Triangle,
            )
        } else {
            img
        };
        // Straight alpha, gamma-encoded: the pipeline's input convention.
        let rgba = img.to_rgba32f();
        let size = rgba.dimensions();
        self.set_size(gpu, size);
        let frame = Frame::new(gpu, size.0, size.1, FRAME_FORMAT);
        frame.write(gpu, bytemuck::cast_slice(rgba.as_raw()));
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.image = Some((name, frame));
        Ok(())
    }

    /// Goes back to the generated patterns.
    pub fn clear_image(&mut self, gpu: &Gpu) {
        if self.image.take().is_some() {
            self.set_size(gpu, PATTERN_SIZE);
        }
    }

    fn generate(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        pattern: usize,
        time: f64,
        target: &Frame,
    ) {
        let (w, h) = self.frames.size;
        // Each pattern gets its own small buffer write; queue writes land before the
        // encoder's commands, so source and layer can't share one buffer in a frame.
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pattern uniform"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        gpu.queue.write_buffer(
            &uniform,
            0,
            bytemuck::bytes_of(&[pattern as f32, time as f32, w as f32, h as f32]),
        );
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.generator.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        self.generator.draw(encoder, &bind, &target.view);
    }

    /// Renders one frame with the panel's current settings.
    pub fn render(&mut self, gpu: &Gpu, controls: &Controls, time: f64) {
        let (w, h) = self.frames.size;
        let extent = wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        };
        let settings = controls.settings(self.frames.size);
        let spec = Spec {
            settings: &settings,
            width: w,
            height: h,
            has_layer: controls.blend_on,
            canvas_origin: [0.0, 0.0],
        };
        let mut lease = Lease::new(gpu, HEADLESS_DEVICE, &spec);

        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let source = match &self.image {
            Some((_, frame)) => frame,
            None => {
                self.generate(
                    gpu,
                    &mut encoder,
                    controls.pattern,
                    time,
                    &self.frames.source,
                );
                &self.frames.source
            }
        };
        encoder.copy_texture_to_texture(
            source.texture.as_image_copy(),
            lease.input().texture.as_image_copy(),
            extent,
        );
        if controls.blend_on {
            self.generate(
                gpu,
                &mut encoder,
                controls.blend_pattern,
                time,
                &self.frames.layer,
            );
            encoder.copy_texture_to_texture(
                self.frames.layer.texture.as_image_copy(),
                lease.layer().texture.as_image_copy(),
                extent,
            );
        }
        gpu.queue.submit(Some(encoder.finish()));
        lease.render(gpu, time);

        let result = if controls.feedback_on {
            self.feedback
                .params_mut(0)
                .set("decay", ParamValue::Float(controls.feedback_decay))
                .expect("feedback has a float decay");
            self.feedback
                .render(gpu, lease.output(), &self.frames.feedback_out, time);
            &self.frames.feedback_out
        } else {
            lease.output()
        };

        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.display_pass.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&result.view),
            }],
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        self.display_pass
            .draw(&mut encoder, &bind, &self.frames.display_view);
        gpu.queue.submit(Some(encoder.finish()));
    }
}
