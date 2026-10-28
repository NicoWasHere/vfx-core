//! After Effects wrapper for vfx-core. This crate maps After Effects parameters and
//! layers onto the core's effect chain and contains no effect logic of its own.
//!
//! Rendering goes through SmartFX, so 8, 16 and 32 bpc projects are all supported, and
//! 32 bpc keeps values above 1.0. Frames are copied to the GPU and back (the CPU path).
//! Feedback is left out on purpose, because After Effects may render frames out of order.

#[cfg(target_os = "macos")]
mod metal;
mod params;
mod pixel;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use after_effects as ae;
use params::{Params, Settings};
use pixel::{Depth, Placement};
use vfx_core::Gpu;
use vfx_wrapper_common::{diag, pipeline};

/// Render-path notes, in `~/Library/Logs/VfxAdobe.log`.
static LOG: diag::Log = diag::Log::new("VfxAdobe.log");

/// One device for the whole plugin, created on first render. `None` if no GPU adapter
/// could be opened. Renders then pass the image through unchanged rather than fail.
static GPU: OnceLock<Option<Gpu>> = OnceLock::new();

/// Set once a fallback has been logged, so a failing GPU doesn't flood the log.
static WARNED: AtomicBool = AtomicBool::new(false);

fn gpu() -> Option<&'static Gpu> {
    GPU.get_or_init(|| match pollster::block_on(Gpu::headless()) {
        Ok(gpu) => Some(gpu),
        Err(e) => {
            warn_once(&format!("no GPU available, passing frames through: {e}"));
            None
        }
    })
    .as_ref()
}

/// The plugin's CPU render path without After Effects, for the test harness in
/// `ae-tests/`, which compares After Effects' output against it. Not a public API.
#[doc(hidden)]
pub mod reference {
    pub use vfx_wrapper_common::{Blur, Settings, Vignette};

    /// Renders one frame of straight-alpha RGBA f32 pixels the way `render_frame` does
    /// for a layer whose buffer lines up with the output. `None` without a GPU.
    pub fn render(
        settings: &Settings,
        width: u32,
        height: u32,
        input: &[f32],
        layer: Option<&[f32]>,
        time: f64,
    ) -> Option<Vec<f32>> {
        let job = vfx_wrapper_common::pipeline::Job {
            settings,
            width,
            height,
            input,
            layer,
            canvas_origin: [0.0, 0.0],
            time,
        };
        Some(vfx_wrapper_common::pipeline::run(crate::gpu()?, &job))
    }
}

fn warn_once(message: &str) {
    LOG.note(&format!("warning: {message}"));
    if !WARNED.swap(true, Ordering::Relaxed) {
        ae::log::error!("VFX Preview: {message}");
    }
}

/// Checkout IDs for SmartFX. They pair `checkout_layer` in pre-render with
/// `checkout_layer_pixels` in render.
const INPUT_ID: i32 = 0;
const BLEND_ID: i32 = 1;

ae::define_effect!(Global, Instance, Params);

#[derive(Default)]
struct Global;

impl AdobePluginGlobal for Global {
    fn params_setup(
        &self,
        params: &mut ae::Parameters<Params>,
        _in_data: ae::InData,
        _out_data: ae::OutData,
    ) -> Result<(), ae::Error> {
        params::setup(params)
    }

    fn handle_command(
        &mut self,
        command: ae::Command,
        in_data: ae::InData,
        // Only the macOS GPU device setup writes to it.
        #[cfg_attr(not(target_os = "macos"), allow(unused_variables, unused_mut))]
        mut out_data: ae::OutData,
        params: &mut ae::Parameters<Params>,
    ) -> Result<(), ae::Error> {
        match command {
            // SmartFX: After Effects asks which input area the effect needs, then renders
            // with buffers it checks out for us. This is the only path that receives 32 bpc.
            ae::Command::SmartPreRender { mut extra } => {
                smart_pre_render(&mut extra, &in_data, params)
            }
            ae::Command::SmartRender { extra } => smart_render(&extra, &in_data, params),
            #[cfg(target_os = "macos")]
            ae::Command::GpuDeviceSetup { mut extra } => {
                gpu_device_setup(&mut extra, &in_data, &mut out_data);
                Ok(())
            }
            #[cfg(target_os = "macos")]
            ae::Command::GpuDeviceSetdown { mut extra } => {
                if extra.what_gpu() == ae::GpuFramework::Metal {
                    extra.destroy_gpu_data::<metal::AeDevice>();
                }
                Ok(())
            }
            #[cfg(target_os = "macos")]
            ae::Command::SmartRenderGpu { extra } => smart_render_gpu(&extra, &in_data, params),
            _ => Ok(()),
        }
    }
}

