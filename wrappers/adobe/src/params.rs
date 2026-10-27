//! After Effects controls, and reading them into plain settings for one frame.
//! This is the only place that knows both the After Effects parameter API and the
//! core effects' value ranges.

use after_effects as ae;
use vfx_core::effects::{BLEND_MODES, BLUR_DIRECTIONS};

#[derive(Eq, PartialEq, Hash, Clone, Copy, Debug)]
pub enum Params {
    LinearLight,
    ColorEnable,
    Brightness,
    Contrast,
    Saturation,
    VignetteEnable,
    VignetteCenter,
    VignetteRadius,
    VignetteSoftness,
    VignetteColor,
    VignetteAmount,
    BlendLayer,
    BlendMode,
    BlendAmount,
    BlurEnable,
    BlurRadius,
    BlurDirection,
    BlurGrow,
}

pub fn setup(params: &mut ae::Parameters<Params>) -> Result<(), ae::Error> {
    checkbox(
        params,
        Params::LinearLight,
        "Linear Light",
        "Process in linear light",
        false,
    )?;

    checkbox(params, Params::ColorEnable, "Color Adjust", "On", true)?;
    slider(params, Params::Brightness, "Brightness", -1.0, 1.0, 0.0)?;
    slider(params, Params::Contrast, "Contrast", 0.0, 4.0, 1.0)?;
    slider(params, Params::Saturation, "Saturation", 0.0, 4.0, 1.0)?;

    checkbox(params, Params::VignetteEnable, "Vignette", "On", false)?;
    params.add(
        Params::VignetteCenter,
        "Vignette Center",
        ae::PointDef::setup(|p| {
            // Percent of the layer, so the default is the middle.
            p.set_default((50.0, 50.0));
        }),
    )?;
    slider(
        params,
        Params::VignetteRadius,
        "Vignette Radius",
        0.0,
        2.0,
        0.75,
    )?;
    slider(
        params,
        Params::VignetteSoftness,
        "Vignette Softness",
        0.0,
        2.0,
        0.5,
    )?;
    params.add(
        Params::VignetteColor,
        "Vignette Color",
        ae::ColorDef::setup(|c| {
            c.set_default(ae::Pixel8 {
                alpha: 255,
                red: 0,
                green: 0,
                blue: 0,
            });
        }),
    )?;
    slider(
        params,
        Params::VignetteAmount,
        "Vignette Amount",
        0.0,
        1.0,
        1.0,
    )?;

    params.add(Params::BlendLayer, "Blend Layer", ae::LayerDef::new())?;
    popup(params, Params::BlendMode, "Blend Mode", BLEND_MODES, 0)?;
    slider(params, Params::BlendAmount, "Blend Amount", 0.0, 1.0, 1.0)?;

    checkbox(params, Params::BlurEnable, "Blur", "On", true)?;
    slider(params, Params::BlurRadius, "Blur Radius", 0.0, 256.0, 0.0)?;
    popup(
        params,
        Params::BlurDirection,
        "Blur Direction",
        BLUR_DIRECTIONS,
        0,
    )?;
    checkbox(
        params,
        Params::BlurGrow,
        "Blur Edges",
        "Grow past layer bounds",
        false,
    )?;
    Ok(())
}

fn slider(
    params: &mut ae::Parameters<Params>,
    id: Params,
    name: &str,
    min: f32,
    max: f32,
    default: f32,
) -> Result<(), ae::Error> {
    params.add(
        id,
        name,
        ae::FloatSliderDef::setup(|f| {
            f.set_valid_min(min);
            f.set_valid_max(max);
            f.set_slider_min(min);
            f.set_slider_max(max);
            f.set_default(default as f64);
            f.set_precision(2);
        }),
    )
}

fn checkbox(
    params: &mut ae::Parameters<Params>,
    id: Params,
    name: &str,
    label: &str,
    default: bool,
) -> Result<(), ae::Error> {
    params.add(
        id,
        name,
        ae::CheckBoxDef::setup(|c| {
            c.set_default(default);
            c.set_label(label);
        }),
    )
}

/// After Effects popups are 1-based; `default` here is 0-based like the core's options.
fn popup(
    params: &mut ae::Parameters<Params>,
    id: Params,
    name: &str,
    options: &[&str],
    default: i32,
) -> Result<(), ae::Error> {
    params.add(
        id,
        name,
        ae::PopupDef::setup(|p| {
            p.set_options(options);
            p.set_default(default + 1);
        }),
    )
}

/// Everything one frame needs, already converted to the core's units.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub linear: bool,
    /// Brightness, contrast, saturation, when the stage is on.
    pub color: Option<[f32; 3]>,
    pub vignette: Option<Vignette>,
    pub blend_mode: i32,
    pub blend_amount: f32,
    pub blur: Option<Blur>,
}

