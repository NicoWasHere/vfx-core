//! The plugin's FFGL parameters, matching the After Effects and OpenFX controls where
//! FFGL can express them. Values are stored in their real units: FFGL hosts send and
//! read values inside a parameter's declared range.

use std::ffi::CStr;

use vfx_wrapper_common::{Blur, Settings, Vignette};

use crate::ffi::*;

pub struct Param {
    pub name: &'static CStr,
    pub kind: u32,
    pub default: f32,
    pub min: f32,
    pub max: f32,
    pub group: &'static str,
    /// Option labels, for FF_TYPE_OPTION. The value is the option's index.
    pub options: &'static [&'static CStr],
}

const fn toggle(name: &'static CStr, default: bool, group: &'static str) -> Param {
    Param {
        name,
        kind: FF_TYPE_BOOLEAN,
        default: if default { 1.0 } else { 0.0 },
        min: 0.0,
        max: 1.0,
        group,
        options: &[],
    }
}

const fn slider(
    name: &'static CStr,
    min: f32,
    max: f32,
    default: f32,
    group: &'static str,
) -> Param {
    Param {
        name,
        kind: FF_TYPE_STANDARD,
        default,
        min,
        max,
        group,
        options: &[],
    }
}

const fn typed(name: &'static CStr, kind: u32, default: f32, group: &'static str) -> Param {
    Param {
        name,
        kind,
        default,
        min: 0.0,
        max: 1.0,
        group,
        options: &[],
    }
}

pub const LINEAR: usize = 0;
pub const COLOR_ENABLE: usize = 1;
pub const BRIGHTNESS: usize = 2;
pub const CONTRAST: usize = 3;
pub const SATURATION: usize = 4;
pub const VIGNETTE_ENABLE: usize = 5;
pub const VIGNETTE_X: usize = 6;
pub const VIGNETTE_Y: usize = 7;
pub const VIGNETTE_RADIUS: usize = 8;
pub const VIGNETTE_SOFTNESS: usize = 9;
pub const VIGNETTE_RED: usize = 10;
pub const VIGNETTE_GREEN: usize = 11;
pub const VIGNETTE_BLUE: usize = 12;
pub const VIGNETTE_AMOUNT: usize = 13;
pub const BLUR_ENABLE: usize = 14;
pub const BLUR_RADIUS: usize = 15;
pub const BLUR_DIRECTION: usize = 16;

/// Option labels match the core's BLUR_DIRECTIONS (checked in a test).
const DIRECTIONS: &[&CStr] = &[c"Both", c"Horizontal", c"Vertical"];

pub const PARAMS: &[Param] = &[
    toggle(c"Linear Light", false, ""),
    toggle(c"Color Adjust", true, "Color"),
    slider(c"Brightness", -1.0, 1.0, 0.0, "Color"),
    slider(c"Contrast", 0.0, 4.0, 1.0, "Color"),
    slider(c"Saturation", 0.0, 4.0, 1.0, "Color"),
    toggle(c"Vignette", false, "Vignette"),
    // XPOS / YPOS: normalised, origin at the bottom left, as Resolume shows them.
    typed(c"Vignette X", FF_TYPE_XPOS, 0.5, "Vignette"),
    typed(c"Vignette Y", FF_TYPE_YPOS, 0.5, "Vignette"),
    slider(c"Vignette Radius", 0.0, 2.0, 0.75, "Vignette"),
    slider(c"Vignette Soft", 0.0, 1.0, 0.5, "Vignette"),
    // Consecutive RED / GREEN / BLUE params become one colour picker in Resolume.
    typed(c"Vignette Red", FF_TYPE_RED, 0.0, "Vignette"),
    typed(c"Vignette Green", FF_TYPE_GREEN, 0.0, "Vignette"),
    typed(c"Vignette Blue", FF_TYPE_BLUE, 0.0, "Vignette"),
    slider(c"Vignette Amount", 0.0, 1.0, 1.0, "Vignette"),
    toggle(c"Blur", true, "Blur"),
    slider(c"Blur Radius", 0.0, 256.0, 0.0, "Blur"),
    Param {
        name: c"Blur Direction",
        kind: FF_TYPE_OPTION,
        default: 0.0,
        min: 0.0,
        max: 2.0,
        group: "Blur",
        options: DIRECTIONS,
    },
];

