//! The last few frames, oldest first, with their allocations recycled at the front.

use segcam_motion::{Argb, Channel, FrameSource, RenderSettings, View, ViewMut};
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

    pub fn push(&mut self, canvas: &[f32], settings: &RenderSettings) {
        let keep = segcam_motion::previous_frames_needed(settings) as usize + 1;
        while self.frames.len() > keep {
            self.frames.pop_front();
        }
        let mut pixels = if self.frames.len() == keep {
            self.frames.pop_front().unwrap()
        } else {
            vec![Argb::default(); self.width as usize * self.height as usize]
        };
        for (p, c) in pixels.iter_mut().zip(canvas.as_chunks::<4>().0) {
            *p = Argb {
                alpha: u8::from_unit(c[3]),
                red: u8::from_unit(c[0]),
                green: u8::from_unit(c[1]),
                blue: u8::from_unit(c[2]),
            };
        }
        self.frames.push_back(pixels);
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

pub fn to_canvas(pixels: &[Argb<u8>], canvas: &mut [f32]) {
    for (p, c) in pixels.iter().zip(canvas.as_chunks_mut::<4>().0) {
        *c = [
            p.red.to_unit(),
            p.green.to_unit(),
            p.blue.to_unit(),
            p.alpha.to_unit(),
        ];
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
        h.push(&[1.0; 8], &s);
        let first = h.frames[0].as_ptr();
        h.push(&[0.0; 8], &s);
        h.push(&[0.5; 8], &s);
        assert_eq!(h.frames.len(), 2);
        assert_eq!(h.frames[1].as_ptr(), first);
        assert_eq!(h.frames[0][0].red, 0);
        assert_eq!(h.frames[1][0].red, 128);
        s.mode = segcam_motion::OutputMode::Swarm;
        s.trail_frames = 3;
        for _ in 0..5 {
            h.push(&[1.0; 8], &s);
        }
        assert_eq!(h.frames.len(), 4);
        s.mode = segcam_motion::OutputMode::Overlay;
        h.push(&[1.0; 8], &s);
        assert_eq!(h.frames.len(), 2);
        h.clear();
        assert!(h.frames.is_empty());
        assert_eq!(h.frame, 0);
        h.resize(3, 2);
        h.push(&[1.0; 24], &s);
        assert_eq!(h.frames[0].len(), 6);
    }
}
