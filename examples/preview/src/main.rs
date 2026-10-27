//! Live preview: a CPU-generated test pattern runs through ColorAdjust → GaussianBlur
//! → FeedbackTrail on the GPU and is shown in a window.
//!
//! Keys: 1/2/3 select a stage. Up/Down change its main parameter.
//! Escape or closing the window quits.
//!
//! The window is a separate crate so vfx-core never depends on windowing.

use std::sync::Arc;

use vfx_core::effects::{ColorAdjust, FeedbackTrail, GaussianBlur};
use vfx_core::{EffectChain, Frame, Gpu, ParamValue};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowAttributes, WindowId};

const FRAME_W: u32 = 640;
const FRAME_H: u32 = 360;
const FRAME_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Each stage's main parameter, with the step used for one key press.
const STAGES: [(&str, &str, f32); 3] = [
    ("color adjust", "brightness", 0.05),
    ("blur", "radius", 0.5),
    ("feedback", "decay", 0.05),
];

const BLIT_WGSL: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<uniform> dst_size: vec4<f32>; // xy: window size in pixels

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i & 1u) * 4.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    // Nearest-neighbour scale from the effect frame to the window.
    let src_size = vec2<f32>(textureDimensions(src));
    let p = vec2<i32>(pos.xy * src_size / dst_size.xy);
    return textureLoad(src, p, 0);
}
"#;

struct Preview {
    window: Arc<dyn Window>,
    gpu: Gpu,
    surface: wgpu::Surface<'static>,
    surface_format: wgpu::TextureFormat,
    blit_layout: wgpu::BindGroupLayout,
    blit_pipeline: wgpu::RenderPipeline,
    blit_size: wgpu::Buffer,
    chain: EffectChain,
    input: Frame,
    output: Frame,
    selected: usize,
    frame: u32,
}

impl Preview {
    fn new(event_loop: &dyn ActiveEventLoop) -> Self {
        let window: Arc<dyn Window> = Arc::from(
            event_loop
                .create_window(WindowAttributes::default().with_title("vfx preview"))
                .expect("create window"),
        );

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(event_loop.owned_display_handle()),
        ));
        let surface = instance
            .create_surface(window.clone())
            .expect("create window surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .expect("no GPU adapter for the window");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("request device");
        let gpu = Gpu::from_parts(device, queue);

        let size = window.surface_size();
        let caps = surface.get_capabilities(&adapter);
        let surface_format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        surface.configure(
            &gpu.device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: surface_format,
                width: size.width.max(1),
                height: size.height.max(1),
                present_mode: wgpu::PresentMode::AutoVsync,
                desired_maximum_frame_latency: 2,
                alpha_mode: caps.alpha_modes[0],
                view_formats: vec![],
                color_space: Default::default(),
            },
        );

        let mut chain = EffectChain::new();
        chain.push(Box::new(ColorAdjust::default()));
        chain.push(Box::new(GaussianBlur::default()));
        chain.push(Box::new(FeedbackTrail::default()));
        chain.prepare(&gpu, FRAME_FORMAT, (FRAME_W, FRAME_H));

        let (blit_layout, blit_pipeline) = blit_pipeline(&gpu.device, surface_format);
        let blit_size = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("blit size"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let input = Frame::new(&gpu, FRAME_W, FRAME_H, FRAME_FORMAT);
        let output = Frame::new(&gpu, FRAME_W, FRAME_H, FRAME_FORMAT);

        let preview = Self {
            window,
            gpu,
            surface,
            surface_format,
            blit_layout,
            blit_pipeline,
            blit_size,
            chain,
            input,
            output,
            selected: 0,
            frame: 0,
        };
        preview.update_title();
        preview
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.surface.configure(
            &self.gpu.device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: self.surface_format,
                width: width.max(1),
                height: height.max(1),
                present_mode: wgpu::PresentMode::AutoVsync,
                desired_maximum_frame_latency: 2,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                view_formats: vec![],
                color_space: Default::default(),
            },
        );
    }

    /// Changes the selected stage's main parameter by one step.
    fn nudge(&mut self, direction: f32) {
        let (_, name, step) = STAGES[self.selected];
        let params = self.chain.params_mut(self.selected);
        let current = match params.get(name) {
            Some(ParamValue::Float(v)) => v,
            other => panic!("expected float parameter `{name}`, found {other:?}"),
        };
        params
            .set(name, ParamValue::Float(current + direction * step))
            .expect("parameter exists and is a float");
        self.update_title();
    }

    fn update_title(&self) {
        let (label, name, _) = STAGES[self.selected];
        let value = match self.chain.params(self.selected).get(name) {
            Some(ParamValue::Float(v)) => v,
            other => panic!("expected float parameter `{name}`, found {other:?}"),
        };
        self.window.set_title(&format!(
            "vfx preview · {label}: {name} = {value:.2}  (1-3 select, ↑↓ adjust)"
        ));
    }

    fn draw(&mut self) {
        let t = self.frame as f32 / 60.0;
        self.frame = self.frame.wrapping_add(1);
        let pixels = test_pattern(t);
        self.input.write(&self.gpu, &pixels);
        self.chain
            .render(&self.gpu, &self.input, &self.output, f64::from(t));

        let texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            _ => {
                let size = self.window.surface_size();
                self.resize(size.width, size.height);
                return;
            }
        };
        let view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let size = texture.texture.size();
        self.gpu.queue.write_buffer(
            &self.blit_size,
            0,
            bytemuck::bytes_of(&[size.width as f32, size.height as f32, 0.0, 0.0]),
        );
        let bind = self
            .gpu
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("blit"),
                layout: &self.blit_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&self.output.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.blit_size.as_entire_binding(),
                    },
                ],
            });

        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit to window"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.blit_pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.gpu.queue.submit(Some(encoder.finish()));
        self.window.pre_present_notify();
        self.gpu.queue.present(texture);
    }
}

