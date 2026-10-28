//! segcam Motion for OpenFX. Each render rebuilds its background from host frames.
mod params;
mod pixels;

use segcam_motion::pixel::Channel;
use segcam_motion::{Argb, FrameSource, RenderSettings, View, ViewMut, previous_frames_needed};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use vfx_ofx::ffi::*;
use vfx_ofx::host::{self, Props, check, suites};
use vfx_ofx::image::{Depth, Image, Rect};

static PLUGIN: OfxPlugin = OfxPlugin {
    pluginApi: IMAGE_EFFECT_PLUGIN_API.as_ptr(),
    apiVersion: 1,
    pluginIdentifier: c"com.lofiscifi.SegcamMotionRust".as_ptr(),
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
    input: PropSet,
    output: PropSet,
) -> OfxStatus {
    if action.is_null() {
        return STAT_FAILED;
    }
    // SAFETY: the host supplies a NUL-terminated action and valid action handles.
    let action = unsafe { CStr::from_ptr(action) };
    catch_unwind(AssertUnwindSafe(|| {
        dispatch(action, handle as ImageEffect, Props(input), Props(output))
    }))
    .unwrap_or(Err(STAT_FAILED))
    .unwrap_or_else(|s| s)
}
fn effect_props(effect: ImageEffect) -> Result<Props, OfxStatus> {
    let mut p = ptr::null_mut();
    // SAFETY: the host supplies a descriptor or instance handle.
    check(unsafe { (suites().effect.getPropertySet)(effect, &mut p) })?;
    Ok(Props(p))
}
fn clip(effect: ImageEffect, name: &CStr) -> Result<Clip, OfxStatus> {
    let mut c = ptr::null_mut();
    // SAFETY: the host supplies an instance handle and writes the clip handle.
    check(unsafe {
        (suites().effect.clipGetHandle)(effect, name.as_ptr(), &mut c, ptr::null_mut())
    })?;
    Ok(c)
}
fn dispatch(
    action: &CStr,
    effect: ImageEffect,
    input: Props,
    output: Props,
) -> Result<OfxStatus, OfxStatus> {
    if action == ACTION_LOAD {
        return Ok(if host::load() {
            STAT_OK
        } else {
            STAT_ERR_MISSING_HOST_FEATURE
        });
    }
    if action == ACTION_DESCRIBE {
        let p = effect_props(effect)?;
        p.set_string(PROP_LABEL, "segcam Motion (Rust)")?;
        p.set_string(PROP_GROUPING, "LO.FI.SCI.FI")?;
        p.set_strs(PROP_SUPPORTED_CONTEXTS, &[CONTEXT_FILTER, CONTEXT_GENERAL])?;
        p.set_strs(
            PROP_SUPPORTED_PIXEL_DEPTHS,
            &[DEPTH_BYTE, DEPTH_SHORT, DEPTH_FLOAT],
        )?;
        p.set_int(PROP_TEMPORAL_CLIP_ACCESS, 0, 1)?;
        p.set_int(PROP_SUPPORTS_TILES, 0, 0)?;
        p.set_int(PROP_SUPPORTS_MULTI_RESOLUTION, 0, 0)?;
        p.set_str(PROP_RENDER_THREAD_SAFETY, 0, RENDER_FULLY_SAFE)?;
    } else if action == ACTION_DESCRIBE_IN_CONTEXT {
        for name in [CLIP_SOURCE, CLIP_OUTPUT] {
            let mut p = ptr::null_mut();
            // SAFETY: describe-in-context supplies a descriptor handle.
            check(unsafe { (suites().effect.clipDefine)(effect, name.as_ptr(), &mut p) })?;
            Props(p).set_str(PROP_SUPPORTED_COMPONENTS, 0, COMPONENT_RGBA)?;
            if name == CLIP_SOURCE {
                Props(p).set_int(PROP_TEMPORAL_CLIP_ACCESS, 0, 1)?;
            }
        }
        let mut set = ptr::null_mut();
        // SAFETY: a descriptor handle and writable parameter set pointer.
        check(unsafe { (suites().effect.getParamSet)(effect, &mut set) })?;
        params::define(set)?;
    } else if action == c"OfxImageEffectActionGetFramesNeeded" {
        let t = input.get_double(PROP_TIME, 0)?;
        let s = params::read(effect, t)?;
        output.set_doubles(
            c"OfxImageClipPropFrameRange_Source",
            &[t - f64::from(previous_frames_needed(&s)), t],
        )?;
    } else if action == ACTION_GET_ROD {
        let time = input.get_double(PROP_TIME, 0)?;
        let mut rod = [0.0f64; 4];
        // SAFETY: a source clip handle and space for four canonical coordinates.
        check(unsafe {
            (suites().effect.clipGetRegionOfDefinition)(clip(effect, CLIP_SOURCE)?, time, &mut rod)
        })?;
        output.set_doubles(PROP_ROD, &rod)?;
    } else if action == ACTION_GET_ROIS {
        let time = input.get_double(PROP_TIME, 0)?;
        let mut rod = [0.0f64; 4];
        // SAFETY: a source clip handle and writable canonical rectangle.
        check(unsafe {
            (suites().effect.clipGetRegionOfDefinition)(clip(effect, CLIP_SOURCE)?, time, &mut rod)
        })?;
        output.set_doubles(CLIP_ROI_SOURCE, &rod)?;
    } else if action == ACTION_RENDER {
        render(effect, input)?;
    } else if action != ACTION_CREATE_INSTANCE
        && action != ACTION_DESTROY_INSTANCE
        && action != ACTION_UNLOAD
    {
        return Ok(STAT_REPLY_DEFAULT);
    }
    Ok(STAT_OK)
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

struct Source {
    clip: Clip,
    start: f64,
    first: f64,
    canvas: Rect,
    error: Option<OfxStatus>,
}
impl<C: Channel> FrameSource<C> for Source {
    fn with_frame(&mut self, index: usize, f: &mut dyn FnMut(Option<View<'_, C>>)) {
        let time = self.first + index as f64;
        if time < self.start {
            f(None);
            return;
        }
        match fetch(self.clip, time) {
            Ok(Some(frame)) => {
                // SAFETY: the fetched image stays alive through the closure.
                let pixels = unsafe { pixels::decode::<C>(&frame.image, self.canvas) };
                f(Some(View::from_pixels(
                    &pixels,
                    self.canvas.width() as i32,
                    self.canvas.height() as i32,
                )));
            }
            Ok(None) => {
                self.error = Some(STAT_FAILED);
                f(None);
            }
            Err(e) => {
                self.error = Some(e);
                f(None);
            }
        }
    }
}
fn render(effect: ImageEffect, input: Props) -> Result<(), OfxStatus> {
    let time = input.get_double(PROP_TIME, 0)?;
    let output = fetch(clip(effect, CLIP_OUTPUT)?, time)?.ok_or(STAT_FAILED)?;
    let source = clip(effect, CLIP_SOURCE)?;
    let mut p = ptr::null_mut();
    // SAFETY: a source clip handle and writable property handle pointer.
    check(unsafe { (suites().effect.clipGetPropertySet)(source, &mut p) })?;
    let start = Props(p).get_double(c"OfxImageEffectPropFrameRange", 0)?;
    let mut settings = params::read(effect, time)?;
    settings.straight_alpha = !Props(p).str_is(PROP_PREMULTIPLICATION, PREMULTIPLIED)?;
    let count = previous_frames_needed(&settings) as usize + 1;
    let mut frames = Source {
        clip: source,
        start,
        first: time - (count - 1) as f64,
        canvas: output.image.bounds,
        error: None,
    };
    let window = input
        .get_rect_i(PROP_RENDER_WINDOW)?
        .intersect(&output.image.bounds);
    if output.image.depth == Depth::Byte {
        draw::<u8>(&settings, &mut frames, count, time, &output.image, window);
    } else {
        draw::<f32>(&settings, &mut frames, count, time, &output.image, window);
    }
    frames.error.map_or(Ok(()), Err)
}
fn draw<C: Channel>(
    s: &RenderSettings,
    frames: &mut Source,
    count: usize,
    time: f64,
    output: &Image,
    window: Rect,
) {
    let canvas = frames.canvas;
    let mut pixels = vec![Argb::<C>::default(); canvas.width() * canvas.height()];
    segcam_motion::render(
        s,
        frames,
        count,
        time.round() as i64,
        &mut ViewMut::from_pixels(&mut pixels, canvas.width() as i32, canvas.height() as i32),
    );
    // SAFETY: the render window is clipped to the output and canvas bounds.
    unsafe { pixels::encode(&pixels, canvas, output, window) };
}
