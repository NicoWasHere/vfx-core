//! FFGL wrapper for vfx-core: the "VFX Preview" effect for Resolume Avenue and Arena,
//! and other FFGL 2 hosts.
//!
//! The host calls `plugMain` on its GL thread with its context current. On macOS each
//! frame goes GPU to GPU: the host's texture is drawn into an IOSurface that the
//! plugin's Metal device renders from (see `surface`). Elsewhere, or if that can't be
//! set up, the texture is read back through RAM, rendered, and uploaded again. If the
//! GPU path fails, the input is drawn unchanged.
//!
//! `VFX_FFGL_PATH=ram` forces the RAM path, for comparing the two.

mod ffi;
mod frame;
mod gl;
mod params;
#[cfg(target_os = "macos")]
mod surface;

use std::ffi::{c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;

use vfx_core::Gpu;
use vfx_wrapper_common::{diag, pipeline};

use ffi::*;
use params::PARAMS;

static LOG: diag::Log = diag::Log::new("VfxFfgl.log");

static GPU: OnceLock<Option<Gpu>> = OnceLock::new();

fn gpu() -> Option<&'static Gpu> {
    GPU.get_or_init(|| match pollster::block_on(Gpu::headless()) {
        Ok(gpu) => Some(gpu),
        Err(e) => {
            LOG.note(&format!(
                "warning: no GPU available, passing frames through: {e}"
            ));
            None
        }
    })
    .as_ref()
}

static INFO: PluginInfoStruct = PluginInfoStruct {
    api_major_version: 2,
    api_minor_version: 1,
    plugin_unique_id: *b"VFXP",
    plugin_name: *b"VFX Preview\0\0\0\0\0",
    plugin_type: FF_EFFECT,
};

static EXTENDED_INFO: PluginExtendedInfoStruct = PluginExtendedInfoStruct {
    plugin_major_version: 1,
    plugin_minor_version: 0,
    description: c"vfx-core effect chain: colour adjust, vignette and Gaussian blur on the GPU."
        .as_ptr(),
    about: c"vfx-core (MIT OR Apache-2.0)".as_ptr(),
    free_frame_extended_data_size: 0,
    free_frame_extended_data_block: std::ptr::null_mut(),
};

struct Instance {
    values: Vec<f32>,
    /// Seconds, from FF_SET_TIME.
    time: f64,
    /// Backing store for FF_GET_PARAMETER_DISPLAY, which returns a pointer.
    display: [u8; 16],
    blit: Option<gl::Blit>,
    output: gl::Texture,
    #[cfg(target_os = "macos")]
    interop: Option<surface::Interop>,
    #[cfg(target_os = "macos")]
    /// Set once the zero-copy path has failed to set up, so it isn't retried per frame.
    interop_failed: bool,
}

impl Instance {
    fn new() -> Self {
        Self {
            values: params::defaults(),
            time: 0.0,
            display: [0; 16],
            blit: None,
            output: gl::Texture::new(),
            #[cfg(target_os = "macos")]
            interop: None,
            #[cfg(target_os = "macos")]
            interop_failed: std::env::var("VFX_FFGL_PATH").as_deref() == Ok("ram"),
        }
    }
}

/// The only exported entry point of an FFGL plugin.
///
/// # Safety
/// Called by the host with the FFGL ABI: `input` is valid for `code`, and `instance` is
/// null or a value this function returned from FF_INSTANTIATE_GL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn plugMain(code: u32, input: FFMixed, instance: *mut c_void) -> FFMixed {
    // A panic must not unwind into the host.
    catch_unwind(AssertUnwindSafe(|| unsafe {
        dispatch(code, input, instance.cast())
    }))
    .unwrap_or_else(|_| {
        LOG.note(&format!("warning: plugMain({code}) panicked"));
        FFMixed::fail()
    })
}

unsafe fn param_index(index: u32) -> Option<usize> {
    let i = index as usize;
    (i < PARAMS.len()).then_some(i)
}

