//! RGBA, bottom-left OpenFX storage to top-down ARGB; keep the host's alpha convention.
use segcam_motion::pixel::{Argb, Channel};
use vfx_ofx::image::{Depth, Image, Rect};

fn channel_bytes(depth: Depth) -> usize {
    match depth {
        Depth::Byte => 1,
        Depth::Short => 2,
        Depth::Float => 4,
    }
}
unsafe fn address(image: &Image, x: i32, y: i32) -> *mut u8 {
    // SAFETY: caller supplies coordinates inside the host image.
    unsafe {
        image.data.offset(
            (y - image.bounds.y1) as isize * image.row_bytes
                + (x - image.bounds.x1) as isize * (4 * channel_bytes(image.depth)) as isize,
        )
    }
}
pub unsafe fn decode<C: Channel>(image: &Image, canvas: Rect) -> Vec<Argb<C>> {
    let mut out = vec![Argb::default(); canvas.width() * canvas.height()];
    let r = canvas.intersect(&image.bounds);
    for y in r.y1..r.y2 {
        for x in r.x1..r.x2 {
            // SAFETY: intersection confines the pointer and four channels to the host image.
            let p = unsafe { address(image, x, y) };
            let read = |i| unsafe {
                match image.depth {
                    Depth::Byte => C::from_unit(f32::from(*p.add(i)) / 255.0),
                    Depth::Short => C::from_unit(
                        f32::from(p.add(i * 2).cast::<u16>().read_unaligned()) / 65535.0,
                    ),
                    Depth::Float => C::from_unit(p.add(i * 4).cast::<f32>().read_unaligned()),
                }
            };
            out[(canvas.y2 - 1 - y) as usize * canvas.width() + (x - canvas.x1) as usize] = Argb {
                alpha: read(3),
                red: read(0),
                green: read(1),
                blue: read(2),
            };
        }
    }
    out
}
pub unsafe fn encode<C: Channel>(pixels: &[Argb<C>], canvas: Rect, image: &Image, window: Rect) {
    for y in window.y1..window.y2 {
        for x in window.x1..window.x2 {
            let v =
                pixels[(canvas.y2 - 1 - y) as usize * canvas.width() + (x - canvas.x1) as usize];
            // SAFETY: caller confines the window to both canvas and output bounds.
            let p = unsafe { address(image, x, y) };
            for (i, c) in [v.red, v.green, v.blue, v.alpha].into_iter().enumerate() {
                let c = c.to_unit();
                // SAFETY: each write addresses one channel of the host pixel, possibly unaligned.
                unsafe {
                    match image.depth {
                        Depth::Byte => *p.add(i) = (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
                        Depth::Short => p
                            .add(i * 2)
                            .cast::<u16>()
                            .write_unaligned((c.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16),
                        Depth::Float => p.add(i * 4).cast::<f32>().write_unaligned(c),
                    }
                }
            }
        }
    }
}
