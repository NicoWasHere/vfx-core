//! The plugin's FFGL parameters, matching the After Effects and OpenFX controls where
//! FFGL can express them. Values are stored in their real units: FFGL hosts send and
//! read values inside a parameter's declared range.

use std::ffi::CStr;

use segcam_motion::{Color, MotionSettings, OutputMode, RenderSettings};

use vfx_ffgl_common::ffi::*;

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

pub const PARAMS: &[Param] = &[
    Param {
        name: c"Output",
        kind: FF_TYPE_OPTION,
        default: 0.0,
        min: 0.0,
        max: 3.0,
        group: "",
        options: &[c"Overlay", c"Motion matte", c"Box matte", c"Swarm"],
    },
    slider(c"Sensitivity", 0.0, 100.0, 86.4, "Motion"),
    slider(c"Adaptation", 0.1, 50.0, 6.0, "Motion"),
    slider(c"Background", 1.0, 120.0, 16.0, "Motion"),
    slider(c"Minimum size", 0.0, 25.0, 0.25, "Motion"),
    slider(c"Max segments", 1.0, 256.0, 16.0, "Motion"),
    slider(c"Grid width", 32.0, 1024.0, 192.0, "Motion"),
    toggle(c"Mirror", false, ""),
    toggle(c"Boxes", true, "Overlay"),
    toggle(c"Labels", true, "Overlay"),
    typed(c"Box red", FF_TYPE_RED, 0.25, "Overlay"),
    typed(c"Box green", FF_TYPE_GREEN, 1.0, "Overlay"),
    typed(c"Box blue", FF_TYPE_BLUE, 0.4, "Overlay"),
    slider(c"Trail frames", 1.0, 120.0, 24.0, "Swarm"),
    slider(c"Swarm max tiles", 1.0, 1000.0, 100.0, "Swarm"),
];

pub fn defaults() -> Vec<f32> {
    PARAMS.iter().map(|p| p.default).collect()
}

fn integer(index: usize) -> bool {
    matches!(index, 3 | 5 | 6 | 13 | 14)
}

pub fn clamp(index: usize, value: f32) -> f32 {
    let p = &PARAMS[index];
    let v = if value.is_finite() {
        value.clamp(p.min, p.max)
    } else {
        p.default
    };
    if p.kind == FF_TYPE_OPTION || integer(index) {
        v.round()
    } else {
        v
    }
}

pub fn display(index: usize, value: f32) -> String {
    match PARAMS[index].kind {
        FF_TYPE_OPTION => PARAMS[index].options[value as usize]
            .to_string_lossy()
            .into_owned(),
        FF_TYPE_BOOLEAN => if value >= 0.5 { "On" } else { "Off" }.to_owned(),
        _ if integer(index) => format!("{value:.0}"),
        _ => format!("{value:.2}"),
    }
}

pub fn settings(v: &[f32]) -> RenderSettings {
    RenderSettings {
        mode: OutputMode::from_index(v[0] as i32 + 1).unwrap_or(OutputMode::Overlay),
        motion: MotionSettings {
            sensitivity: f64::from(v[1]) / 100.0,
            adaptation: v[2] / 100.0,
            min_area_fraction: f64::from(v[4]) / 100.0,
            max_segments: v[5] as i32,
        },
        background_frames: v[3] as i32,
        grid_width: v[6] as i32,
        mirror: v[7] >= 0.5,
        boxes: v[8] >= 0.5,
        labels: v[9] >= 0.5,
        box_color: Color {
            r: v[10],
            g: v[11],
            b: v[12],
        },
        trail_frames: v[13] as i32,
        max_tiles: v[14] as i32,
        straight_alpha: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_fit_ffgl() {
        for p in PARAMS {
            assert!(p.name.to_bytes().len() < 16, "{:?}", p.name);
        }
        assert!(b"segcam Motion".len() <= 16);
    }
    #[test]
    fn modes_match_core_and_ranges_hold() {
        for (option, name) in PARAMS[0].options.iter().zip(OutputMode::NAMES) {
            assert_eq!(option.to_str().unwrap(), name);
        }
        for (i, p) in PARAMS.iter().enumerate() {
            assert_eq!(clamp(i, f32::NAN), p.default);
            assert_eq!(clamp(i, -f32::MAX), p.min);
            assert_eq!(clamp(i, f32::MAX), p.max);
        }
        assert_eq!(clamp(3, 2.8), 3.0);
    }
}