/// Moving gradient with a bright disc, built on the CPU each frame.
fn test_pattern(t: f32) -> Vec<u8> {
    let mut pixels = vec![0u8; (FRAME_W * FRAME_H * 4) as usize];
    let cx = FRAME_W as f32 * (0.5 + 0.35 * (t * 0.8).cos());
    let cy = FRAME_H as f32 * (0.5 + 0.35 * (t * 0.6).sin());
    for y in 0..FRAME_H {
        for x in 0..FRAME_W {
            let i = ((y * FRAME_W + x) * 4) as usize;
            let (fx, fy) = (x as f32, y as f32);
            let inside = (fx - cx).hypot(fy - cy) < 40.0;
            pixels[i] = (fx / FRAME_W as f32 * 255.0) as u8;
            pixels[i + 1] = (fy / FRAME_H as f32 * 255.0) as u8;
            pixels[i + 2] = if inside { 255 } else { 40 };
            pixels[i + 3] = 255;
        }
    }
    pixels
}

fn blit_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> (wgpu::BindGroupLayout, wgpu::RenderPipeline) {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("blit layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("blit"),
        source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("blit"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("blit"),
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
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    (layout, pipeline)
}

#[derive(Default)]
struct App {
    preview: Option<Preview>,
}

impl ApplicationHandler for App {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.preview.is_none() {
            self.preview = Some(Preview::new(event_loop));
        }
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        let Some(preview) = self.preview.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::SurfaceResized(size) => preview.resize(size.width, size.height),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                match event.logical_key.as_ref() {
                    Key::Named(NamedKey::Escape) => event_loop.exit(),
                    Key::Character("1") => preview.selected = 0,
                    Key::Character("2") => preview.selected = 1,
                    Key::Character("3") => preview.selected = 2,
                    Key::Named(NamedKey::ArrowUp) => preview.nudge(1.0),
                    Key::Named(NamedKey::ArrowDown) => preview.nudge(-1.0),
                    _ => {}
                }
            }
            WindowEvent::RedrawRequested => {
                preview.draw();
                preview.window.request_redraw();
            }
            _ => {}
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::new()?;
    event_loop.run_app(App::default())?;
    Ok(())
}
