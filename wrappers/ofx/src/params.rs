//! OpenFX controls, matching the After Effects plugin's, and reading them at a time.

use std::ffi::{CStr, c_double, c_int};
use std::ptr;

use vfx_core::effects::{BLEND_MODES, BLUR_DIRECTIONS};

use crate::ffi::*;
use crate::host::{Props, check, suites};
use crate::optional;

const LINEAR: &CStr = c"linear";
const COLOR_ENABLE: &CStr = c"color_enable";
const BRIGHTNESS: &CStr = c"brightness";
const CONTRAST: &CStr = c"contrast";
const SATURATION: &CStr = c"saturation";
const VIGNETTE_ENABLE: &CStr = c"vignette_enable";
const VIGNETTE_CENTER: &CStr = c"vignette_center";
const VIGNETTE_RADIUS: &CStr = c"vignette_radius";
const VIGNETTE_SOFTNESS: &CStr = c"vignette_softness";
const VIGNETTE_COLOR: &CStr = c"vignette_color";
const VIGNETTE_AMOUNT: &CStr = c"vignette_amount";
const BLEND_MODE: &CStr = c"blend_mode";
const BLEND_AMOUNT: &CStr = c"blend_amount";
const BLUR_ENABLE: &CStr = c"blur_enable";
const BLUR_RADIUS: &CStr = c"blur_radius";
const BLUR_DIRECTION: &CStr = c"blur_direction";
const BLUR_GROW: &CStr = c"blur_grow";

/// One frame's control values, in full-resolution canonical units.
#[derive(Clone, Copy, Debug)]
pub struct Values {
    pub linear: bool,
    pub color: Option<[f32; 3]>,
    pub vignette: Option<VignetteValues>,
    pub blend_mode: i32,
    pub blend_amount: f32,
    pub blur: Option<BlurValues>,
}

#[derive(Clone, Copy, Debug)]
pub struct VignetteValues {
    /// Canonical coordinates: full-resolution pixels, origin at the bottom left.
    pub center: [f64; 2],
    pub radius: f32,
    pub softness: f32,
    pub color: [f32; 4],
    pub amount: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct BlurValues {
    /// Full-resolution pixels.
    pub radius: f32,
    pub direction: i32,
    pub grow: bool,
}

struct Definer {
    set: ParamSet,
}

impl Definer {
    fn define(
        &self,
        kind: &CStr,
        name: &CStr,
        label: &str,
        parent: Option<&CStr>,
    ) -> Result<Props, OfxStatus> {
        let mut props = ptr::null_mut();
        // SAFETY: a descriptor parameter set from the describe-in-context action.
        check(unsafe {
            (suites().param.paramDefine)(self.set, kind.as_ptr(), name.as_ptr(), &mut props)
        })?;
        let p = Props(props);
        p.set_string(PROP_LABEL, label)?;
        optional(
            PARAM_PROP_SCRIPT_NAME,
            p.set_str(PARAM_PROP_SCRIPT_NAME, 0, name),
        );
        if let Some(parent) = parent {
            optional(PARAM_PROP_PARENT, p.set_str(PARAM_PROP_PARENT, 0, parent));
        }
        Ok(p)
    }

    fn group(&self, name: &CStr, label: &str) -> Result<(), OfxStatus> {
        self.define(PARAM_TYPE_GROUP, name, label, None).map(|_| ())
    }

    fn toggle(
        &self,
        name: &CStr,
        label: &str,
        default: bool,
        parent: Option<&CStr>,
    ) -> Result<(), OfxStatus> {
        let p = self.define(PARAM_TYPE_BOOLEAN, name, label, parent)?;
        p.set_int(PARAM_PROP_DEFAULT, 0, default as i32)
    }

    #[allow(clippy::too_many_arguments)]
    fn slider(
        &self,
        name: &CStr,
        label: &str,
        min: f64,
        max: f64,
        default: f64,
        parent: &CStr,
    ) -> Result<(), OfxStatus> {
        let p = self.define(PARAM_TYPE_DOUBLE, name, label, Some(parent))?;
        p.set_double(PARAM_PROP_DEFAULT, 0, default)?;
        p.set_double(PARAM_PROP_MIN, 0, min)?;
        p.set_double(PARAM_PROP_MAX, 0, max)?;
        optional(
            PARAM_PROP_DISPLAY_MIN,
            p.set_double(PARAM_PROP_DISPLAY_MIN, 0, min),
        );
        optional(
            PARAM_PROP_DISPLAY_MAX,
            p.set_double(PARAM_PROP_DISPLAY_MAX, 0, max),
        );
        optional(
            PARAM_PROP_INCREMENT,
            p.set_double(PARAM_PROP_INCREMENT, 0, (max - min) / 200.0),
        );
        optional(PARAM_PROP_DIGITS, p.set_int(PARAM_PROP_DIGITS, 0, 2));
        Ok(())
    }