/// Copies `text` into a host string buffer, as the FFGL SDK does (no terminator).
unsafe fn write_string(buffer: &StringBufferStruct, text: &str) {
    let n = text.len().min(buffer.max_to_write as usize);
    // SAFETY: the host owns `max_to_write` bytes at `address`.
    unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), buffer.address.cast(), n) };
}

unsafe fn dispatch(code: u32, input: FFMixed, inst: *mut Instance) -> FFMixed {
    // SAFETY: the host passes an instance from FF_INSTANTIATE_GL or null.
    let me = unsafe { inst.as_mut() };
    // SAFETY (all reads of `input`): the host puts the value type `code` defines there.
    let index = unsafe { input.uint };
    match code {
        FF_GET_INFO => FFMixed::ptr(&INFO),
        FF_INITIALISE_V2 | FF_DEINITIALISE => FFMixed::success(),
        FF_GET_EXTENDED_INFO => FFMixed::ptr(&EXTENDED_INFO),
        FF_GET_PLUGIN_SHORT_NAME => FFMixed::ptr(c"VFX Preview".as_ptr()),
        FF_GET_PLUGIN_CAPS => FFMixed::uint(match index {
            FF_CAP_SET_TIME => FF_TRUE,
            FF_CAP_MINIMUM_INPUT_FRAMES | FF_CAP_MAXIMUM_INPUT_FRAMES => 1,
            _ => FF_FALSE,
        }),
        FF_ENABLE_PLUGIN_CAP => FFMixed::fail(),
        FF_GET_NUM_PARAMETERS => FFMixed::uint(PARAMS.len() as u32),
        FF_GET_PARAMETER_NAME => match unsafe { param_index(index) } {
            Some(i) => FFMixed::ptr(PARAMS[i].name.as_ptr()),
            None => FFMixed::ptr(std::ptr::null::<c_char>()),
        },
        FF_GET_PARAMETER_TYPE => match unsafe { param_index(index) } {
            Some(i) => FFMixed::uint(PARAMS[i].kind),
            None => FFMixed::fail(),
        },
        FF_GET_PARAMETER_DEFAULT => match unsafe { param_index(index) } {
            Some(i) => FFMixed::float(PARAMS[i].default),
            None => FFMixed::fail(),
        },
        FF_GET_PARAMETER_USAGE => FFMixed::uint(FF_USAGE_STANDARD),
        FF_GET_PARAMETER_VISIBILITY => FFMixed::uint(FF_TRUE),
        FF_GET_RANGE => {
            // SAFETY: a host-owned GetRangeStruct.
            let r = unsafe { &mut *input.ptr.cast::<GetRangeStruct>() };
            match unsafe { param_index(r.parameter_number) } {
                Some(i) => {
                    r.range = RangeStruct {
                        min: PARAMS[i].min,
                        max: PARAMS[i].max,
                    };
                    FFMixed::success()
                }
                None => FFMixed::fail(),
            }
        }
        FF_GET_PARAM_GROUP | FF_GET_PARAM_DISPLAY_NAME => {
            // SAFETY: a host-owned GetStringStruct.
            let g = unsafe { &*input.ptr.cast::<GetStringStruct>() };
            match unsafe { param_index(g.parameter_number) } {
                Some(i) if g.string_buffer.max_to_write > 0 => {
                    let text = if code == FF_GET_PARAM_GROUP {
                        PARAMS[i].group.to_owned()
                    } else {
                        PARAMS[i].name.to_string_lossy().into_owned()
                    };
                    unsafe { write_string(&g.string_buffer, &text) };
                    FFMixed::success()
                }
                _ => FFMixed::fail(),
            }
        }
        FF_GET_NUM_PARAMETER_ELEMENTS => match unsafe { param_index(index) } {
            Some(i) => FFMixed::uint(PARAMS[i].options.len() as u32),
            None => FFMixed::fail(),
        },
        FF_GET_PARAMETER_ELEMENT_NAME | FF_GET_PARAMETER_ELEMENT_VALUE => {
            // SAFETY: a host-owned GetParameterElement*Struct (same layout).
            let e = unsafe { &*input.ptr.cast::<GetParameterElementStruct>() };
            let option = unsafe { param_index(e.parameter_number) }
                .and_then(|i| PARAMS[i].options.get(e.element_number as usize));
            match (code, option) {
                (FF_GET_PARAMETER_ELEMENT_NAME, Some(name)) => FFMixed::ptr(name.as_ptr()),
                (_, Some(_)) => FFMixed::float(e.element_number as f32),
                (FF_GET_PARAMETER_ELEMENT_NAME, None) => FFMixed::ptr(std::ptr::null::<c_char>()),
                (_, None) => FFMixed::fail(),
            }
        }
        FF_GET_NUM_ELEMENT_SEPARATORS => FFMixed::uint(0),
        FF_GET_PARAMETER_EVENTS => {
            // SAFETY: a host-owned GetParamEventsStruct; the plugin never raises events.
            let ev = unsafe { &mut *input.ptr.cast::<GetParamEventsStruct>() };
            ev.num_events = 0;
            FFMixed::ok(me.is_some())
        }

        FF_INSTANTIATE_GL => {
            // SAFETY: the host passes a viewport and has its context current.
            let viewport = unsafe { input.ptr.cast::<FFGLViewportStruct>().as_ref() };
            if viewport.is_none() {
                return FFMixed::fail();
            }
            #[cfg(target_os = "windows")]
            if let Err(e) = gl::load() {
                LOG.note(&format!("warning: {e}"));
                return FFMixed::fail();
            }
            let mut instance = Instance::new();
            match unsafe { gl::Blit::new(gl::PASS) } {
                Ok(blit) => instance.blit = Some(blit),
                Err(e) => {
                    LOG.note(&format!("warning: {e}"));
                    return FFMixed::fail();
                }
            }
            FFMixed::ptr(Box::into_raw(Box::new(instance)))
        }
        FF_DEINSTANTIATE_GL => match me {
            Some(_) => {
                // SAFETY: the pointer came from Box::into_raw and is released once.
                let mut instance = unsafe { Box::from_raw(inst) };
                unsafe {
                    instance.output.delete();
                    if let Some(blit) = instance.blit.take() {
                        blit.delete();
                    }
                    #[cfg(target_os = "macos")]
                    if let Some(interop) = instance.interop.take() {
                        interop.delete();
                    }
                }
                FFMixed::success()
            }
            None => FFMixed::fail(),
        },
        FF_CONNECT | FF_DISCONNECT | FF_RESIZE | FF_SET_BEATINFO | FF_SET_HOSTINFO
        | FF_SET_SAMPLERATE => FFMixed::ok(me.is_some()),
        FF_GET_INPUT_STATUS => match me {
            Some(_) if index == 0 => FFMixed::uint(FF_INPUT_INUSE),
            _ => FFMixed::fail(),
        },
        FF_SET_TIME => match me {
            Some(me) if !unsafe { input.ptr }.is_null() => {
                // SAFETY: the host passes a pointer to a double, in seconds.
                me.time = unsafe { *input.ptr.cast::<f64>() };
                FFMixed::success()
            }
            _ => FFMixed::fail(),
        },
        FF_SET_PARAMETER => {
            // SAFETY: a host-owned SetParameterStruct.
            let s = unsafe { &*input.ptr.cast::<SetParameterStruct>() };
            match (me, unsafe { param_index(s.parameter_number) }) {
                (Some(me), Some(i)) => {
                    let value = f32::from_bits(unsafe { s.new_parameter_value.uint });
                    me.values[i] = params::clamp(i, value);
                    FFMixed::success()
                }
                _ => FFMixed::fail(),
            }
        }
        FF_SET_PARAMETER_ELEMENT_VALUE => FFMixed::fail(),
        FF_GET_PARAMETER => match (me, unsafe { param_index(index) }) {
            (Some(me), Some(i)) => FFMixed::float(me.values[i]),
            _ => FFMixed::fail(),
        },
        FF_GET_PARAMETER_DISPLAY => match (me, unsafe { param_index(index) }) {
            (Some(me), Some(i)) => {
                let text = params::display(i, me.values[i]);
                let n = text.len().min(me.display.len() - 1);
                me.display = [0; 16];
                me.display[..n].copy_from_slice(&text.as_bytes()[..n]);
                FFMixed::ptr(me.display.as_ptr())
            }
            _ => FFMixed::fail(),
        },
        FF_PROCESS_OPENGL => match me {
            // SAFETY: the host passes a ProcessOpenGLStruct with its context current.
            Some(me) => FFMixed::ok(unsafe { process(me, input.ptr.cast()) }),
            None => FFMixed::fail(),
        },
        _ => FFMixed::fail(),
    }
}

