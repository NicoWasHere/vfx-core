//! The control panel: every effect's parameters, with the same defaults and ranges
//! as the After Effects, OpenFX and FFGL plugins, plus the preview's own input and
//! feedback controls.

use egui::{Color32, RichText, Ui};
use vfx_core::effects::{BLEND_MODES, BLUR_DIRECTIONS};
use vfx_wrapper_common::{Blur, Settings, Vignette};

use crate::effects::PATTERNS;
use crate::modulation::{self, Expressions, Sources, target};

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
    /// Mirror the source before everything else, when above 0.5.
    pub flip_x: f32,
    pub flip_y: f32,
    /// The webcam picked in the Input section.
    pub camera: usize,
    /// segcam motion, on video and webcam frames before everything else.
    pub segcam_on: bool,
    /// Index into segcam's OutputMode::NAMES.
    pub segcam_mode: usize,
    /// 0 to 1: how small a change counts as motion.
    pub segcam_sensitivity: f32,
    /// Smallest segment, as a percentage of the frame.
    pub segcam_min_size: f32,
    pub segcam_background: usize,
    pub segcam_max_segments: usize,
    pub segcam_boxes: bool,
    pub segcam_labels: bool,
    pub segcam_trail: usize,
    pub segcam_max_tiles: usize,
    /// segcam-live: the swarm with a pile kept between frames. Takes over from segcam
    /// motion while on.
    pub segcam_live_on: bool,
    pub segcam_live_sensitivity: f32,
    pub segcam_live_min_size: f32,
    pub segcam_live_background: usize,
    pub segcam_live_max_segments: usize,
    /// Frames a window stays up.
    pub segcam_live_lifetime: f32,
    pub segcam_live_max_tiles: usize,
    /// Windows show the video playing where they are, not a snapshot of when they opened.
    pub segcam_live_video: bool,
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
            flip_x: 0.0,
            flip_y: 0.0,
            camera: 0,
            segcam_on: false,
            segcam_mode: 0,
            segcam_sensitivity: 0.864,
            segcam_min_size: 0.25,
            segcam_background: 16,
            segcam_max_segments: 16,
            segcam_boxes: true,
            segcam_labels: true,
            segcam_trail: 24,
            segcam_max_tiles: 100,
            segcam_live_on: false,
            segcam_live_sensitivity: 0.864,
            segcam_live_min_size: 0.25,
            segcam_live_background: 16,
            segcam_live_max_segments: 16,
            segcam_live_lifetime: 60.0,
            segcam_live_max_tiles: 100,
            segcam_live_video: false,
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
    /// The video's sound was switched on or off.
    pub sound: Option<bool>,
    pub start_export: bool,
    pub cancel_export: bool,
    /// Open a file picker for a video or image.
    pub import: bool,
    /// Start the camera at this index of `Status::cameras`.
    pub start_camera: Option<usize>,
    pub refresh_cameras: bool,
    /// Face tracking was switched on or off.
    pub face_tracking: Option<bool>,
    pub toggle_play: bool,
    /// The position slider is being dragged here (seconds).
    pub scrub: Option<f64>,
    /// The position slider was let go (or clicked) here.
    pub scrub_done: Option<f64>,
}

/// A video file's transport, for the play button and position slider.
pub struct Playback {
    pub position: f64,
    pub duration: f64,
    pub playing: bool,
}

/// Status shown at the top of the panel.
pub struct Status<'a> {
    /// How often the window redraws.
    pub fps: f32,
    /// Video frames shown in the last second, and the video's own rate.
    pub video_fps: Option<(f32, f32)>,
    pub size: (u32, u32),
    pub image: Option<&'a str>,
    pub message: Option<&'a str>,
    /// The source is a video file (so it can play sound and be rendered).
    pub video: bool,
    pub sound_on: bool,
    /// How far a running render is, 0–1.
    pub export: Option<f32>,
    pub cameras: &'a [String],
    pub face_tracking: bool,
    /// Set for video files.
    pub playback: Option<Playback>,
}

/// Seconds as m:ss.s.
fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.0);
    format!("{}:{:04.1}", (seconds / 60.0) as u32, seconds % 60.0)
}

