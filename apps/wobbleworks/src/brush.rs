//! Turning strokes into hard-edged, boiling marks.
//!
//! Every mark is stamped from a cached 1-bit bitmap at a whole-pixel position, so edges stay
//! chunky. The wobble is deterministic noise keyed by (stroke seed, point index, frame): each
//! frame is stable and the loop repeats forever, and a stroke can be rendered incrementally
//! while it is being drawn with exactly the same result as a full re-render.

use std::collections::HashMap;
use std::sync::Arc;

use egui::Color32;

use crate::geom;
use crate::model::{Kind, Stroke, Tip};
use crate::pixels::{IRect, Pixmap, mul255};

/// Deterministic noise in `[0, 1)`; bit-for-bit the original Wobbleworks generator (so `.wob`
/// files from it boil the same way).
pub fn rnd(seed: u32) -> f64 {
    let mut t = seed.wrapping_add(0x6D2B_79F5);
    t = (t ^ (t >> 15)).wrapping_mul(t | 1);
    t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
    f64::from(t ^ (t >> 14)) / 4_294_967_296.0
}

/// Noise for three integer coordinates (JavaScript `ToInt32` wrapping semantics).
pub fn jr(a: i64, b: i64, c: i64) -> f64 {
    let a = (a as u32).wrapping_mul(73_856_093);
    let b = (b as u32).wrapping_mul(19_349_663);
    let c = (c as u32).wrapping_mul(83_492_791);
    rnd(a ^ b ^ c)
}

/// `Math.round`: halves round up, also for negatives.
#[inline]
pub fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Shape {
    Tip(Tip),
    /// The slanted calligraphy nib.
    Nib,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct StampKey {
    shape: Shape,
    r: usize,
    patt: u8,
}

/// A 1-bit square mask, `r`×`r`.
#[derive(Debug)]
pub struct Stamp {
    pub r: usize,
    pub mask: Vec<bool>,
}

/// The biggest stamp side (brush size 200 at full pressure is 270).
const MAX_STAMP: usize = 400;
const MAX_CACHED: usize = 600;

fn make_stamp(shape: Shape, r: usize, patt: u8) -> Stamp {
    let r = r.clamp(1, MAX_STAMP);
    let mid = (r as f64 - 1.0) / 2.0;
    let rad = r as f64 / 2.0;
    let mut mask = vec![false; r * r];
    for y in 0..r {
        for x in 0..r {
            let (dx, dy) = (x as f64 - mid, y as f64 - mid);
            let mut on = match shape {
                Shape::Tip(Tip::Round) => dx * dx + dy * dy <= rad * rad,
                Shape::Tip(Tip::Square) => true,
                Shape::Tip(Tip::Diamond) => dx.abs() + dy.abs() <= rad,
                Shape::Tip(Tip::Star) => in_star(dx / rad.max(0.5), dy / rad.max(0.5)),
                Shape::Tip(Tip::Heart) => in_heart(dx / rad.max(0.5), dy / rad.max(0.5)),
                Shape::Nib => (dx + dy).abs() <= (r as f64 * 0.14).max(0.75) && (dx - dy).abs() <= rad * 1.35,
            };
            if on && patt == 1 {
                on = (x + y) & 1 == 0;
            }
            if on && patt == 2 {
                on = x & 1 == 0 && y & 1 == 0;
            }
            if let Some(m) = mask.get_mut(y * r + x) {
                *m = on;
            }
        }
    }
    // Tiny stars and hearts can come out empty: keep at least the middle pixel.
    if !mask.iter().any(|m| *m)
        && let Some(m) = mask.get_mut((r / 2) * r + r / 2)
    {
        *m = true;
    }
    Stamp { r, mask }
}

/// Five-pointed star in the unit disc (point up).
fn in_star(x: f64, y: f64) -> bool {
    let d = x.hypot(y);
    if d > 1.0 {
        return false;
    }
    let a = x.atan2(-y).rem_euclid(std::f64::consts::TAU / 5.0) - std::f64::consts::TAU / 10.0;
    // Edge of a star with inner radius 0.45: interpolate between tip and notch in polar form.
    let inner = 0.45;
    // t = 0 on a point's axis, 1 halfway between two points.
    let t = 1.0 - a.abs() / (std::f64::consts::TAU / 10.0);
    let edge = 1.0 / ((1.0 - t) + t / inner);
    d <= edge.max(inner)
}

/// Heart in the unit square (point down).
fn in_heart(x: f64, y: f64) -> bool {
    let (u, v) = (x * 1.25, -y * 1.25 + 0.25);
    let a = u * u + v * v - 1.0;
    a * a * a - u * u * v * v * v <= 0.0
}

/// How marks combine with what's already on the layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Over,
    Erase,
    /// Alpha lock: recolour existing pixels, never add new ones.
    Atop,
}

