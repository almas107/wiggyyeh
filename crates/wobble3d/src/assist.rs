//! Drawing aids: Stable Stroke (a lazy-string stabiliser), Draw Shape (straight line, smooth
//! curve or perfect circle, adjustable by holding) and Mirror (X, Y, Z in any combination).
//!
//! Draw Shape only corrects in its own tool: plain Draw never turns a deliberate line into
//! something else (App Store feedback on Feather: smoothing fought straight lines).

use crate::math::{Vec3, v3};

/// Stable Stroke: the drawn point trails the pen on a string; longer string = smoother.
#[derive(Debug, Clone, Default)]
pub struct Stabilizer {
    /// 0..1 (string length up to MAX_STRING px).
    pub amount: f32,
    pos: Option<[f32; 3]>,
}

pub const MAX_STRING: f32 = 48.0;

impl Stabilizer {
    pub fn new(amount: f32) -> Self {
        Stabilizer { amount: if amount.is_finite() { amount.clamp(0.0, 1.0) } else { 0.0 }, pos: None }
    }

    /// Feed a raw sample (x, y, pressure); returns the stabilised sample, if the string moved.
    pub fn push(&mut self, x: f32, y: f32, p: f32) -> Option<[f32; 3]> {
        if !(x.is_finite() && y.is_finite()) {
            return None;
        }
        let p = if p.is_finite() { p.clamp(0.0, 1.0) } else { 1.0 };
        let len = self.amount * MAX_STRING;
        match self.pos {
            None => {
                self.pos = Some([x, y, p]);
                self.pos
            }
            Some(cur) => {
                let (dx, dy) = (x - cur[0], y - cur[1]);
                let d = (dx * dx + dy * dy).sqrt();
                if d <= len || d < 1e-6 {
                    return None;
                }
                let k = (d - len) / d;
                let next = [cur[0] + dx * k, cur[1] + dy * k, cur[2] + (p - cur[2]) * k.max(0.3)];
                self.pos = Some(next);
                Some(next)
            }
        }
    }

    /// Where the string ends when the pen lifts: the remaining straight run to the pen.
    pub fn finish(&mut self, x: f32, y: f32, p: f32) -> Vec<[f32; 3]> {
        let Some(cur) = self.pos.take() else { return Vec::new() };
        if !(x.is_finite() && y.is_finite()) {
            return Vec::new();
        }
        let (dx, dy) = (x - cur[0], y - cur[1]);
        let d = (dx * dx + dy * dy).sqrt();
        let steps = (d / 4.0).ceil().clamp(0.0, 64.0) as usize;
        (1..=steps).map(|i| {
            let t = i as f32 / steps as f32;
            [cur[0] + dx * t, cur[1] + dy * t, cur[2] + (p - cur[2]) * t]
        })
        .collect()
    }
}

/// A corrected shape on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Line { a: [f32; 2], b: [f32; 2] },
    /// A smooth curve: quadratic Bézier from `a` to `b` through `mid` at its middle.
    Curve { a: [f32; 2], mid: [f32; 2], b: [f32; 2] },
    Circle { c: [f32; 2], r: f32, start: f32 },
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn seg_dist(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (abx, aby) = (b[0] - a[0], b[1] - a[1]);
    let l2 = abx * abx + aby * aby;
    if l2 < 1e-12 {
        return dist(p, a);
    }
    let t = (((p[0] - a[0]) * abx + (p[1] - a[1]) * aby) / l2).clamp(0.0, 1.0);
    dist(p, [a[0] + abx * t, a[1] + aby * t])
}

