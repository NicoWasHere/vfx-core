//! The last few frames, oldest first, with their allocations recycled at the front.

use segcam_motion::{Argb, FrameSource, RenderSettings, View, ViewMut};
use std::collections::VecDeque;

#[derive(Default)]
pub struct History {
    frames: VecDeque<Vec<Argb<u8>>>,
    width: i32,
    height: i32,
    frame: i64,
}

impl History {
    pub fn clear(&mut self) {
        self.frames.clear();
        self.frame = 0;
    }

    pub fn resize(&mut self, width: i32, height: i32) {
        self.clear();
        self.width = width;
        self.height = height;
    }

    pub fn next(&mut self, settings: &RenderSettings) -> &mut [Argb<u8>] {
        let keep = segcam_motion::previous_frames_needed(settings) as usize + 1;
        while self.frames.len() > keep {
            self.frames.pop_front();
        }
        let pixels = if self.frames.len() == keep {
            self.frames.pop_front().unwrap()
        } else {
            vec![Argb::default(); self.width as usize * self.height as usize]
        };
        self.frames.push_back(pixels);
        self.frames.back_mut().unwrap()
    }

    pub fn render(&mut self, settings: &RenderSettings, output: &mut [Argb<u8>]) {
        let count = self.frames.len();
        let frame = self.frame;
        let mut view = ViewMut::from_pixels(output, self.width, self.height);
        segcam_motion::render(settings, self, count, frame, &mut view);
        self.frame += 1;
    }
}

impl FrameSource<u8> for History {
    fn with_frame(&mut self, index: usize, f: &mut dyn FnMut(Option<View<'_, u8>>)) {
        f(self
            .frames
            .get(index)
            .map(|p| View::from_pixels(p, self.width, self.height)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recycles_bounds_and_resets() {
        let mut h = History::default();
        h.resize(2, 1);
        let mut s = RenderSettings {
            background_frames: 1,
            ..RenderSettings::default()
        };
        h.next(&s).fill(Argb {
            alpha: 255,
            red: 255,
            green: 255,
            blue: 255,
        });
        let first = h.frames[0].as_ptr();
        h.next(&s).fill(Argb::default());
        h.next(&s).fill(Argb {
            alpha: 128,
            red: 128,
            green: 128,
            blue: 128,
        });
        assert_eq!(h.frames.len(), 2);
        assert_eq!(h.frames[1].as_ptr(), first);
        assert_eq!(h.frames[0][0].red, 0);
        assert_eq!(h.frames[1][0].red, 128);
        s.mode = segcam_motion::OutputMode::Swarm;
        s.trail_frames = 3;
        for _ in 0..5 {
            h.next(&s).fill(Argb {
                alpha: 255,
                red: 255,
                green: 255,
                blue: 255,
            });
        }
        assert_eq!(h.frames.len(), 4);
        s.mode = segcam_motion::OutputMode::Overlay;
        h.next(&s).fill(Argb {
            alpha: 255,
            red: 255,
            green: 255,
            blue: 255,
        });
        assert_eq!(h.frames.len(), 2);
        h.clear();
        assert!(h.frames.is_empty());
        assert_eq!(h.frame, 0);
        h.resize(3, 2);
        h.next(&s);
        assert_eq!(h.frames[0].len(), 6);
    }
}
