//! Stroke feel: what happens between the pen and the curve, after Blender's Grease Pencil
//! draw brush (its documented settings, so Blender users find them where they expect).
//!
//! - **Pressure curve** (GP's *Sensitivity* curve): a power and a floor applied to the pen's
//!   pressure before anything else.
//! - **Stabilizer** lives in [`crate::assist::Stabilizer`]; Shift while drawing toggles it, as
//!   Shift-LMB does in Grease Pencil.
//! - **Post-processing** when the pen lifts (GP's panel of the same name): *Smooth* (strength,
//!   iterations), *Smooth Thickness* (strength, iterations), *Subdivision Steps*, *Simplify*
//!   (adaptive) and *Trim Strokes End* (cut the overshoot where a stroke crosses itself).
//!
//! Everything works on the screen samples `[x, y, pressure]` before they are projected onto the
//! guide, so processed curves stay on their surface.

use serde::{Deserialize, Serialize};

/// Most points a processed stroke may have (subdivision doubles the count each step).
pub const MAX_POINTS: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StrokeFeel {
    /// Pressure curve: pressure is raised to this power (1 = linear, < 1 = lighter touch).
    pub pressure_gamma: f32,
    /// The lightest touch still counts as this much pressure (0..0.9).
    pub pressure_min: f32,
    /// Post-processing on (GP's panel checkbox).
    pub post: bool,
    /// Smooth: strength 0..2 and iterations 0..10.
    pub smooth: f32,
    pub smooth_iterations: u32,
    /// Smooth Thickness: strength 0..1 and iterations 0..10.
    pub smooth_thickness: f32,
    pub thickness_iterations: u32,
    /// Subdivision Steps 0..3.
    pub subdivide: u32,
    /// Simplify 0..1: how far (up to 3 px) points may stray before they are kept.
    pub simplify: f32,
    /// Trim Strokes End: cut the ends that overshoot where the stroke crosses itself.
    pub trim: bool,
}

impl Default for StrokeFeel {
    fn default() -> Self {
        StrokeFeel {
            pressure_gamma: 1.0,
            pressure_min: 0.0,
            post: true,
            smooth: 0.35,
            smooth_iterations: 1,
            smooth_thickness: 0.3,
            thickness_iterations: 1,
            subdivide: 0,
            simplify: 0.0,
            trim: false,
        }
    }
}

fn fin(v: f32, lo: f32, hi: f32, fallback: f32) -> f32 {
    if v.is_finite() { v.clamp(lo, hi) } else { fallback }
}

impl StrokeFeel {
    /// Every field in range (NaN becomes the default).
    pub fn sanitize(&mut self) {
        let d = StrokeFeel::default();
        self.pressure_gamma = fin(self.pressure_gamma, 0.25, 4.0, d.pressure_gamma);
        self.pressure_min = fin(self.pressure_min, 0.0, 0.9, d.pressure_min);
        self.smooth = fin(self.smooth, 0.0, 2.0, d.smooth);
        self.smooth_iterations = self.smooth_iterations.min(10);
        self.smooth_thickness = fin(self.smooth_thickness, 0.0, 1.0, d.smooth_thickness);
        self.thickness_iterations = self.thickness_iterations.min(10);
        self.subdivide = self.subdivide.min(3);
        self.simplify = fin(self.simplify, 0.0, 1.0, d.simplify);
    }

    /// The pen's pressure through the curve.
    pub fn pressure(&self, p: f32) -> f32 {
        let p = fin(p, 0.0, 1.0, 1.0);
        let g = fin(self.pressure_gamma, 0.25, 4.0, 1.0);
        let min = fin(self.pressure_min, 0.0, 0.9, 0.0);
        min + (1.0 - min) * p.powf(g)
    }

    /// Post-process a finished stroke's screen samples (unchanged when post-processing is off).
    pub fn process(&self, pts: &[[f32; 3]]) -> Vec<[f32; 3]> {
        let mut out: Vec<[f32; 3]> = pts.iter().copied().filter(|q| q.iter().all(|v| v.is_finite())).collect();
        if !self.post || out.len() < 3 {
            return out;
        }
        let mut f = *self;
        f.sanitize();
        if f.trim {
            out = trim(&out);
        }
        for _ in 0..f.subdivide {
            if out.len().saturating_mul(2) > MAX_POINTS {
                break;
            }
            out = subdivide(&out);
        }
        for _ in 0..f.smooth_iterations {
            smooth(&mut out, f.smooth, &[0, 1]);
        }
        for _ in 0..f.thickness_iterations {
            smooth(&mut out, f.smooth_thickness, &[2]);
        }
        if f.simplify > 0.0 {
            out = simplify(&out, f.simplify * 3.0);
        }
        out
    }
}

