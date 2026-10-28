//! The After Effects glue: a SmartFX effect that checks out the frames before the current
//! one in pre-render, hands them to the core one at a time in render, and writes the result.
//! Parameters, ranges and defaults are segcam's C++ effect's.

use std::panic::{AssertUnwindSafe, catch_unwind};

use after_effects as ae;

use crate::pixel::{Channel, Color, View, ViewMut};
use crate::render::{self, FrameSource, OutputMode, RenderSettings};

#[derive(Eq, PartialEq, Hash, Clone, Copy, Debug)]
enum Params {
    Output,
    Sensitivity,
    Adaptation,
    Background,
    MinSize,
    MaxSegments,
    Grid,
    Mirror,
    Boxes,
    Labels,
    BoxColor,
    Trail,
    MaxTiles,
}

/// The current frame's checkout ID. Checkout ID `k` is the frame `k` before it.
const CURRENT_ID: i32 = 0;
const INPUT_INDEX: i32 = 0;

/// What pre-render works out and render needs.
struct Plan {
    settings: RenderSettings,
    /// How many frames were checked out, the current one included.
    frame_count: usize,
    frame_number: i64,
}

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
        let d = RenderSettings::default();
        params.add(
            Params::Output,
            "Output",
            ae::PopupDef::setup(|p| {
                p.set_options(&OutputMode::NAMES);
                p.set_default(d.mode as i32);
            }),
        )?;
        percent(
            params,
            Params::Sensitivity,
            "Sensitivity",
            [0.0, 100.0, 0.0, 100.0],
            d.motion.sensitivity * 100.0,
            1,
        )?;
        percent(
            params,
            Params::Adaptation,
            "Adaptation",
            [0.1, 50.0, 0.1, 25.0],
            f64::from(d.motion.adaptation) * 100.0,
            1,
        )?;
        slider(
            params,
            Params::Background,
            "Background",
            [1, 120, 1, 60],
            d.background_frames,
        )?;
        percent(
            params,
            Params::MinSize,
            "Minimum size",
            [0.0, 25.0, 0.0, 5.0],
            d.motion.min_area_fraction * 100.0,
            2,
        )?;
        slider(
            params,
            Params::MaxSegments,
            "Max segments",
            [1, 256, 1, 64],
            d.motion.max_segments,
        )?;
        slider(
            params,
            Params::Grid,
            "Grid width",
            [32, 1024, 64, 384],
            d.grid_width,
        )?;
        checkbox(params, Params::Mirror, "Mirror", d.mirror)?;
        checkbox(params, Params::Boxes, "Boxes", d.boxes)?;
        checkbox(params, Params::Labels, "Labels", d.labels)?;
        let c = d.box_color;
        let byte = |v: f32| (v * 255.0 + 0.5) as u8;
        params.add(
            Params::BoxColor,
            "Box color",
            ae::ColorDef::setup(|def| {
                def.set_default(ae::Pixel8 {
                    alpha: 255,
                    red: byte(c.r),
                    green: byte(c.g),
                    blue: byte(c.b),
                });
            }),
        )?;
        slider(
            params,
            Params::Trail,
            "Trail frames",
            [1, 120, 1, 60],
            d.trail_frames,
        )?;
        slider(
            params,
            Params::MaxTiles,
            "Swarm max tiles",
            [1, 1000, 1, 300],
            d.max_tiles,
        )?;
        Ok(())
    }

    fn handle_command(
        &mut self,
        command: ae::Command,
        in_data: ae::InData,
        _out_data: ae::OutData,
        params: &mut ae::Parameters<Params>,
    ) -> Result<(), ae::Error> {
        match command {
            ae::Command::SmartPreRender { mut extra } => pre_render(&mut extra, &in_data, params),
            ae::Command::SmartRender { extra } => smart_render(&extra),
            _ => Ok(()),
        }
    }
}