/// Least-squares circle (Kåsa). Returns (centre, radius).
fn fit_circle(pts: &[[f32; 2]]) -> Option<([f32; 2], f32)> {
    let n = pts.len() as f64;
    if n < 5.0 {
        return None;
    }
    let (mx, my) = (pts.iter().map(|p| p[0] as f64).sum::<f64>() / n, pts.iter().map(|p| p[1] as f64).sum::<f64>() / n);
    let (mut suu, mut svv, mut suv, mut suuu, mut svvv, mut suvv, mut svuu) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for p in pts {
        let (u, v) = (p[0] as f64 - mx, p[1] as f64 - my);
        suu += u * u;
        svv += v * v;
        suv += u * v;
        suuu += u * u * u;
        svvv += v * v * v;
        suvv += u * v * v;
        svuu += v * u * u;
    }
    let det = suu * svv - suv * suv;
    if det.abs() < 1e-9 {
        return None;
    }
    let (r1, r2) = (0.5 * (suuu + suvv), 0.5 * (svvv + svuu));
    let uc = (r1 * svv - r2 * suv) / det;
    let vc = (r2 * suu - r1 * suv) / det;
    let r = (uc * uc + vc * vc + (suu + svv) / n).sqrt();
    let c = [(uc + mx) as f32, (vc + my) as f32];
    (r.is_finite() && r > 0.5).then_some((c, r as f32))
}

/// Pick the shape a rough stroke is closest to.
pub fn recognize(raw: &[[f32; 2]]) -> Option<Shape> {
    let pts: Vec<[f32; 2]> = raw.iter().copied().filter(|p| p[0].is_finite() && p[1].is_finite()).collect();
    let (a, b) = (*pts.first()?, *pts.last()?);
    let length: f32 = pts.windows(2).map(|w| dist(w[0], w[1])).sum();
    if length < 2.0 {
        return None;
    }
    let chord = dist(a, b);
    let dev = pts.iter().map(|p| seg_dist(*p, a, b)).fold(0.0f32, f32::max);
    if chord > 1.0 && dev <= (chord * 0.06).max(4.0) {
        return Some(Shape::Line { a, b });
    }
    if chord < length * 0.2
        && let Some((c, r)) = fit_circle(&pts)
    {
        let resid = pts.iter().map(|p| (dist(*p, c) - r).abs()).sum::<f32>() / pts.len() as f32;
        let turn = length / (std::f32::consts::TAU * r);
        if resid < r * 0.14 && (0.75..1.5).contains(&turn) {
            let start = (a[1] - c[1]).atan2(a[0] - c[0]);
            return Some(Shape::Circle { c, r, start });
        }
    }
    // Smooth curve through the middle of the stroke.
    let mut acc = 0.0;
    let mut mid = a;
    for w in pts.windows(2) {
        let d = dist(w[0], w[1]);
        if acc + d >= length * 0.5 {
            let t = if d > 0.0 { (length * 0.5 - acc) / d } else { 0.0 };
            mid = [w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t];
            break;
        }
        acc += d;
    }
    Some(Shape::Curve { a, mid, b })
}

impl Shape {
    /// Hold-to-adjust: a line's end follows the pen, a curve bends through it, a circle's size
    /// follows its distance from the centre.
    pub fn adjust(&mut self, pen: [f32; 2]) {
        if !(pen[0].is_finite() && pen[1].is_finite()) {
            return;
        }
        match self {
            Shape::Line { b, .. } => *b = pen,
            Shape::Curve { mid, .. } => *mid = pen,
            Shape::Circle { c, r, .. } => *r = dist(*c, pen).max(1.0),
        }
    }

    /// Points along the shape (about one every 3 pixels).
    pub fn points(&self) -> Vec<[f32; 2]> {
        match *self {
            Shape::Line { a, b } => {
                let n = ((dist(a, b) / 3.0).ceil() as usize).clamp(1, 4000);
                (0..=n).map(|i| {
                    let t = i as f32 / n as f32;
                    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
                })
                .collect()
            }
            Shape::Curve { a, mid, b } => {
                // Control point so the curve passes through `mid` at t = 0.5.
                let c = [2.0 * mid[0] - 0.5 * (a[0] + b[0]), 2.0 * mid[1] - 0.5 * (a[1] + b[1])];
                let n = (((dist(a, c) + dist(c, b)) / 3.0).ceil() as usize).clamp(2, 4000);
                (0..=n).map(|i| {
                    let t = i as f32 / n as f32;
                    let u = 1.0 - t;
                    [u * u * a[0] + 2.0 * u * t * c[0] + t * t * b[0], u * u * a[1] + 2.0 * u * t * c[1] + t * t * b[1]]
                })
                .collect()
            }
            Shape::Circle { c, r, start } => {
                let n = ((std::f32::consts::TAU * r / 3.0).ceil() as usize).clamp(12, 4000);
                (0..=n).map(|i| {
                    let a = start + std::f32::consts::TAU * i as f32 / n as f32;
                    [c[0] + r * a.cos(), c[1] + r * a.sin()]
                })
                .collect()
            }
        }
    }
}

