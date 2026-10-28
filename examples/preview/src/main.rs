//! Standalone preview of the vfx-core effect chain, with a control panel.
//!
//! The image is a generated test pattern (or a PNG / JPEG dropped on the window). It
//! runs through the same pipeline the After Effects, OpenFX and FFGL plugins use, plus
//! a feedback trail, all on the GPU. The panel on the right drives every parameter;
//! clicking the image moves the vignette centre. Escape quits.
//!
//! This is a separate crate so vfx-core never depends on windowing or UI code.

mod controls;
mod effects;

use std::sync::Arc;
use web_time::Instant;

use controls::{Controls, Status};
use effects::Effects;
use vfx_core::Gpu;
use winit::application::ApplicationHandler;
#[cfg(not(target_arch = "wasm32"))]
use winit::event::ElementState;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
#[cfg(not(target_arch = "wasm32"))]
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

struct Preview {
    window: Arc<Window>,
    gpu: Gpu,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    egui: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    effects: Effects,
    /// The effects output, registered with egui; re-registered when its size changes.
    image: egui::TextureId,
    image_size: (u32, u32),
    controls: Controls,
    time: f64,
    last_frame: Instant,
    fps: f32,
    message: Option<String>,
}

impl Preview {
    /// Opens the GPU for `window` and builds everything else. Async because browsers
    /// can't block on the GPU; the native build blocks on it with pollster.
    async fn new(window: Arc<Window>, instance: wgpu::Instance) -> Result<Self, String> {
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("couldn't create a surface: {e}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|e| format!("no GPU adapter: {e}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(|e| format!("couldn't open the GPU: {e}"))?;
        let gpu = Gpu::from_parts(device, queue);

        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        // Browsers offer only non-sRGB canvas formats; egui handles either.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: Default::default(),
        };
        surface.configure(&gpu.device, &config);

        let egui = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(gpu.device.limits().max_texture_dimension_2d as usize),
        );
        let mut egui_renderer =
            egui_wgpu::Renderer::new(&gpu.device, format, egui_wgpu::RendererOptions::default());

        let effects = Effects::new(&gpu);
        let image = egui_renderer.register_native_texture(
            &gpu.device,
            effects.display_view(),
            wgpu::FilterMode::Linear,
        );
        let image_size = effects.size();

        Ok(Self {
            window,
            gpu,
            surface,
            config,
            egui,
            egui_state,
            egui_renderer,
            effects,
            image,
            image_size,
            controls: Controls::default(),
            time: 0.0,
            last_frame: Instant::now(),
            fps: 0.0,
            message: None,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.gpu.device, &self.config);
    }

    /// Points egui at the effects output again after its size (and texture) changed.
    fn refresh_image(&mut self) {
        if self.effects.size() != self.image_size {
            self.egui_renderer.free_texture(&self.image);
            self.image = self.egui_renderer.register_native_texture(
                &self.gpu.device,
                self.effects.display_view(),
                wgpu::FilterMode::Linear,
            );
            self.image_size = self.effects.size();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn open(&mut self, path: &std::path::Path) {
        self.message = self
            .effects
            .load_image(&self.gpu, path)
            .err()
            .map(|e| format!("Couldn't open {}: {e}", path.display()));
        self.refresh_image();
    }

    fn draw(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f64();
        self.last_frame = now;
        if dt > 0.0 {
            self.fps = self.fps * 0.9 + (1.0 / dt as f32) * 0.1;
        }
        if self.controls.animate {
            self.time += dt;
        }

        self.effects.render(&self.gpu, &self.controls, self.time);

        let input = self.egui_state.take_egui_input(&self.window);
        let (image, image_size) = (self.image, self.image_size);
        let status = Status {
            fps: self.fps,
            size: image_size,
            image: self.effects.image_name(),
            message: self.message.as_deref(),
        };
        let controls = &mut self.controls;
        let mut actions = controls::Actions::default();
        let output = self.egui.run_ui(input, |ui| {
            egui::Panel::right("controls")
                .resizable(false)
                .exact_size(320.0)
                .show(ui, |ui| actions = controls::panel(ui, controls, &status));
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(egui::Color32::from_gray(18)))
                .show(ui, |ui| show_image(ui, image, image_size, controls));
        });
        if actions.clear_image {
            self.effects.clear_image(&self.gpu);
            self.message = None;
            self.refresh_image();
        }
        self.egui_state
            .handle_platform_output(&self.window, output.platform_output);
        // Apply texture updates before anything can skip the frame: egui sends each
        // update once (the font atlas only on the first frame), so a skipped update is
        // lost and text and fills disappear.
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                self.egui_renderer
                    .update_texture(&self.gpu.device, &self.gpu.queue, *id, delta);
            }
        }

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => {
                let size = self.window.inner_size();
                self.resize(size.width, size.height);
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let jobs = self.egui.tessellate(output.shapes, output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: output.pixels_per_point,
        };
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        let mut commands = self.egui_renderer.update_buffers(
            &self.gpu.device,
            &self.gpu.queue,
            &mut encoder,
            &jobs,
            &screen,
        );
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
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
            self.egui_renderer
                .render(&mut pass.forget_lifetime(), &jobs, &screen);
        }
        commands.push(encoder.finish());
        self.gpu.queue.submit(commands);
        self.window.pre_present_notify();
        self.gpu.queue.present(frame);
        for id in &output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
    }
}

