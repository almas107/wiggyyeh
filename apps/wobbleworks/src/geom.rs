//! Path helpers: resampling, polygons and shape outlines.

use crate::model::Pt;
use crate::pixels::IRect;

/// Coordinates further outside the canvas than this are clamped (keeps walks finite).
pub const COORD_LIMIT: f64 = 40_000.0;

pub fn clamp_coord(v: f64) -> f64 {
    if v.is_finite() { v.clamp(-COORD_LIMIT, COORD_LIMIT) } else { 0.0 }
}

/// Streams raw pointer samples into evenly spaced points (`step` apart), the same spacing the
/// original Wobbleworks applied when a stroke ended. Doing it live means what you see while
/// drawing is exactly what gets kept.
#[derive(Clone, Debug)]
pub struct Resampler {
    step: f64,
    acc: f64,
    prev: Option<Pt>,
    last_out: Option<Pt>,
}

impl Resampler {
    pub fn new(step: f64) -> Self {
        Resampler { step: if step.is_finite() { step.max(0.5) } else { 1.0 }, acc: 0.0, prev: None, last_out: None }
    }

    /// Feed one raw sample; returns the points it produced.
    pub fn push(&mut self, p: Pt, out: &mut Vec<Pt>) {
        let Some(prev) = self.prev else {
            self.prev = Some(p);
            self.last_out = Some(p);
            out.push(p);
            return;
        };
        let (mut px, mut py, mut pp) = (prev.x, prev.y, prev.p);
        let (mut dx, mut dy) = (p.x - px, p.y - py);
        let mut d = dx.hypot(dy);
        if d == 0.0 || !d.is_finite() {
            return;
        }
        let mut guard = 0u32;
        while self.acc + d >= self.step && guard < 100_000 {
            guard += 1;
            let t = (self.step - self.acc) / d;
            px += dx * t;
            py += dy * t;
            pp += (p.p - pp) * t as f32;
            let q = Pt { x: px, y: py, p: pp };
            out.push(q);
            self.last_out = Some(q);
            dx = p.x - px;
            dy = p.y - py;
            d = dx.hypot(dy);
            self.acc = 0.0;
        }
        self.acc += d;
        self.prev = Some(p);
    }

    /// The pen lifted: keep the final sample unless it sits on top of the last kept point.
    pub fn finish(&mut self, out: &mut Vec<Pt>) {
        if let (Some(prev), Some(last)) = (self.prev, self.last_out)
            && (prev.x - last.x).hypot(prev.y - last.y) > self.step * 0.4
        {
            out.push(prev);
            self.last_out = Some(prev);
        }
    }
}

/// Resample a whole path at once.
pub fn resample(pts: &[Pt], step: f64) -> Vec<Pt> {
    let mut r = Resampler::new(step);
    let mut out = Vec::new();
    for p in pts {
        r.push(*p, &mut out);
    }
    r.finish(&mut out);
    out
}

/// Even-odd point in polygon test.
pub fn in_poly(x: f64, y: f64, poly: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (Some(&(xi, yi)), Some(&(xj, yj))) = (poly.get(i), poly.get(j)) else { break };
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

pub fn bbox(poly: &[(f64, f64)]) -> IRect {
    let mut r = IRect::EMPTY;
    for &(x, y) in poly {
        if !(x.is_finite() && y.is_finite()) {
            continue;
        }
        r = r.union(IRect::new(x.floor() as i32, y.floor() as i32, x.floor() as i32 + 1, y.floor() as i32 + 1));
    }
    r
}

/// Calls `span(y, x0, x1)` (x1 exclusive) for every run of pixels whose centres are inside the
/// polygon (even-odd), limited to `clip`.
pub fn fill_spans(poly: &[(f64, f64)], clip: IRect, mut span: impl FnMut(i32, i32, i32)) {
    if poly.len() < 3 {
        return;
    }
    let r = bbox(poly).intersect(clip);
    if r.is_empty() {
        return;
    }
    let mut xs: Vec<f64> = Vec::with_capacity(16);
    for y in r.y0..r.y1 {
        let cy = f64::from(y) + 0.5;
        xs.clear();
        let n = poly.len();
        let mut j = n - 1;
        for i in 0..n {
            if let (Some(&(xi, yi)), Some(&(xj, yj))) = (poly.get(i), poly.get(j))
                && (yi > cy) != (yj > cy)
            {
                xs.push((xj - xi) * (cy - yi) / (yj - yi) + xi);
            }
            j = i;
        }
        xs.sort_by(f64::total_cmp);
        for pair in xs.chunks_exact(2) {
            let &[a, b] = pair else { continue };
            // Pixel x is inside when its centre x+0.5 lies in [a, b).
            let x0 = ((a - 0.5).ceil() as i32).max(r.x0);
            let x1 = (((b - 0.5).ceil()) as i32).min(r.x1);
            if x1 > x0 {
                span(y, x0, x1);
            }
        }
    }
}

/// Outline points for the shape tools. `closed` shapes repeat their first point at the end.
pub fn line_pts(a: (f64, f64), b: (f64, f64)) -> Vec<Pt> {
    vec![Pt::new(a.0, a.1), Pt::new(b.0, b.1)]
}

pub fn rect_pts(a: (f64, f64), b: (f64, f64)) -> Vec<Pt> {
    let (x0, y0, x1, y1) = (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1));
    vec![Pt::new(x0, y0), Pt::new(x1, y0), Pt::new(x1, y1), Pt::new(x0, y1), Pt::new(x0, y0)]
}

