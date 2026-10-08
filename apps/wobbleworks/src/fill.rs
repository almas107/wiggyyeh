//! Bucket fill, computed separately for every frame so the fill boils with its outline.

use egui::Color32;

use crate::model::Doc;
use crate::pixels::{IRect, Pixmap};
use crate::render::Renderer;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FillOpts {
    /// Per-channel tolerance, 0..=255.
    pub tolerance: u8,
    /// Grow the filled area by this many pixels (tucks it under the outline).
    pub grow: u8,
    /// Look at every visible layer (true) or only the current one.
    pub sample_all: bool,
}

impl Default for FillOpts {
    fn default() -> Self {
        FillOpts { tolerance: 40, grow: 1, sample_all: true }
    }
}

/// Most grow passes.
pub const MAX_GROW: u8 = 8;

/// Pixels connected to (sx, sy) that match its colour within `tol`. `None` if the seed is
/// outside the image.
pub fn flood(src: &Pixmap, sx: i32, sy: i32, tol: u8, grow: u8) -> Option<Vec<bool>> {
    let seed = src.index(sx, sy)?;
    let (w, h) = (src.w, src.h);
    let target = *src.px.get(seed)?;
    let tol = i32::from(tol);
    let ok = |c: Color32| -> bool {
        if target.a() < 8 {
            return c.a() < 8;
        }
        let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).abs() <= tol;
        d(c.r(), target.r()) && d(c.g(), target.g()) && d(c.b(), target.b()) && d(c.a(), target.a())
    };
    let matches = |i: usize| src.px.get(i).is_some_and(|c| ok(*c));
    let mut mask = vec![false; w * h];
    let mut stack = vec![seed];
    while let Some(i) = stack.pop() {
        if mask.get(i).copied().unwrap_or(true) {
            continue;
        }
        let row = i / w * w;
        let (mut l, mut r) = (i, i);
        while l > row && !mask.get(l - 1).copied().unwrap_or(true) && matches(l - 1) {
            l -= 1;
        }
        while r + 1 < row + w && !mask.get(r + 1).copied().unwrap_or(true) && matches(r + 1) {
            r += 1;
        }
        for k in l..=r {
            if let Some(m) = mask.get_mut(k) {
                *m = true;
            }
            if k >= w && !mask.get(k - w).copied().unwrap_or(true) && matches(k - w) {
                stack.push(k - w);
            }
            if k + w < w * h && !mask.get(k + w).copied().unwrap_or(true) && matches(k + w) {
                stack.push(k + w);
            }
        }
    }
    for _ in 0..grow.min(MAX_GROW) {
        let prev = mask.clone();
        let at = |x: usize, y: usize| prev.get(y * w + x).copied().unwrap_or(false);
        for y in 0..h {
            for x in 0..w {
                if at(x, y) {
                    continue;
                }
                let near = (x > 0 && at(x - 1, y)) || (x + 1 < w && at(x + 1, y)) || (y > 0 && at(x, y - 1)) || (y + 1 < h && at(x, y + 1));
                if near && let Some(m) = mask.get_mut(y * w + x) {
                    *m = true;
                }
            }
        }
    }
    Some(mask)
}