    fn choice(
        &self,
        name: &CStr,
        label: &str,
        options: &[&str],
        parent: &CStr,
    ) -> Result<(), OfxStatus> {
        let p = self.define(PARAM_TYPE_CHOICE, name, label, Some(parent))?;
        for (i, option) in options.iter().enumerate() {
            let option = std::ffi::CString::new(*option).map_err(|_| STAT_FAILED)?;
            p.set_str(PARAM_PROP_CHOICE_OPTION, i, &option)?;
        }
        p.set_int(PARAM_PROP_DEFAULT, 0, 0)
    }
}

/// Defines every control. `blend` is false in the filter context, which has no second
/// input clip to blend with.
pub fn define(set: ParamSet, blend: bool) -> Result<(), OfxStatus> {
    let d = Definer { set };
    d.toggle(LINEAR, "Linear Light", false, None)?;

    d.group(c"color_group", "Color")?;
    d.toggle(COLOR_ENABLE, "Color Adjust", true, Some(c"color_group"))?;
    d.slider(BRIGHTNESS, "Brightness", -1.0, 1.0, 0.0, c"color_group")?;
    d.slider(CONTRAST, "Contrast", 0.0, 4.0, 1.0, c"color_group")?;
    d.slider(SATURATION, "Saturation", 0.0, 4.0, 1.0, c"color_group")?;

    d.group(c"vignette_group", "Vignette")?;
    d.toggle(VIGNETTE_ENABLE, "Vignette", false, Some(c"vignette_group"))?;
    let center = d.define(
        PARAM_TYPE_DOUBLE_2D,
        VIGNETTE_CENTER,
        "Vignette Center",
        Some(c"vignette_group"),
    )?;
    center.set_str(PARAM_PROP_DOUBLE_TYPE, 0, DOUBLE_TYPE_XY_ABSOLUTE)?;
    center.set_str(PARAM_PROP_DEFAULT_COORDINATES, 0, COORDINATES_NORMALISED)?;
    center.set_doubles(PARAM_PROP_DEFAULT, &[0.5, 0.5])?;
    d.slider(
        VIGNETTE_RADIUS,
        "Vignette Radius",
        0.0,
        2.0,
        0.75,
        c"vignette_group",
    )?;
    d.slider(
        VIGNETTE_SOFTNESS,
        "Vignette Softness",
        0.0,
        1.0,
        0.5,
        c"vignette_group",
    )?;
    let color = d.define(
        PARAM_TYPE_RGBA,
        VIGNETTE_COLOR,
        "Vignette Color",
        Some(c"vignette_group"),
    )?;
    color.set_doubles(PARAM_PROP_DEFAULT, &[0.0, 0.0, 0.0, 1.0])?;
    d.slider(
        VIGNETTE_AMOUNT,
        "Vignette Amount",
        0.0,
        1.0,
        1.0,
        c"vignette_group",
    )?;

    if blend {
        d.group(c"blend_group", "Blend")?;
        d.choice(BLEND_MODE, "Blend Mode", BLEND_MODES, c"blend_group")?;
        d.slider(BLEND_AMOUNT, "Blend Amount", 0.0, 1.0, 1.0, c"blend_group")?;
    }

    d.group(c"blur_group", "Blur")?;
    d.toggle(BLUR_ENABLE, "Blur", true, Some(c"blur_group"))?;
    d.slider(BLUR_RADIUS, "Blur Radius", 0.0, 256.0, 0.0, c"blur_group")?;
    d.choice(
        BLUR_DIRECTION,
        "Blur Direction",
        BLUR_DIRECTIONS,
        c"blur_group",
    )?;
    d.toggle(BLUR_GROW, "Blur Edges", false, Some(c"blur_group"))?;
    Ok(())
}

/// An instance's parameters, read at one time.
pub struct Reader {
    set: ParamSet,
    time: OfxTime,
}

impl Reader {
    pub fn new(effect: ImageEffect, time: OfxTime) -> Result<Self, OfxStatus> {
        let mut set = ptr::null_mut();
        // SAFETY: an instance handle from the host.
        check(unsafe { (suites().effect.getParamSet)(effect, &mut set) })?;
        Ok(Self { set, time })
    }