pub fn ellipse_pts(a: (f64, f64), b: (f64, f64)) -> Vec<Pt> {
    let (cx, cy) = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let (rx, ry) = ((a.0 - b.0).abs() / 2.0, (a.1 - b.1).abs() / 2.0);
    let n = ((rx + ry) * 0.6).clamp(12.0, 720.0) as usize;
    (0..=n)
        .map(|i| {
            let t = i as f64 / n as f64 * std::f64::consts::TAU;
            Pt::new(cx + rx * t.cos(), cy - ry * t.sin())
        })
        .collect()
}

/// Constrain a drag for Shift: 45° lines, squares and circles.
pub fn constrain(a: (f64, f64), b: (f64, f64), line: bool) -> (f64, f64) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    if line {
        let ang = dy.atan2(dx);
        let snap = (ang / (std::f64::consts::PI / 4.0)).round() * (std::f64::consts::PI / 4.0);
        let len = dx.hypot(dy);
        (a.0 + len * snap.cos(), a.1 + len * snap.sin())
    } else {
        let m = dx.abs().max(dy.abs());
        (a.0 + m * dx.signum(), a.1 + m * dy.signum())
    }
}

/// Symmetry modes for drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Mirror {
    #[default]
    Off,
    Horizontal,
    Vertical,
    Quad,
    Radial6,
}

impl Mirror {
    pub const ALL: [Mirror; 5] = [Mirror::Off, Mirror::Horizontal, Mirror::Vertical, Mirror::Quad, Mirror::Radial6];

    pub fn label(self) -> &'static str {
        match self {
            Mirror::Off => "Off",
            Mirror::Horizontal => "Left ↔ right",
            Mirror::Vertical => "Top ↕ bottom",
            Mirror::Quad => "Four-way",
            Mirror::Radial6 => "Kaleidoscope",
        }
    }

    /// Copies of a point (the original first) for a `w`×`h` canvas.
    pub fn images(self, p: Pt, w: f64, h: f64) -> Vec<Pt> {
        let (cx, cy) = (w / 2.0, h / 2.0);
        let fx = Pt { x: w - p.x, ..p };
        let fy = Pt { y: h - p.y, ..p };
        match self {
            Mirror::Off => vec![p],
            Mirror::Horizontal => vec![p, fx],
            Mirror::Vertical => vec![p, fy],
            Mirror::Quad => vec![p, fx, fy, Pt { x: w - p.x, y: h - p.y, ..p }],
            Mirror::Radial6 => (0..6)
                .map(|k| {
                    let a = f64::from(k) * std::f64::consts::TAU / 6.0;
                    let (dx, dy) = (p.x - cx, p.y - cy);
                    Pt { x: cx + dx * a.cos() - dy * a.sin(), y: cy + dx * a.sin() + dy * a.cos(), p: p.p }
                })
                .collect(),
        }
    }

    pub fn count(self) -> usize {
        self.images(Pt::new(0.0, 0.0), 1.0, 1.0).len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_resample_matches_spacing() {
        let raw: Vec<Pt> = (0..=100).map(|i| Pt::new(f64::from(i) * 0.37, 0.0)).collect();
        let out = resample(&raw, 5.0);
        assert!(out.len() >= 7);
        for w in out.windows(2).take(out.len() - 2) {
            assert!(((w[1].x - w[0].x) - 5.0).abs() < 1e-9);
        }
        // The 2px tail is under 0.4 steps, so the last kept point is the 35px one.
        assert!((out.last().unwrap().x - 35.0).abs() < 1e-9);
    }

    #[test]
    fn resample_survives_garbage() {
        let raw = [Pt::new(f64::NAN, 0.0), Pt::new(f64::INFINITY, 1.0), Pt::new(0.0, 0.0)];
        let _ = resample(&raw, 0.0);
        let _ = resample(&raw, f64::NAN);
        let _ = resample(&[], 3.0);
    }

    #[test]
    fn fill_spans_cover_a_square() {
        let sq = [(2.0, 2.0), (6.0, 2.0), (6.0, 6.0), (2.0, 6.0)];
        let mut n = 0;
        fill_spans(&sq, IRect::new(0, 0, 100, 100), |_, a, b| n += b - a);
        assert_eq!(n, 16);
        let mut clipped = 0;
        fill_spans(&sq, IRect::new(0, 0, 4, 4), |_, a, b| clipped += b - a);
        assert_eq!(clipped, 4);
        assert!(in_poly(3.0, 3.0, &sq));
        assert!(!in_poly(7.0, 3.0, &sq));
        fill_spans(&[(0.0, 0.0)], IRect::new(0, 0, 9, 9), |_, _, _| panic!("degenerate polygon"));
    }

    #[test]
    fn mirror_images() {
        let p = Pt::new(10.0, 20.0);
        assert_eq!(Mirror::Off.images(p, 100.0, 100.0).len(), 1);
        let h = Mirror::Horizontal.images(p, 100.0, 100.0);
        assert_eq!(h[1].x, 90.0);
        assert_eq!(Mirror::Radial6.count(), 6);
        assert_eq!(Mirror::Quad.images(p, 100.0, 50.0)[3].y, 30.0);
    }

    #[test]
    fn constrain_snaps() {
        let (x, y) = constrain((0.0, 0.0), (10.0, 9.0), true);
        assert!((x - y).abs() < 1e-9);
        assert_eq!(constrain((0.0, 0.0), (10.0, -3.0), false), (10.0, -10.0));
    }
}