/// One smoothing pass on the given channels: each inner point moves toward its neighbours'
/// average by `strength` (above 1 it overshoots a little further, as GP's does up to 2); the
/// ends stay where they were drawn.
fn smooth(pts: &mut [[f32; 3]], strength: f32, channels: &[usize]) {
    if pts.len() < 3 || strength <= 0.0 {
        return;
    }
    let k = (strength * 0.5).clamp(0.0, 1.0);
    let src = pts.to_vec();
    for i in 1..src.len() - 1 {
        for &c in channels {
            let avg = (src[i - 1][c] + src[i + 1][c]) * 0.5;
            pts[i][c] = src[i][c] + (avg - src[i][c]) * k;
        }
    }
}

/// A point halfway along every segment.
fn subdivide(pts: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let mut out = Vec::with_capacity(pts.len() * 2);
    for w in pts.windows(2) {
        out.push(w[0]);
        out.push([(w[0][0] + w[1][0]) * 0.5, (w[0][1] + w[1][1]) * 0.5, (w[0][2] + w[1][2]) * 0.5]);
    }
    if let Some(l) = pts.last() {
        out.push(*l);
    }
    out
}

/// Douglas–Peucker on screen positions (iterative, so long strokes can't overflow the stack).
fn simplify(pts: &[[f32; 3]], tol: f32) -> Vec<[f32; 3]> {
    if pts.len() < 3 || tol <= 0.0 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0usize, pts.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (mut far, mut at) = (0.0f32, a);
        for i in a + 1..b {
            let d = seg_dist(pts[i], pts[a], pts[b]);
            if d > far {
                far = d;
                at = i;
            }
        }
        if far > tol {
            keep[at] = true;
            stack.push((a, at));
            stack.push((at, b));
        }
    }
    pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

fn seg_dist(p: [f32; 3], a: [f32; 3], b: [f32; 3]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 1e-12 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    ((p[0] - a[0] - dx * t).powi(2) + (p[1] - a[1] - dy * t).powi(2)).sqrt()
}

