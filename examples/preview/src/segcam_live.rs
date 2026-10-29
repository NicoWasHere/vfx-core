//! segcam-live: the swarm as the live app runs it, with a pile kept between frames, drawn
//! on the GPU.
//!
//! segcam-motion's swarm is rebuilt from the last 24 frames on every frame, on the CPU, as
//! After Effects needs (it can ask for any frame, in any order). The preview only goes
//! forward, so this keeps the pile instead: each frame is segmented once (on the CPU, from
//! a small luma grid) against the frames before it, and its moving regions become new
//! windows on top. Each window's picture is copied out of the frame on the GPU when it's
//! made, and its title bar drawn once by segcam-motion. Every frame the pile is drawn
//! with each pixel painted about once, however deep it is: first the windows' solid
//! insides, newest first, each leaving its place in the pile in a depth buffer so the
//! GPU skips whatever a newer window already covers; then their soft parts (rounded
//! corners, edges and shadows), oldest first and blended, again only where no newer
//! window's inside is on top. Windows expire after `lifetime` frames, or oldest first
//! past `max_tiles`.
//!
//! Detection, crops, titles and title bars are segcam-motion's own. The shadow is a close
//! match rather than an exact one: its rounded corners are approximated.

use std::collections::VecDeque;

use segcam_motion::render::{self, PixelRect};
use segcam_motion::swarm::{self, ShadowKernel, SwarmMetrics};
use segcam_motion::{LumaGrid, MotionSettings, motion};
use vfx_core::{Frame, Gpu};
use vfx_wrapper_common::pipeline::FRAME_FORMAT;

use crate::controls::Controls;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub motion: MotionSettings,
    /// How many frames before each one build its background.
    pub background_frames: usize,
    pub grid_width: i32,
    /// Frames a window stays up.
    pub lifetime: u32,
    pub max_tiles: usize,
}

/// segcam-live's settings from the panel, or None while it's switched off.
pub fn settings(c: &Controls) -> Option<Settings> {
    c.segcam_live_on.then(|| Settings {
        motion: MotionSettings {
            sensitivity: f64::from(c.segcam_live_sensitivity.clamp(0.0, 1.0)),
            min_area_fraction: f64::from(c.segcam_live_min_size.max(0.0)) / 100.0,
            max_segments: c.segcam_live_max_segments as _,
            ..Default::default()
        },
        background_frames: c.segcam_live_background,
        grid_width: segcam_motion::RenderSettings::default().grid_width,
        lifetime: c.segcam_live_lifetime.max(1.0).round() as u32,
        max_tiles: c.segcam_live_max_tiles,
    })
}

/// The pile is drawn here and blended, then copied to the float frame the effects take:
/// 32-bit float targets can't be blended on every GPU.
const CANVAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Window pictures and title bars.
const PICTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Each window's place in the pile, so newer windows hide older ones before they're drawn.
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

// The two constants are from segcam-motion's chrome.rs, which is private to it.
const WINDOW_WGSL: &str = r#"
const SHADOW_OPACITY: f32 = 0.45;
const BAR_EDGE_SHADE: f32 = 0.372;

struct W {
    video: vec4<f32>,  // the picture's rect on the canvas: x, y, w, h
    frame: vec4<f32>,  // the window's rect, title bar included
    canvas: vec4<f32>, // w, h, title bar height, scale
    shadow: vec4<f32>, // sigma, drop, corner radius, reach
    sizes: vec4<f32>,  // picture w, h; title bar w, h (at 1x)
}
@group(0) @binding(0) var<uniform> u: W;
@group(0) @binding(1) var picture: texture_2d<f32>;
@group(0) @binding(2) var bar: texture_2d<f32>;

// The instance index is the window's place in the pile, oldest 0: newer is nearer.
fn place(p: vec2<f32>, pile: u32) -> vec4<f32> {
    let depth = 1.0 - f32(pile + 1u) / 65536.0;
    return vec4<f32>(p.x / u.canvas.x * 2.0 - 1.0, 1.0 - p.y / u.canvas.y * 2.0, depth, 1.0);
}

var<private> QUAD: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
    vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0));

