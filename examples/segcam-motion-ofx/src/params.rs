//! segcam's controls in the same units, ranges and order as After Effects.
use segcam_motion::{OutputMode, RenderSettings};
use std::{ffi::CStr, ptr};
use vfx_ofx::ffi::*;
use vfx_ofx::host::{Props, check, suites};

struct Control {
    name: &'static CStr,
    label: &'static str,
    kind: &'static CStr,
    default: f64,
    range: [f64; 4],
}
const INT: &CStr = c"OfxParamTypeInteger";
const RGB: &CStr = c"OfxParamTypeRGB";
const CONTROLS: &[Control] = &[
    Control {
        name: c"output",
        label: "Output",
        kind: PARAM_TYPE_CHOICE,
        default: 0.0,
        range: [0.0, 3.0, 0.0, 3.0],
    },
    Control {
        name: c"sensitivity",
        label: "Sensitivity",
        kind: PARAM_TYPE_DOUBLE,
        default: 86.4,
        range: [0.0, 100.0, 0.0, 100.0],
    },
    Control {
        name: c"adaptation",
        label: "Adaptation",
        kind: PARAM_TYPE_DOUBLE,
        default: 6.0,
        range: [0.1, 50.0, 0.1, 25.0],
    },
    Control {
        name: c"background",
        label: "Background",
        kind: INT,
        default: 16.0,
        range: [1.0, 120.0, 1.0, 60.0],
    },
    Control {
        name: c"min_size",
        label: "Minimum size",
        kind: PARAM_TYPE_DOUBLE,
        default: 0.25,
        range: [0.0, 25.0, 0.0, 5.0],
    },
    Control {
        name: c"max_segments",
        label: "Max segments",
        kind: INT,
        default: 16.0,
        range: [1.0, 256.0, 1.0, 64.0],
    },
    Control {
        name: c"grid",
        label: "Grid width",
        kind: INT,
        default: 192.0,
        range: [32.0, 1024.0, 64.0, 384.0],
    },
    Control {
        name: c"mirror",
        label: "Mirror",
        kind: PARAM_TYPE_BOOLEAN,
        default: 0.0,
        range: [0.0, 1.0, 0.0, 1.0],
    },
    Control {
        name: c"boxes",
        label: "Boxes",
        kind: PARAM_TYPE_BOOLEAN,
        default: 1.0,
        range: [0.0, 1.0, 0.0, 1.0],
    },
    Control {
        name: c"labels",
        label: "Labels",
        kind: PARAM_TYPE_BOOLEAN,
        default: 1.0,
        range: [0.0, 1.0, 0.0, 1.0],
    },
    Control {
        name: c"box_color",
        label: "Box color",
        kind: RGB,
        default: 0.0,
        range: [0.0, 1.0, 0.0, 1.0],
    },
    Control {
        name: c"trail",
        label: "Trail frames",
        kind: INT,
        default: 24.0,
        range: [1.0, 120.0, 1.0, 60.0],
    },
    Control {
        name: c"max_tiles",
        label: "Swarm max tiles",
        kind: INT,
        default: 100.0,
        range: [1.0, 1000.0, 1.0, 300.0],
    },
];
pub fn define(set: ParamSet) -> Result<(), OfxStatus> {
    for c in CONTROLS {
        let mut p = ptr::null_mut();
        // SAFETY: descriptor parameter set, static strings, writable property handle.
        check(unsafe {
            (suites().param.paramDefine)(set, c.kind.as_ptr(), c.name.as_ptr(), &mut p)
        })?;
        let p = Props(p);
        p.set_string(PROP_LABEL, c.label)?;
        p.set_str(PARAM_PROP_SCRIPT_NAME, 0, c.name)?;
        if c.kind == PARAM_TYPE_DOUBLE || c.kind == RGB {
            p.set_double(PARAM_PROP_DEFAULT, 0, c.default)?;
            for (name, value) in [
                PARAM_PROP_MIN,
                PARAM_PROP_MAX,
                PARAM_PROP_DISPLAY_MIN,
                PARAM_PROP_DISPLAY_MAX,
            ]
            .into_iter()
            .zip(c.range)
            {
                p.set_double(name, 0, value)?;
            }
        } else {
            p.set_int(PARAM_PROP_DEFAULT, 0, c.default as i32)?;
            if c.kind == INT {
                for (name, value) in [
                    PARAM_PROP_MIN,
                    PARAM_PROP_MAX,
                    PARAM_PROP_DISPLAY_MIN,
                    PARAM_PROP_DISPLAY_MAX,
                ]
                .into_iter()
                .zip(c.range)
                {
                    p.set_int(name, 0, value as i32)?;
                }
            }
        }
        if c.kind == PARAM_TYPE_CHOICE {
            for (i, label) in OutputMode::NAMES.iter().enumerate() {
                let label = std::ffi::CString::new(*label).unwrap();
                p.set_str(PARAM_PROP_CHOICE_OPTION, i, &label)?;
            }
        }
        if c.kind == RGB {
            p.set_doubles(PARAM_PROP_DEFAULT, &[0.25, 1.0, 0.4])?;
        }
    }
    Ok(())
}
pub fn read(effect: ImageEffect, time: f64) -> Result<RenderSettings, OfxStatus> {
    let mut set = ptr::null_mut();
    // SAFETY: instance handle and writable parameter set pointer.
    check(unsafe { (suites().effect.getParamSet)(effect, &mut set) })?;
    let handle = |name: &CStr| -> Result<Param, OfxStatus> {
        let mut p = ptr::null_mut();
        // SAFETY: valid parameter set, static name and writable handle pointer.
        check(unsafe {
            (suites().param.paramGetHandle)(set, name.as_ptr(), &mut p, ptr::null_mut())
        })?;
        Ok(p)
    };
    let int = |name| -> Result<i32, OfxStatus> {
        let mut v = 0i32;
        // SAFETY: integer, boolean and choice parameters write one int.
        check(unsafe { (suites().param.paramGetValueAtTime)(handle(name)?, time, &mut v) })?;
        Ok(v)
    };
    let double = |name| -> Result<f64, OfxStatus> {
        let mut v = 0.0f64;
        // SAFETY: double parameters write one double.
        check(unsafe { (suites().param.paramGetValueAtTime)(handle(name)?, time, &mut v) })?;
        Ok(v)
    };
    let mut s = RenderSettings {
        mode: OutputMode::from_index(int(c"output")? + 1).ok_or(STAT_FAILED)?,
        background_frames: int(c"background")?,
        grid_width: int(c"grid")?,
        mirror: int(c"mirror")? != 0,
        boxes: int(c"boxes")? != 0,
        labels: int(c"labels")? != 0,
        trail_frames: int(c"trail")?,
        max_tiles: int(c"max_tiles")?,
        ..RenderSettings::default()
    };
    s.motion.sensitivity = double(c"sensitivity")? / 100.0;
    s.motion.adaptation = (double(c"adaptation")? / 100.0) as f32;
    s.motion.min_area_fraction = double(c"min_size")? / 100.0;
    s.motion.max_segments = int(c"max_segments")?;
    let (mut r, mut g, mut b) = (0.0f64, 0.0f64, 0.0f64);
    // SAFETY: an RGB parameter writes three doubles.
    check(unsafe {
        (suites().param.paramGetValueAtTime)(handle(c"box_color")?, time, &mut r, &mut g, &mut b)
    })?;
    s.box_color = segcam_motion::pixel::Color {
        r: r as f32,
        g: g as f32,
        b: b as f32,
    };
    Ok(s)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn labels_fit_hosts() {
        for c in CONTROLS {
            assert!(c.label.len() < 16, "{}", c.label);
        }
    }
}