/// `[valid_min, valid_max, slider_min, slider_max]`, displayed as a percentage.
fn percent(
    params: &mut ae::Parameters<Params>,
    id: Params,
    name: &str,
    [valid_min, valid_max, slider_min, slider_max]: [f32; 4],
    default: f64,
    precision: i16,
) -> Result<(), ae::Error> {
    params.add(
        id,
        name,
        ae::FloatSliderDef::setup(|f| {
            f.set_valid_min(valid_min);
            f.set_valid_max(valid_max);
            f.set_slider_min(slider_min);
            f.set_slider_max(slider_max);
            f.set_default(default);
            f.set_precision(precision);
            f.set_display_flags(ae::ValueDisplayFlag::PERCENT);
        }),
    )
}

fn slider(
    params: &mut ae::Parameters<Params>,
    id: Params,
    name: &str,
    [valid_min, valid_max, slider_min, slider_max]: [i32; 4],
    default: i32,
) -> Result<(), ae::Error> {
    params.add(
        id,
        name,
        ae::SliderDef::setup(|s| {
            s.set_valid_min(valid_min);
            s.set_valid_max(valid_max);
            s.set_slider_min(slider_min);
            s.set_slider_max(slider_max);
            s.set_default(default);
        }),
    )
}

fn checkbox(
    params: &mut ae::Parameters<Params>,
    id: Params,
    name: &str,
    default: bool,
) -> Result<(), ae::Error> {
    params.add(
        id,
        name,
        ae::CheckBoxDef::setup(|c| {
            c.set_default(default);
        }),
    )
}

/// Reads every control at the current time into the core's units.
fn read_settings(params: &ae::Parameters<Params>) -> Result<RenderSettings, ae::Error> {
    let param = |id| params.get(id);
    let float = |id| match param(id)?.as_param()? {
        ae::Param::FloatSlider(s) => Ok(s.value()),
        _ => Err(ae::Error::BadCallbackParameter),
    };
    let int = |id| match param(id)?.as_param()? {
        ae::Param::Slider(s) => Ok(s.value()),
        _ => Err(ae::Error::BadCallbackParameter),
    };
    let check = |id| match param(id)?.as_param()? {
        ae::Param::CheckBox(c) => Ok(c.value()),
        _ => Err(ae::Error::BadCallbackParameter),
    };
    let mode = match param(Params::Output)?.as_param()? {
        ae::Param::Popup(p) => OutputMode::from_index(p.value()),
        _ => None,
    }
    .ok_or(ae::Error::BadCallbackParameter)?;
    let box_color = match param(Params::BoxColor)?.as_param()? {
        ae::Param::Color(c) => {
            let p = c.value();
            let unit = |v: u8| f32::from(v) / 255.0;
            Color {
                r: unit(p.red),
                g: unit(p.green),
                b: unit(p.blue),
            }
        }
        _ => return Err(ae::Error::BadCallbackParameter),
    };

    let mut s = RenderSettings {
        mode,
        // After Effects hands effects straight (unpremultiplied) pixels.
        straight_alpha: true,
        background_frames: int(Params::Background)?,
        grid_width: int(Params::Grid)?,
        mirror: check(Params::Mirror)?,
        boxes: check(Params::Boxes)?,
        labels: check(Params::Labels)?,
        box_color,
        trail_frames: int(Params::Trail)?,
        max_tiles: int(Params::MaxTiles)?,
        ..RenderSettings::default()
    };
    s.motion.sensitivity = float(Params::Sensitivity)? / 100.0;
    s.motion.adaptation = (float(Params::Adaptation)? / 100.0) as f32;
    s.motion.min_area_fraction = float(Params::MinSize)? / 100.0;
    s.motion.max_segments = int(Params::MaxSegments)?;
    Ok(s)
}