/// Called once per GPU After Effects might render on. We only take Metal devices. If
/// opening one fails, we don't advertise GPU support for it, so After Effects stays on
/// the CPU path for that device.
#[cfg(target_os = "macos")]
fn gpu_device_setup(
    extra: &mut ae::GpuDeviceSetupExtra,
    in_data: &ae::InData,
    out_data: &mut ae::OutData,
) {
    if extra.what_gpu() != ae::GpuFramework::Metal {
        return;
    }
    let index = extra.device_index();
    let opened = ae::pf::suites::GPUDevice::new()
        .and_then(|suite| suite.device_info(in_data.effect_ref(), index))
        .map_err(|e| format!("{e:?}"))
        .and_then(|info| unsafe { metal::AeDevice::open(info.devicePV, index) });
    match opened {
        Ok(device) => {
            LOG.note(&format!(
                "Metal device {index} opened for GPU-native rendering"
            ));
            extra.set_gpu_data(device);
            out_data.set_out_flag2(ae::OutFlags2::SupportsGpuRenderF32, true);
        }
        Err(e) => warn_once(&format!(
            "Metal device {index} unavailable, using CPU path: {e}"
        )),
    }
}

/// A world pointer in the form `GPUDevice::gpu_world_size` asks for. The crate
/// doesn't implement that conversion for `Layer` itself.
#[cfg(target_os = "macos")]
struct RawWorld(*mut ae::sys::PF_EffectWorld);

#[cfg(target_os = "macos")]
impl ae::AsPtr<*mut ae::sys::PF_EffectWorld> for RawWorld {
    fn as_ptr(&self) -> *mut ae::sys::PF_EffectWorld {
        self.0
    }
}

/// GPU-native render: frames stay in After Effects' Metal buffers throughout.
#[cfg(target_os = "macos")]
fn smart_render_gpu(
    extra: &ae::SmartRenderExtra,
    in_data: &ae::InData,
    params: &ae::Parameters<Params>,
) -> Result<(), ae::Error> {
    let device = extra
        .gpu_data::<metal::AeDevice>()
        .ok_or(ae::Error::BadCallbackParameter)?;
    let suite = ae::pf::suites::GPUDevice::new()?;
    let callbacks = extra.callbacks();
    let has_blend = params.index(Params::BlendLayer).is_some();
    let mut input = callbacks
        .checkout_layer_pixels(INPUT_ID as u32)?
        .ok_or(ae::Error::BadCallbackParameter)?;
    let mut layer = if has_blend {
        callbacks.checkout_layer_pixels(BLEND_ID as u32)?
    } else {
        None
    };
    let mut output = callbacks
        .checkout_output()?
        .ok_or(ae::Error::BadCallbackParameter)?;

    let mut wrap = |l: &mut ae::Layer| -> Result<metal::World, ae::Error> {
        use ae::AsMutPtr;
        let world = RawWorld(l.as_mut_ptr());
        let data = suite.gpu_world_data(in_data.effect_ref(), &mut *l)?;
        let size = suite.gpu_world_size(in_data.effect_ref(), world)?;
        let buffer = unsafe { device.wrap_buffer(data, size as u64) }.map_err(|e| {
            warn_once(&e);
            ae::Error::BadCallbackParameter
        })?;
        Ok(metal::World {
            buffer,
            width: l.width() as u32,
            height: l.height() as u32,
            pitch: (stride(l)? / metal::BYTES_PER_PIXEL) as u32,
            origin: [l.origin().h, l.origin().v],
        })
    };
    let result = (|| {
        let settings = params::read_settings(params, in_data)?;
        let input = wrap(&mut input)?;
        let layer = layer.as_mut().map(&mut wrap).transpose()?;
        let output = wrap(&mut output)?;
        if output.width == 0 || output.height == 0 {
            return Ok(());
        }
        LOG.note("render path: GPU-native (After Effects' Metal buffers, 32 bpc float)");
        catch_unwind(AssertUnwindSafe(|| {
            metal::render(
                device,
                &settings,
                &input,
                layer.as_ref(),
                &output,
                seconds(in_data),
            )
        }))
        .map_err(|_| {
            warn_once("GPU-native render failed");
            ae::Error::BadCallbackParameter
        })
    })();

    callbacks.checkin_layer_pixels(INPUT_ID as u32)?;
    if has_blend {
        callbacks.checkin_layer_pixels(BLEND_ID as u32)?;
    }
    result
}

