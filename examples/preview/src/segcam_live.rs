//! segcam-live: the swarm as the live app runs it, with a pile kept between frames.
//!
//! segcam-motion's swarm is rebuilt from the last 24 frames on every frame, as After
//! Effects needs (it can ask for any frame, in any order). The preview only goes forward,
//! so this keeps the pile instead: each frame is segmented once, against the luma grids of
//! the frames before it, and its moving regions become new windows on top. It uses
//! segcam-motion's own detection, crops and window drawing, so the windows look the same.
//!
//! Drawing a window is the slow part (its shadow and corners are worked out per pixel), so
//! each one is drawn once: a frame's windows go into a layer of their own, cropped to where
//! they and their shadows land, and the picture is the layers stacked oldest first. A new
//! frame lays its layer on the picture; a layer expires whole, after `lifetime` frames or
//! oldest first past `max_tiles` windows, and the picture is then stacked again from the
//! layers left, which is copying pixels rather than drawing.

use std::collections::VecDeque;

use segcam_motion::render::PixelRect;
use segcam_motion::swarm::{self, ShadowKernel, SwarmMetrics};
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

/// One frame's windows, drawn: premultiplied pixels over `rect` of the canvas.
struct Layer {
    born: i64,
    rect: PixelRect,
    pixels: Vec<Argb<u8>>,
    windows: usize,
}

/// The layers, the picture they make and the frames' luma grids.
#[derive(Default)]
pub struct Live {
    /// The last `background_frames` frames' grids, oldest first.
    grids: VecDeque<LumaGrid>,
    /// Oldest first.
    layers: VecDeque<Layer>,
    frame: i64,
    size: (i32, i32),
    /// Clear, for drawing a frame's windows into before they're cropped to a layer.
    scratch: Vec<Argb<u8>>,
    /// The layers stacked.
    picture: Vec<Argb<u8>>,
    /// The newest layer in `picture`, by frame; None when it needs stacking again.
    stacked: Option<i64>,
    /// For the canvas height it was made for.
    shadow: Option<(i32, ShadowKernel)>,
    /// Every window made, to check the layers against segcam-motion's own drawing.
    #[cfg(test)]
    tiles: VecDeque<swarm::Tile>,
}

impl Live {
    /// Starts over, after a seek.
    pub fn reset(&mut self) {
        self.grids.clear();
        self.layers.clear();
        self.stacked = None;
    }

    /// Segments the next frame and lays its moving regions on the pile.
    pub fn push(&mut self, frame: &View<u8>, s: &Settings) {
        self.frame += 1;
        let grid = render::downsample(frame, s.grid_width);
        let found = motion::detect(&self.grids, &grid, &s.motion);
        self.grids.push_back(grid);
        while self.grids.len() > s.background_frames.max(1) {
            self.grids.pop_front();
        }

        let (w, h) = (frame.width, frame.height);
        if self.size != (w, h) {
            self.size = (w, h);
            self.scratch = vec![Argb::default(); (w * h) as usize];
            self.layers.clear();
            self.stacked = None;
        }
        if !found.blobs.is_empty() {
            let m = SwarmMetrics::new(h);
            if self.shadow.as_ref().is_none_or(|(at, _)| *at != h) {
                self.shadow = Some((h, ShadowKernel::new(m.scale)));
            }
            let shadow = &self.shadow.as_ref().expect("made above").1;

            let mut view = ViewMut::from_pixels(&mut self.scratch, w, h);
            let mut reach = PixelRect::new(0, 0, 0, 0);
            for (b, blob) in found.blobs.iter().enumerate() {
                let title = format!("LO.FI.SCI.FI {}", render::segment_label(self.frame, b));
                let tile = swarm::make_tile(blob, Some(frame), title, w, h, false);
                let extent = swarm::tile_extent(tile.video, m.title_height);
                reach = union(
                    reach,
                    PixelRect::new(
                        extent.x - shadow.reach,
                        extent.y - shadow.reach,
                        extent.w + 2 * shadow.reach,
                        extent.h + 2 * shadow.reach,
                    ),
                );
                swarm::draw_window(&mut view, &tile, &m, shadow, false);
                #[cfg(test)]
                self.tiles.push_back(tile);
            }

            // Crop the layer out of the scratch canvas, leaving it clear for next time.
            let rect = render::intersect(reach, PixelRect::new(0, 0, w, h));
            let mut pixels = Vec::with_capacity((rect.w * rect.h).max(0) as usize);
            for y in rect.y..rect.y + rect.h {
                let row = &mut view.row_mut(y)[rect.x as usize..(rect.x + rect.w) as usize];
                pixels.extend_from_slice(row);
                row.fill(Argb::default());
            }
            self.layers.push_back(Layer {
                born: self.frame,
                rect,
                pixels,
                windows: found.blobs.len(),
            });
        }
        self.expire(s);
    }