/// Play/pause and a position slider that seeks as it's dragged.
fn transport(ui: &mut Ui, playback: &Playback, actions: &mut Actions) {
    ui.horizontal(|ui| {
        let label = if playback.playing { "Pause" } else { "Play" };
        if ui.button(label).on_hover_text("Space").clicked() {
            actions.toggle_play = true;
        }
        ui.label(
            RichText::new(format!(
                "{} / {}",
                clock(playback.position),
                clock(playback.duration)
            ))
            .monospace(),
        );
    });
    let mut position = playback.position;
    ui.spacing_mut().slider_width = ui.available_width();
    let response = ui.add(
        egui::Slider::new(&mut position, 0.0..=playback.duration.max(0.001)).show_value(false),
    );
    if response.drag_stopped() || (response.changed() && !response.dragged()) {
        actions.scrub_done = Some(position);
    } else if response.changed() {
        actions.scrub = Some(position);
    }
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
pub fn panel(
    ui: &mut Ui,
    c: &mut Controls,
    exprs: &mut Expressions,
    sources: &Sources,
    status: &Status,
) -> Actions {
    let mut actions = Actions::default();
    ui.add_space(4.0);
    ui.heading("VFX Preview");
    ui.label(
        RichText::new(format!(
            "{} × {}   window {:.0} fps",
            status.size.0, status.size.1, status.fps
        ))
        .weak(),
    );
    if let Some((shown, rate)) = status.video_fps {
        ui.label(RichText::new(format!("video {shown:.0} of {rate:.0} fps")).weak());
    }
    if let Some(message) = status.message {
        ui.colored_label(Color32::from_rgb(230, 120, 90), message);
    }
    ui.separator();

    egui::ScrollArea::vertical().show(ui, |ui| {
        section(ui, "Input", None, |ui| {
            match status.image {
                Some(name) => {
                    ui.label(format!("Source: {name}"));
                    if let Some(playback) = &status.playback {
                        transport(ui, playback, &mut actions);
                    }
                    if status.video {
                        let mut sound = status.sound_on;
                        if ui.checkbox(&mut sound, "Play sound").changed() {
                            actions.sound = Some(sound);
                        }
                    }
                    if ui.button("Back to test patterns").clicked() {
                        actions.clear_image = true;
                    }
                }
                None => {
                    choice(ui, "pattern", &mut c.pattern, PATTERNS);
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                if ui.button("Import video or image…").clicked() {
                    actions.import = true;
                }
                ui.label(RichText::new("Or drop one on the window.").weak());
                ui.horizontal(|ui| {
                    if status.cameras.is_empty() {
                        ui.label(RichText::new("No webcam found").weak());
                    } else {
                        c.camera = c.camera.min(status.cameras.len() - 1);
                        let names: Vec<&str> =
                            status.cameras.iter().map(String::as_str).collect();
                        choice(ui, "camera", &mut c.camera, &names);
                        if ui.button("Webcam").clicked() {
                            actions.start_camera = Some(c.camera);
                        }
                    }
                    if ui.small_button("⟳").on_hover_text("Look for cameras again").clicked() {
                        actions.refresh_cameras = true;
                    }
                });
                let mut tracking = status.face_tracking;
                if ui
                    .checkbox(&mut tracking, "Face tracking")
                    .on_hover_text(
                        "Finds a face in video or webcam frames for the face, eye, nose and \
                         mouth variables in expressions",
                    )
                    .changed()
                {
                    actions.face_tracking = Some(tracking);
                }
            }
            ui.checkbox(&mut c.animate, "Animate");
            ui.checkbox(&mut c.linear, "Process in linear light");
        });

        egui::CollapsingHeader::new(RichText::new("Expressions").strong())
            .default_open(false)
            .show(ui, |ui| modulation::help(ui, sources));

        // Each slider has an expression box under it; a filled-in box overrides it.
        let mut param = |ui: &mut Ui, value: &mut f32, range, label, target| {
            slider(ui, value, range, label);
            modulation::field(ui, exprs, target);
        };

        #[cfg(not(target_arch = "wasm32"))]
        section(ui, "Segcam motion", Some(&mut c.segcam_on), |ui| {
            let modes = segcam_motion::OutputMode::NAMES;
            choice(ui, "segcam mode", &mut c.segcam_mode, &modes);
            param(ui, &mut c.segcam_sensitivity, 0.0..=1.0, "Sensitivity", target::SEGCAM_SENSITIVITY);
            param(ui, &mut c.segcam_min_size, 0.0..=10.0, "Min size %", target::SEGCAM_MIN_SIZE);
            ui.add(egui::Slider::new(&mut c.segcam_background, 1..=60).text("Background frames"));
            ui.add(egui::Slider::new(&mut c.segcam_max_segments, 1..=64).text("Max segments"));
            if c.segcam_mode == 3 {
                ui.add(egui::Slider::new(&mut c.segcam_trail, 1..=60).text("Trail frames"));
                ui.add(egui::Slider::new(&mut c.segcam_max_tiles, 1..=400).text("Max tiles"));
            } else {
                ui.checkbox(&mut c.segcam_boxes, "Boxes");
                ui.checkbox(&mut c.segcam_labels, "Labels");
            }
            ui.label(RichText::new("Video and webcam only; starts over on a seek.").weak());
        });

        #[cfg(not(target_arch = "wasm32"))]
        section(ui, "Segcam live", Some(&mut c.segcam_live_on), |ui| {
            ui.horizontal(|ui| {
                ui.label("Windows show");
                ui.radio_value(&mut c.segcam_live_video, false, "Snapshot")
                    .on_hover_text("The moment each window opened, frozen");
                ui.radio_value(&mut c.segcam_live_video, true, "Live video")
                    .on_hover_text("The video playing behind each window, as if cut out of it");
            });
            param(
                ui,
                &mut c.segcam_live_sensitivity,
                0.0..=1.0,
                "Sensitivity",
                target::SEGCAM_LIVE_SENSITIVITY,
            );
            ui.add(egui::Slider::new(&mut c.segcam_live_min_size, 0.0..=10.0).text("Min size %"));
            param(
                ui,
                &mut c.segcam_live_lifetime,
                1.0..=300.0,
                "Lifetime (frames)",
                target::SEGCAM_LIVE_LIFETIME,
            );
            ui.add(egui::Slider::new(&mut c.segcam_live_max_tiles, 1..=400).text("Max tiles"));
            ui.add(
                egui::Slider::new(&mut c.segcam_live_background, 1..=60).text("Background frames"),
            );
            ui.add(egui::Slider::new(&mut c.segcam_live_max_segments, 1..=64).text("Max segments"));
            ui.label(
                RichText::new(
                    "The swarm, kept between frames: windows stay up for their lifetime \
                     or until max tiles pushes them out. Replaces Segcam motion while on.",
                )
                .weak(),
            );
        });

        section(ui, "Flip", None, |ui| {
            param(ui, &mut c.flip_x, 0.0..=1.0, "Horizontal", target::FLIP_X);
            param(ui, &mut c.flip_y, 0.0..=1.0, "Vertical", target::FLIP_Y);
            ui.label(RichText::new("Flips above 0.5.").weak());
        });

        section(ui, "Colour", Some(&mut c.color_on), |ui| {
            param(ui, &mut c.brightness, -1.0..=1.0, "Brightness", target::BRIGHTNESS);
            param(ui, &mut c.contrast, 0.0..=4.0, "Contrast", target::CONTRAST);
            param(ui, &mut c.saturation, 0.0..=4.0, "Saturation", target::SATURATION);
        });

        section(ui, "Vignette", Some(&mut c.vignette_on), |ui| {
            param(ui, &mut c.vignette_center[0], 0.0..=1.0, "Centre X", target::VIGNETTE_X);
            param(ui, &mut c.vignette_center[1], 0.0..=1.0, "Centre Y", target::VIGNETTE_Y);
            ui.label(RichText::new("Or click the image to place it.").weak());
            param(ui, &mut c.vignette_radius, 0.0..=2.0, "Radius", target::VIGNETTE_RADIUS);
            param(ui, &mut c.vignette_softness, 0.0..=1.0, "Softness", target::VIGNETTE_SOFTNESS);
            ui.horizontal(|ui| {
                ui.color_edit_button_rgb(&mut c.vignette_color);
                ui.label("Colour");
            });
            param(ui, &mut c.vignette_amount, 0.0..=1.0, "Amount", target::VIGNETTE_AMOUNT);
        });

        section(ui, "Blend", Some(&mut c.blend_on), |ui| {
            ui.label("Layer");
            choice(ui, "blend_pattern", &mut c.blend_pattern, PATTERNS);
            ui.label("Mode");
            choice(ui, "blend_mode", &mut c.blend_mode, BLEND_MODES);
            param(ui, &mut c.blend_amount, 0.0..=1.0, "Amount", target::BLEND_AMOUNT);
        });

        section(ui, "Blur", Some(&mut c.blur_on), |ui| {
            param(ui, &mut c.blur_radius, 0.0..=64.0, "Radius (px)", target::BLUR_RADIUS);
            ui.label("Direction");
            choice(ui, "blur_direction", &mut c.blur_direction, BLUR_DIRECTIONS);
        });

        section(ui, "Feedback trail", Some(&mut c.feedback_on), |ui| {
            param(ui, &mut c.feedback_decay, 0.0..=0.99, "Decay", target::FEEDBACK_DECAY);
        });

        #[cfg(not(target_arch = "wasm32"))]
        section(ui, "Render", None, |ui| match status.export {
            Some(progress) => {
                ui.add(egui::ProgressBar::new(progress).show_percentage());
                if ui.button("Cancel").clicked() {
                    actions.cancel_export = true;
                }
            }
            None if status.video => {
                if ui.button("Render to MP4").clicked() {
                    actions.start_export = true;
                }
                ui.label(
                    RichText::new("Saves every frame with these settings and expressions, plus the original sound, next to the video.")
                        .weak(),
                );
            }
            None => {
                ui.label(RichText::new("Drop a video on the window to render it.").weak());
            }
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