fn smart_pre_render(
    extra: &mut ae::PreRenderExtra,
    in_data: &ae::InData,
    params: &ae::Parameters<Params>,
) -> Result<(), ae::Error> {
    let settings = params::read_settings(params, in_data)?;
    let (reach_x, reach_y) = settings.blur_reach();
    let request = extra.output_request();

    // Blur needs pixels just outside the area it is asked for, so ask for more input.
    let mut input_request = request;
    input_request.rect = grow(request.rect.into(), reach_x, reach_y).into();

    let callbacks = extra.callbacks();
    let checkout = |index: i32, id: i32| {
        callbacks.checkout_layer(
            index,
            id,
            &input_request,
            in_data.current_time(),
            in_data.time_step(),
            in_data.time_scale(),
        )
    };
    let input = checkout(0, INPUT_ID)?;
    if let Some(index) = params.index(Params::BlendLayer) {
        checkout(index as i32, BLEND_ID)?;
    }

    // Blur spreads pixels outward. After Effects trims the input to a mask's bounds, so
    // without Blur Edges the spread is clipped to the layer (or to the input, when an
    // earlier effect already grew it past the layer), not to what the input covers.
    let input_max: ae::Rect = input.max_result_rect.into();
    let mut result = grow(input.result_rect.into(), reach_x, reach_y);
    let mut max = grow(input_max, reach_x, reach_y);
    if !settings.grows() {
        let bounds = union(layer_bounds(in_data), input_max);
        result = intersect(result, bounds);
        max = intersect(max, bounds);
    }
    // Never return more than was asked for, so no RETURNS_EXTRA_PIXELS handling is needed.
    extra.set_result_rect(intersect(result, request.rect.into()));
    extra.set_max_result_rect(max);
    // GPU-native render is possible on any Metal device we opened in GPU device setup.
    // After Effects only offers Metal here if that setup advertised support.
    if cfg!(target_os = "macos") && extra.what_gpu() == ae::GpuFramework::Metal {
        extra.set_gpu_render_possible(true);
    }
    Ok(())
}

fn smart_render(
    extra: &ae::SmartRenderExtra,
    in_data: &ae::InData,
    params: &ae::Parameters<Params>,
) -> Result<(), ae::Error> {
    let callbacks = extra.callbacks();
    let has_blend = params.index(Params::BlendLayer).is_some();
    let input = callbacks
        .checkout_layer_pixels(INPUT_ID as u32)?
        .ok_or(ae::Error::BadCallbackParameter)?;
    // No pixels when the Blend Layer menu is set to None.
    let layer = if has_blend {
        callbacks.checkout_layer_pixels(BLEND_ID as u32)?
    } else {
        None
    };
    let mut output = callbacks
        .checkout_output()?
        .ok_or(ae::Error::BadCallbackParameter)?;

    let result = params::read_settings(params, in_data).and_then(|settings| {
        render_frame(
            &settings,
            &input,
            layer.as_ref(),
            &mut output,
            seconds(in_data),
        )
    });

    callbacks.checkin_layer_pixels(INPUT_ID as u32)?;
    if has_blend {
        callbacks.checkin_layer_pixels(BLEND_ID as u32)?;
    }
    result
}

