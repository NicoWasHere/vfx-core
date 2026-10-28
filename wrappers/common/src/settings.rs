//! One frame's settings in the core's units, read from whatever host the plugin runs
//! in. Each wrapper fills these from its own parameter API.

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
    /// Layer pixel coordinates at the current render scale.
    pub center_px: [f32; 2],
    pub radius: f32,
    pub softness: f32,
    pub color: [f32; 4],
    pub amount: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Blur {
    /// Horizontal radius in buffer pixels, already scaled for the render scale.
    pub radius: f32,
    /// Vertical radius ÷ horizontal radius, for uneven render scales.
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
    /// How far the blur reads past each edge; (0, 0) without blur.
    pub fn blur_reach(&self) -> (i32, i32) {
        self.blur.map_or((0, 0), |b| b.reach())
    }

    /// Whether the output should extend past the input's bounds.
    pub fn grows(&self) -> bool {
        self.blur.is_some_and(|b| b.grow)
    }
}