    /// Drops whole layers: those older than the lifetime, then the oldest while there are
    /// more than `max_tiles` windows (always keeping the newest layer).
    pub fn expire(&mut self, s: &Settings) {
        let lifetime = i64::from(s.lifetime.max(1));
        let mut windows: usize = self.layers.iter().map(|l| l.windows).sum();
        while let Some(oldest) = self.layers.front()
            && (self.frame - oldest.born >= lifetime
                || (windows > s.max_tiles.max(1) && self.layers.len() > 1))
        {
            windows -= oldest.windows;
            self.layers.pop_front();
            self.stacked = None;
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.layers.iter().map(|l| l.windows).sum()
    }

    pub fn draw(&mut self, out: &mut ViewMut<u8>) {
        let (w, h) = (out.width, out.height);
        if self.picture.len() != (w * h) as usize {
            self.picture = vec![Argb::default(); (w * h) as usize];
            self.stacked = None;
        }
        // Lay new layers on the picture, or stack them all again after one expired.
        let from = match self.stacked {
            Some(newest) => newest,
            None => {
                self.picture.fill(Argb::default());
                i64::MIN
            }
        };
        for layer in self.layers.iter().filter(|l| l.born > from) {
            lay(&mut self.picture, w, layer);
        }
        self.stacked = Some(self.layers.back().map_or(self.frame, |l| l.born));
        for y in 0..h {
            out.row_mut(y)
                .copy_from_slice(&self.picture[(y * w) as usize..((y + 1) * w) as usize]);
        }
    }
}

fn union(a: PixelRect, b: PixelRect) -> PixelRect {
    if a.is_empty() {
        return b;
    }
    let (x0, y0) = (a.x.min(b.x), a.y.min(b.y));
    let (x1, y1) = ((a.x + a.w).max(b.x + b.w), (a.y + a.h).max(b.y + b.h));
    PixelRect::new(x0, y0, x1 - x0, y1 - y0)
}

/// Premultiplied "over": the layer on top of the picture.
fn lay(picture: &mut [Argb<u8>], width: i32, layer: &Layer) {
    let r = layer.rect;
    for (y, src) in (r.y..).zip(layer.pixels.chunks_exact(r.w.max(1) as usize)) {
        let start = (y * width + r.x) as usize;
        for (d, s) in picture[start..start + r.w as usize].iter_mut().zip(src) {
            if s.alpha == 0 {
                continue;
            }
            let keep = u32::from(255 - s.alpha);
            let over = |s: u8, d: u8| s.saturating_add(((u32::from(d) * keep + 127) / 255) as u8);
            *d = Argb {
                alpha: over(s.alpha, d.alpha),
                red: over(s.red, d.red),
                green: over(s.green, d.green),
                blue: over(s.blue, d.blue),
            };
        }
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
                live.layers.iter().all(|l| live.frame - l.born < 5),
                "none older than 5 frames"
            );
            most = most.max(live.len());
        }
        assert!(most > 1, "windows build up while things move");
        assert_eq!(live.len(), 0, "every window has expired");
    }

    /// Stacking the layers gives the picture segcam-motion's swarm draws for the same
    /// windows, but for rounding (layers are kept at 8 bits).
    #[test]
    fn layers_match_segcam_motion() {
        let s = settings(1000, 1000);
        let mut live = Live::default();
        for i in 0..8 {
            push(&mut live, Some(9 * i), &s);
        }
        assert!(live.tiles.len() > 2, "several windows overlap");
        let (w, h) = SIZE;
        let mut layered = vec![Argb::default(); (w * h) as usize];
        live.draw(&mut ViewMut::from_pixels(&mut layered, w, h));
        let mut reference = vec![Argb::<u8>::default(); (w * h) as usize];
        swarm::draw_swarm(
            &mut ViewMut::from_pixels(&mut reference, w, h),
            &live.tiles,
            false,
        );
        let worst = layered
            .iter()
            .zip(&reference)
            .flat_map(|(a, b)| {
                [
                    a.alpha.abs_diff(b.alpha),
                    a.red.abs_diff(b.red),
                    a.green.abs_diff(b.green),
                    a.blue.abs_diff(b.blue),
                ]
            })
            .max()
            .unwrap();
        // Faint shadows overlapping across many layers gather a level or so of rounding each.
        assert!(worst <= 6, "off by up to {worst} levels");
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