fn seconds(in_data: &ae::InData) -> f64 {
    let scale = in_data.time_scale();
    if scale == 0 {
        0.0
    } else {
        f64::from(in_data.current_time()) / f64::from(scale)
    }
}

/// Renders one frame. All buffers are placed on a canvas the size of `output`, using
/// each buffer's origin, because the output can be larger than the input (blur growth)
/// and the blend layer can be any size.
fn render_frame(
    settings: &Settings,
    input: &ae::Layer,
    layer: Option<&ae::Layer>,
    output: &mut ae::Layer,
    time: f64,
) -> Result<(), ae::Error> {
    let depth = Depth::from_bits(input.bit_depth()).ok_or(ae::Error::BadCallbackParameter)?;
    LOG.note(&format!("render path: CPU (frames through RAM), {depth:?}"));
    if Depth::from_bits(output.bit_depth()) != Some(depth) {
        return Err(ae::Error::BadCallbackParameter);
    }
    let (width, height) = (output.width(), output.height());
    if width == 0 || height == 0 {
        return Ok(());
    }
    let origin = output.origin();
    let place = |l: &ae::Layer| Placement {
        canvas_width: width,
        canvas_height: height,
        dx: (l.origin().h - origin.h) as isize,
        dy: (l.origin().v - origin.v) as isize,
    };
    let decode = |l: &ae::Layer| -> Result<Vec<f32>, ae::Error> {
        let d = Depth::from_bits(l.bit_depth()).ok_or(ae::Error::BadCallbackParameter)?;
        Ok(pixel::decode_onto(
            l.buffer(),
            stride(l)?,
            l.width(),
            l.height(),
            d,
            place(l),
        ))
    };
    let canvas = decode(input)?;
    let layer_canvas = layer.map(decode).transpose()?;

    let job = pipeline::Job {
        settings,
        width: width as u32,
        height: height as u32,
        input: &canvas,
        layer: layer_canvas.as_deref(),
        canvas_origin: [origin.h as f32, origin.v as f32],
        time,
    };
    // A GPU failure shouldn't take After Effects down or block the render. Pass the
    // image through, and say why once in the log.
    let result = match gpu() {
        Some(gpu) => {
            catch_unwind(AssertUnwindSafe(|| pipeline::run(gpu, &job))).unwrap_or_else(|_| {
                warn_once("GPU render failed, passing frames through");
                canvas.clone()
            })
        }
        None => canvas.clone(),
    };
    let out_stride = stride(output)?;
    pixel::encode(
        &result,
        output.buffer_mut(),
        out_stride,
        width,
        height,
        depth,
    );
    Ok(())
}

fn stride(layer: &ae::Layer) -> Result<usize, ae::Error> {
    usize::try_from(layer.row_bytes()).map_err(|_| ae::Error::BadCallbackParameter)
}

fn grow(r: ae::Rect, x: i32, y: i32) -> ae::Rect {
    if r.is_empty() {
        return r;
    }
    ae::Rect {
        left: r.left - x,
        top: r.top - y,
        right: r.right + x,
        bottom: r.bottom + y,
    }
}

fn union(a: ae::Rect, b: ae::Rect) -> ae::Rect {
    if a.is_empty() {
        return b;
    }
    if b.is_empty() {
        return a;
    }
    ae::Rect {
        left: a.left.min(b.left),
        top: a.top.min(b.top),
        right: a.right.max(b.right),
        bottom: a.bottom.max(b.bottom),
    }
}

/// The layer's own rect in the current (downsampled) render, origin at its top-left.
fn layer_bounds(in_data: &ae::InData) -> ae::Rect {
    let scaled = |size: i32, r: ae::RationalScale| {
        if r.den == 0 {
            size
        } else {
            let (n, d) = (i64::from(size) * i64::from(r.num), i64::from(r.den));
            ((n + d - 1) / d) as i32
        }
    };
    ae::Rect {
        left: 0,
        top: 0,
        right: scaled(in_data.width(), in_data.downsample_x()),
        bottom: scaled(in_data.height(), in_data.downsample_y()),
    }
}

