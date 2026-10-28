//! The control panel: every effect's parameters, with the same defaults and ranges
//! as the After Effects, OpenFX and FFGL plugins, plus the preview's own input and
//! feedback controls.

use egui::{Color32, RichText, Ui};
use vfx_core::effects::{BLEND_MODES, BLUR_DIRECTIONS};
use vfx_wrapper_common::{Blur, Settings, Vignette};

use crate::effects::PATTERNS;

#[derive(Clone, PartialEq)]
pub struct Controls {
    pub pattern: usize,
    pub animate: bool,
    pub linear: bool,
    pub color_on: bool,
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub vignette_on: bool,
    /// Normalised, origin at the top left.
    pub vignette_center: [f32; 2],
    pub vignette_radius: f32,
    pub vignette_softness: f32,
    pub vignette_color: [f32; 3],
    pub vignette_amount: f32,
    pub blend_on: bool,
    pub blend_pattern: usize,
    pub blend_mode: usize,
    pub blend_amount: f32,
    pub blur_on: bool,
    pub blur_radius: f32,
    pub blur_direction: usize,
    pub feedback_on: bool,
    pub feedback_decay: f32,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            pattern: 0,
            animate: true,
            linear: false,
            color_on: true,
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            vignette_on: false,
            vignette_center: [0.5, 0.5],
            vignette_radius: 0.75,
            vignette_softness: 0.5,
            vignette_color: [0.0, 0.0, 0.0],
            vignette_amount: 1.0,
            blend_on: false,
            blend_pattern: 1,
            blend_mode: 3,
            blend_amount: 0.5,
            blur_on: true,
            blur_radius: 0.0,
            blur_direction: 0,
            feedback_on: false,
            feedback_decay: 0.85,
        }
    }
}

impl Controls {
    /// One frame's pipeline settings for a `size` source.
    pub fn settings(&self, size: (u32, u32)) -> Settings {
        Settings {
            linear: self.linear,
            color: self
                .color_on
                .then_some([self.brightness, self.contrast, self.saturation]),
            vignette: self.vignette_on.then(|| Vignette {
                center_px: [
                    self.vignette_center[0] * size.0 as f32,
                    self.vignette_center[1] * size.1 as f32,
                ],
                radius: self.vignette_radius,
                softness: self.vignette_softness,
                color: [
                    self.vignette_color[0],
                    self.vignette_color[1],
                    self.vignette_color[2],
                    1.0,
                ],
                amount: self.vignette_amount,
            }),
            blend_mode: self.blend_mode as i32,
            blend_amount: self.blend_amount,
            blur: (self.blur_on && self.blur_radius > 0.0).then_some(Blur {
                radius: self.blur_radius,
                vertical_scale: 1.0,
                direction: self.blur_direction as i32,
                grow: false,
            }),
        }
    }
}

/// What the panel reports back to the app.
#[derive(Default)]
pub struct Actions {
    pub clear_image: bool,
}

/// Status shown at the top of the panel.
pub struct Status<'a> {
    pub fps: f32,
    pub size: (u32, u32),
    pub image: Option<&'a str>,
    pub message: Option<&'a str>,
}

fn section(ui: &mut Ui, title: &str, on: Option<&mut bool>, body: impl FnOnce(&mut Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong())
        .default_open(true)
        .show(ui, |ui| match on {
            Some(on) => {
                ui.checkbox(on, "Enabled");
                let enabled = *on;
                ui.add_enabled_ui(enabled, body);
            }
            None => body(ui),
        });
}

fn slider(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, label: &str) {
    ui.add(
        egui::Slider::new(value, range)
            .text(label)
            .fixed_decimals(2),
    );
}

fn choice(ui: &mut Ui, id: &str, value: &mut usize, options: &[&str]) {
    egui::ComboBox::from_id_salt(id)
        .width(170.0)
        .selected_text(options[*value])
        .show_ui(ui, |ui| {
            for (i, option) in options.iter().enumerate() {
                ui.selectable_value(value, i, *option);
            }
        });
}

