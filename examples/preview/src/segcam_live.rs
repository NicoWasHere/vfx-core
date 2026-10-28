//! segcam-live: the swarm as the live app runs it, with a pile kept between frames.
//!
//! segcam-motion's swarm is rebuilt from the last 24 frames on every frame, as After
//! Effects needs (it can ask for any frame, in any order). The preview only goes forward,
//! so this keeps the pile instead: each frame is segmented once, against the luma grids of
//! the frames before it, and its moving regions become new windows on top. Windows expire
//! after `lifetime` frames, or oldest first past `max_tiles`. It uses segcam-motion's own
//! detection, crops and window drawing, so the windows look the same.

use std::collections::VecDeque;

use segcam_motion::swarm::{self, SwarmMetrics, Tile};
use segcam_motion::{Argb, LumaGrid, MotionSettings, View, ViewMut, motion, render};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub motion: MotionSettings,
    /// How many frames before each one build its background.
    pub background_frames: usize,
    pub grid_width: i32,
    /// Frames a window stays up.
    pub lifetime: u32,
    pub max_tiles: usize,
}

/// The pile and the frames' luma grids.
#[derive(Default)]
pub struct Live {
    /// The last `background_frames` frames' grids, oldest first.
    grids: VecDeque<LumaGrid>,
    /// Oldest first, as segcam-motion's swarm draws it.
    pile: VecDeque<Tile>,
    /// The number of the frame each window in `pile` was cut from.
    born: VecDeque<i64>,
    frame: i64,
}

impl Live {
    /// Starts over, after a seek.
    pub fn reset(&mut self) {
        self.grids.clear();
        self.pile.clear();
        self.born.clear();
    }

    /// Segments the next frame and adds its moving regions to the pile.
    pub fn push(&mut self, frame: &View<u8>, s: &Settings) {
        self.frame += 1;
        let grid = render::downsample(frame, s.grid_width);
        let found = motion::detect(&self.grids, &grid, &s.motion);
        self.grids.push_back(grid);
        while self.grids.len() > s.background_frames.max(1) {
            self.grids.pop_front();
        }

        let (w, h) = (frame.width, frame.height);
        let title_height = SwarmMetrics::new(h).title_height;
        let canvas = render::PixelRect::new(0, 0, w, h);
        for (b, blob) in found.blobs.iter().enumerate() {
            let title = format!("LO.FI.SCI.FI {}", render::segment_label(self.frame, b));
            let tile = swarm::make_tile(blob, Some(frame), title, w, h, false);
            // A window buried whole under the new one is never seen again: it keeps its
            // place in the pile (and its turn to expire) but drops its pixels, as segcam-motion does.
            let cover = swarm::tile_extent(tile.video, title_height);
            for older in self.pile.iter_mut().filter(|t| !t.hidden) {
                let shown =
                    render::intersect(swarm::tile_extent(older.video, title_height), canvas);
                if render::contains(cover, shown) {
                    older.hidden = true;
                    older.pixels = Vec::new();
                }
            }
            self.pile.push_back(tile);
            self.born.push_back(self.frame);
        }
        self.expire(s);
    }