/// Fill at canvas point `at` on the current layer. The renderer must be synced with `doc`.
/// Returns the filled area (empty if nothing changed).
pub fn fill(doc: &mut Doc, renderer: &Renderer, at: (f64, f64), color: Color32, opts: FillOpts) -> IRect {
    let (sx, sy) = (at.0.floor() as i32, at.1.floor() as i32);
    let (w, h, frames) = (doc.w, doc.h, doc.frames);
    let Some(layer) = doc.layers.get(doc.current) else { return IRect::EMPTY };
    let cache = renderer.cache(layer.id);
    let lock = layer.alpha_lock;
    // An empty layer keeps no images; looking at it alone means looking at nothing.
    let blank = (!opts.sample_all && cache.is_none_or(|c| c.frames.is_empty())).then(|| Pixmap::new(w, h));
    let mut masks = Vec::with_capacity(frames);
    for f in 0..frames {
        let src = if opts.sample_all { renderer.frame(f) } else { cache.and_then(|c| c.frames.get(f)).or(blank.as_ref()) };
        let m = src.and_then(|s| flood(s, sx, sy, opts.tolerance, opts.grow));
        // Alpha lock: only where the layer already has pixels in this frame.
        let m = m.map(|mut m| {
            if lock {
                match cache.and_then(|c| c.frames.get(f)) {
                    Some(own) => {
                        for (k, v) in m.iter_mut().enumerate() {
                            if own.px.get(k).is_none_or(|c| c.a() < 8) {
                                *v = false;
                            }
                        }
                    }
                    // An empty layer has no pixels to keep paint on.
                    None => m.fill(false),
                }
            }
            m
        });
        masks.push(m);
    }
    let Some(layer) = doc.layers.get_mut(doc.current) else { return IRect::EMPTY };
    let mut area = IRect::EMPTY;
    let solid = Color32::from_rgb(color.r(), color.g(), color.b());
    for (f, m) in masks.into_iter().enumerate() {
        let Some(m) = m else { continue };
        if !m.iter().any(|v| *v) {
            continue;
        }
        let Some(r) = layer.raster_mut(f, w, h) else { continue };
        for (k, on) in m.iter().enumerate() {
            if *on && let Some(p) = r.px.get_mut(k) {
                *p = solid;
                let (x, y) = (i32::try_from(k % w).unwrap_or(0), i32::try_from(k / w).unwrap_or(0));
                area = area.union(IRect::new(x, y, x + 1, y + 1));
            }
        }
    }
    if !area.is_empty() {
        layer.touch();
    }
    area
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring() -> Pixmap {
        let mut p = Pixmap::filled(20, 20, Color32::WHITE);
        for i in 4..16 {
            p.set(i, 4, Color32::BLACK);
            p.set(i, 15, Color32::BLACK);
            p.set(4, i, Color32::BLACK);
            p.set(15, i, Color32::BLACK);
        }
        p
    }

    #[test]
    fn flood_stays_inside_the_outline_and_grows() {
        let p = ring();
        let m = flood(&p, 10, 10, 40, 0).unwrap();
        assert_eq!(m.iter().filter(|v| **v).count(), 10 * 10);
        let g = flood(&p, 10, 10, 40, 1).unwrap();
        assert_eq!(g.iter().filter(|v| **v).count(), 10 * 10 + 40);
        assert!(flood(&p, -1, 3, 0, 0).is_none());
        assert!(flood(&p, 3, 99, 0, 0).is_none());
        let outside = flood(&p, 0, 0, 0, 0).unwrap();
        assert!(!outside[10 * 20 + 10]);
    }

    #[test]
    fn fill_writes_every_frame_and_respects_alpha_lock() {
        let mut d = Doc::new(20, 20);
        let mut r = Renderer::default();
        r.sync(&d);
        let a = fill(&mut d, &r, (5.0, 5.0), Color32::RED, FillOpts::default());
        assert!(!a.is_empty());
        for f in 0..3 {
            assert_eq!(d.layers[0].raster[f].as_ref().unwrap().get(5, 5), Color32::RED);
        }
        // Alpha lock on an empty layer: nothing to fill.
        d.layers.push(crate::model::Layer::new("x", 3));
        d.current = 1;
        d.layers[1].alpha_lock = true;
        r.sync(&d);
        assert!(fill(&mut d, &r, (5.0, 5.0), Color32::BLUE, FillOpts::default()).is_empty());
        assert!(fill(&mut d, &r, (f64::NAN, 1e30), Color32::BLUE, FillOpts::default()).is_empty());
        // Only this (empty) layer: the whole canvas is one region.
        d.layers[1].alpha_lock = false;
        let a = fill(&mut d, &r, (5.0, 5.0), Color32::BLUE, FillOpts { sample_all: false, grow: 0, ..FillOpts::default() });
        assert_eq!((a.width(), a.height()), (20, 20));
    }
}
