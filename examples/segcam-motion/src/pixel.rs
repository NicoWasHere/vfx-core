//! Pixels as After Effects lays them out: alpha, red, green, blue, of one channel type.
//! `Argb<u8>`, `Argb<u16>` and `Argb<f32>` are byte for byte `PF_Pixel8`, `PF_Pixel16` and
//! `PF_PixelFloat`, so host buffers are read and written in place, at their own row stride.

use std::marker::PhantomData;

mod sealed {
    pub trait Sealed {}
    impl Sealed for u8 {}
    impl Sealed for u16 {}
    impl Sealed for f32 {}
}

/// One channel type: `u8`, `u16` or `f32`, each valid for any bit pattern. Integer channels
/// clamp when written; float channels keep any value.
pub trait Channel: sealed::Sealed + Copy + Default + PartialEq + std::fmt::Debug + 'static {
    /// The channel's white. After Effects' 16-bit white is 32768, not 65535.
    const MAX: f32;
    fn to_unit(self) -> f32;
    fn from_unit(f: f32) -> Self;
}

impl Channel for u8 {
    const MAX: f32 = 255.0;
    fn to_unit(self) -> f32 {
        f32::from(self) / <Self as Channel>::MAX
    }
    fn from_unit(f: f32) -> Self {
        (f.clamp(0.0, 1.0) * <Self as Channel>::MAX + 0.5) as u8
    }
}

impl Channel for u16 {
    const MAX: f32 = 32768.0;
    fn to_unit(self) -> f32 {
        f32::from(self) / <Self as Channel>::MAX
    }
    fn from_unit(f: f32) -> Self {
        (f.clamp(0.0, 1.0) * <Self as Channel>::MAX + 0.5) as u16
    }
}

impl Channel for f32 {
    const MAX: f32 = 1.0;
    fn to_unit(self) -> f32 {
        self
    }
    fn from_unit(f: f32) -> Self {
        f
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Argb<C> {
    pub alpha: C,
    pub red: C,
    pub green: C,
    pub blue: C,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

/// A premultiplied pixel of `c` at `alpha`.
pub fn make_pixel<C: Channel>(c: Color, alpha: f32) -> Argb<C> {
    Argb {
        alpha: C::from_unit(alpha),
        red: C::from_unit(c.r * alpha),
        green: C::from_unit(c.g * alpha),
        blue: C::from_unit(c.b * alpha),
    }
}

pub fn scale_pixel<C: Channel>(p: &Argb<C>, k: f32) -> Argb<C> {
    Argb {
        alpha: C::from_unit(p.alpha.to_unit() * k),
        red: C::from_unit(p.red.to_unit() * k),
        green: C::from_unit(p.green.to_unit() * k),
        blue: C::from_unit(p.blue.to_unit() * k),
    }
}

/// Converts between channel types through the unit range.
pub fn convert<A: Channel, B: Channel>(p: &Argb<A>) -> Argb<B> {
    Argb {
        alpha: B::from_unit(p.alpha.to_unit()),
        red: B::from_unit(p.red.to_unit()),
        green: B::from_unit(p.green.to_unit()),
        blue: B::from_unit(p.blue.to_unit()),
    }
}

/// A read-only picture: rows top to bottom, `stride` bytes apart.
#[derive(Clone, Copy)]
pub struct View<'a, C> {
    data: &'a [u8],
    pub width: i32,
    pub height: i32,
    stride: usize,
    _channel: PhantomData<C>,
}

impl<'a, C: Channel> View<'a, C> {
    /// Panics if `data` can't hold `height` rows of `width` pixels `stride` bytes apart, or
    /// isn't aligned for the channel type.
    pub fn new(data: &'a [u8], width: i32, height: i32, stride: usize) -> Self {
        check::<C>(data, width, height, stride);
        Self {
            data,
            width,
            height,
            stride,
            _channel: PhantomData,
        }
    }

    pub fn from_pixels(pixels: &'a [Argb<C>], width: i32, height: i32) -> Self {
        let bytes = unsafe {
            std::slice::from_raw_parts(pixels.as_ptr().cast::<u8>(), size_of_val(pixels))
        };
        Self::new(bytes, width, height, row_bytes::<C>(width))
    }

    pub fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    pub fn row(&self, y: i32) -> &'a [Argb<C>] {
        row(self.data, y, self.width, self.stride)
    }
}

/// A picture being drawn into.
pub struct ViewMut<'a, C> {
    data: &'a mut [u8],
    pub width: i32,
    pub height: i32,
    stride: usize,
    _channel: PhantomData<C>,
}

impl<'a, C: Channel> ViewMut<'a, C> {
    pub fn new(data: &'a mut [u8], width: i32, height: i32, stride: usize) -> Self {
        check::<C>(data, width, height, stride);
        Self {
            data,
            width,
            height,
            stride,
            _channel: PhantomData,
        }
    }

    pub fn from_pixels(pixels: &'a mut [Argb<C>], width: i32, height: i32) -> Self {
        let len = size_of_val(pixels);
        let bytes =
            unsafe { std::slice::from_raw_parts_mut(pixels.as_mut_ptr().cast::<u8>(), len) };
        Self::new(bytes, width, height, row_bytes::<C>(width))
    }

    pub fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    pub fn row(&self, y: i32) -> &[Argb<C>] {
        row(self.data, y, self.width, self.stride)
    }

    pub fn row_mut(&mut self, y: i32) -> &mut [Argb<C>] {
        let start = y as usize * self.stride;
        let bytes = &mut self.data[start..start + row_bytes::<C>(self.width)];
        // `check` made sure every row start is aligned, and any bits are a valid channel.
        unsafe { std::slice::from_raw_parts_mut(bytes.as_mut_ptr().cast(), self.width as usize) }
    }
}

fn row_bytes<C>(width: i32) -> usize {
    width.max(0) as usize * size_of::<Argb<C>>()
}

fn row<C>(data: &[u8], y: i32, width: i32, stride: usize) -> &[Argb<C>] {
    let start = y as usize * stride;
    let bytes = &data[start..start + row_bytes::<C>(width)];
    // `check` made sure every row start is aligned, and any bits are a valid channel.
    unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast(), width as usize) }
}

fn check<C>(data: &[u8], width: i32, height: i32, stride: usize) {
    let align = align_of::<Argb<C>>();
    assert!(
        (data.as_ptr() as usize).is_multiple_of(align) && stride.is_multiple_of(align),
        "pixel rows not aligned for their channel type"
    );
    if width <= 0 || height <= 0 {
        return;
    }
    let row = row_bytes::<C>(width);
    assert!(stride >= row, "row stride smaller than a row");
    assert!(
        data.len() >= stride * (height as usize - 1) + row,
        "pixel buffer too small for {width}x{height}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_channels_clamp_and_round() {
        assert_eq!(u8::from_unit(1.5), 255);
        assert_eq!(u8::from_unit(-1.0), 0);
        assert_eq!(u8::from_unit(0.5), 128);
        assert_eq!(u16::from_unit(1.0), 32768);
        assert_eq!(f32::from_unit(1.5), 1.5);
    }

    #[test]
    fn strided_rows() {
        // Two pixels a row in room for three: the third is padding.
        let mut bytes = vec![0u8; 3 * 4 * 2];
        let mut v = ViewMut::<u8>::new(&mut bytes, 2, 2, 12);
        v.row_mut(1)[1].red = 9;
        assert_eq!(bytes[12 + 4 + 1], 9);
    }
}