fn pre_render(
    extra: &mut ae::PreRenderExtra,
    in_data: &ae::InData,
    params: &ae::Parameters<Params>,
) -> Result<(), ae::Error> {
    let settings = read_settings(params)?;

    // Detection looks at the whole frame, and the swarm draws over all of it, so ask for
    // the whole layer whatever region After Effects wants back.
    let scaled = |size: i32, r: ae::RationalScale| {
        if r.den == 0 {
            size
        } else {
            (f64::from(size) * f64::from(r.num) / f64::from(r.den)).ceil() as i32
        }
    };
    let mut request = extra.output_request();
    request.rect = ae::Rect {
        left: 0,
        top: 0,
        right: scaled(in_data.width(), in_data.downsample_x()),
        bottom: scaled(in_data.height(), in_data.downsample_y()),
    }
    .into();
    request.preserve_rgb_of_zero_alpha = 1;

    let (now, step, scale) = (
        in_data.current_time(),
        in_data.time_step(),
        in_data.time_scale(),
    );
    let callbacks = extra.callbacks();
    let current = callbacks.checkout_layer(INPUT_INDEX, CURRENT_ID, &request, now, step, scale)?;

    // Then the frames before it, nearest first, stopping at the start of the layer.
    let mut frame_count = 1;
    for k in 1..=render::previous_frames_needed(&settings) {
        let time = now - k * step;
        if time < 0 {
            break;
        }
        let previous = callbacks.checkout_layer(INPUT_INDEX, k, &request, time, step, scale)?;
        if ae::Rect::from(previous.result_rect).is_empty() {
            break;
        }
        frame_count += 1;
    }

    extra.set_result_rect(current.result_rect.into());
    extra.set_max_result_rect(current.max_result_rect.into());
    extra.set_returns_extra_pixels(true);
    extra.set_pre_render_data(Plan {
        settings,
        frame_count,
        frame_number: if step > 0 { i64::from(now / step) } else { 0 },
    });
    Ok(())
}

/// Hands the core one checked-out frame at a time, oldest first, each checked back in as
/// soon as the core is done with it. The current frame arrives already checked out: After
/// Effects refuses the output until at least one input is.
struct LayerFrames<'a> {
    callbacks: ae::SmartRenderCallbacks,
    frame_count: usize,
    current: Option<&'a ae::Layer>,
    error: Option<ae::Error>,
}

impl<C: Channel> FrameSource<C> for LayerFrames<'_> {
    fn with_frame(&mut self, index: usize, f: &mut dyn FnMut(Option<View<'_, C>>)) {
        let id = (self.frame_count - 1 - index) as i32;
        if id == CURRENT_ID {
            f(self.current.and_then(view::<C>));
            return;
        }
        match self.callbacks.checkout_layer_pixels(id as u32) {
            Ok(layer) => {
                f(layer.as_ref().and_then(view::<C>));
                if let Err(e) = self.callbacks.checkin_layer_pixels(id as u32) {
                    self.error.get_or_insert(e);
                }
            }
            Err(e) => {
                self.error.get_or_insert(e);
                f(None);
            }
        }
    }
}

fn bits<C: Channel>() -> i16 {
    (size_of::<C>() * 8) as i16
}

/// A layer's pixels as the core's view, if it is at the depth being rendered.
fn view<C: Channel>(layer: &ae::Layer) -> Option<View<'_, C>> {
    let stride = usize::try_from(layer.row_bytes()).ok()?;
    (layer.bit_depth() == bits::<C>()).then(|| {
        View::new(
            layer.buffer(),
            layer.width() as i32,
            layer.height() as i32,
            stride,
        )
    })
}