    fn handle(&self, name: &CStr) -> Result<Option<Param>, OfxStatus> {
        let mut param = ptr::null_mut();
        // SAFETY: an instance parameter set.
        let status = unsafe {
            (suites().param.paramGetHandle)(self.set, name.as_ptr(), &mut param, ptr::null_mut())
        };
        // Missing means not defined in this context (Blend in the filter context).
        Ok((status == STAT_OK).then_some(param))
    }

    fn doubles<const N: usize>(&self, name: &CStr) -> Result<[f64; N], OfxStatus> {
        let p = self.handle(name)?.ok_or(STAT_FAILED)?;
        let mut v = [0.0 as c_double; N];
        let s = suites().param;
        // SAFETY: the parameter has N double components, written through these pointers.
        check(unsafe {
            match N {
                1 => (s.paramGetValueAtTime)(p, self.time, &mut v[0] as *mut c_double),
                2 => (s.paramGetValueAtTime)(
                    p,
                    self.time,
                    &mut v[0] as *mut c_double,
                    &mut v[1] as *mut c_double,
                ),
                4 => (s.paramGetValueAtTime)(
                    p,
                    self.time,
                    &mut v[0] as *mut c_double,
                    &mut v[1] as *mut c_double,
                    &mut v[2] as *mut c_double,
                    &mut v[3] as *mut c_double,
                ),
                _ => unreachable!("no parameter has {N} components"),
            }
        })?;
        Ok(v)
    }

    fn float(&self, name: &CStr) -> Result<f32, OfxStatus> {
        Ok(self.doubles::<1>(name)?[0] as f32)
    }

    /// Booleans and choices are both read as ints. `None` if not defined here.
    fn int(&self, name: &CStr) -> Result<Option<i32>, OfxStatus> {
        let Some(p) = self.handle(name)? else {
            return Ok(None);
        };
        let mut v: c_int = 0;
        // SAFETY: a boolean or choice parameter, written through one int pointer.
        check(unsafe { (suites().param.paramGetValueAtTime)(p, self.time, &mut v as *mut c_int) })?;
        Ok(Some(v))
    }

    fn flag(&self, name: &CStr) -> Result<bool, OfxStatus> {
        Ok(self.int(name)?.unwrap_or(0) != 0)
    }

    pub fn values(&self) -> Result<Values, OfxStatus> {
        let color = if self.flag(COLOR_ENABLE)? {
            Some([
                self.float(BRIGHTNESS)?,
                self.float(CONTRAST)?,
                self.float(SATURATION)?,
            ])
        } else {
            None
        };
        let vignette = if self.flag(VIGNETTE_ENABLE)? {
            let c = self.doubles::<4>(VIGNETTE_COLOR)?;
            Some(VignetteValues {
                center: self.doubles::<2>(VIGNETTE_CENTER)?,
                radius: self.float(VIGNETTE_RADIUS)?,
                softness: self.float(VIGNETTE_SOFTNESS)?,
                color: [c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32],
                amount: self.float(VIGNETTE_AMOUNT)?,
            })
        } else {
            None
        };
        let radius = self.float(BLUR_RADIUS)?;
        let blur = (self.flag(BLUR_ENABLE)? && radius > 0.0)
            .then(|| -> Result<_, OfxStatus> {
                Ok(BlurValues {
                    radius,
                    direction: self.int(BLUR_DIRECTION)?.unwrap_or(0),
                    grow: self.flag(BLUR_GROW)?,
                })
            })
            .transpose()?;
        Ok(Values {
            linear: self.flag(LINEAR)?,
            color,
            vignette,
            blend_mode: self.int(BLEND_MODE)?.unwrap_or(0),
            blend_amount: match self.handle(BLEND_AMOUNT)? {
                Some(_) => self.float(BLEND_AMOUNT)?,
                None => 0.0,
            },
            blur,
        })
    }
}