    /// Drops windows older than the lifetime, then the oldest past `max_tiles`.
    pub fn expire(&mut self, s: &Settings) {
        let lifetime = i64::from(s.lifetime.max(1));
        while let Some(born) = self.born.front()
            && (self.frame - born >= lifetime || self.pile.len() > s.max_tiles.max(1))
        {
            self.pile.pop_front();
            self.born.pop_front();
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.pile.len()
    }

    pub fn draw(&self, out: &mut ViewMut<u8>) {
        swarm::draw_swarm(out, &self.pile, false);
    }
}

/// segcam-motion's pixels, from float RGBA.
pub fn to_argb(pixels: &[f32], out: &mut Vec<Argb<u8>>) {
    use segcam_motion::Channel;
    out.clear();
    out.extend(pixels.as_chunks::<4>().0.iter().map(|&[r, g, b, a]| Argb {
        alpha: u8::from_unit(a),
        red: u8::from_unit(r),
        green: u8::from_unit(g),
        blue: u8::from_unit(b),
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (i32, i32) = (96, 64);

    /// Grey, with a white square at `x0` (None: no square).
    fn frame(x0: Option<i32>) -> Vec<Argb<u8>> {
        let (w, h) = SIZE;
        (0..h)
            .flat_map(|y| {
                (0..w).map(move |x| {
                    let lit =
                        x0.is_some_and(|x0| (x0..x0 + 16).contains(&x) && (24..40).contains(&y));
                    let v = if lit { 255 } else { 64 };
                    Argb {
                        alpha: 255,
                        red: v,
                        green: v,
                        blue: v,
                    }
                })
            })
            .collect()
    }

    fn settings(lifetime: u32, max_tiles: usize) -> Settings {
        Settings {
            motion: MotionSettings::default(),
            background_frames: 16,
            grid_width: 96,
            lifetime,
            max_tiles,
        }
    }

    fn push(live: &mut Live, x0: Option<i32>, s: &Settings) {
        let pixels = frame(x0);
        live.push(&View::from_pixels(&pixels, SIZE.0, SIZE.1), s);
    }

    #[test]
    fn windows_stay_until_their_lifetime() {
        let s = settings(5, 100);
        let mut live = Live::default();
        let mut most = 0;
        for i in 0..30 {
            // A square moving for 6 frames, then a still frame (which reads as motion
            // until the background has settled).
            push(&mut live, (i < 6).then_some(8 * i), &s);
            assert!(
                live.born.iter().all(|&b| live.frame - b < 5),
                "none older than 5 frames"
            );
            most = most.max(live.len());
        }
        assert!(most > 1, "windows build up while things move");
        assert_eq!(live.len(), 0, "every window has expired");
    }

    #[test]
    fn max_tiles_pushes_out_the_oldest() {
        let s = settings(1000, 2);
        let mut live = Live::default();
        for i in 0..6 {
            push(&mut live, Some(8 * i), &s);
            assert!(live.len() <= 2);
        }
        assert_eq!(live.len(), 2);
    }

    #[test]
    fn a_seek_clears_the_pile() {
        let s = settings(100, 100);
        let mut live = Live::default();
        for i in 0..6 {
            push(&mut live, Some(8 * i), &s);
        }
        live.reset();
        assert_eq!(live.len(), 0);
    }

    /// Per-frame time against segcam-motion's swarm at 1080p:
    /// `cargo test --release -p vfx-preview time_live -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn time_live() {
        use crate::controls::Controls;
        use crate::segcam::{self, Segcam};
        let (w, h) = (1920u32, 1080u32);
        for live_on in [false, true] {
            let look = segcam::settings(&Controls {
                segcam_on: true,
                segcam_mode: 3,
                segcam_live_on: live_on,
                ..Controls::default()
            });
            let mut seg = Segcam::new((w, h));
            let mut total = std::time::Duration::ZERO;
            let mut push_total = std::time::Duration::ZERO;
            for i in 0..60u32 {
                let x0 = (i * 23) % (w - 300);
                let px: Vec<f32> = (0..h)
                    .flat_map(|y| {
                        (0..w).flat_map(move |x| {
                            let lit = (x0..x0 + 300).contains(&x) && (200..500).contains(&y);
                            let v = if lit {
                                1.0
                            } else {
                                ((x ^ y) & 31) as f32 / 64.0
                            };
                            [v, v * 0.8, v * 0.6, 1.0]
                        })
                    })
                    .collect();
                let t = std::time::Instant::now();
                seg.push(px, look.as_ref());
                let pushed = t.elapsed();
                seg.output(look);
                if i >= 40 {
                    total += t.elapsed();
                    push_total += pushed;
                }
            }
            let name = if live_on {
                "segcam-live"
            } else {
                "segcam-motion swarm"
            };
            eprintln!(
                "{name}: {:?}/frame, {:?} of it taking the frame in",
                total / 20,
                push_total / 20
            );
        }
    }
}