fn smart_render(extra: &ae::SmartRenderExtra) -> Result<(), ae::Error> {
    let plan = extra
        .pre_render_data::<Plan>()
        .ok_or(ae::Error::BadCallbackParameter)?;
    let callbacks = extra.callbacks();
    let current = callbacks.checkout_layer_pixels(CURRENT_ID as u32)?;
    let result = (|| {
        let mut output = callbacks
            .checkout_output()?
            .ok_or(ae::Error::BadCallbackParameter)?;
        let mut frames = LayerFrames {
            callbacks,
            frame_count: plan.frame_count,
            current: current.as_ref(),
            error: None,
        };
        catch_unwind(AssertUnwindSafe(|| match output.bit_depth() {
            8 => render_as::<u8>(plan, &mut frames, &mut output),
            16 => render_as::<u16>(plan, &mut frames, &mut output),
            32 => render_as::<f32>(plan, &mut frames, &mut output),
            _ => Err(ae::Error::BadCallbackParameter),
        }))
        .unwrap_or(Err(ae::Error::BadCallbackParameter))?;
        frames.error.map_or(Ok(()), Err)
    })();
    // Every way out checks the current frame back in.
    callbacks.checkin_layer_pixels(CURRENT_ID as u32)?;
    result
}

fn render_as<C: Channel>(
    plan: &Plan,
    frames: &mut LayerFrames,
    output: &mut ae::Layer,
) -> Result<(), ae::Error> {
    let stride =
        usize::try_from(output.row_bytes()).map_err(|_| ae::Error::BadCallbackParameter)?;
    let (width, height) = (output.width() as i32, output.height() as i32);
    let mut out = ViewMut::<C>::new(output.buffer_mut(), width, height, stride);
    render::render(
        &plan.settings,
        frames,
        plan.frame_count,
        plan.frame_number,
        &mut out,
    );
    Ok(())
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

    // After Effects renders through SmartFX; this path is for hosts that don't. They give no
    // earlier frames, so it renders the current frame alone, as the first frame of a layer.
    fn render(
        &self,
        plugin: &mut PluginState,
        in_layer: &ae::Layer,
        out_layer: &mut ae::Layer,
    ) -> Result<(), ae::Error> {
        let settings = read_settings(plugin.params)?;
        let frame_number = match plugin.in_data.time_step() {
            step if step > 0 => i64::from(plugin.in_data.current_time() / step),
            _ => 0,
        };
        let plan = Plan {
            settings,
            frame_count: 1,
            frame_number,
        };
        let mut one = OneFrame(in_layer);
        let stride =
            usize::try_from(out_layer.row_bytes()).map_err(|_| ae::Error::BadCallbackParameter)?;
        let (width, height) = (out_layer.width() as i32, out_layer.height() as i32);
        match out_layer.bit_depth() {
            8 => {
                let mut out = ViewMut::<u8>::new(out_layer.buffer_mut(), width, height, stride);
                render::render(&plan.settings, &mut one, 1, plan.frame_number, &mut out);
            }
            16 => {
                let mut out = ViewMut::<u16>::new(out_layer.buffer_mut(), width, height, stride);
                render::render(&plan.settings, &mut one, 1, plan.frame_number, &mut out);
            }
            _ => return Err(ae::Error::BadCallbackParameter),
        }
        Ok(())
    }

    fn handle_command(
        &mut self,
        _plugin: &mut PluginState,
        _command: ae::Command,
    ) -> Result<(), ae::Error> {
        Ok(())
    }
}

struct OneFrame<'a>(&'a ae::Layer);

impl<C: Channel> FrameSource<C> for OneFrame<'_> {
    fn with_frame(&mut self, _index: usize, f: &mut dyn FnMut(Option<View<'_, C>>)) {
        f(view::<C>(self.0));
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
/// and current After Effects builds may look for version 3 first. Kind `eFKT` and API
/// version 13.0 match `PF_PLUG_IN_VERSION` in the SDK headers. Name, match name and
/// category must match the PiPL in `build.rs`.
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
            c"segcam Motion (Rust)".as_ptr().cast(),
            c"LOFISCIFI segcam Motion Rust".as_ptr().cast(),
            c"LO.FI.SCI.FI".as_ptr().cast(),
            c"EffectMain".as_ptr().cast(),
            EFKT,
            13,
            0,
            0,
            c"".as_ptr().cast(),
            c"motion segmentation swarm".as_ptr().cast(),
            c"Motion segmentation from segcam: overlay, mattes and the swarm collage."
                .as_ptr()
                .cast(),
        )
    }
}