/// Draws the control panel and returns what the app should do.
pub fn panel(ui: &mut Ui, c: &mut Controls, status: &Status) -> Actions {
    let mut actions = Actions::default();
    ui.add_space(4.0);
    ui.heading("VFX Preview");
    ui.label(
        RichText::new(format!(
            "{} × {}   {:.0} fps",
            status.size.0, status.size.1, status.fps
        ))
        .weak(),
    );
    if let Some(message) = status.message {
        ui.colored_label(Color32::from_rgb(230, 120, 90), message);
    }
    ui.separator();

    egui::ScrollArea::vertical().show(ui, |ui| {
        section(ui, "Input", None, |ui| {
            match status.image {
                Some(name) => {
                    ui.label(format!("Image: {name}"));
                    if ui.button("Back to test patterns").clicked() {
                        actions.clear_image = true;
                    }
                }
                None => {
                    choice(ui, "pattern", &mut c.pattern, PATTERNS);
                    #[cfg(not(target_arch = "wasm32"))]
                    ui.label(RichText::new("Drop a PNG or JPEG on the window to use it.").weak());
                }
            }
            ui.checkbox(&mut c.animate, "Animate");
            ui.checkbox(&mut c.linear, "Process in linear light");
        });

        section(ui, "Colour", Some(&mut c.color_on), |ui| {
            slider(ui, &mut c.brightness, -1.0..=1.0, "Brightness");
            slider(ui, &mut c.contrast, 0.0..=4.0, "Contrast");
            slider(ui, &mut c.saturation, 0.0..=4.0, "Saturation");
        });

        section(ui, "Vignette", Some(&mut c.vignette_on), |ui| {
            slider(ui, &mut c.vignette_center[0], 0.0..=1.0, "Centre X");
            slider(ui, &mut c.vignette_center[1], 0.0..=1.0, "Centre Y");
            ui.label(RichText::new("Or click the image to place it.").weak());
            slider(ui, &mut c.vignette_radius, 0.0..=2.0, "Radius");
            slider(ui, &mut c.vignette_softness, 0.0..=1.0, "Softness");
            ui.horizontal(|ui| {
                ui.color_edit_button_rgb(&mut c.vignette_color);
                ui.label("Colour");
            });
            slider(ui, &mut c.vignette_amount, 0.0..=1.0, "Amount");
        });

        section(ui, "Blend", Some(&mut c.blend_on), |ui| {
            ui.label("Layer");
            choice(ui, "blend_pattern", &mut c.blend_pattern, PATTERNS);
            ui.label("Mode");
            choice(ui, "blend_mode", &mut c.blend_mode, BLEND_MODES);
            slider(ui, &mut c.blend_amount, 0.0..=1.0, "Amount");
        });

        section(ui, "Blur", Some(&mut c.blur_on), |ui| {
            slider(ui, &mut c.blur_radius, 0.0..=64.0, "Radius (px)");
            ui.label("Direction");
            choice(ui, "blur_direction", &mut c.blur_direction, BLUR_DIRECTIONS);
        });

        section(ui, "Feedback trail", Some(&mut c.feedback_on), |ui| {
            slider(ui, &mut c.feedback_decay, 0.0..=0.99, "Decay");
        });

        ui.add_space(8.0);
        if ui.button("Reset all").clicked() {
            let image_keeps = c.pattern;
            *c = Controls {
                pattern: image_keeps,
                ..Controls::default()
            };
        }
    });
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_change_nothing() {
        let s = Controls::default().settings((64, 48));
        assert!(!s.linear && s.vignette.is_none() && s.blur.is_none());
        assert_eq!(s.color, Some([0.0, 1.0, 1.0]));
    }

    #[test]
    fn vignette_centre_is_in_source_pixels() {
        let c = Controls {
            vignette_on: true,
            vignette_center: [0.25, 0.75],
            ..Controls::default()
        };
        assert_eq!(
            c.settings((200, 100)).vignette.unwrap().center_px,
            [50.0, 75.0]
        );
    }
}