/// Where one stroke's incremental render has got to, for one frame.
#[derive(Clone, Debug, Default)]
pub struct Progress {
    walk: [Walker; 3],
    next_pt: usize,
}

#[derive(Clone, Copy, Debug, Default)]
struct Walker {
    acc: f64,
    n: u32,
    seg: usize,
    started: bool,
}

/// Most stamps a single segment may emit (guards absurd coordinates).
const MAX_STEPS: u32 = 200_000;

impl Walker {
    /// The original `walk`: call `cb(x, y, n, i)` every `spacing` pixels along the path given by
    /// `pt`, continuing from where the last call stopped. `i` is the index of the segment's end.
    fn advance(&mut self, len: usize, pt: &dyn Fn(usize) -> (f64, f64), spacing: f64, cb: &mut dyn FnMut(f64, f64, u32, usize)) {
        if len == 0 {
            return;
        }
        if !self.started {
            let (x, y) = pt(0);
            cb(x, y, 0, 0);
            self.started = true;
            self.seg = 1;
        }
        while self.seg < len {
            let (mut x, mut y) = pt(self.seg - 1);
            let (tx, ty) = pt(self.seg);
            let (mut dx, mut dy) = (tx - x, ty - y);
            let mut d = dx.hypot(dy);
            if d == 0.0 || !d.is_finite() {
                self.seg += 1;
                continue;
            }
            let mut steps = 0;
            while self.acc + d >= spacing && steps < MAX_STEPS {
                steps += 1;
                let t = (spacing - self.acc) / d;
                x += dx * t;
                y += dy * t;
                self.n = self.n.wrapping_add(1);
                cb(x, y, self.n, self.seg);
                dx = tx - x;
                dy = ty - y;
                d = dx.hypot(dy);
                self.acc = 0.0;
            }
            self.acc += d;
            self.seg += 1;
        }
    }
}

/// Stamp cache plus the stroke renderer.
#[derive(Default)]
pub struct Brushes {
    stamps: HashMap<StampKey, Arc<Stamp>>,
}

/// One stroke being rendered into one frame.
struct Job<'a> {
    s: &'a Stroke,
    frame: i64,
    amp: f64,
    mode: Mode,
    dst: &'a mut Pixmap,
    org: (i32, i32),
    dirty: IRect,
}

