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
        (1..=steps)
            .map(|i| {
                let t = i as f32 / steps as f32;
                [cur[0] + dx * t, cur[1] + dy * t, cur[2] + (p - cur[2]) * t]
            })
            .collect()
    }
}

/// A corrected shape on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Line {
        a: [f32; 2],
        b: [f32; 2],
    },
    /// A smooth curve: quadratic Bézier from `a` to `b` through `mid` at its middle.
    Curve {
        a: [f32; 2],
        mid: [f32; 2],
        b: [f32; 2],
    },
    Circle {
        c: [f32; 2],
        r: f32,
        start: f32,
    },
    /// An ellipse: radii along its own axes, turned by `rot` (radians).
    Ellipse {
        c: [f32; 2],
        rx: f32,
        ry: f32,
        rot: f32,
        start: f32,
    },
    /// Straight sides through corners: a rectangle (square-cornered, snapped level when close),
    /// a triangle or any polygon, or an open zigzag. `base` is the shape as recognised and
    /// `grab` where the pen rested, so holding and dragging scales it about its middle.
    Poly {
        pts: [[f32; 2]; POLY_MAX],
        base: [[f32; 2]; POLY_MAX],
        n: u8,
        closed: bool,
        grab: [f32; 2],
    },
}

/// Most corners a recognised polygon keeps.
pub const POLY_MAX: usize = 8;

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
    let closed = chord < length * 0.15;
    if let Some(poly) = polygon(&pts, length, closed) {
        return Some(poly);
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
        if let Some(e) = ellipse(&pts) {
            return Some(e);
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

/// Corners of a stroke: where its direction turns sharply (over about 12 px each side),
/// strongest first, at most `POLY_MAX`.
fn corners(pts: &[[f32; 2]], closed: bool) -> Vec<usize> {
    // Resample every 3 px so turning is measured over distance, not over pointer samples.
    let mut rs: Vec<[f32; 2]> = Vec::new();
    for p in pts {
        if rs.last().is_none_or(|l| dist(*l, *p) >= 3.0) {
            rs.push(*p);
        }
    }
    let n = rs.len();
    if n < 8 {
        return Vec::new();
    }
    const K: usize = 4;
    let angle = |i: usize| -> f32 {
        let (pa, pb) = if closed { (rs[(i + n - K) % n], rs[(i + K) % n]) } else { (rs[i.saturating_sub(K)], rs[(i + K).min(n - 1)]) };
        let p = rs[i];
        let (ax, ay, bx, by) = (p[0] - pa[0], p[1] - pa[1], pb[0] - p[0], pb[1] - p[1]);
        let (la, lb) = ((ax * ax + ay * ay).sqrt(), (bx * bx + by * by).sqrt());
        if la < 1e-3 || lb < 1e-3 {
            return 0.0;
        }
        ((ax * bx + ay * by) / (la * lb)).clamp(-1.0, 1.0).acos()
    };
    let turn: Vec<f32> = (0..n).map(angle).collect();
    let mut found: Vec<(f32, usize)> = Vec::new();
    let range = if closed { 0..n } else { K..n.saturating_sub(K) };
    for i in range {
        let t = turn[i];
        if t < 0.75 {
            continue;
        }
        let peak = (1..=K).all(|k| {
            let l = if closed { turn[(i + n - k) % n] } else { turn.get(i.wrapping_sub(k)).copied().unwrap_or(0.0) };
            let r = if closed { turn[(i + k) % n] } else { turn.get(i + k).copied().unwrap_or(0.0) };
            t >= l && t > r
        });
        if peak {
            found.push((t, i));
        }
    }
    found.sort_by(|a, b| b.0.total_cmp(&a.0));
    found.truncate(POLY_MAX);
    let mut idx: Vec<usize> = found.into_iter().map(|(_, i)| i).collect();
    idx.sort_unstable();
    // Map back to the caller's points (nearest original sample).
    idx.into_iter()
        .filter_map(|i| {
            let q = rs[i];
            pts.iter().enumerate().min_by(|a, b| dist(*a.1, q).total_cmp(&dist(*b.1, q))).map(|(j, _)| j)
        })
        .collect()
}

/// A polygon through a stroke's corners, when every side between them is nearly straight.
fn polygon(pts: &[[f32; 2]], length: f32, closed: bool) -> Option<Shape> {
    let cs = corners(pts, closed);
    let (a, b) = (*pts.first()?, *pts.last()?);
    // Closed: the corners (the start is just somewhere on a side). Open: start, corners, end.
    let mut ix: Vec<usize> = Vec::new();
    if !closed {
        ix.push(0);
    }
    ix.extend(cs.iter().copied().filter(|&i| closed || (i > 0 && i + 1 < pts.len())));
    if !closed {
        ix.push(pts.len() - 1);
    }
    let corners_n = if closed { ix.len() } else { ix.len().saturating_sub(2) };
    if (closed && !(3..=POLY_MAX).contains(&ix.len())) || (!closed && (corners_n == 0 || ix.len() > POLY_MAX)) {
        return None;
    }
    let mut verts: Vec<[f32; 2]> = ix.iter().filter_map(|&i| pts.get(i).copied()).collect();
    // Every point lies near a side, and sides are straight for their length (a smooth wave
    // between its crests is not a zigzag).
    let _ = length;
    let m = verts.len();
    let side_count = if closed { m } else { m - 1 };
    let near_side = |p: [f32; 2]| {
        (0..side_count).any(|k| {
            let (a, b) = (verts[k], verts[(k + 1) % m]);
            seg_dist(p, a, b) <= (dist(a, b) * 0.045).max(3.5)
        })
    };
    if !pts.iter().all(|p| near_side(*p)) {
        return None;
    }
    if closed && m == 4 {
        verts = square_up(&verts);
    }
    let _ = (a, b);
    let mut arr = [[0.0f32; 2]; POLY_MAX];
    for (slot, v) in arr.iter_mut().zip(&verts) {
        *slot = *v;
    }
    Some(Shape::Poly { pts: arr, base: arr, n: verts.len() as u8, closed, grab: b })
}

/// Four rough corners → a true rectangle (right angles), level with the screen when it was
/// drawn within 6° of level.
fn square_up(v: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let n = v.len();
    // The rectangle's angle: side directions folded into one quarter turn (×4 trick).
    let (mut sx, mut sy) = (0.0f32, 0.0f32);
    for k in 0..n {
        let (p, q) = (v[k], v[(k + 1) % n]);
        let a = (q[1] - p[1]).atan2(q[0] - p[0]);
        let w = dist(p, q);
        sx += (4.0 * a).cos() * w;
        sy += (4.0 * a).sin() * w;
    }
    let mut theta = sy.atan2(sx) / 4.0;
    if theta.abs() < 6f32.to_radians() {
        theta = 0.0;
    }
    let (u, w) = ([theta.cos(), theta.sin()], [-theta.sin(), theta.cos()]);
    let c = [v.iter().map(|p| p[0]).sum::<f32>() / n as f32, v.iter().map(|p| p[1]).sum::<f32>() / n as f32];
    let pu: Vec<f32> = v.iter().map(|p| (p[0] - c[0]) * u[0] + (p[1] - c[1]) * u[1]).collect();
    let pw: Vec<f32> = v.iter().map(|p| (p[0] - c[0]) * w[0] + (p[1] - c[1]) * w[1]).collect();
    // Half sizes: the average reach of the corners on each axis.
    let hu = pu.iter().map(|x| x.abs()).sum::<f32>() / n as f32;
    let hw = pw.iter().map(|x| x.abs()).sum::<f32>() / n as f32;
    v.iter()
        .zip(pu.iter().zip(&pw))
        .map(|(_, (a, b))| {
            let (su, sw) = (hu * a.signum(), hw * b.signum());
            [c[0] + u[0] * su + w[0] * sw, c[1] + u[1] * su + w[1] * sw]
        })
        .collect()
}

/// An ellipse through a closed stroke (principal axes of its points), when it fits well.
fn ellipse(pts: &[[f32; 2]]) -> Option<Shape> {
    let n = pts.len() as f32;
    if n < 8.0 {
        return None;
    }
    let c = [pts.iter().map(|p| p[0]).sum::<f32>() / n, pts.iter().map(|p| p[1]).sum::<f32>() / n];
    let (mut sxx, mut syy, mut sxy) = (0.0f32, 0.0f32, 0.0f32);
    for p in pts {
        let (x, y) = (p[0] - c[0], p[1] - c[1]);
        sxx += x * x;
        syy += y * y;
        sxy += x * y;
    }
    let rot = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    let (cs, sn) = (rot.cos(), rot.sin());
    let local = |p: [f32; 2]| {
        let (x, y) = (p[0] - c[0], p[1] - c[1]);
        (x * cs + y * sn, -x * sn + y * cs)
    };
    // Radii from the spread along each axis (a uniform ellipse has variance r² / 2).
    let (mut vx, mut vy) = (0.0f32, 0.0f32);
    for p in pts {
        let (x, y) = local(*p);
        vx += x * x;
        vy += y * y;
    }
    let (rx, ry) = ((2.0 * vx / n).sqrt(), (2.0 * vy / n).sqrt());
    if !(rx.is_finite() && ry.is_finite()) || rx < 2.0 || ry < 2.0 {
        return None;
    }
    let resid = pts
        .iter()
        .map(|p| {
            let (x, y) = local(*p);
            ((x / rx).powi(2) + (y / ry).powi(2)).sqrt() - 1.0
        })
        .map(f32::abs)
        .sum::<f32>()
        / n;
    if resid > 0.12 {
        return None;
    }
    let a = pts[0];
    let (x, y) = local(a);
    let start = (y / ry).atan2(x / rx);
    Some(Shape::Ellipse { c, rx, ry, rot, start })
}

impl Shape {
    /// Hold-to-adjust: a line's end follows the pen, a curve bends through it, a circle's size
    /// follows its distance from the centre, an ellipse and a polygon grow or shrink about their
    /// middle as the pen moves away from or towards it.
    pub fn adjust(&mut self, pen: [f32; 2]) {
        if !(pen[0].is_finite() && pen[1].is_finite()) {
            return;
        }
        match self {
            Shape::Line { b, .. } => *b = pen,
            Shape::Curve { mid, .. } => *mid = pen,
            Shape::Circle { c, r, .. } => *r = dist(*c, pen).max(1.0),
            Shape::Ellipse { c, rx, ry, .. } => {
                let k = dist(*c, pen).max(1.0) / rx.max(*ry).max(1.0);
                *rx = (*rx * k).max(1.0);
                *ry = (*ry * k).max(1.0);
            }
            Shape::Poly { pts, base, n, grab, .. } => {
                let m = usize::from(*n).min(POLY_MAX).max(1);
                let c = [base[..m].iter().map(|p| p[0]).sum::<f32>() / m as f32, base[..m].iter().map(|p| p[1]).sum::<f32>() / m as f32];
                let d0 = dist(c, *grab);
                if d0 < 1.0 {
                    return;
                }
                let k = (dist(c, pen) / d0).clamp(0.05, 50.0);
                for (p, b) in pts.iter_mut().zip(base.iter()).take(m) {
                    *p = [c[0] + (b[0] - c[0]) * k, c[1] + (b[1] - c[1]) * k];
                }
            }
        }
    }

    /// Which kind of shape, for the status line.
    pub fn name(&self) -> &'static str {
        match self {
            Shape::Line { .. } => "line",
            Shape::Curve { .. } => "curve",
            Shape::Circle { .. } => "circle",
            Shape::Ellipse { .. } => "ellipse",
            Shape::Poly { n: 4, closed: true, .. } => "rectangle",
            Shape::Poly { n: 3, closed: true, .. } => "triangle",
            Shape::Poly { closed: true, .. } => "polygon",
            Shape::Poly { .. } => "straight lines",
        }
    }

    /// Points along the shape (about one every 3 pixels).
    pub fn points(&self) -> Vec<[f32; 2]> {
        match *self {
            Shape::Line { a, b } => {
                let n = ((dist(a, b) / 3.0).ceil() as usize).clamp(1, 4000);
                (0..=n)
                    .map(|i| {
                        let t = i as f32 / n as f32;
                        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
                    })
                    .collect()
            }
            Shape::Curve { a, mid, b } => {
                // Control point so the curve passes through `mid` at t = 0.5.
                let c = [2.0 * mid[0] - 0.5 * (a[0] + b[0]), 2.0 * mid[1] - 0.5 * (a[1] + b[1])];
                let n = (((dist(a, c) + dist(c, b)) / 3.0).ceil() as usize).clamp(2, 4000);
                (0..=n)
                    .map(|i| {
                        let t = i as f32 / n as f32;
                        let u = 1.0 - t;
                        [u * u * a[0] + 2.0 * u * t * c[0] + t * t * b[0], u * u * a[1] + 2.0 * u * t * c[1] + t * t * b[1]]
                    })
                    .collect()
            }
            Shape::Circle { c, r, start } => {
                let n = ((std::f32::consts::TAU * r / 3.0).ceil() as usize).clamp(12, 4000);
                (0..=n)
                    .map(|i| {
                        let a = start + std::f32::consts::TAU * i as f32 / n as f32;
                        [c[0] + r * a.cos(), c[1] + r * a.sin()]
                    })
                    .collect()
            }
            Shape::Ellipse { c, rx, ry, rot, start } => {
                let n = ((std::f32::consts::TAU * rx.max(ry) / 3.0).ceil() as usize).clamp(12, 4000);
                let (cs, sn) = (rot.cos(), rot.sin());
                (0..=n)
                    .map(|i| {
                        let a = start + std::f32::consts::TAU * i as f32 / n as f32;
                        let (x, y) = (rx * a.cos(), ry * a.sin());
                        [c[0] + x * cs - y * sn, c[1] + x * sn + y * cs]
                    })
                    .collect()
            }
            Shape::Poly { pts, n, closed, .. } => {
                let m = usize::from(n).min(POLY_MAX);
                let mut out = Vec::new();
                let sides = if closed { m } else { m.saturating_sub(1) };
                for k in 0..sides {
                    let (a, b) = (pts[k], pts[(k + 1) % m]);
                    let steps = ((dist(a, b) / 3.0).ceil() as usize).clamp(1, 4000);
                    for i in 0..steps {
                        let t = i as f32 / steps as f32;
                        out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
                    }
                }
                if let Some(last) = if closed { pts.first() } else { pts.get(m.saturating_sub(1)) } {
                    out.push(*last);
                }
                out
            }
        }
    }
}

/// Reflect across the planes of the chosen world axes (X: x → -x, …), every combination.
pub fn mirror_maps(axes: [bool; 3]) -> Vec<[f32; 3]> {
    let mut out = Vec::new();
    for mask in 1u8..8 {
        if (0..3).all(|k| mask & (1 << k) == 0 || axes[k]) {
            out.push([if mask & 1 != 0 { -1.0 } else { 1.0 }, if mask & 2 != 0 { -1.0 } else { 1.0 }, if mask & 4 != 0 { -1.0 } else { 1.0 }]);
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
        let circle: Vec<[f32; 2]> = (0..=60)
            .map(|i| {
                let a = i as f32 / 60.0 * std::f32::consts::TAU;
                [200.0 + 80.0 * a.cos() + (i % 2) as f32, 200.0 + 80.0 * a.sin()]
            })
            .collect();
        match recognize(&circle) {
            Some(Shape::Circle { c, r, .. }) => assert!((r - 80.0).abs() < 4.0 && dist(c, [200.0, 200.0]) < 4.0),
            other => panic!("{other:?}"),
        }
        let arc: Vec<[f32; 2]> = (0..=40)
            .map(|i| {
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

    /// A wobbly hand-drawn rectangle becomes a clean one: square corners, level, each corner
    /// within a few pixels of where it was drawn.
    #[test]
    fn rough_rectangles_triangles_and_ovals_are_cleaned_up() {
        let corners = [[100.0, 100.0], [300.0, 104.0], [302.0, 220.0], [98.0, 216.0]];
        let mut pts = Vec::new();
        for k in 0..4 {
            let (a, b): ([f32; 2], [f32; 2]) = (corners[k], corners[(k + 1) % 4]);
            for i in 0..30 {
                let t = i as f32 / 30.0;
                pts.push([a[0] + (b[0] - a[0]) * t + ((i * 7 + k) % 3) as f32 - 1.0, a[1] + (b[1] - a[1]) * t + ((i * 5) % 3) as f32 - 1.0]);
            }
        }
        pts.push([101.0, 102.0]);
        let shape = recognize(&pts).expect("shape");
        assert_eq!(shape.name(), "rectangle", "{shape:?}");
        let Shape::Poly { pts: r, .. } = shape else { panic!() };
        for k in 0..4 {
            let (a, b, c) = (r[k], r[(k + 1) % 4], r[(k + 2) % 4]);
            let dot = (b[0] - a[0]) * (c[0] - b[0]) + (b[1] - a[1]) * (c[1] - b[1]);
            assert!(dot.abs() < 1e-2, "square corner {k}: {r:?}");
            assert!(corners.iter().any(|q| dist(*q, a) < 6.0), "corner {a:?} stays near a drawn one");
        }
        // Level: within 6° of level snaps level.
        assert!((r[0][1] - r[1][1]).abs() < 1e-3 || (r[0][0] - r[1][0]).abs() < 1e-3, "{r:?}");
        // Closed outline back to the start.
        let out = shape.points();
        assert!(dist(out[0], *out.last().expect("pts")) < 1e-3);

        let tri: Vec<[f32; 2]> = [[0.0, 0.0], [200.0, 0.0], [100.0, 170.0], [0.0, 0.0]]
            .windows(2)
            .flat_map(|w| (0..40).map(move |i| [w[0][0] + (w[1][0] - w[0][0]) * i as f32 / 40.0, w[0][1] + (w[1][1] - w[0][1]) * i as f32 / 40.0]))
            .chain([[1.0, 1.0]])
            .collect();
        assert_eq!(recognize(&tri).map(|s| s.name()), Some("triangle"));

        let oval: Vec<[f32; 2]> = (0..=80)
            .map(|i| {
                let a = i as f32 / 80.0 * std::f32::consts::TAU;
                [300.0 + 150.0 * a.cos(), 200.0 + 60.0 * a.sin()]
            })
            .collect();
        match recognize(&oval) {
            Some(Shape::Ellipse { rx, ry, .. }) => assert!((rx.max(ry) - 150.0).abs() < 8.0 && (rx.min(ry) - 60.0).abs() < 6.0, "{rx} {ry}"),
            other => panic!("{other:?}"),
        }

        // An L: two straight sides.
        let ell: Vec<[f32; 2]> = (0..40).map(|i| [50.0, 50.0 + i as f32 * 4.0]).chain((1..40).map(|i| [50.0 + i as f32 * 4.0, 206.0])).collect();
        let mut s = recognize(&ell).expect("L");
        assert_eq!(s.name(), "straight lines");
        // Hold and drag away from the middle: it grows.
        let before = s.points();
        s.adjust([400.0, 400.0]);
        let after = s.points();
        let span = |v: &[[f32; 2]]| v.iter().map(|p| p[0]).fold(f32::MIN, f32::max) - v.iter().map(|p| p[0]).fold(f32::MAX, f32::min);
        assert!(span(&after) > span(&before) * 1.5);
        s.adjust([f32::NAN, 0.0]);
    }

    #[test]
    fn mirror_covers_every_combination() {
        assert_eq!(mirror_maps([true, false, false]), vec![[-1.0, 1.0, 1.0]]);
        assert_eq!(mirror_maps([true, true, false]).len(), 3);
        assert_eq!(mirror_maps([true, true, true]).len(), 7);
        assert!(mirror_maps([false; 3]).is_empty());
    }
}
