//! Pixel buffers and the small amount of blend math WobbleWorks needs.
//!
//! Every buffer is premultiplied [`Color32`] (what egui uploads), row-major, top to bottom. Marks
//! are hard-edged, so most writes are plain stores; layer opacity and blend modes are applied
//! only when compositing.

use egui::Color32;
use serde::{Deserialize, Serialize};

/// Largest canvas side we accept, from the UI, files or anything else.
pub const MAX_SIDE: usize = 4096;
/// Smallest canvas side.
pub const MIN_SIDE: usize = 16;

/// Integer rectangle, `x1`/`y1` exclusive. An empty rectangle has `x1 <= x0` or `y1 <= y0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct IRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl IRect {
    pub const EMPTY: IRect = IRect { x0: 0, y0: 0, x1: 0, y1: 0 };

    pub fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        IRect { x0, y0, x1, y1 }
    }

    pub fn full(w: usize, h: usize) -> Self {
        IRect { x0: 0, y0: 0, x1: side(w), y1: side(h) }
    }

    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    pub fn union(self, o: IRect) -> IRect {
        if self.is_empty() {
            return o;
        }
        if o.is_empty() {
            return self;
        }
        IRect { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }

    pub fn intersect(self, o: IRect) -> IRect {
        let r = IRect { x0: self.x0.max(o.x0), y0: self.y0.max(o.y0), x1: self.x1.min(o.x1), y1: self.y1.min(o.y1) };
        if r.is_empty() { IRect::EMPTY } else { r }
    }

    pub fn clamp_to(self, w: usize, h: usize) -> IRect {
        self.intersect(IRect::full(w, h))
    }

    pub fn expand(self, by: i32) -> IRect {
        if self.is_empty() {
            return self;
        }
        IRect { x0: self.x0.saturating_sub(by), y0: self.y0.saturating_sub(by), x1: self.x1.saturating_add(by), y1: self.y1.saturating_add(by) }
    }

    pub fn width(&self) -> usize {
        usize::try_from(self.x1.saturating_sub(self.x0)).unwrap_or(0)
    }

    pub fn height(&self) -> usize {
        usize::try_from(self.y1.saturating_sub(self.y0)).unwrap_or(0)
    }
}