impl Job<'_> {
    /// The stroke's point `i`, wobbled for this frame (pass `k` offsets the noise).
    fn wob(&self, k: i64, i: usize) -> (f64, f64) {
        let Some(p) = self.s.pts.get(i) else { return (0.0, 0.0) };
        if self.s.lock || self.amp < 0.25 {
            return (p.x, p.y);
        }
        let seed = i64::from(self.s.seed);
        let ii = i64::try_from(i).unwrap_or(0);
        let x = p.x + js_round((jr(seed + k, ii, self.frame) * 2.0 - 1.0) * self.amp);
        let y = p.y + js_round((jr(seed + k + 4177, ii, self.frame) * 2.0 - 1.0) * self.amp);
        (x, y)
    }

    fn pressure(&self, i: usize) -> f64 {
        self.s.pts.get(i).map_or(1.0, |p| f64::from(p.p).clamp(0.05, 4.0))
    }

    /// Stamp `st` centred at canvas position (x, y). `grain` punches shimmering holes (chalk).
    fn put(&mut self, st: &Stamp, x: f64, y: f64, grain: Option<u32>) {
        let r = st.r as f64;
        let x0 = (js_round(x - r / 2.0) as i32).saturating_sub(self.org.0);
        let y0 = (js_round(y - r / 2.0) as i32).saturating_sub(self.org.1);
        let ri = i32::try_from(st.r).unwrap_or(1);
        let area = IRect::new(x0, y0, x0.saturating_add(ri), y0.saturating_add(ri)).clamp_to(self.dst.w, self.dst.h);
        if area.is_empty() {
            return;
        }
        self.dirty = self.dirty.union(area);
        let c = self.s.color;
        let (w, mode) = (self.dst.w, self.mode);
        for py in area.y0..area.y1 {
            let my = usize::try_from(py - y0).unwrap_or(0);
            let row = usize::try_from(py).unwrap_or(0) * w;
            for px in area.x0..area.x1 {
                let mx = usize::try_from(px - x0).unwrap_or(0);
                if !st.mask.get(my * st.r + mx).copied().unwrap_or(false) {
                    continue;
                }
                if let Some(g) = grain
                    && rnd(g ^ (u32::try_from(my * st.r + mx).unwrap_or(0)).wrapping_mul(2_654_435_761)) < 0.38
                {
                    continue;
                }
                let Some(d) = self.dst.px.get_mut(row + usize::try_from(px).unwrap_or(0)) else { continue };
                plot(d, c, mode);
            }
        }
    }

    fn span(&mut self, y: i32, x0: i32, x1: i32) {
        let (c, mode, w) = (self.s.color, self.mode, self.dst.w);
        let yy = y.saturating_sub(self.org.1);
        let area = IRect::new(x0.saturating_sub(self.org.0), yy, x1.saturating_sub(self.org.0), yy.saturating_add(1)).clamp_to(self.dst.w, self.dst.h);
        if area.is_empty() {
            return;
        }
        self.dirty = self.dirty.union(area);
        let row = usize::try_from(yy).unwrap_or(0) * w;
        for px in area.x0..area.x1 {
            if let Some(d) = self.dst.px.get_mut(row + usize::try_from(px).unwrap_or(0)) {
                plot(d, c, mode);
            }
        }
    }
}

#[inline]
fn plot(d: &mut Color32, c: Color32, mode: Mode) {
    match mode {
        Mode::Over => *d = if c.a() == 255 { c } else { crate::pixels::blend(*d, c, 255, crate::pixels::Blend::Normal) },
        Mode::Erase => *d = Color32::TRANSPARENT,
        Mode::Atop => {
            let a = u32::from(d.a());
            if a > 0 {
                let k = |v: u8| u8::try_from(mul255(u32::from(v), a)).unwrap_or(255);
                *d = Color32::from_rgba_premultiplied(k(c.r()), k(c.g()), k(c.b()), d.a());
            }
        }
    }
}

impl Brushes {
    fn stamp(&mut self, shape: Shape, size: f64, patt: u8) -> Arc<Stamp> {
        let r = if size.is_finite() { js_round(size).clamp(1.0, MAX_STAMP as f64) as usize } else { 1 };
        let key = StampKey { shape, r, patt };
        if let Some(s) = self.stamps.get(&key) {
            return s.clone();
        }
        if self.stamps.len() > MAX_CACHED {
            self.stamps.clear();
        }
        let s = Arc::new(make_stamp(shape, r, patt));
        self.stamps.insert(key, s.clone());
        s
    }