// The soft parts: the whole window and its shadow.
@vertex
fn vs(@builtin(vertex_index) i: u32, @builtin(instance_index) pile: u32)
    -> @builtin(position) vec4<f32> {
    let reach = u.shadow.w;
    let lo = u.frame.xy - vec2(reach);
    let hi = u.frame.xy + u.frame.zw + vec2(reach);
    return place(mix(lo, hi, QUAD[i % 6u]), pile);
}

// The solid inside: two rects that together cover everything but the rounded corners,
// where every pixel is fully the window's.
@vertex
fn vs_solid(@builtin(vertex_index) i: u32, @builtin(instance_index) pile: u32)
    -> @builtin(position) vec4<f32> {
    let f = u.frame;
    let r = u.shadow.z;
    var lo = f.xy + vec2(0.0, r);
    var hi = f.xy + f.zw - vec2(0.0, r);
    if i >= 6u {
        lo = f.xy + vec2(r, 0.0);
        hi = f.xy + f.zw - vec2(r, 0.0);
    }
    return place(mix(lo, hi, QUAD[i % 6u]), pile);
}

// Abramowitz and Stegun 7.1.26, within 1.5e-7.
fn erf(x: f32) -> f32 {
    let t = 1.0 / (1.0 + 0.3275911 * abs(x));
    let y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t
        + 0.254829592) * t * exp(-x * x);
    return select(-y, y, x >= 0.0);
}

fn normal_cdf(z: f32) -> f32 {
    return 0.5 * (1.0 + erf(z * 0.70710678));
}

// segcam-motion's sample_bar: the title bar at 1x, sampled bilinearly.
fn sample_bar(fx: f32, fy: f32) -> vec3<f32> {
    let size = u.sizes.zw;
    let f = clamp(vec2(fx, fy) - 0.5, vec2(0.0), size - 1.0);
    let p0 = vec2<i32>(f);
    let p1 = min(p0 + 1, vec2<i32>(size) - 1);
    let t = f - vec2<f32>(p0);
    let top = mix(textureLoad(bar, p0, 0).rgb, textureLoad(bar, vec2(p1.x, p0.y), 0).rgb, t.x);
    let bottom = mix(textureLoad(bar, vec2(p0.x, p1.y), 0).rgb, textureLoad(bar, p1, 0).rgb, t.x);
    return mix(top, bottom, t.y);
}

// The window's own pixel at `c`: its title bar, or its picture.
fn window_colour(c: vec2<f32>, l: vec2<f32>) -> vec3<f32> {
    let title = u.canvas.z;
    if l.y < title {
        return sample_bar((l.x + 0.5) / u.canvas.w, (l.y + 0.5) * u.sizes.w / title);
    }
    // segcam-motion's crop: the segment's box stretched over the (maybe grown) tile.
    let t = floor(c - u.video.xy);
    let s = min(u.sizes.xy - 1.0, floor((t + 0.5) * u.sizes.xy / u.video.zw));
    var col = textureLoad(picture, vec2<i32>(max(s, vec2(0.0))), 0).rgb;
    if l.y - title < 1.0 {
        col = col * BAR_EDGE_SHADE;
    }
    return col;
}

@fragment
fn fs_solid(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(window_colour(pos.xy, floor(pos.xy - u.frame.xy)), 1.0);
}