fn intersect(a: ae::Rect, b: ae::Rect) -> ae::Rect {
    let r = ae::Rect {
        left: a.left.max(b.left),
        top: a.top.max(b.top),
        right: a.right.min(b.right),
        bottom: a.bottom.min(b.bottom),
    };
    if r.is_empty() { ae::Rect::empty() } else { r }
}

/// The macro rejects zero-sized instance types, so keep a field. It is saved with the project.
#[derive(Default)]
struct Instance {
    revision: u16,
}

impl AdobePluginInstance for Instance {
    fn flatten(&self) -> Result<(u16, Vec<u8>), ae::Error> {
        Ok((1, self.revision.to_le_bytes().to_vec()))
    }

    fn unflatten(_version: u16, serialized: &[u8]) -> Result<Self, ae::Error> {
        let revision = serialized
            .get(..2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .unwrap_or(0);
        Ok(Instance { revision })
    }

    // Non-smart path, kept for hosts that don't use SmartFX. After Effects itself
    // renders through `Command::SmartRender` once the PiPL advertises it. Here the output
    // is the input's size, so blur growth doesn't apply.
    fn render(
        &self,
        plugin: &mut PluginState,
        in_layer: &ae::Layer,
        out_layer: &mut ae::Layer,
    ) -> Result<(), ae::Error> {
        let settings = params::read_settings(plugin.params, &plugin.in_data)?;
        let layer = match plugin.params.get(Params::BlendLayer)?.as_param()? {
            ae::Param::Layer(l) => l.value(),
            _ => None,
        };
        render_frame(
            &settings,
            in_layer,
            layer.as_ref(),
            out_layer,
            seconds(&plugin.in_data),
        )
    }

    fn handle_command(
        &mut self,
        _plugin: &mut PluginState,
        _command: ae::Command,
    ) -> Result<(), ae::Error> {
        Ok(())
    }
}

/// Callback the host passes to `PluginDataEntryFunction3` (SDK `PF_PluginDataCB3`).
type PluginDataCb3 = unsafe extern "C" fn(
    *mut std::ffi::c_void,
    *const u8,
    *const u8,
    *const u8,
    *const u8,
    i32,
    i32,
    i32,
    i32,
    *const u8,
    *const u8,
    *const u8,
) -> i32;

/// Version 3 of the plug-in entry point. The after-effects crate only emits version 2,
/// and current After Effects builds may look for version 3 first.
/// Kind `eFKT` and API version 13.0 match `PF_PLUG_IN_VERSION` in the SDK headers.
///
/// # Safety
/// Only the host should call this. `callback` must be a valid `PF_PluginDataCB3`, and
/// `in_ptr` must be the opaque pointer the host expects back.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn PluginDataEntryFunction3(
    in_ptr: *mut std::ffi::c_void,
    callback: Option<PluginDataCb3>,
    _sp_basic_suite: *const std::ffi::c_void,
    host_name: *const std::ffi::c_char,
    host_version: *const std::ffi::c_char,
) -> i32 {
    const EFKT: i32 = 0x6546_4B54; // 'eFKT'
    let Some(callback) = callback else {
        return 1;
    };
    if host_name.is_null() || host_version.is_null() {
        return 1;
    }
    unsafe {
        callback(
            in_ptr,
            c"VFX Preview".as_ptr().cast(),
            c"VFXPreview".as_ptr().cast(),
            c"vfx-core".as_ptr().cast(),
            c"EffectMain".as_ptr().cast(),
            EFKT,
            13,
            0,
            0,
            c"".as_ptr().cast(),
            c"color blur vignette blend".as_ptr().cast(),
            c"Color adjust, vignette, layer blend and blur, rendered on the GPU."
                .as_ptr()
                .cast(),
        )
    }
}