    /// Render a whole stroke into `dst`, whose top-left sits at canvas position `org`.
    pub fn render(&mut self, s: &Stroke, frame: usize, wiggle: f64, dst: &mut Pixmap, org: (i32, i32)) -> IRect {
        let mut p = Progress::default();
        self.render_more(s, frame, wiggle, dst, org, &mut p, true)
    }

    /// Render the part of `s` that `prog` hasn't drawn yet. Blob fills only draw when `finish`
    /// is set (their outline isn't known until the pen lifts). Returns the touched area in
    /// `dst` pixels.
    #[allow(clippy::too_many_arguments)]
    pub fn render_more(&mut self, s: &Stroke, frame: usize, wiggle: f64, dst: &mut Pixmap, org: (i32, i32), prog: &mut Progress, finish: bool) -> IRect {
        let mode = if s.brush.erases() {
            Mode::Erase
        } else if s.lock {
            Mode::Atop
        } else {
            Mode::Over
        };
        let amp = s.brush.amp() * if wiggle.is_finite() { wiggle.clamp(0.0, 4.0) } else { 1.0 };
        let mut job = Job { s, frame: i64::try_from(frame).unwrap_or(0), amp, mode, dst, org, dirty: IRect::EMPTY };
        let len = s.pts.len();
        let size = if s.size.is_finite() { s.size.clamp(1.0, crate::model::MAX_SIZE) } else { 6.0 };
        let shape = if s.brush == crate::model::Brush::Nib { Shape::Nib } else { Shape::Tip(s.tip) };
        let patt = s.brush.pattern();
        match s.brush.kind() {
            Kind::Stamp => {
                let chalk = s.brush == crate::model::Brush::Chalk;
                let spacing = (size * 0.3).max(1.0);
                let mut w = prog.walk[0];
                let mut marks: Vec<(f64, f64, u32, usize)> = Vec::new();
                w.advance(len, &|i| job.wob(0, i), spacing, &mut |x, y, n, i| marks.push((x, y, n, i)));
                prog.walk[0] = w;
                for (x, y, n, i) in marks {
                    let st = self.stamp(shape, size * job.pressure(i), patt);
                    let grain = chalk.then(|| s.seed.wrapping_add(n.wrapping_mul(7919)).wrapping_add(u32::try_from(frame).unwrap_or(0).wrapping_mul(104_729)));
                    job.put(&st, x, y, grain);
                }
            }
            Kind::Sketch => {
                let sz = (size * 0.45).max(1.0);
                for pass in 0..3 {
                    let k = i64::from(u8::try_from(pass).unwrap_or(0)) * 977;
                    let mut w = prog.walk.get(pass).copied().unwrap_or_default();
                    let mut marks: Vec<(f64, f64, usize)> = Vec::new();
                    w.advance(len, &|i| job.wob(k, i), (sz * 0.4).max(1.0), &mut |x, y, _, i| marks.push((x, y, i)));
                    if let Some(slot) = prog.walk.get_mut(pass) {
                        *slot = w;
                    }
                    for (x, y, i) in marks {
                        let st = self.stamp(shape, sz * job.pressure(i), 0);
                        job.put(&st, x, y, None);
                    }
                }
            }
            Kind::Ribbon => {
                let mut w = prog.walk[0];
                let mut marks: Vec<(f64, f64, u32, usize)> = Vec::new();
                w.advance(len, &|i| job.wob(0, i), (size * 0.22).max(1.0), &mut |x, y, n, i| marks.push((x, y, n, i)));
                prog.walk[0] = w;
                for (x, y, n, i) in marks {
                    let sz = (size * (0.2 + 0.8 * (f64::from(n) * 0.22 + f64::from(s.seed)).sin().abs())).max(1.0);
                    let st = self.stamp(shape, sz * job.pressure(i), patt);
                    job.put(&st, x, y, None);
                }
            }
            Kind::Spray => {
                let dot = self.stamp(Shape::Tip(Tip::Square), js_round(size * 0.09).max(1.0), 0);
                let n = js_round(size * 0.8).max(3.0) as i64;
                let seed = i64::from(s.seed);
                for i in prog.next_pt..len {
                    let (px, py) = job.wob(0, i);
                    let ii = i64::try_from(i).unwrap_or(0);
                    let reach = size * job.pressure(i);
                    for d in 0..n {
                        let a = jr(seed + d * 31, ii, job.frame) * std::f64::consts::TAU;
                        let rr = jr(seed + d * 57, ii, job.frame + 7).sqrt() * reach;
                        job.put(&dot, px + a.cos() * rr, py + a.sin() * rr, None);
                    }
                }
                prog.next_pt = len;
            }
            Kind::Beads => {
                let seed = i64::from(s.seed);
                for i in prog.next_pt..len {
                    let (px, py) = job.wob(0, i);
                    let ii = i64::try_from(i).unwrap_or(0);
                    let sz = (size * (0.5 + 0.8 * jr(seed + 11, ii, job.frame))).max(1.0) * job.pressure(i);
                    let st = self.stamp(shape, sz, patt);
                    job.put(&st, px, py, None);
                }
                prog.next_pt = len;
            }
            Kind::Blob => {
                if finish && prog.next_pt < len {
                    let poly: Vec<(f64, f64)> = (0..len).map(|i| job.wob(0, i)).collect();
                    if poly.len() >= 3 {
                        let clip = IRect::full(job.dst.w, job.dst.h);
                        let clip = IRect::new(
                            clip.x0.saturating_add(org.0),
                            clip.y0.saturating_add(org.1),
                            clip.x1.saturating_add(org.0),
                            clip.y1.saturating_add(org.1),
                        );
                        let mut spans = Vec::new();
                        geom::fill_spans(&poly, clip, |y, a, b| spans.push((y, a, b)));
                        for (y, a, b) in spans {
                            job.span(y, a, b);
                        }
                    } else {
                        // Too short to enclose anything: leave a dot so the tap isn't lost.
                        let st = self.stamp(Shape::Tip(s.tip), size, 0);
                        for (x, y) in poly {
                            job.put(&st, x, y, None);
                        }
                    }
                    prog.next_pt = len;
                }
            }
        }
        job.dirty
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Brush, Pt};