/// Where segments p0→p1 and q0→q1 cross: the fractions along each.
fn cross(p0: [f32; 3], p1: [f32; 3], q0: [f32; 3], q1: [f32; 3]) -> Option<(f32, f32)> {
    let (rx, ry) = (p1[0] - p0[0], p1[1] - p0[1]);
    let (sx, sy) = (q1[0] - q0[0], q1[1] - q0[1]);
    let den = rx * sy - ry * sx;
    if den.abs() < 1e-9 {
        return None;
    }
    let (qx, qy) = (q0[0] - p0[0], q0[1] - p0[1]);
    let t = (qx * sy - qy * sx) / den;
    let u = (qx * ry - qy * rx) / den;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some((t, u))
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Trim Strokes End: where the stroke first crosses itself (the crossing nearest its start),
/// keep the loop between the two passes and drop the overshooting ends. Strokes that never
/// cross are kept whole. Long strokes are checked up to a bounded amount of work.
fn trim(pts: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let n = pts.len();
    if !(4..=4000).contains(&n) {
        return pts.to_vec();
    }
    for i in 0..n - 1 {
        // The farthest later segment crossing segment i: the loop is as big as it can be.
        let found = (i + 2..n - 1).rev().find_map(|j| cross(pts[i], pts[i + 1], pts[j], pts[j + 1]).map(|(t, u)| (j, t, u)));
        if let Some((j, t, u)) = found {
            let start = lerp3(pts[i], pts[i + 1], t);
            let end = lerp3(pts[j], pts[j + 1], u);
            let mut out = Vec::with_capacity(j - i + 2);
            out.push(start);
            out.extend_from_slice(&pts[i + 1..=j]);
            out.push(end);
            return out;
        }
    }
    pts.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(n: usize) -> Vec<[f32; 3]> {
        (0..n).map(|i| [i as f32 * 4.0, if i % 2 == 0 { 0.0 } else { 3.0 }, if i % 2 == 0 { 0.2 } else { 1.0 }]).collect()
    }

    #[test]
    fn the_pressure_curve_lightens_or_floors_the_touch() {
        let mut f = StrokeFeel::default();
        assert_eq!(f.pressure(0.5), 0.5, "linear by default");
        f.pressure_gamma = 2.0;
        assert!((f.pressure(0.5) - 0.25).abs() < 1e-6);
        f.pressure_min = 0.5;
        assert!((f.pressure(0.0) - 0.5).abs() < 1e-6);
        assert!((f.pressure(1.0) - 1.0).abs() < 1e-6);
        assert_eq!(f.pressure(f32::NAN), 1.0);
        f.pressure_gamma = f32::INFINITY;
        assert!(f.pressure(0.3).is_finite());
    }

    #[test]
    fn smoothing_calms_jitter_and_keeps_the_ends() {
        let raw = line(20);
        let f = StrokeFeel { smooth: 1.0, smooth_iterations: 4, smooth_thickness: 1.0, thickness_iterations: 4, ..StrokeFeel::default() };
        let out = f.process(&raw);
        assert_eq!(out.len(), raw.len());
        assert_eq!(out[0], raw[0]);
        assert_eq!(out[19], raw[19]);
        let wiggle = |p: &[[f32; 3]]| p.windows(2).map(|w| (w[1][1] - w[0][1]).abs()).sum::<f32>();
        assert!(wiggle(&out) < wiggle(&raw) * 0.3, "{} vs {}", wiggle(&out), wiggle(&raw));
        let thick = |p: &[[f32; 3]]| p.windows(2).map(|w| (w[1][2] - w[0][2]).abs()).sum::<f32>();
        assert!(thick(&out) < thick(&raw) * 0.3);
        // Off: untouched.
        let off = StrokeFeel { post: false, ..f };
        assert_eq!(off.process(&raw), raw);
    }

    #[test]
    fn subdivide_and_simplify_change_the_point_count() {
        let raw: Vec<[f32; 3]> = (0..10).map(|i| [i as f32 * 10.0, 0.0, 1.0]).collect();
        let f = StrokeFeel { smooth_iterations: 0, thickness_iterations: 0, subdivide: 2, ..StrokeFeel::default() };
        assert_eq!(f.process(&raw).len(), 37);
        let f = StrokeFeel { smooth_iterations: 0, thickness_iterations: 0, simplify: 1.0, ..StrokeFeel::default() };
        assert_eq!(f.process(&raw), vec![raw[0], raw[9]], "a straight run needs only its ends");
        // Subdivision is capped.
        let long: Vec<[f32; 3]> = (0..15_000).map(|i| [i as f32, 0.0, 1.0]).collect();
        let f = StrokeFeel { subdivide: 3, ..StrokeFeel::default() };
        assert!(f.process(&long).len() <= MAX_POINTS);
    }

    #[test]
    fn trim_cuts_the_overshoot_of_a_closed_loop() {
        // A circle drawn a little past its start (spiralling out a touch, as hands do): the
        // tail crosses the lead-in.
        let mut raw: Vec<[f32; 3]> = (0..=40)
            .map(|i| {
                let a = std::f32::consts::TAU * 1.15 * i as f32 / 40.0 - 0.3;
                let r = 50.0 + 8.0 * i as f32 / 40.0;
                [100.0 + r * a.cos(), 100.0 + r * a.sin(), 1.0]
            })
            .collect();
        // A lead-in from outside the circle.
        raw.insert(0, [160.0, 80.0, 1.0]);
        let f = StrokeFeel { trim: true, smooth_iterations: 0, thickness_iterations: 0, ..StrokeFeel::default() };
        let out = f.process(&raw);
        assert!(out.len() < raw.len());
        let (a, b) = (out[0], out[out.len() - 1]);
        assert!(((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt() < 1e-3, "the loop closes exactly: {a:?} {b:?}");
        // No crossing: whole.
        let open = line(10);
        assert_eq!(StrokeFeel { trim: true, smooth_iterations: 0, thickness_iterations: 0, ..StrokeFeel::default() }.process(&open), open);
    }

    #[test]
    fn hostile_input_is_harmless() {
        let mut f = StrokeFeel { smooth: f32::NAN, simplify: -3.0, subdivide: 99, smooth_iterations: 1000, ..StrokeFeel::default() };
        f.sanitize();
        assert!(f.smooth.is_finite() && f.simplify == 0.0 && f.subdivide == 3 && f.smooth_iterations == 10);
        let junk = vec![[f32::NAN, 0.0, 1.0], [1.0, f32::INFINITY, 1.0], [0.0, 0.0, 0.5], [1.0, 1.0, 0.5], [1.0, 1.0, 0.5], [1.0, 1.0, 0.5]];
        let out = StrokeFeel { trim: true, simplify: 0.5, subdivide: 1, ..StrokeFeel::default() }.process(&junk);
        assert!(out.iter().all(|q| q.iter().all(|v| v.is_finite())));
        assert!(StrokeFeel::default().process(&[]).is_empty());
    }
}