// segcam-motion's mask_coverage: the rounded window shape, antialiased.
fn coverage(l: vec2<f32>, size: vec2<f32>, r: f32) -> f32 {
    let c = l + 0.5;
    let n = min(max(c, vec2(r)), size - r);
    return clamp(r + 0.5 - distance(c, n), 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let c = pos.xy;
    let f = u.frame;
    let sigma = u.shadow.x;
    let drop = u.shadow.y;
    let radius = u.shadow.z;

    // The window: its title bar or picture, and how much of the pixel its rounded shape covers.
    var col = vec3(0.0);
    var cov = 0.0;
    let l = floor(c - f.xy);
    if l.x >= 0.0 && l.y >= 0.0 && l.x < f.z && l.y < f.w {
        col = window_colour(c, l);
        cov = coverage(l, f.zw, radius);
        // Fully inside, the window hides its own shadow: most pixels stop here.
        if cov >= 1.0 {
            return vec4<f32>(col, 1.0);
        }
    }

    // The shadow: a blurred box, a little lower, less what its rounded corners miss
    // (taken as the missing area's weight at its centroid).
    let px = normal_cdf((c.x - f.x) / sigma) - normal_cdf((c.x - f.x - f.z) / sigma);
    let py = normal_cdf((c.y - f.y - drop) / sigma) - normal_cdf((c.y - f.y - f.w - drop) / sigma);
    let ox = select(f.x + f.z - c.x, c.x - f.x, c.x < f.x + f.z * 0.5);
    let oy = select(f.y + drop + f.w - c.y, c.y - f.y - drop, c.y < f.y + drop + f.w * 0.5);
    let d = vec2(ox, oy) - vec2(0.2234 * radius);
    let missing = (1.0 - 0.78539816) * radius * radius;
    let loss = missing / (6.2831853 * sigma * sigma) * exp(-dot(d, d) / (2.0 * sigma * sigma));
    let shade = max(0.0, SHADOW_OPACITY * (px * py - loss));

    // Premultiplied: the shadow is black laid over, the window is laid over that.
    let p = col * cov;
    let a = shade + cov * (1.0 - shade);
    return vec4<f32>(p, a);
}
"#;

/// Copies a window's picture out of the frame, at 8 bits.
const CROP_WGSL: &str = r#"
@group(0) @binding(0) var<uniform> offset: vec4<f32>;
@group(0) @binding(1) var src: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i & 1u) * 4.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(src, vec2<i32>(pos.xy + offset.xy), 0);
}
"#;

/// The blended canvas into the effects' float frame.
const COPY_WGSL: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i & 1u) * 4.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(src, vec2<i32>(pos.xy), 0);
}
"#;

struct Pass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

impl Pass {
    /// `uniform`: binding 0 is a uniform buffer; then `textures` textures.
    fn new(
        device: &wgpu::Device,
        wgsl: &str,
        uniform: bool,
        textures: u32,
        format: wgpu::TextureFormat,
        blend: Option<wgpu::BlendState>,
    ) -> Self {
        Self::with(
            device,
            wgsl,
            uniform,
            textures,
            format,
            blend,
            None,
            ("vs", "fs"),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn with(
        device: &wgpu::Device,
        wgsl: &str,
        uniform: bool,
        textures: u32,
        format: wgpu::TextureFormat,
        blend: Option<wgpu::BlendState>,
        depth: Option<wgpu::DepthStencilState>,
        (vs, fs): (&str, &str),
    ) -> Self {
        let mut entries = Vec::new();
        if uniform {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            });
        }
        for i in 0..textures {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: u32::from(uniform) + i,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &entries,
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
            label: Some("segcam-live"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some(vs),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: depth,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(fs),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, layout }
    }

    fn bind(&self, gpu: &Gpu, resources: &[wgpu::BindingResource]) -> wgpu::BindGroup {
        let entries: Vec<_> = resources
            .iter()
            .enumerate()
            .map(|(i, resource)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: resource.clone(),
            })
            .collect();
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &entries,
        })
    }
}

/// A render pass that clears `target` first.
fn pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
    })
}

fn uniform(gpu: &Gpu, values: &[f32]) -> wgpu::Buffer {
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("segcam-live window"),
        size: std::mem::size_of_val(values) as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    gpu.queue
        .write_buffer(&buffer, 0, bytemuck::cast_slice(values));
    buffer
}

fn texture(gpu: &Gpu, (w, h): (u32, u32), format: wgpu::TextureFormat) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("segcam-live"),
        size: wgpu::Extent3d {
            width: w.max(1),
            height: h.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

struct Passes {
    /// The windows' soft parts, blended; tested against the solid insides' depth.
    window: Pass,
    /// The windows' solid insides, opaque, writing depth. Shares `window`'s bind groups.
    solid: Pass,
    crop: Pass,
    copy: Pass,
}

impl Passes {
    fn new(gpu: &Gpu) -> Self {
        let d = &gpu.device;
        let depth = |write| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(write),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        };
        Self {
            window: Pass::with(
                d,
                WINDOW_WGSL,
                true,
                2,
                CANVAS_FORMAT,
                Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                Some(depth(false)),
                ("vs", "fs"),
            ),
            solid: Pass::with(
                d,
                WINDOW_WGSL,
                true,
                2,
                CANVAS_FORMAT,
                None,
                Some(depth(true)),
                ("vs_solid", "fs_solid"),
            ),
            crop: Pass::new(d, CROP_WGSL, true, 1, PICTURE_FORMAT, None),
            copy: Pass::new(d, COPY_WGSL, false, 1, FRAME_FORMAT, None),
        }
    }
}