#[derive(Clone, Copy, Debug)]
pub struct Vignette {
    /// Layer pixel coordinates at the current preview resolution.
    pub center_px: [f32; 2],
    pub radius: f32,
    pub softness: f32,
    pub color: [f32; 4],
    pub amount: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Blur {
    /// Horizontal radius in buffer pixels, already scaled for preview downsampling.
    pub radius: f32,
    /// Vertical radius ÷ horizontal radius, for uneven downsampling.
    pub vertical_scale: f32,
    pub direction: i32,
    pub grow: bool,
}

impl Blur {
    /// Pixels the blur reaches beyond each edge, horizontally and vertically.
    pub fn reach(&self) -> (i32, i32) {
        let x = if self.direction == 2 {
            0.0
        } else {
            self.radius
        };
        let y = if self.direction == 1 {
            0.0
        } else {
            self.radius * self.vertical_scale
        };
        (x.ceil() as i32, y.ceil() as i32)
    }
}

impl Settings {
    pub fn read(params: &ae::Parameters<Params>, in_data: &ae::InData) -> Result<Self, ae::Error> {
        // Slider values are in full-resolution pixels. At Half, Third or Quarter preview
        // resolution, After Effects hands us a smaller buffer, so pixel distances shrink
        // by the same factor.
        let scale = |r: ae::RationalScale| {
            if r.den == 0 {
                1.0
            } else {
                r.num as f32 / r.den as f32
            }
        };
        let (dx, dy) = (scale(in_data.downsample_x()), scale(in_data.downsample_y()));

        let color = checkbox_value(params, Params::ColorEnable)?.then_some([
            float(params, Params::Brightness)?,
            float(params, Params::Contrast)?,
            float(params, Params::Saturation)?,
        ]);
        let vignette = if checkbox_value(params, Params::VignetteEnable)? {
            Some(Vignette {
                center_px: point(params, Params::VignetteCenter)?,
                radius: float(params, Params::VignetteRadius)?,
                softness: float(params, Params::VignetteSoftness)?,
                color: color_value(params, Params::VignetteColor)?,
                amount: float(params, Params::VignetteAmount)?,
            })
        } else {
            None
        };
        let blur_radius = float(params, Params::BlurRadius)? * dx;
        let blur =
            (checkbox_value(params, Params::BlurEnable)? && blur_radius > 0.0).then_some(Blur {
                radius: blur_radius,
                vertical_scale: if dx > 0.0 { dy / dx } else { 1.0 },
                direction: popup_value(params, Params::BlurDirection)?,
                grow: checkbox_value(params, Params::BlurGrow)?,
            });
        Ok(Self {
            linear: checkbox_value(params, Params::LinearLight)?,
            color,
            vignette,
            blend_mode: popup_value(params, Params::BlendMode)?,
            blend_amount: float(params, Params::BlendAmount)?,
            blur,
        })
    }

    /// How far the blur reads past each edge; (0, 0) without blur.
    pub fn blur_reach(&self) -> (i32, i32) {
        self.blur.map_or((0, 0), |b| b.reach())
    }

    /// Whether the output should extend past the input's bounds.
    pub fn grows(&self) -> bool {
        self.blur.is_some_and(|b| b.grow)
    }
}

fn float(params: &ae::Parameters<Params>, id: Params) -> Result<f32, ae::Error> {
    match params.get(id)?.as_param()? {
        ae::Param::FloatSlider(s) => Ok(s.value() as f32),
        _ => Err(ae::Error::BadCallbackParameter),
    }
}

fn checkbox_value(params: &ae::Parameters<Params>, id: Params) -> Result<bool, ae::Error> {
    match params.get(id)?.as_param()? {
        ae::Param::CheckBox(c) => Ok(c.value()),
        _ => Err(ae::Error::BadCallbackParameter),
    }
}

/// 0-based option index.
fn popup_value(params: &ae::Parameters<Params>, id: Params) -> Result<i32, ae::Error> {
    match params.get(id)?.as_param()? {
        ae::Param::Popup(p) => Ok(p.value() - 1),
        _ => Err(ae::Error::BadCallbackParameter),
    }
}

fn point(params: &ae::Parameters<Params>, id: Params) -> Result<[f32; 2], ae::Error> {
    match params.get(id)?.as_param()? {
        ae::Param::Point(p) => {
            let (x, y) = p.value();
            Ok([x, y])
        }
        _ => Err(ae::Error::BadCallbackParameter),
    }
}

fn color_value(params: &ae::Parameters<Params>, id: Params) -> Result<[f32; 4], ae::Error> {
    match params.get(id)?.as_param()? {
        ae::Param::Color(c) => {
            // Prefer the float colour (it keeps precision in 16/32 bpc projects).
            if let Ok(f) = c.float_value() {
                Ok([f.red, f.green, f.blue, f.alpha])
            } else {
                let p = c.value();
                let n = |v: u8| f32::from(v) / 255.0;
                Ok([n(p.red), n(p.green), n(p.blue), n(p.alpha)])
            }
        }
        _ => Err(ae::Error::BadCallbackParameter),
    }
}
