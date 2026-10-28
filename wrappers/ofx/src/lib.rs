//! OpenFX wrapper for vfx-core: the same effect as the After Effects
//! plugin, for OpenFX hosts such as DaVinci Resolve, Nuke and Natron.
//!
//! Frames go through RAM: the source image is read into a straight-alpha f32 canvas,
//! rendered by the shared pipeline on the plugin's own GPU device, and written back.
//! The filter context has one input; the general context adds an optional "Layer"
//! clip for the Blend stage.

mod ffi;
mod host;
mod image;
mod params;

use std::ffi::{CStr, c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::OnceLock;

use vfx_core::Gpu;
use vfx_wrapper_common::{Blur, Settings, Vignette, diag, pipeline};

use ffi::*;
use host::{Props, check, suites};
use image::{Depth, Image, Rect};

static LOG: diag::Log = diag::Log::new(concat!(env!("VFX_PLUGIN_FILE"), "-OpenFX.log"));

/// One device for the whole plugin, created on first render. `None` without a GPU;
/// renders then pass the image through.
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

static PLUGIN: OfxPlugin = OfxPlugin {
    pluginApi: IMAGE_EFFECT_PLUGIN_API.as_ptr(),
    apiVersion: 1,
    pluginIdentifier: vfx_wrapper_common::plugin_cstr!("VFX_PLUGIN_ID").as_ptr(),
    pluginVersionMajor: 1,
    pluginVersionMinor: 0,
    setHost: host::set_host,
    mainEntry: main_entry,
};

#[unsafe(no_mangle)]
pub extern "C" fn OfxGetNumberOfPlugins() -> c_int {
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn OfxGetPlugin(nth: c_int) -> *const OfxPlugin {
    if nth == 0 { &PLUGIN } else { ptr::null() }
}

unsafe extern "C" fn main_entry(
    action: *const c_char,
    handle: *const c_void,
    in_args: PropSet,
    out_args: PropSet,
) -> OfxStatus {
    if action.is_null() {
        return STAT_FAILED;
    }
    // SAFETY: the host passes a NUL-terminated action name.
    let action = unsafe { CStr::from_ptr(action) };
    let effect = handle as ImageEffect;
    // A panic must not unwind into the host.
    let result = catch_unwind(AssertUnwindSafe(|| {
        dispatch(action, effect, Props(in_args), Props(out_args))
    }));
    match result {
        Ok(Ok(status)) => status,
        Ok(Err(status)) => status,
        Err(_) => {
            LOG.note(&format!("warning: {action:?} panicked"));
            STAT_FAILED
        }
    }
}

fn dispatch(
    action: &CStr,
    effect: ImageEffect,
    in_args: Props,
    out_args: Props,
) -> Result<OfxStatus, OfxStatus> {
    Ok(if action == ACTION_LOAD {
        if host::load() {
            STAT_OK
        } else {
            STAT_ERR_MISSING_HOST_FEATURE
        }
    } else if action == ACTION_UNLOAD
        || action == ACTION_CREATE_INSTANCE
        || action == ACTION_DESTROY_INSTANCE
    {
        STAT_OK
    } else if action == ACTION_DESCRIBE {
        describe(effect)?;
        STAT_OK
    } else if action == ACTION_DESCRIBE_IN_CONTEXT {
        describe_in_context(effect, in_args)?;
        STAT_OK
    } else if action == ACTION_GET_ROD {
        region_of_definition(effect, in_args, out_args)?
    } else if action == ACTION_GET_ROIS {
        regions_of_interest(effect, in_args, out_args)?;
        STAT_OK
    } else if action == ACTION_RENDER {
        render(effect, in_args)?;
        STAT_OK
    } else {
        STAT_REPLY_DEFAULT
    })
}

fn effect_props(effect: ImageEffect) -> Result<Props, OfxStatus> {
    let mut props = ptr::null_mut();
    // SAFETY: a descriptor or instance handle from the host.
    check(unsafe { (suites().effect.getPropertySet)(effect, &mut props) })?;
    Ok(Props(props))
}

/// For properties a host may not know (OpenFX grew over versions, and hosts return
/// kOfxStatErrUnknown for names they don't recognise). Failing describe over one of
/// these would hide the plugin, so note it and carry on.
pub(crate) fn optional(name: &CStr, result: Result<(), OfxStatus>) {
    if let Err(status) = result {
        LOG.note(&format!(
            "host rejected optional property {name:?} (status {status})"
        ));
    }
}

fn describe(effect: ImageEffect) -> Result<(), OfxStatus> {
    let p = effect_props(effect)?;
    p.set_string(PROP_LABEL, env!("VFX_PLUGIN_NAME"))?;
    p.set_strs(PROP_SUPPORTED_CONTEXTS, &[CONTEXT_FILTER, CONTEXT_GENERAL])?;
    p.set_strs(
        PROP_SUPPORTED_PIXEL_DEPTHS,
        &[DEPTH_BYTE, DEPTH_SHORT, DEPTH_FLOAT],
    )?;
    optional(
        PROP_GROUPING,
        p.set_string(PROP_GROUPING, env!("VFX_PLUGIN_CATEGORY")),
    );
    // DaVinci Resolve 21 rejects this one with kOfxStatErrUnknown.
    optional(
        PROP_PLUGIN_DESCRIPTION,
        p.set_string(
            PROP_PLUGIN_DESCRIPTION,
            "vfx-core effect chain: colour adjust, vignette, blend and Gaussian blur on the GPU.",
        ),
    );
    optional(
        PROP_SUPPORTS_MULTI_RESOLUTION,
        p.set_int(PROP_SUPPORTS_MULTI_RESOLUTION, 0, 1),
    );
    optional(PROP_SUPPORTS_TILES, p.set_int(PROP_SUPPORTS_TILES, 0, 0));
    optional(
        PROP_TEMPORAL_CLIP_ACCESS,
        p.set_int(PROP_TEMPORAL_CLIP_ACCESS, 0, 0),
    );
    optional(
        PROP_MULTIPLE_CLIP_DEPTHS,
        p.set_int(PROP_MULTIPLE_CLIP_DEPTHS, 0, 0),
    );
    // Each render leases its own chain from the pool, so concurrent renders are safe.
    optional(
        PROP_RENDER_THREAD_SAFETY,
        p.set_str(PROP_RENDER_THREAD_SAFETY, 0, RENDER_FULLY_SAFE),
    );
    optional(
        PROP_HOST_FRAME_THREADING,
        p.set_int(PROP_HOST_FRAME_THREADING, 0, 0),
    );
    Ok(())
}

fn define_clip(effect: ImageEffect, name: &CStr, optional: bool) -> Result<(), OfxStatus> {
    let mut props = ptr::null_mut();
    // SAFETY: a descriptor handle in the describe-in-context action.
    check(unsafe { (suites().effect.clipDefine)(effect, name.as_ptr(), &mut props) })?;
    let p = Props(props);
    p.set_str(PROP_SUPPORTED_COMPONENTS, 0, COMPONENT_RGBA)?;
    self::optional(PROP_SUPPORTS_TILES, p.set_int(PROP_SUPPORTS_TILES, 0, 0));
    if optional {
        p.set_int(CLIP_PROP_OPTIONAL, 0, 1)?;
    }
    Ok(())
}

fn describe_in_context(effect: ImageEffect, in_args: Props) -> Result<(), OfxStatus> {
    let general = in_args.str_is(PROP_CONTEXT, CONTEXT_GENERAL)?;
    define_clip(effect, CLIP_SOURCE, false)?;
    if general {
        define_clip(effect, CLIP_LAYER, true)?;
    }
    define_clip(effect, CLIP_OUTPUT, false)?;
    let mut set = ptr::null_mut();
    // SAFETY: a descriptor handle.
    check(unsafe { (suites().effect.getParamSet)(effect, &mut set) })?;
    params::define(set, general)
}

fn clip(effect: ImageEffect, name: &CStr) -> Option<Clip> {
    let mut clip = ptr::null_mut();
    // SAFETY: an instance handle; fails for clips this context doesn't define.
    let status = unsafe {
        (suites().effect.clipGetHandle)(effect, name.as_ptr(), &mut clip, ptr::null_mut())
    };
    (status == STAT_OK && !clip.is_null()).then_some(clip)
}

fn clip_connected(clip: Clip) -> bool {
    let mut props = ptr::null_mut();
    // SAFETY: a clip handle from clipGetHandle.
    if unsafe { (suites().effect.clipGetPropertySet)(clip, &mut props) } != STAT_OK {
        return false;
    }
    Props(props).get_int(CLIP_PROP_CONNECTED, 0).unwrap_or(0) != 0
}

/// Blur growth in full-resolution pixels, and whether it grows past the source.
fn blur_reach(values: &params::Values) -> (f64, f64, bool) {
    match values.blur {
        Some(b) => {
            let r = f64::from(b.radius).ceil();
            let (x, y) = match b.direction {
                1 => (r, 0.0),
                2 => (0.0, r),
                _ => (r, r),
            };
            (x, y, b.grow)
        }
        None => (0.0, 0.0, false),
    }
}

/// With Blur Edges on, the output reaches past the source by the blur radius.
fn region_of_definition(
    effect: ImageEffect,
    in_args: Props,
    out_args: Props,
) -> Result<OfxStatus, OfxStatus> {
    let time = in_args.get_double(PROP_TIME, 0)?;
    let values = params::Reader::new(effect, time)?.values()?;
    let (rx, ry, grow) = blur_reach(&values);
    if !grow {
        return Ok(STAT_REPLY_DEFAULT);
    }
    let source = clip(effect, CLIP_SOURCE).ok_or(STAT_FAILED)?;
    let mut rod = [0.0f64; 4];
    // SAFETY: a clip handle and a four-double rectangle.
    check(unsafe { (suites().effect.clipGetRegionOfDefinition)(source, time, &mut rod) })?;
    out_args.set_doubles(
        PROP_ROD,
        &[rod[0] - rx, rod[1] - ry, rod[2] + rx, rod[3] + ry],
    )?;
    Ok(STAT_OK)
}

/// The blur reads pixels beyond the area it renders, so ask for that much more input.
fn regions_of_interest(
    effect: ImageEffect,
    in_args: Props,
    out_args: Props,
) -> Result<(), OfxStatus> {
    let time = in_args.get_double(PROP_TIME, 0)?;
    let values = params::Reader::new(effect, time)?.values()?;
    let (rx, ry, _) = blur_reach(&values);
    let r = in_args.get_rect_d(PROP_ROI)?;
    let roi = [r[0] - rx, r[1] - ry, r[2] + rx, r[3] + ry];
    out_args.set_doubles(CLIP_ROI_SOURCE, &roi)?;
    if clip(effect, CLIP_LAYER).is_some() {
        out_args.set_doubles(CLIP_ROI_LAYER, &roi)?;
    }
    Ok(())
}

/// A fetched image, released when dropped.
struct Fetched {
    props: Props,
    image: Image,
}

impl Drop for Fetched {
    fn drop(&mut self) {
        // SAFETY: an image handle from clipGetImage, released once.
        unsafe { (suites().effect.clipReleaseImage)(self.props.0) };
    }
}

fn fetch(clip: Clip, time: OfxTime) -> Result<Option<Fetched>, OfxStatus> {
    let mut props = ptr::null_mut();
    // SAFETY: a clip handle; a null region asks for the whole image.
    let status = unsafe { (suites().effect.clipGetImage)(clip, time, ptr::null(), &mut props) };
    if status != STAT_OK || props.is_null() {
        return Ok(None);
    }
    let p = Props(props);
    // Release the image even if describing it fails.
    let mut fetched = Fetched {
        props: p,
        image: Image {
            data: ptr::null_mut(),
            bounds: Rect {
                x1: 0,
                y1: 0,
                x2: 0,
                y2: 0,
            },
            row_bytes: 0,
            depth: Depth::Float,
            premultiplied: false,
        },
    };
    if !p.str_is(PROP_COMPONENTS, COMPONENT_RGBA)? {
        return Err(STAT_ERR_IMAGE_FORMAT);
    }
    let depth = if p.str_is(PROP_PIXEL_DEPTH, DEPTH_FLOAT)? {
        Depth::Float
    } else if p.str_is(PROP_PIXEL_DEPTH, DEPTH_SHORT)? {
        Depth::Short
    } else if p.str_is(PROP_PIXEL_DEPTH, DEPTH_BYTE)? {
        Depth::Byte
    } else {
        return Err(STAT_ERR_IMAGE_FORMAT);
    };
    fetched.image = Image {
        data: p.get_pointer(IMAGE_PROP_DATA)?.cast(),
        bounds: p.get_rect_i(IMAGE_PROP_BOUNDS)?,
        row_bytes: p.get_int(IMAGE_PROP_ROW_BYTES, 0)? as isize,
        depth,
        premultiplied: p.str_is(PROP_PREMULTIPLICATION, PREMULTIPLIED)?,
    };
    if fetched.image.data.is_null() {
        return Err(STAT_FAILED);
    }
    Ok(Some(fetched))
}

/// Converts control values into the pipeline's units for a canvas at `scale`.
fn settings(values: &params::Values, scale: [f64; 2], canvas: Rect) -> Settings {
    let (sx, sy) = (scale[0] as f32, scale[1] as f32);
    Settings {
        linear: values.linear,
        color: values.color,
        vignette: values.vignette.map(|v| Vignette {
            // Canonical (bottom-left, full resolution) to top-down canvas pixels.
            center_px: [
                (v.center[0] * scale[0]) as f32 - canvas.x1 as f32,
                canvas.y2 as f32 - (v.center[1] * scale[1]) as f32,
            ],
            radius: v.radius,
            softness: v.softness,
            color: v.color,
            amount: v.amount,
        }),
        blend_mode: values.blend_mode,
        blend_amount: values.blend_amount,
        blur: values.blur.map(|b| Blur {
            radius: b.radius * sx,
            vertical_scale: if sx > 0.0 { sy / sx } else { 1.0 },
            direction: b.direction,
            grow: b.grow,
        }),
    }
}

fn render(effect: ImageEffect, in_args: Props) -> Result<(), OfxStatus> {
    let time = in_args.get_double(PROP_TIME, 0)?;
    let window = in_args.get_rect_i(PROP_RENDER_WINDOW)?;
    let scale = [
        in_args.get_double(PROP_RENDER_SCALE, 0)?,
        in_args.get_double(PROP_RENDER_SCALE, 1)?,
    ];
    let output = fetch(clip(effect, CLIP_OUTPUT).ok_or(STAT_FAILED)?, time)?.ok_or(STAT_FAILED)?;
    let source = fetch(clip(effect, CLIP_SOURCE).ok_or(STAT_FAILED)?, time)?;
    let layer = match clip(effect, CLIP_LAYER) {
        Some(c) if clip_connected(c) => fetch(c, time)?,
        _ => None,
    };
    let values = params::Reader::new(effect, time)?.values()?;

    let window = window.intersect(&output.image.bounds);
    if window.is_empty() {
        return Ok(());
    }
    // Render a canvas that also covers the pixels the blur reads around the window.
    let (rx, ry, grow) = blur_reach(&values);
    let reach = window.grow((rx * scale[0]).ceil() as i32, (ry * scale[1]).ceil() as i32);
    let canvas = match &source {
        Some(s) if !grow => reach.intersect(&s.image.bounds).union(&window),
        _ => reach,
    };
    let settings = settings(&values, scale, canvas);
    // SAFETY: images come from the host and are described by their own properties.
    let input = match &source {
        Some(s) => unsafe { image::decode(&s.image, canvas) },
        None => vec![0.0; canvas.width() * canvas.height() * 4],
    };
    let layer_pixels = layer
        .as_ref()
        .map(|l| unsafe { image::decode(&l.image, canvas) });

    let frame_rate = effect_props(effect)?
        .get_double(PROP_FRAME_RATE, 0)
        .unwrap_or(24.0);
    let job = pipeline::Job {
        settings: &settings,
        width: canvas.width() as u32,
        height: canvas.height() as u32,
        input: &input,
        layer: layer_pixels.as_deref(),
        canvas_origin: [0.0, 0.0],
        time: if frame_rate > 0.0 {
            time / frame_rate
        } else {
            time
        },
    };
    let depth = output.image.depth;
    LOG.note(&format!("render path: CPU (frames through RAM), {depth:?}"));
    let result = match gpu() {
        Some(gpu) => {
            catch_unwind(AssertUnwindSafe(|| pipeline::run(gpu, &job))).unwrap_or_else(|_| {
                LOG.note("warning: GPU render failed, passing frames through");
                input.clone()
            })
        }
        None => input.clone(),
    };
    // SAFETY: the window lies inside the output image's bounds and the canvas.
    unsafe { image::encode(&result, canvas, &output.image, window) };
    Ok(())
}