/// One window of the pile. The bind group holds its uniform, picture and title bar.
struct Window {
    born: i64,
    bind: wgpu::BindGroup,
    /// segcam-motion's own tile for it, pixels and all, to compare against.
    #[cfg(test)]
    tile: Option<swarm::Tile>,
}

/// The pile and the frames' luma grids.
#[derive(Default)]
pub struct Live {
    /// The last `background_frames` frames' grids, oldest first.
    grids: VecDeque<LumaGrid>,
    /// Oldest first.
    windows: VecDeque<Window>,
    frame: i64,
    size: (u32, u32),
    /// Made on first use.
    passes: Option<Passes>,
    canvas: Option<wgpu::TextureView>,
    depth: Option<wgpu::TextureView>,
    /// For the canvas height it was made for.
    shadow: Option<(u32, ShadowKernel)>,
    /// A frame has come in since the last reset: until then the source is shown as is.
    started: bool,
    /// Keep segcam-motion's own tile for each window, to compare against.
    #[cfg(test)]
    keep_tiles: bool,
}

impl Live {
    /// Starts over, after a seek or a new source.
    pub fn reset(&mut self) {
        self.grids.clear();
        self.windows.clear();
        self.started = false;
    }

    pub fn started(&self) -> bool {
        self.started
    }