    /// Values printed by the original JavaScript generator.
    #[test]
    fn noise_matches_the_original() {
        assert_eq!(jr(0, 0, 0), 0.26642920868471265);
        assert_eq!(jr(123_456_789, 7, 2), 0.7799772853031754);
        assert_eq!(jr(999_999_999 + 4177, 300, 1), 0.5306690616998821);
        assert_eq!(jr(5 + 977 * 2, 12, 0), 0.2461761669255793);
        assert_eq!(rnd(-5i32 as u32), 0.48384718922898173);
        assert_eq!(js_round(-1.5), -1.0);
        assert_eq!(js_round(2.5), 3.0);
    }

    fn stroke(brush: Brush, pts: Vec<Pt>) -> Stroke {
        Stroke { brush, tip: Tip::Round, color: Color32::BLACK, size: 6.0, seed: 42, lock: false, pts }
    }

    fn line() -> Vec<Pt> {
        (0..30).map(|i| Pt::new(10.0 + f64::from(i) * 3.0, 40.0 + f64::from(i % 5))).collect()
    }

    #[test]
    fn incremental_render_equals_full_render() {
        for brush in Brush::ALL {
            let s = stroke(brush, line());
            for frame in 0..3 {
                let mut b = Brushes::default();
                let mut full = Pixmap::new(128, 96);
                if brush.erases() {
                    full.px.fill(Color32::RED);
                }
                let mut inc = full.clone();
                b.render(&s, frame, 1.0, &mut full, (0, 0));
                let mut prog = Progress::default();
                for k in 1..=s.pts.len() {
                    let part = Stroke { pts: s.pts[..k].to_vec(), ..s.clone() };
                    b.render_more(&part, frame, 1.0, &mut inc, (0, 0), &mut prog, k == s.pts.len());
                }
                assert!(full == inc, "{brush:?} frame {frame}");
                assert!(!full.is_blank(), "{brush:?} drew nothing");
            }
        }
    }