/// A side length as `i32` (sides are capped far below `i32::MAX`).
pub fn side(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// A premultiplied RGBA image.
#[derive(Clone, Debug, PartialEq)]
pub struct Pixmap {
    pub w: usize,
    pub h: usize,
    pub px: Vec<Color32>,
}

impl Pixmap {
    /// A transparent image. Sides are clamped to `1..=MAX_SIDE`.
    pub fn new(w: usize, h: usize) -> Self {
        let (w, h) = (w.clamp(1, MAX_SIDE), h.clamp(1, MAX_SIDE));
        Pixmap { w, h, px: vec![Color32::TRANSPARENT; w * h] }
    }

    #[cfg(test)]
    pub fn filled(w: usize, h: usize, c: Color32) -> Self {
        let mut p = Pixmap::new(w, h);
        p.px.fill(c);
        p
    }

    #[inline]
    pub fn index(&self, x: i32, y: i32) -> Option<usize> {
        let (x, y) = (usize::try_from(x).ok()?, usize::try_from(y).ok()?);
        (x < self.w && y < self.h).then_some(y * self.w + x)
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> Color32 {
        self.index(x, y).and_then(|i| self.px.get(i).copied()).unwrap_or(Color32::TRANSPARENT)
    }

    #[inline]
    pub fn set(&mut self, x: i32, y: i32, c: Color32) {
        if let Some(p) = self.index(x, y).and_then(|i| self.px.get_mut(i)) {
            *p = c;
        }
    }

    pub fn is_blank(&self) -> bool {
        self.px.iter().all(|c| c.a() == 0)
    }

    /// A copy resized to `w`×`h`, keeping the top-left content (crop or pad, no scaling).
    pub fn resized(&self, w: usize, h: usize) -> Pixmap {
        let mut out = Pixmap::new(w, h);
        let cw = self.w.min(out.w);
        for y in 0..self.h.min(out.h) {
            let (s, d) = (y * self.w, y * out.w);
            if let (Some(src), Some(dst)) = (self.px.get(s..s + cw), out.px.get_mut(d..d + cw)) {
                dst.copy_from_slice(src);
            }
        }
        out
    }

    /// Copy of a sub-rectangle (clamped to the image), as a new image.
    pub fn crop(&self, r: IRect) -> Pixmap {
        let r = r.clamp_to(self.w, self.h);
        let mut out = Pixmap::new(r.width().max(1), r.height().max(1));
        if r.is_empty() {
            return out;
        }
        let (x0, w) = (usize::try_from(r.x0).unwrap_or(0), r.width());
        for (oy, y) in (r.y0..r.y1).enumerate() {
            let s = usize::try_from(y).unwrap_or(0) * self.w + x0;
            if let (Some(src), Some(dst)) = (self.px.get(s..s + w), out.px.get_mut(oy * out.w..oy * out.w + w)) {
                dst.copy_from_slice(src);
            }
        }
        out
    }

    /// Overwrite the area at `(ox, oy)` with `src` (no blending; clipped to this image).
    pub fn paste(&mut self, src: &Pixmap, (ox, oy): (i32, i32)) {
        let area = IRect::new(ox, oy, ox.saturating_add(side(src.w)), oy.saturating_add(side(src.h))).clamp_to(self.w, self.h);
        let (sx0, w) = (usize::try_from(area.x0 - ox).unwrap_or(0), area.width());
        for y in area.y0..area.y1 {
            let sy = usize::try_from(y - oy).unwrap_or(0);
            let d = usize::try_from(y).unwrap_or(0) * self.w + usize::try_from(area.x0).unwrap_or(0);
            let s = sy * src.w + sx0;
            if let (Some(dst), Some(srow)) = (self.px.get_mut(d..d + w), src.px.get(s..s + w)) {
                dst.copy_from_slice(srow);
            }
        }
    }

    /// Straight (unmultiplied) RGBA bytes, for encoders.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.px.len() * 4);
        for c in &self.px {
            v.extend_from_slice(&c.to_srgba_unmultiplied());
        }
        v
    }

    /// From straight RGBA bytes. `None` when the byte count doesn't match.
    pub fn from_rgba(w: usize, h: usize, rgba: &[u8]) -> Option<Pixmap> {
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || rgba.len() != w.checked_mul(h)?.checked_mul(4)? {
            return None;
        }
        let px = rgba.chunks_exact(4).map(|c| Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])).collect();
        Some(Pixmap { w, h, px })
    }

    /// Paint `src` over this image at integer offset `(ox, oy)` (source-over).
    pub fn draw_over(&mut self, src: &Pixmap, ox: i32, oy: i32) {
        for sy in 0..src.h {
            let y = oy.saturating_add(side(sy));
            for sx in 0..src.w {
                let s = src.px.get(sy * src.w + sx).copied().unwrap_or(Color32::TRANSPARENT);
                if s.a() == 0 {
                    continue;
                }
                let x = ox.saturating_add(side(sx));
                if let Some(d) = self.index(x, y).and_then(|i| self.px.get_mut(i)) {
                    *d = blend(*d, s, 255, Blend::Normal);
                }
            }
        }
    }

    /// Nearest-neighbour scale to an exact size (for exports at 2×, 3×, …).
    pub fn scaled_nearest(&self, w: usize, h: usize) -> Pixmap {
        let mut out = Pixmap::new(w, h);
        for y in 0..out.h {
            let sy = y * self.h / out.h;
            for x in 0..out.w {
                let sx = x * self.w / out.w;
                if let (Some(d), Some(s)) = (out.px.get_mut(y * w + x), self.px.get(sy * self.w + sx)) {
                    *d = *s;
                }
            }
        }
        out
    }
}

/// Layer blend modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Blend {
    #[default]
    Normal,
    Multiply,
    Screen,
    Add,
    Darken,
    Lighten,
}

impl Blend {
    pub const ALL: [Blend; 6] = [Blend::Normal, Blend::Multiply, Blend::Screen, Blend::Add, Blend::Darken, Blend::Lighten];

    pub fn label(self) -> &'static str {
        match self {
            Blend::Normal => "Normal",
            Blend::Multiply => "Multiply",
            Blend::Screen => "Screen",
            Blend::Add => "Add",
            Blend::Darken => "Darken",
            Blend::Lighten => "Lighten",
        }
    }
}

/// `a * b / 255`, rounded.
#[inline]
pub fn mul255(a: u32, b: u32) -> u32 {
    let t = a * b + 128;
    (t + (t >> 8)) >> 8
}