    /// Segments the next frame (`pixels`, float RGBA, already uploaded to `source`) and
    /// adds its moving regions to the pile.
    pub fn push(&mut self, gpu: &Gpu, source: &Frame, pixels: &[f32], s: &Settings) {
        let (w, h) = (source.width, source.height);
        if self.size != (w, h) {
            self.reset();
            self.size = (w, h);
            self.canvas = None;
            self.depth = None;
        }
        self.started = true;
        self.frame += 1;
        let grid = downsample(pixels, (w as i32, h as i32), s.grid_width);
        let found = motion::detect(&self.grids, &grid, &s.motion);
        self.grids.push_back(grid);
        while self.grids.len() > s.background_frames.max(1) {
            self.grids.pop_front();
        }

        if !found.blobs.is_empty() {
            let passes = self.passes.get_or_insert_with(|| Passes::new(gpu));
            let m = SwarmMetrics::new(h as i32);
            if self.shadow.as_ref().is_none_or(|(at, _)| *at != h) {
                self.shadow = Some((h, ShadowKernel::new(m.scale)));
            }
            let shadow = &self.shadow.as_ref().expect("made above").1;
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            for (b, blob) in found.blobs.iter().enumerate() {
                let title = format!("LO.FI.SCI.FI {}", render::segment_label(self.frame, b));
                // Where segcam-motion puts the window; its pixels come from the GPU instead.
                let tile = swarm::make_tile::<u8>(blob, None, title, w as i32, h as i32, false);
                let window = swarm::tile_extent(tile.video, m.title_height);
                let crop = render::intersect(
                    render::to_pixels(&blob.bounds, w as i32, h as i32, false),
                    PixelRect::new(0, 0, w as i32, h as i32),
                );
                let crop_size = (crop.w.max(1) as u32, crop.h.max(1) as u32);

                let picture =
                    texture(gpu, crop_size, PICTURE_FORMAT).create_view(&Default::default());
                let offset = uniform(gpu, &[crop.x as f32, crop.y as f32, 0.0, 0.0]);
                let crop_bind = passes.crop.bind(
                    gpu,
                    &[
                        offset.as_entire_binding(),
                        wgpu::BindingResource::TextureView(&source.view),
                    ],
                );
                {
                    let mut pass = pass(&mut encoder, &picture);
                    pass.set_pipeline(&passes.crop.pipeline);
                    pass.set_bind_group(0, &crop_bind, &[]);
                    pass.draw(0..3, 0..1);
                }

                let bar_w = 1.max((f64::from(window.w) / m.scale).round() as i32);
                let colors = swarm::render_title_bar(bar_w, &tile.title);
                let bar_h = (colors.len() / bar_w as usize).max(1) as i32;
                let bar = texture(gpu, (bar_w as u32, bar_h as u32), PICTURE_FORMAT);
                let bytes: Vec<u8> = colors
                    .iter()
                    .flat_map(|c| {
                        [c.r, c.g, c.b, 1.0].map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
                    })
                    .collect();
                gpu.queue.write_texture(
                    bar.as_image_copy(),
                    &bytes,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(bar_w as u32 * 4),
                        rows_per_image: None,
                    },
                    wgpu::Extent3d {
                        width: bar_w as u32,
                        height: bar_h as u32,
                        depth_or_array_layers: 1,
                    },
                );

                let v = tile.video;
                let values = [
                    v.x as f32,
                    v.y as f32,
                    v.w as f32,
                    v.h as f32,
                    window.x as f32,
                    window.y as f32,
                    window.w as f32,
                    window.h as f32,
                    w as f32,
                    h as f32,
                    m.title_height as f32,
                    m.scale as f32,
                    shadow.sigma as f32,
                    shadow.drop as f32,
                    shadow.radius as f32,
                    shadow.reach as f32,
                    crop_size.0 as f32,
                    crop_size.1 as f32,
                    bar_w as f32,
                    bar_h as f32,
                ];
                let bind = passes.window.bind(
                    gpu,
                    &[
                        uniform(gpu, &values).as_entire_binding(),
                        wgpu::BindingResource::TextureView(&picture),
                        wgpu::BindingResource::TextureView(&bar.create_view(&Default::default())),
                    ],
                );
                self.windows.push_back(Window {
                    born: self.frame,
                    bind,
                    #[cfg(test)]
                    tile: self
                        .keep_tiles
                        .then(|| test_tile(tile, blob, pixels, (w as i32, h as i32))),
                });
            }
            gpu.queue.submit(Some(encoder.finish()));
        }
        self.expire(s);
    }

    /// Drops windows older than the lifetime, then the oldest past `max_tiles`.
    pub fn expire(&mut self, s: &Settings) {
        let lifetime = i64::from(s.lifetime.max(1));
        while let Some(oldest) = self.windows.front()
            && (self.frame - oldest.born >= lifetime || self.windows.len() > s.max_tiles.max(1))
        {
            self.windows.pop_front();
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    /// Draws the pile over a clear background into `target`, as if oldest first.
    pub fn draw(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, target: &Frame) {
        let passes = self.passes.get_or_insert_with(|| Passes::new(gpu));
        let size = (target.width, target.height);
        let canvas = self.canvas.get_or_insert_with(|| {
            texture(gpu, size, CANVAS_FORMAT).create_view(&Default::default())
        });
        let depth = self.depth.get_or_insert_with(|| {
            texture(gpu, size, DEPTH_FORMAT).create_view(&Default::default())
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("segcam-live pile"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: canvas,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // Solid insides newest first, so each pixel is painted by the window on top.
            pass.set_pipeline(&passes.solid.pipeline);
            for (pile, window) in self.windows.iter().enumerate().rev() {
                let pile = pile as u32;
                pass.set_bind_group(0, &window.bind, &[]);
                pass.draw(0..12, pile..pile + 1);
            }
            // Soft parts oldest first, blended, wherever no newer window's inside is.
            pass.set_pipeline(&passes.window.pipeline);
            for (pile, window) in self.windows.iter().enumerate() {
                let pile = pile as u32;
                pass.set_bind_group(0, &window.bind, &[]);
                pass.draw(0..6, pile..pile + 1);
            }
        }
        let bind = passes
            .copy
            .bind(gpu, &[wgpu::BindingResource::TextureView(canvas)]);
        let mut pass = pass(encoder, &target.view);
        pass.set_pipeline(&passes.copy.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// segcam-motion's `downsample`, read straight from float RGBA (as 8-bit values, as
/// segcam sees them): four samples per cell, Rec.601 luma.
fn downsample(pixels: &[f32], (src_w, src_h): (i32, i32), target_width: i32) -> LumaGrid {
    if src_w <= 0 || src_h <= 0 || target_width <= 0 {
        return LumaGrid::default();
    }
    let gw = target_width.min(src_w);
    let gh = 1.max((f64::from(gw) * f64::from(src_h) / f64::from(src_w)).round() as i32);
    let mut out = vec![0u8; (gw * gh) as usize];
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5).floor() / 255.0;
    let luma = |x: i32, y: i32| {
        let i = ((y * src_w + x) * 4) as usize;
        255.0 * (0.114 * to8(pixels[i + 2]) + 0.587 * to8(pixels[i + 1]) + 0.299 * to8(pixels[i]))
    };
    let scale = |i: i32, n: i32, d: i32| (i64::from(i) * i64::from(n) / i64::from(d)) as i32;
    for gy in 0..gh {
        let y0 = scale(gy, src_h, gh);
        let y1 = (y0 + 1).max(scale(gy + 1, src_h, gh));
        let yb = (src_h - 1).min((y0 + y1) / 2);
        for gx in 0..gw {
            let x0 = scale(gx, src_w, gw);
            let x1 = (x0 + 1).max(scale(gx + 1, src_w, gw));
            let xb = (src_w - 1).min((x0 + x1) / 2);
            let acc = luma(x0, y0) + luma(xb, y0) + luma(x0, yb) + luma(xb, yb);
            out[(gy * gw + gx) as usize] = (acc / 4.0).clamp(0.0, 255.0) as u8;
        }
    }
    LumaGrid {
        width: gw,
        height: gh,
        pixels: out,
    }
}

/// The window as segcam-motion would make it, pixels and all.
#[cfg(test)]
fn test_tile(
    tile: swarm::Tile,
    blob: &segcam_motion::Blob,
    pixels: &[f32],
    (w, h): (i32, i32),
) -> swarm::Tile {
    use segcam_motion::{Argb, Channel, View};
    let argb: Vec<Argb<u8>> = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&[r, g, b, a]| Argb {
            alpha: u8::from_unit(a),
            red: u8::from_unit(r),
            green: u8::from_unit(g),
            blue: u8::from_unit(b),
        })
        .collect();
    swarm::make_tile(
        blob,
        Some(&View::from_pixels(&argb, w, h)),
        tile.title,
        w,
        h,
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use segcam_motion::{Argb, ViewMut};

    const SIZE: (u32, u32) = (320, 180);

    /// Grey with a little texture, and a white block at `x0` (None: no block).
    fn frame(x0: Option<u32>) -> Vec<f32> {
        let (w, h) = SIZE;
        (0..h)
            .flat_map(|y| {
                (0..w).flat_map(move |x| {
                    let lit =
                        x0.is_some_and(|x0| (x0..x0 + 40).contains(&x) && (60..110).contains(&y));
                    let v = if lit {
                        1.0
                    } else {
                        0.25 + ((x / 8 + y / 8) % 2) as f32 * 0.05
                    };
                    [v, v * 0.9, v * 0.8, 1.0]
                })
            })
            .collect()
    }

    fn settings(lifetime: u32, max_tiles: usize) -> Settings {
        Settings {
            motion: MotionSettings::default(),
            background_frames: 16,
            grid_width: 192,
            lifetime,
            max_tiles,
        }
    }

    struct Rig {
        gpu: Gpu,
        source: Frame,
        live: Live,
    }

    impl Rig {
        fn new() -> Self {
            let gpu = pollster::block_on(Gpu::headless()).unwrap();
            let source = Frame::new(&gpu, SIZE.0, SIZE.1, FRAME_FORMAT);
            Self {
                gpu,
                source,
                live: Live::default(),
            }
        }

        fn push(&mut self, x0: Option<u32>, s: &Settings) {
            let pixels = frame(x0);
            self.source.write(&self.gpu, bytemuck::cast_slice(&pixels));
            self.live.push(&self.gpu, &self.source, &pixels, s);
        }

        fn draw(&mut self) -> Vec<f32> {
            let out = Frame::new(&self.gpu, SIZE.0, SIZE.1, FRAME_FORMAT);
            let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
            self.live.draw(&self.gpu, &mut encoder, &out);
            self.gpu.queue.submit(Some(encoder.finish()));
            vfx_core::Readback::new(&self.gpu, &out).read_f32(&self.gpu, &out)
        }
    }

    #[test]
    fn windows_stay_until_their_lifetime() {
        let s = settings(5, 100);
        let mut rig = Rig::new();
        let mut most = 0;
        for i in 0..40 {
            // A block moving for 8 frames, then still frames (which read as motion
            // until the background has settled).
            rig.push((i < 8).then_some(12 * i), &s);
            let live = &rig.live;
            assert!(
                live.windows.iter().all(|w| live.frame - w.born < 5),
                "none older than 5 frames"
            );
            most = most.max(live.len());
        }
        assert!(most > 1, "windows build up while things move");
        assert_eq!(rig.live.len(), 0, "every window has expired");
    }

    #[test]
    fn max_tiles_drops_the_oldest() {
        let s = settings(1000, 2);
        let mut rig = Rig::new();
        for i in 0..8 {
            rig.push(Some(12 * i), &s);
            assert!(rig.live.len() <= 2);
        }
        assert_eq!(rig.live.len(), 2);
    }

    /// The GPU's pile against segcam-motion's swarm drawing the same windows on the CPU.
    /// Pictures and title bars match; the shadow's rounded corners are approximated.
    #[test]
    fn draws_what_segcam_motion_draws() {
        let s = settings(1000, 1000);
        let mut rig = Rig::new();
        rig.live.keep_tiles = true;
        for i in 0..8 {
            rig.push(Some(12 * i), &s);
        }
        assert!(rig.live.len() > 2, "several windows overlap");
        let gpu_pixels = rig.draw();

        let (w, h) = (SIZE.0 as i32, SIZE.1 as i32);
        let tiles: VecDeque<swarm::Tile> = rig
            .live
            .windows
            .iter()
            .map(|w| w.tile.clone().unwrap())
            .collect();
        let mut cpu = vec![Argb::<u8>::default(); (w * h) as usize];
        swarm::draw_swarm(&mut ViewMut::from_pixels(&mut cpu, w, h), &tiles, false);

        let mut worst = 0f32;
        let mut total = 0f32;
        for (g, c) in gpu_pixels.as_chunks::<4>().0.iter().zip(&cpu) {
            let c = [c.red, c.green, c.blue, c.alpha].map(|v| f32::from(v) / 255.0);
            for k in 0..4 {
                let d = (g[k] - c[k]).abs();
                worst = worst.max(d);
                total += d;
            }
        }
        let mean = total / (w * h * 4) as f32;
        eprintln!("GPU vs CPU swarm: worst {worst:.4}, mean {mean:.5}");
        assert!(mean < 0.002, "mean difference {mean}");
        assert!(worst < 0.06, "worst difference {worst}");
    }

    /// Per-frame time at 1080p, GPU work waited for:
    /// `cargo test --release -p vfx-preview time_live -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn time_live() {
        use std::time::{Duration, Instant};
        let gpu = pollster::block_on(Gpu::headless()).unwrap();
        let (w, h) = (1920u32, 1080u32);
        let source = Frame::new(&gpu, w, h, FRAME_FORMAT);
        let target = Frame::new(&gpu, w, h, FRAME_FORMAT);
        let mut live = Live::default();
        let s = settings(60, 100);
        let wait = || {
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
        };
        let (mut pushing, mut drawing) = (Duration::ZERO, Duration::ZERO);
        for i in 0..120u32 {
            let x0 = (i * 23) % (w - 300);
            let pixels: Vec<f32> = (0..h)
                .flat_map(|y| {
                    (0..w).flat_map(move |x| {
                        let lit = (x0..x0 + 300).contains(&x) && (200..500).contains(&y);
                        let v = if lit {
                            1.0
                        } else {
                            ((x ^ y) & 31) as f32 / 64.0
                        };
                        [v, v * 0.8, v * 0.6, 1.0]
                    })
                })
                .collect();
            source.write(&gpu, bytemuck::cast_slice(&pixels));
            wait();
            let t = Instant::now();
            live.push(&gpu, &source, &pixels, &s);
            wait();
            let pushed = t.elapsed();
            let t = Instant::now();
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            live.draw(&gpu, &mut encoder, &target);
            gpu.queue.submit(Some(encoder.finish()));
            wait();
            if i >= 60 {
                pushing += pushed;
                drawing += t.elapsed();
            }
        }
        eprintln!(
            "segcam-live on the GPU, {} windows up: taking a frame in {:?}, drawing {:?}",
            live.len(),
            pushing / 60,
            drawing / 60
        );
        // The same with nothing in the pile: what drawing costs whatever the windows.
        live.reset();
        let t = Instant::now();
        for _ in 0..30 {
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            live.draw(&gpu, &mut encoder, &target);
            gpu.queue.submit(Some(encoder.finish()));
            wait();
        }
        eprintln!(
            "segcam-live on the GPU, empty pile: drawing {:?}",
            t.elapsed() / 30
        );
    }
}