/// Reflect across the planes of the chosen world axes (X: x → -x, …), every combination.
pub fn mirror_maps(axes: [bool; 3]) -> Vec<[f32; 3]> {
    let mut out = Vec::new();
    for mask in 1u8..8 {
        if (0..3).all(|k| mask & (1 << k) == 0 || axes[k]) {
            out.push([
                if mask & 1 != 0 { -1.0 } else { 1.0 },
                if mask & 2 != 0 { -1.0 } else { 1.0 },
                if mask & 4 != 0 { -1.0 } else { 1.0 },
            ]);
        }
    }
    out
}

pub fn reflect(p: Vec3, m: [f32; 3]) -> Vec3 {
    v3(p.x * m[0], p.y * m[1], p.z * m[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stabiliser_lags_then_catches_up() {
        let mut s = Stabilizer::new(0.5);
        assert!(s.push(0.0, 0.0, 1.0).is_some());
        assert!(s.push(10.0, 0.0, 1.0).is_none(), "inside the string");
        let p = s.push(100.0, 0.0, 1.0).expect("pulled");
        assert!((p[0] - (100.0 - MAX_STRING * 0.5)).abs() < 1e-3);
        let tail = s.finish(100.0, 0.0, 1.0);
        assert!(tail.last().is_some_and(|t| (t[0] - 100.0).abs() < 1e-3));
        assert!(Stabilizer::new(f32::NAN).push(f32::NAN, 0.0, 1.0).is_none());
    }

    #[test]
    fn shapes_are_recognised() {
        let line: Vec<[f32; 2]> = (0..50).map(|i| [i as f32 * 4.0, 100.0 + (i % 3) as f32]).collect();
        assert!(matches!(recognize(&line), Some(Shape::Line { .. })));
        let circle: Vec<[f32; 2]> = (0..=60).map(|i| {
            let a = i as f32 / 60.0 * std::f32::consts::TAU;
            [200.0 + 80.0 * a.cos() + (i % 2) as f32, 200.0 + 80.0 * a.sin()]
        })
        .collect();
        match recognize(&circle) {
            Some(Shape::Circle { c, r, .. }) => assert!((r - 80.0).abs() < 4.0 && dist(c, [200.0, 200.0]) < 4.0),
            other => panic!("{other:?}"),
        }
        let arc: Vec<[f32; 2]> = (0..=40).map(|i| {
            let t = i as f32 / 40.0;
            [t * 300.0, 200.0 - (t * std::f32::consts::PI).sin() * 90.0]
        })
        .collect();
        let mut s = recognize(&arc).expect("curve");
        assert!(matches!(s, Shape::Curve { .. }));
        let pts = s.points();
        assert!(pts.first().is_some_and(|p| dist(*p, [0.0, 200.0]) < 1e-3));
        s.adjust([150.0, 50.0]);
        let mid = s.points()[s.points().len() / 2];
        assert!(dist(mid, [150.0, 50.0]) < 6.0);
        assert!(recognize(&[]).is_none());
        assert!(recognize(&[[1.0, 1.0], [1.0, 1.0]]).is_none());
    }

    #[test]
    fn mirror_covers_every_combination() {
        assert_eq!(mirror_maps([true, false, false]), vec![[-1.0, 1.0, 1.0]]);
        assert_eq!(mirror_maps([true, true, false]).len(), 3);
        assert_eq!(mirror_maps([true, true, true]).len(), 7);
        assert!(mirror_maps([false; 3]).is_empty());
    }
}
