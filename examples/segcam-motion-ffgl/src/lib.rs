//! segcam Motion for Resolume: FFGL supplies the current frame; a bounded history
//! supplies the earlier ones the CPU core needs. All GL work stays on the host thread.

mod history;
mod params;

use params::PARAMS;
use std::ffi::{c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use vfx_ffgl_common::{ffi::*, gl, ram};
use vfx_wrapper_common::diag;

static LOG: diag::Log = diag::Log::new("SegcamMotionRust-FFGL.log");

/// `text` in a fixed FFGL field, zero-padded. build.rs checks that it fits.
const fn padded<const N: usize>(text: &str) -> [u8; N] {
    let bytes = text.as_bytes();
    assert!(bytes.len() <= N, "too long for its FFGL field");
    let mut out = [0; N];
    let mut i = 0;
    while i < bytes.len() {
        out[i] = bytes[i];
        i += 1;
    }
    out
}

static INFO: PluginInfoStruct = PluginInfoStruct {
    api_major_version: 2,
    api_minor_version: 1,
    plugin_unique_id: *b"SGCM",
    plugin_name: padded("segcam Motion"),
    plugin_type: FF_EFFECT,
};

static EXTENDED_INFO: PluginExtendedInfoStruct = PluginExtendedInfoStruct {
    plugin_major_version: 1,
    plugin_minor_version: 0,
    description: c"segcam Motion: motion segmentation, mattes and the swarm collage.".as_ptr(),
    about: c"segcam Motion, Rust (MIT OR Apache-2.0)".as_ptr(),
    free_frame_extended_data_size: 0,
    free_frame_extended_data_block: std::ptr::null_mut(),
};

struct Instance {
    values: Vec<f32>,
    time: Option<f64>,
    display: [u8; 16],
    blit: Option<gl::Blit>,
    output: gl::Texture,
    ram: Option<ram::Canvas>,
    history: history::History,
    canvas: Vec<f32>,
    rendered: Vec<segcam_motion::Argb<u8>>,
}

impl Instance {
    fn new() -> Self {
        Self {
            values: params::defaults(),
            time: None,
            display: [0; 16],
            blit: None,
            output: gl::Texture::new(),
            ram: None,
            history: history::History::default(),
            canvas: Vec::new(),
            rendered: Vec::new(),
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
        FF_GET_PLUGIN_SHORT_NAME => FFMixed::ptr(c"segcam Motion".as_ptr()),
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
                    if let Some(canvas) = instance.ram.take() {
                        canvas.delete();
                    }
                    if let Some(blit) = instance.blit.take() {
                        blit.delete();
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
                let time = unsafe { *input.ptr.cast::<f64>() };
                if time.is_finite() {
                    if me.time.is_some_and(|previous| time < previous) {
                        me.history.clear();
                    }
                    me.time = Some(time);
                }
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

/// Renders into the host's framebuffer with its GL context current.
unsafe fn process(me: &mut Instance, args: *const ProcessOpenGLStruct) -> bool {
    // SAFETY: the host supplies the process struct and its texture pointers.
    let Some(args) = (unsafe { args.as_ref() }) else {
        return false;
    };
    if args.num_input_textures < 1 || args.input_textures.is_null() {
        return false;
    }
    // SAFETY: at least one texture pointer was supplied.
    let Some(tex) = (unsafe { (*args.input_textures).as_ref() }) else {
        return false;
    };
    let (w, h) = (tex.width, tex.height);
    if w == 0 || h == 0 {
        return true;
    }
    if w > i32::MAX as u32
        || h > i32::MAX as u32
        || tex.hardware_width < w
        || tex.hardware_height < h
    {
        return false;
    }
    if me.ram.as_ref().is_none_or(|canvas| canvas.size != (w, h)) {
        // SAFETY: all objects belong to the current host context.
        unsafe {
            if let Some(old) = me.ram.take() {
                old.delete();
            }
            match ram::Canvas::new((w, h)) {
                Ok(canvas) => me.ram = Some(canvas),
                Err(e) => {
                    LOG.note(&format!("warning: {e}"));
                    return false;
                }
            }
        }
        me.history.resize(w as i32, h as i32);
        me.canvas.resize(w as usize * h as usize * 4, 0.0);
        me.rendered
            .resize(w as usize * h as usize, segcam_motion::Argb::default());
    }
    let settings = params::settings(&me.values);
    let canvas = me.ram.as_ref().unwrap();
    // SAFETY: the context is current and the canvas has the declared image size.
    unsafe {
        canvas.read_into(
            tex.handle,
            (tex.hardware_width, tex.hardware_height),
            args.host_fbo,
            &mut me.canvas,
        )
    };
    me.history.push(&me.canvas, &settings);
    me.history.render(&settings, &mut me.rendered);
    history::to_canvas(&me.rendered, &mut me.canvas);
    // SAFETY: the output storage holds the full straight, top-down image.
    unsafe {
        me.output.upload((w, h), &me.canvas);
        canvas.draw(me.output.handle);
    }
    LOG.note("render path: CPU through RAM (texture readback)");
    true
}