/// The effects output, fitted into the central area. Clicking or dragging on it moves
/// the vignette centre.
fn show_image(
    ui: &mut egui::Ui,
    image: egui::TextureId,
    size: (u32, u32),
    controls: &mut Controls,
) {
    let area = ui.available_rect_before_wrap().shrink(12.0);
    let aspect = size.0 as f32 / size.1 as f32;
    let (w, h) = if area.width() / area.height() > aspect {
        (area.height() * aspect, area.height())
    } else {
        (area.width(), area.width() / aspect)
    };
    let rect = egui::Rect::from_center_size(area.center(), egui::vec2(w, h));
    let response = ui.put(
        rect,
        egui::Image::new(egui::load::SizedTexture::new(image, egui::vec2(w, h)))
            .sense(egui::Sense::click_and_drag()),
    );
    if (response.clicked() || response.dragged())
        && let Some(p) = response.interact_pointer_pos()
    {
        let n = (p - rect.min) / rect.size();
        controls.vignette_center = [n.x.clamp(0.0, 1.0), n.y.clamp(0.0, 1.0)];
        controls.vignette_on = true;
    }
}

/// Sent when the (async, on the web) GPU setup finishes.
enum Ready {
    Preview(Box<Preview>),
    Failed(String),
}

struct App {
    proxy: EventLoopProxy<Ready>,
    preview: Option<Preview>,
    starting: bool,
}

impl App {
    fn new(proxy: EventLoopProxy<Ready>) -> Self {
        Self {
            proxy,
            preview: None,
            starting: false,
        }
    }
}

fn window_attributes() -> winit::window::WindowAttributes {
    let attributes = Window::default_attributes().with_title("VFX Preview");
    #[cfg(not(target_arch = "wasm32"))]
    let attributes = attributes.with_inner_size(winit::dpi::LogicalSize::new(1440.0, 820.0));
    // On the web, winit makes a canvas; the page's CSS sizes it to the window.
    #[cfg(target_arch = "wasm32")]
    let attributes = {
        use winit::platform::web::WindowAttributesExtWebSys;
        attributes.with_append(true)
    };
    attributes
}

fn instance(event_loop: &ActiveEventLoop) -> wgpu::Instance {
    #[allow(unused_mut)]
    let mut descriptor = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(
        event_loop.owned_display_handle(),
    ));
    // The pipeline renders to RGBA32F textures, which WebGL can't, so the web build
    // needs WebGPU.
    #[cfg(target_arch = "wasm32")]
    {
        descriptor.backends = wgpu::Backends::BROWSER_WEBGPU;
    }
    wgpu::Instance::new(descriptor)
}

/// Shows a message in the page (web) or on stderr (native).
fn report(message: &str) {
    #[cfg(target_arch = "wasm32")]
    if let Some(status) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("status"))
    {
        status.set_text_content(Some(message));
        return;
    }
    eprintln!("{message}");
}

impl ApplicationHandler<Ready> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.preview.is_some() || self.starting {
            return;
        }
        self.starting = true;
        let window = match event_loop.create_window(window_attributes()) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                report(&format!("Couldn't create a window: {e}"));
                event_loop.exit();
                return;
            }
        };
        let setup = Preview::new(window, instance(event_loop));
        let proxy = self.proxy.clone();
        let finish = move |result: Result<Preview, String>| {
            let _ = proxy.send_event(match result {
                Ok(preview) => Ready::Preview(Box::new(preview)),
                Err(e) => Ready::Failed(e),
            });
        };
        #[cfg(not(target_arch = "wasm32"))]
        finish(pollster::block_on(setup));
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(async move { finish(setup.await) });
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Ready) {
        match event {
            Ready::Preview(preview) => {
                report("");
                preview.window.request_redraw();
                self.preview = Some(*preview);
            }
            Ready::Failed(e) => {
                report(&format!(
                    "VFX Preview needs WebGPU, which this browser didn't provide ({e}). \
                     Try a current Chrome, Edge or Safari."
                ));
                #[cfg(not(target_arch = "wasm32"))]
                event_loop.exit();
                #[cfg(target_arch = "wasm32")]
                let _ = event_loop;
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(preview) = self.preview.as_mut() else {
            return;
        };
        #[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
        let consumed = preview
            .egui_state
            .on_window_event(&preview.window, &event)
            .consumed;
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => preview.resize(size.width, size.height),
            #[cfg(not(target_arch = "wasm32"))]
            WindowEvent::DroppedFile(path) => preview.open(&path),
            // Escape quits the native app; a web page shouldn't close itself.
            #[cfg(not(target_arch = "wasm32"))]
            WindowEvent::KeyboardInput { event, .. }
                if !consumed
                    && event.state == ElementState::Pressed
                    && event.logical_key == Key::Named(NamedKey::Escape) =>
            {
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => preview.draw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(preview) = &self.preview {
            preview.window.request_redraw();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::with_user_event().build()?;
    let mut app = App::new(event_loop.create_proxy());
    event_loop.run_app(&mut app)?;
    Ok(())
}

/// The web entry point, called by wasm-bindgen's generated JavaScript.
#[cfg(target_arch = "wasm32")]
fn main() {
    use winit::platform::web::EventLoopExtWebSys;
    console_error_panic_hook::set_once();
    let event_loop = EventLoop::with_user_event().build().expect("event loop");
    let app = App::new(event_loop.create_proxy());
    event_loop.spawn_app(app);
}