/// Composite premultiplied `src` (scaled by `opacity` 0..=255) over premultiplied `dst`.
#[inline]
pub fn blend(dst: Color32, src: Color32, opacity: u32, mode: Blend) -> Color32 {
    let k = |v: u8| mul255(u32::from(v), opacity);
    let (sr, sg, sb, sa) = (k(src.r()), k(src.g()), k(src.b()), k(src.a()));
    if sa == 0 {
        return dst;
    }
    let (dr, dg, db, da) = (u32::from(dst.r()), u32::from(dst.g()), u32::from(dst.b()), u32::from(dst.a()));
    let isa = 255 - sa;
    let ida = 255 - da;
    let out_a = sa + mul255(da, isa);
    let ch = |s: u32, d: u32| -> u32 {
        let mixed = match mode {
            Blend::Normal => s * 255,
            Blend::Multiply => s * d,
            Blend::Screen => (s * da + d * sa).saturating_sub(s * d),
            Blend::Add => (s * da + d * sa).min(sa * da),
            Blend::Darken => (s * da).min(d * sa),
            Blend::Lighten => (s * da).max(d * sa),
        };
        // Porter-Duff over with the mixed colour where both are present.
        let both = if mode == Blend::Normal { s * 255 } else { mixed };
        let v = if mode == Blend::Normal { both + d * isa } else { both + s * ida + d * isa };
        ((v + 127) / 255).min(255)
    };
    let c = |v: u32| u8::try_from(v.min(255)).unwrap_or(255);
    let a = c(out_a);
    // Keep premultiplied invariant (colour channels never exceed alpha).
    let clamp = |v: u32| c(v).min(a);
    Color32::from_rgba_premultiplied(clamp(ch(sr, dr)), clamp(ch(sg, dg)), clamp(ch(sb, db)), a)
}

/// Parse `#rrggbb` (or `rrggbb`) into an opaque colour.
pub fn parse_hex(s: &str) -> Option<Color32> {
    let s = s.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 || !s.is_ascii() {
        return None;
    }
    let p = |i: usize| s.get(i..i + 2).and_then(|t| u8::from_str_radix(t, 16).ok());
    Some(Color32::from_rgb(p(0)?, p(2)?, p(4)?))
}

pub fn to_hex(c: Color32) -> String {
    let [r, g, b, _] = c.to_srgba_unmultiplied();
    format!("#{r:02x}{g:02x}{b:02x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip_and_bad_input() {
        let c = parse_hex("#ff2e88").unwrap();
        assert_eq!(to_hex(c), "#ff2e88");
        for bad in ["", "#", "#12345", "#gggggg", "#ff2e8€", "#1234567"] {
            assert!(parse_hex(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn normal_blend_is_source_over() {
        let red = Color32::from_rgb(255, 0, 0);
        let blue = Color32::from_rgb(0, 0, 255);
        assert_eq!(blend(blue, red, 255, Blend::Normal), red);
        assert_eq!(blend(blue, red, 0, Blend::Normal), blue);
        let half = blend(Color32::TRANSPARENT, red, 128, Blend::Normal);
        assert_eq!(half.a(), 128);
        assert!(half.r() <= half.a());
    }

    #[test]
    fn multiply_and_screen() {
        let grey = Color32::from_rgb(128, 128, 128);
        let white = Color32::WHITE;
        assert_eq!(blend(white, grey, 255, Blend::Multiply), grey);
        assert_eq!(blend(grey, white, 255, Blend::Screen), white);
        assert_eq!(blend(Color32::BLACK, grey, 255, Blend::Lighten), grey);
        assert_eq!(blend(white, grey, 255, Blend::Darken), grey);
    }

    #[test]
    fn every_mode_keeps_premultiplied_invariant() {
        let samples = [Color32::TRANSPARENT, Color32::from_rgba_premultiplied(40, 20, 10, 60), Color32::WHITE, Color32::from_rgb(10, 200, 90)];
        for m in Blend::ALL {
            for d in samples {
                for s in samples {
                    for op in [0, 77, 255] {
                        let o = blend(d, s, op, m);
                        assert!(o.r() <= o.a() && o.g() <= o.a() && o.b() <= o.a(), "{m:?} {d:?} {s:?} -> {o:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn resize_crop_and_bounds() {
        let mut p = Pixmap::new(4, 3);
        p.set(3, 2, Color32::RED);
        p.set(-1, 0, Color32::RED);
        p.set(9, 9, Color32::RED);
        assert_eq!(p.get(3, 2), Color32::RED);
        assert_eq!(p.get(-5, 2), Color32::TRANSPARENT);
        let r = p.resized(8, 8);
        assert_eq!(r.get(3, 2), Color32::RED);
        let c = p.crop(IRect::new(2, 1, 100, 100));
        assert_eq!((c.w, c.h), (2, 2));
        assert_eq!(c.get(1, 1), Color32::RED);
        let mut q = Pixmap::new(4, 3);
        q.paste(&c, (2, 1));
        assert_eq!(q, p);
        q.paste(&c, (-100, i32::MAX));
        q.paste(&c, (3, 2));
        assert_eq!(q.get(3, 2), Color32::TRANSPARENT);
        assert_eq!(Pixmap::new(0, usize::MAX).w, 1);
        assert!(Pixmap::from_rgba(2, 2, &[0; 3]).is_none());
    }
}
