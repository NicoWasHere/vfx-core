//! segcam's motion segmenter (examples/segcam-motion) on video and webcam frames, before
//! the GPU effects. It runs on the CPU and needs the frames before each one, so it keeps
//! a short history; a seek starts the history again.
//!
//! The core rebuilds everything from that history on every frame, as After Effects needs.
//! That's quick for the overlay and mattes but a few hundred milliseconds for the swarm at
//! 1080p, so the live preview runs it on a [`Worker`] thread: a slow mode slows segcam's
//! picture, not the app. Exports use [`Segcam`] directly and wait for every frame.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

use segcam_motion::{Argb, Channel, FrameSource, OutputMode, RenderSettings, View, ViewMut};

use crate::controls::Controls;

/// segcam's settings from the panel, or None while it is switched off.
pub fn settings(c: &Controls) -> Option<RenderSettings> {
    if !c.segcam_on {
        return None;
    }
    let mut s = RenderSettings {
        mode: OutputMode::from_index(c.segcam_mode as i32 + 1).unwrap_or(OutputMode::Overlay),
        background_frames: c.segcam_background as i32,
        boxes: c.segcam_boxes,
        labels: c.segcam_labels,
        trail_frames: c.segcam_trail as i32,
        max_tiles: c.segcam_max_tiles as i32,
        ..RenderSettings::default()
    };
    s.motion.sensitivity = f64::from(c.segcam_sensitivity.clamp(0.0, 1.0));
    s.motion.min_area_fraction = f64::from(c.segcam_min_size.max(0.0)) / 100.0;
    s.motion.max_segments = c.segcam_max_segments as _;
    Some(s)
}

/// The newest decoded frame and, while segcam is on, the frames before it.
pub struct Segcam {
    size: (u32, u32),
    /// The newest frame as decoded (float RGBA), shown as is while segcam is off.
    raw: Option<Vec<f32>>,
    /// Oldest first, at 8 bits: up to 40 frames are kept, and segcam cuts at 8 bits anyway.
    history: VecDeque<Vec<Argb<u8>>>,
    /// Counts frames for segcam's IDs (`motion#412.1`).
    frame: i64,
    out: Vec<Argb<u8>>,
    pixels: Vec<f32>,
    /// A frame arrived since the last output.
    fresh: bool,
    /// The settings the last output was made with (None: segcam off).
    shown: Option<Option<RenderSettings>>,
}

impl Segcam {
    pub fn new(size: (u32, u32)) -> Self {
        Self {
            size,
            raw: None,
            history: VecDeque::new(),
            frame: 0,
            out: Vec::new(),
            pixels: Vec::new(),
            fresh: false,
            shown: None,
        }
    }

    /// Forgets the history, after a seek: the frames before it no longer lead up to it.
    pub fn reset(&mut self) {
        self.history.clear();
    }

    /// Shows the newest frame again, after something else (an export) used the source.
    pub fn redraw(&mut self) {
        self.shown = None;
    }

    /// Takes the next frame, adding it to the history while segcam is on.
    pub fn push(&mut self, pixels: Vec<f32>, settings: Option<&RenderSettings>) {
        self.raw = Some(pixels);
        self.fresh = true;
        match settings {
            Some(s) => self.record(s),
            None => self.history.clear(),
        }
    }

    fn record(&mut self, s: &RenderSettings) {
        let Some(raw) = &self.raw else {
            return;
        };
        self.history.push_back(
            raw.as_chunks::<4>()
                .0
                .iter()
                .map(|&[r, g, b, a]| Argb {
                    alpha: u8::from_unit(a),
                    red: u8::from_unit(r),
                    green: u8::from_unit(g),
                    blue: u8::from_unit(b),
                })
                .collect(),
        );
        self.frame += 1;
        let keep = segcam_motion::previous_frames_needed(s) as usize + 1;
        while self.history.len() > keep {
            self.history.pop_front();
        }
    }

    /// The source pixels to show, when they changed: a new frame came in or the settings
    /// changed. Paused, segcam re-renders the same history as its settings change.
    pub fn output(&mut self, settings: Option<RenderSettings>) -> Option<&[f32]> {
        self.raw.as_ref()?;
        let fresh = std::mem::take(&mut self.fresh);
        if !fresh && self.shown == Some(settings) {
            return None;
        }
        self.shown = Some(settings);
        let Some(s) = settings else {
            self.history.clear();
            return self.raw.as_deref();
        };
        // Switched on since the last frame came in.
        if self.history.is_empty() {
            self.record(&s);
        }
        let keep = segcam_motion::previous_frames_needed(&s) as usize + 1;
        while self.history.len() > keep {
            self.history.pop_front();
        }

        let (w, h) = (self.size.0 as i32, self.size.1 as i32);
        self.out
            .resize(self.size.0 as usize * self.size.1 as usize, Argb::default());
        let count = self.history.len();
        let mut source = History {
            frames: &self.history,
            width: w,
            height: h,
        };
        let mut view = ViewMut::from_pixels(&mut self.out, w, h);
        segcam_motion::render(&s, &mut source, count, self.frame, &mut view);

        self.pixels.clear();
        self.pixels.extend(
            self.out
                .iter()
                .flat_map(|p| [p.red, p.green, p.blue, p.alpha].map(Channel::to_unit)),
        );
        Some(&self.pixels)
    }
}

struct History<'a> {
    frames: &'a VecDeque<Vec<Argb<u8>>>,
    width: i32,
    height: i32,
}

impl FrameSource<u8> for History<'_> {
    fn with_frame(&mut self, index: usize, f: &mut dyn FnMut(Option<View<'_, u8>>)) {
        f(self
            .frames
            .get(index)
            .map(|p| View::from_pixels(p, self.width, self.height)));
    }
}