    #[test]
    fn frames_differ_but_are_stable() {
        let s = stroke(Brush::Shaky, line());
        let mut b = Brushes::default();
        let mut f0 = Pixmap::new(128, 96);
        let mut f1 = Pixmap::new(128, 96);
        let mut again = Pixmap::new(128, 96);
        b.render(&s, 0, 1.0, &mut f0, (0, 0));
        b.render(&s, 1, 1.0, &mut f1, (0, 0));
        b.render(&s, 0, 1.0, &mut again, (0, 0));
        assert!(f0 != f1, "frames should boil");
        assert!(f0 == again, "a frame must be stable");
        // No wiggle: all frames identical.
        let mut s0 = Pixmap::new(128, 96);
        let mut s1 = Pixmap::new(128, 96);
        b.render(&s, 0, 0.0, &mut s0, (0, 0));
        b.render(&s, 1, 0.0, &mut s1, (0, 0));
        assert!(s0 == s1);
    }

    #[test]
    fn alpha_lock_only_recolours_existing_pixels() {
        let mut dst = Pixmap::new(64, 64);
        dst.set(20, 20, Color32::BLUE);
        let mut s = stroke(Brush::Marker, line());
        s.lock = true;
        s.color = Color32::RED;
        let mut b = Brushes::default();
        b.render(&s, 0, 1.0, &mut dst, (0, 0));
        let n = dst.px.iter().filter(|c| c.a() > 0).count();
        assert_eq!(n, 1);
    }

    #[test]
    fn origin_offsets_the_render() {
        let s = stroke(Brush::Steady, vec![Pt::new(50.0, 50.0)]);
        let mut b = Brushes::default();
        let mut a = Pixmap::new(100, 100);
        let mut c = Pixmap::new(20, 20);
        b.render(&s, 0, 1.0, &mut a, (0, 0));
        b.render(&s, 0, 1.0, &mut c, (40, 40));
        assert_eq!(a.get(50, 50), c.get(10, 10));
        assert!(c.get(10, 10).a() > 0);
    }

    #[test]
    fn hostile_strokes_do_not_panic() {
        let mut b = Brushes::default();
        let mut dst = Pixmap::new(32, 32);
        for brush in Brush::ALL {
            for size in [f64::NAN, f64::INFINITY, -5.0, 0.0, 1e9] {
                let pts = vec![Pt::new(f64::NAN, 1.0), Pt::new(1e7, -1e7), Pt { x: 3.0, y: 3.0, p: f32::NAN }, Pt::new(5.0, 5.0)];
                let s = Stroke { size, ..stroke(brush, pts) };
                b.render(&s, usize::MAX, f64::NAN, &mut dst, (i32::MIN, i32::MAX));
                b.render(&s, 1, 1.0, &mut dst, (0, 0));
            }
            b.render(&stroke(brush, vec![]), 0, 1.0, &mut dst, (0, 0));
        }
    }

    #[test]
    fn every_tip_has_ink() {
        for tip in Tip::ALL {
            for r in [1, 2, 3, 7, 40] {
                assert!(make_stamp(Shape::Tip(tip), r, 0).mask.iter().any(|m| *m), "{tip:?} {r}");
            }
        }
        let heart = make_stamp(Shape::Tip(Tip::Heart), 21, 0);
        let star = make_stamp(Shape::Tip(Tip::Star), 21, 0);
        let round = make_stamp(Shape::Tip(Tip::Round), 21, 0);
        let count = |s: &Stamp| s.mask.iter().filter(|m| **m).count();
        assert!(count(&star) < count(&round));
        assert!(count(&heart) < count(&round) && count(&heart) > 50);
    }
}