/// Renders one frame into the host's bound framebuffer.
unsafe fn process(me: &mut Instance, args: *const ProcessOpenGLStruct) -> bool {
    // SAFETY: the host's struct, valid for this call.
    let Some(args) = (unsafe { args.as_ref() }) else {
        return false;
    };
    if args.num_input_textures < 1 || args.input_textures.is_null() {
        return false;
    }
    // SAFETY: the host passes `num_input_textures` texture pointers.
    let Some(tex) = (unsafe { (*args.input_textures).as_ref() }) else {
        return false;
    };
    let Some(blit) = me.blit.as_ref() else {
        return false;
    };
    let (w, h) = (tex.width, tex.height);
    if w == 0 || h == 0 {
        return true;
    }
    let pass_through = |blit: &gl::Blit| unsafe {
        let max_uv = [
            w as f32 / tex.hardware_width.max(1) as f32,
            h as f32 / tex.hardware_height.max(1) as f32,
        ];
        blit.draw(tex.handle, gl::TEXTURE_2D, max_uv);
        true
    };
    let Some(gpu) = gpu() else {
        return pass_through(blit);
    };
    let settings = params::settings(&me.values, w, h);

    #[cfg(target_os = "macos")]
    if !me.interop_failed {
        if me.interop.as_ref().is_none_or(|i| i.size() != (w, h)) {
            // SAFETY: the context is current; the old interop belongs to it.
            if let Some(old) = me.interop.take() {
                unsafe { old.delete() };
            }
            match unsafe { surface::Interop::new(gpu, (w, h)) } {
                Ok(interop) => me.interop = Some(interop),
                Err(e) => {
                    LOG.note(&format!(
                        "warning: zero-copy path unavailable, using RAM: {e}"
                    ));
                    me.interop_failed = true;
                }
            }
        }
        if let Some(interop) = &me.interop {
            LOG.note("render path: GPU-native (IOSurface shared with OpenGL, no CPU copies)");
            let hardware = (tex.hardware_width, tex.hardware_height);
            let time = me.time;
            // SAFETY: the context is current; the texture is the host's input.
            let rendered = catch_unwind(AssertUnwindSafe(|| unsafe {
                interop.render(gpu, &settings, time, tex.handle, hardware, args.host_fbo)
            }));
            if rendered.is_err() {
                LOG.note("warning: GPU render failed, passing frames through");
                return pass_through(blit);
            }
            return true;
        }
    }

    // SAFETY: the host's texture is a 2D texture of its hardware size.
    let texels = unsafe { gl::read_texture(tex.handle, (tex.hardware_width, tex.hardware_height)) };
    let canvas = frame::texture_to_canvas(&texels, tex.hardware_width, w, h);
    let job = pipeline::Job {
        settings: &settings,
        width: w,
        height: h,
        input: &canvas,
        layer: None,
        canvas_origin: [0.0, 0.0],
        time: me.time,
    };
    LOG.note("render path: GPU through RAM (texture readback)");
    let Ok(result) = catch_unwind(AssertUnwindSafe(|| pipeline::run(gpu, &job))) else {
        LOG.note("warning: GPU render failed, passing frames through");
        return pass_through(blit);
    };
    let texels = frame::canvas_to_texture(&result, w, h);
    // SAFETY: the context is current; the texture is the plugin's own.
    unsafe {
        me.output.upload((w, h), &texels);
        blit.draw(me.output.handle, gl::TEXTURE_2D, [1.0, 1.0]);
    }
    true
}