/// Frames waiting for the worker: a full-size float frame is 33 MB at 1080p, so while
/// segcam is behind, older ones are dropped (it then sees every few frames, not each one).
const QUEUE: usize = 4;

/// A [`Segcam`] on its own thread, for the live preview.
pub struct Worker {
    shared: Arc<(Mutex<Shared>, Condvar)>,
}

struct Shared {
    frames: VecDeque<Vec<f32>>,
    settings: Option<RenderSettings>,
    reset: bool,
    redraw: bool,
    /// The newest output, until the app uploads it.
    result: Option<Vec<f32>>,
    stop: bool,
}

impl Shared {
    fn has_work(&self, settings: Option<RenderSettings>) -> bool {
        !self.frames.is_empty() || self.redraw || self.reset || self.settings != settings
    }
}

impl Worker {
    pub fn start(size: (u32, u32)) -> Self {
        let shared = Arc::new((
            Mutex::new(Shared {
                frames: VecDeque::new(),
                settings: None,
                reset: false,
                redraw: false,
                result: None,
                stop: false,
            }),
            Condvar::new(),
        ));
        let worker = Arc::clone(&shared);
        std::thread::spawn(move || {
            let (lock, wake) = &*worker;
            let mut segcam = Segcam::new(size);
            let mut shown = None;
            loop {
                let (frames, settings) = {
                    let mut s = lock.lock().unwrap();
                    while !s.has_work(shown) && !s.stop {
                        s = wake.wait(s).unwrap();
                    }
                    if s.stop {
                        return;
                    }
                    if std::mem::take(&mut s.reset) {
                        segcam.reset();
                    }
                    if std::mem::take(&mut s.redraw) {
                        segcam.redraw();
                    }
                    (std::mem::take(&mut s.frames), s.settings)
                };
                shown = settings;
                for frame in frames {
                    segcam.push(frame, settings.as_ref());
                }
                if let Some(pixels) = segcam.output(settings) {
                    lock.lock().unwrap().result = Some(pixels.to_vec());
                }
            }
        });
        Self { shared }
    }

    fn with(&self, f: impl FnOnce(&mut Shared)) {
        let (lock, wake) = &*self.shared;
        f(&mut lock.lock().unwrap());
        wake.notify_one();
    }

    pub fn push(&self, pixels: Vec<f32>) {
        self.with(|s| {
            s.frames.push_back(pixels);
            while s.frames.len() > QUEUE {
                s.frames.pop_front();
            }
        });
    }

    /// Settings for the frames from here on; re-renders the current one when they change.
    pub fn set(&self, settings: Option<RenderSettings>) {
        let (lock, wake) = &*self.shared;
        let mut s = lock.lock().unwrap();
        if s.settings != settings {
            s.settings = settings;
            wake.notify_one();
        }
    }

    /// After a seek: drops queued frames and the history.
    pub fn reset(&self) {
        self.with(|s| {
            s.frames.clear();
            s.reset = true;
        });
    }

    pub fn redraw(&self) {
        self.with(|s| s.redraw = true);
    }

    /// segcam's newest picture, once.
    pub fn take(&self) -> Option<Vec<f32>> {
        self.shared.0.lock().unwrap().result.take()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.with(|s| s.stop = true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame((w, h): (u32, u32), x0: u32) -> Vec<f32> {
        (0..h)
            .flat_map(|y| {
                (0..w).flat_map(move |x| {
                    let v = if (x0..x0 + 16).contains(&x) && (24..40).contains(&y) {
                        1.0
                    } else {
                        0.25
                    };
                    [v, v, v, 1.0]
                })
            })
            .collect()
    }

    /// The box colour (0.25, 1, 0.4); the input has only greys.
    fn has_box(pixels: &[f32]) -> bool {
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[1] > 0.9 && p[0] < 0.5 && p[2] < 0.5)
    }

    fn on() -> Option<RenderSettings> {
        settings(&Controls {
            segcam_on: true,
            ..Controls::default()
        })
    }

    /// A white square moving across a grey frame: off passes frames through, overlay
    /// finds the square and draws its box.
    #[test]
    fn passes_through_when_off_and_boxes_motion_when_on() {
        let size = (96, 64);
        let mut segcam = Segcam::new(size);
        segcam.push(frame(size, 0), None);
        assert_eq!(segcam.output(None).unwrap(), frame(size, 0).as_slice());
        assert!(segcam.output(None).is_none(), "nothing new to show");

        let mut last = Vec::new();
        for i in 0..8 {
            segcam.push(frame(size, 8 * i), on().as_ref());
            last = segcam.output(on()).unwrap().to_vec();
        }
        assert!(has_box(&last), "a motion box is drawn");
        // Changing a setting while paused re-renders the same frame.
        let boxes_off = settings(&Controls {
            segcam_on: true,
            segcam_boxes: false,
            segcam_labels: false,
            ..Controls::default()
        });
        assert!(segcam.output(boxes_off).is_some());
        assert!(segcam.output(boxes_off).is_none());
    }

    #[test]
    fn worker_renders_in_the_background() {
        let size = (96, 64);
        let worker = Worker::start(size);
        worker.set(on());
        let mut last = None;
        for i in 0..8 {
            worker.push(frame(size, 8 * i));
            // Waits for each, so none is dropped.
            let started = std::time::Instant::now();
            loop {
                if let Some(pixels) = worker.take() {
                    last = Some(pixels);
                    break;
                }
                assert!(started.elapsed().as_secs() < 5, "no output");
                std::thread::yield_now();
            }
        }
        assert!(has_box(&last.unwrap()));
    }
}