/// Formats a value for the host's 16-character parameter display.
pub fn display(index: usize, value: f32) -> String {
    let p = &PARAMS[index];
    match p.kind {
        FF_TYPE_BOOLEAN => if value >= 0.5 { "On" } else { "Off" }.to_owned(),
        FF_TYPE_OPTION => p
            .options
            .get(value.round().max(0.0) as usize)
            .map(|o| o.to_string_lossy().into_owned())
            .unwrap_or_default(),
        _ => format!("{value:.2}"),
    }
}

/// Clamps a host value into the parameter's range; options snap to an index.
pub fn clamp(index: usize, value: f32) -> f32 {
    let p = &PARAMS[index];
    let v = value.clamp(p.min, p.max);
    if p.kind == FF_TYPE_OPTION {
        v.round()
    } else {
        v
    }
}

/// One frame's settings for a `width` x `height` frame.
pub fn settings(values: &[f32], width: u32, height: u32) -> Settings {
    let on = |i: usize| values[i] >= 0.5;
    Settings {
        linear: on(LINEAR),
        color: on(COLOR_ENABLE).then(|| [values[BRIGHTNESS], values[CONTRAST], values[SATURATION]]),
        vignette: on(VIGNETTE_ENABLE).then(|| Vignette {
            // Normalised, y up, to top-down canvas pixels.
            center_px: [
                values[VIGNETTE_X] * width as f32,
                (1.0 - values[VIGNETTE_Y]) * height as f32,
            ],
            radius: values[VIGNETTE_RADIUS],
            softness: values[VIGNETTE_SOFTNESS],
            color: [
                values[VIGNETTE_RED],
                values[VIGNETTE_GREEN],
                values[VIGNETTE_BLUE],
                1.0,
            ],
            amount: values[VIGNETTE_AMOUNT],
        }),
        blend_mode: 0,
        blend_amount: 0.0,
        blur: (on(BLUR_ENABLE) && values[BLUR_RADIUS] > 0.0).then(|| Blur {
            radius: values[BLUR_RADIUS],
            vertical_scale: 1.0,
            direction: values[BLUR_DIRECTION].round() as i32,
            grow: false,
        }),
    }
}

pub fn defaults() -> Vec<f32> {
    PARAMS.iter().map(|p| p.default).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use vfx_core::effects::BLUR_DIRECTIONS;

    #[test]
    fn direction_labels_match_the_core() {
        let labels: Vec<String> = DIRECTIONS
            .iter()
            .map(|c| c.to_string_lossy().into_owned())
            .collect();
        assert_eq!(labels, BLUR_DIRECTIONS);
    }

    #[test]
    fn indices_name_the_right_params() {
        assert_eq!(PARAMS[BRIGHTNESS].name, c"Brightness");
        assert_eq!(PARAMS[VIGNETTE_Y].name, c"Vignette Y");
        assert_eq!(PARAMS[VIGNETTE_AMOUNT].name, c"Vignette Amount");
        assert_eq!(PARAMS[BLUR_DIRECTION].name, c"Blur Direction");
        assert_eq!(PARAMS.len(), BLUR_DIRECTION + 1);
    }

    #[test]
    fn names_stay_under_16_characters() {
        // FFGL hosts show 16 bytes of a name at most; Resolume cuts the rest. The
        // other wrappers use the same names, so this keeps them all short.
        for p in PARAMS {
            let n = p.name.to_bytes().len();
            assert!(n < 16, "{:?} is {n} bytes; keep names under 16", p.name);
        }
    }

    #[test]
    fn defaults_change_nothing() {
        let s = settings(&defaults(), 64, 48);
        assert!(!s.linear && s.vignette.is_none() && s.blur.is_none());
        assert_eq!(s.color, Some([0.0, 1.0, 1.0]));
    }

    #[test]
    fn vignette_y_is_flipped_to_top_down() {
        let mut v = defaults();
        v[VIGNETTE_ENABLE] = 1.0;
        v[VIGNETTE_X] = 0.25;
        v[VIGNETTE_Y] = 0.25;
        let c = settings(&v, 100, 80).vignette.unwrap().center_px;
        assert_eq!(c, [25.0, 60.0]);
    }

    #[test]
    fn values_clamp_and_options_snap() {
        assert_eq!(clamp(BRIGHTNESS, 3.0), 1.0);
        assert_eq!(clamp(BLUR_DIRECTION, 1.4), 1.0);
        assert_eq!(display(BLUR_DIRECTION, 2.0), "Vertical");
        assert_eq!(display(BLUR_ENABLE, 1.0), "On");
    }
}
