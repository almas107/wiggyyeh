//! 3D Guides: the sketching surfaces strokes are drawn onto (Feather's core idea).
//!
//! - **Drawn**: the first stroke, drawn from one view, is extruded along that view's direction
//!   into a surface ("bent paper"). A later **bend** stroke drawn from another view becomes the
//!   path the surface sweeps along, keeping the first stroke's shape as its cross-section: a
//!   circle from the front is a straight tube; bending it with a larger circle from the top makes
//!   a doughnut.
//! - **Loft**: two or more curves joined in order into a surface, with a tension (smoothness).
//! - **Primitives**: cube, pyramid, sphere and tube with a segment count (more or fewer segments
//!   give cylinders, cones, prisms…), plus a plain plane.
//!
//! Every guide is stored as its recipe plus a transform; its surface is a set of point grids
//! rebuilt from the recipe ([`Guide::rebuild`]), used for raycasting and drawing.

use serde::{Deserialize, Serialize};

use crate::camera::View;
use crate::math::{Quat, Vec3, Xform, ray_hits_box, ray_triangle, v3};

/// Samples across a drawn guide's cross-section.
pub const SECTION_SAMPLES: usize = 64;
/// Samples along a guide's sweep.
pub const SWEEP_SAMPLES: usize = 48;
/// Samples along each lofted curve.
pub const LOFT_SAMPLES: usize = 48;
/// Rows between two lofted curves.
pub const LOFT_ROWS_PER_SPAN: usize = 12;
pub const SEGMENTS_MIN: u32 = 3;
pub const SEGMENTS_MAX: u32 = 64;
/// The most opaque a guide gets (Feather: never fully opaque, so strokes behind stay visible).
pub const OPACITY_MAX: f32 = 0.95;
/// Most points accepted in any recipe curve.
pub const CURVE_POINTS_MAX: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Primitive {
    Cube,
    Pyramid,
    Sphere,
    Tube,
    /// A flat square (WobbleWorks extra: handy as a ground or a wall).
    Plane,
}

impl Primitive {
    pub const ALL: [Primitive; 5] = [Primitive::Cube, Primitive::Pyramid, Primitive::Sphere, Primitive::Tube, Primitive::Plane];

    pub fn name(self) -> &'static str {
        match self {
            Primitive::Cube => "cube",
            Primitive::Pyramid => "pyramid",
            Primitive::Sphere => "sphere",
            Primitive::Tube => "tube",
            Primitive::Plane => "plane",
        }
    }

    pub fn parse(s: &str) -> Option<Primitive> {
        Primitive::ALL.into_iter().find(|p| p.name().eq_ignore_ascii_case(s))
    }

    pub fn default_segments(self) -> u32 {
        match self {
            Primitive::Cube => 4,
            Primitive::Pyramid => 4,
            Primitive::Sphere => 24,
            Primitive::Tube => 24,
            Primitive::Plane => 8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Recipe {
    Drawn {
        /// The first stroke in world space (on the plane through `center` facing the view).
        section: Vec<Vec3>,
        /// Whether the first stroke closed on itself.
        closed: bool,
        /// Where the first stroke was drawn (the cross-section's origin).
        center: Vec3,
        /// The extrusion direction (the first view's direction, towards the viewer).
        axis: Vec3,
        /// The straight extrusion's length.
        length: f32,
        /// The bend path, when bent.
        bend: Option<Vec<Vec3>>,
    },
    Loft {
        curves: Vec<Vec<Vec3>>,
        /// 0 = sharp bends through each curve, 1 = smooth.
        tension: f32,
    },
    Primitive {
        kind: Primitive,
        segments: u32,
    },
}

/// A rectangular grid of points: `rows` rows of `cols` points.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Grid {
    pub cols: usize,
    pub rows: usize,
    pub points: Vec<Vec3>,
    /// The last column joins the first.
    pub closed_u: bool,
    pub lo: Vec3,
    pub hi: Vec3,
}

impl Grid {
    fn new(cols: usize, rows: usize, points: Vec<Vec3>, closed_u: bool) -> Option<Grid> {
        if cols < 2 || rows < 2 || points.len() != cols.checked_mul(rows)? || points.iter().any(|p| !p.is_finite()) {
            return None;
        }
        let mut lo = v3(f32::INFINITY, f32::INFINITY, f32::INFINITY);
        let mut hi = -lo;
        for p in &points {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        Some(Grid { cols, rows, points, closed_u, lo, hi })
    }

    pub fn at(&self, col: usize, row: usize) -> Option<Vec3> {
        let c = if self.closed_u { col % self.cols } else { col };
        self.points.get(row.checked_mul(self.cols)?.checked_add(c)?).copied()
    }

    /// The quads as pairs of triangles: [a, b, c] corners.
    pub fn triangles(&self) -> impl Iterator<Item = [Vec3; 3]> + '_ {
        let quads_u = if self.closed_u { self.cols } else { self.cols - 1 };
        (0..self.rows - 1).flat_map(move |r| {
            (0..quads_u).flat_map(move |c| {
                let (a, b, cc, d) = (self.at(c, r), self.at(c + 1, r), self.at(c + 1, r + 1), self.at(c, r + 1));
                match (a, b, cc, d) {
                    (Some(a), Some(b), Some(cc), Some(d)) => vec![[a, b, cc], [a, cc, d]],
                    _ => Vec::new(),
                }
            })
        })
    }

    pub fn transformed(&self, x: &Xform) -> Option<Grid> {
        Grid::new(self.cols, self.rows, self.points.iter().map(|p| x.apply(*p)).collect(), self.closed_u)
    }
}

/// A raycast hit on a guide.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub t: f32,
    pub point: Vec3,
    pub normal: Vec3,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    pub id: u64,
    pub name: String,
    pub recipe: Recipe,
    pub xform: Xform,
    pub opacity: f32,
    /// Saved to the Resource tab.
    #[serde(default)]
    pub saved: bool,
    /// The surface in world space (rebuilt from the recipe; not saved).
    #[serde(skip)]
    pub grids: Vec<Grid>,
}

/// Uniformly resample a polyline to `n` points by arc length.
pub fn resample(points: &[Vec3], n: usize) -> Vec<Vec3> {
    let pts: Vec<Vec3> = points.iter().copied().filter(|p| p.is_finite()).collect();
    if pts.is_empty() || n == 0 {
        return Vec::new();
    }
    if pts.len() == 1 || n == 1 {
        return vec![pts[0]; n];
    }
    let mut cum = Vec::with_capacity(pts.len());
    let mut total = 0.0f32;
    cum.push(0.0);
    for w in pts.windows(2) {
        total += w[0].distance(w[1]);
        cum.push(total);
    }
    if total <= 1e-9 {
        return vec![pts[0]; n];
    }
    let mut out = Vec::with_capacity(n);
    let mut seg = 0usize;
    for i in 0..n {
        let s = total * i as f32 / (n - 1) as f32;
        while seg + 2 < cum.len() && cum[seg + 1] < s {
            seg += 1;
        }
        let (s0, s1) = (cum[seg], cum[seg + 1]);
        let t = if s1 > s0 { ((s - s0) / (s1 - s0)).clamp(0.0, 1.0) } else { 0.0 };
        out.push(pts[seg].lerp(pts[seg + 1], t));
    }
    out
}

pub fn polyline_length(points: &[Vec3]) -> f32 {
    points.windows(2).map(|w| w[0].distance(w[1])).sum()
}

fn bbox(points: &[Vec3]) -> (Vec3, Vec3) {
    let mut lo = v3(f32::INFINITY, f32::INFINITY, f32::INFINITY);
    let mut hi = -lo;
    for p in points {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    (lo, hi)
}

fn sanitize_curve(points: &[Vec3]) -> Result<Vec<Vec3>, String> {
    let pts: Vec<Vec3> = points.iter().copied().filter(|p| p.is_finite()).take(CURVE_POINTS_MAX).collect();
    if pts.len() < 2 || polyline_length(&pts) < 1e-5 {
        return Err("the curve is too short to make a guide".into());
    }
    Ok(pts)
}

impl Guide {
    /// A drawn guide from a stroke drawn on screen: the stroke lands on the plane through
    /// `anchor` facing the camera and is extruded along the view direction.
    pub fn drawn(id: u64, view: &View, screen: &[(f32, f32)], anchor: Vec3) -> Result<Guide, String> {
        let world: Vec<Vec3> = screen
            .iter()
            .filter(|(x, y)| x.is_finite() && y.is_finite())
            .take(CURVE_POINTS_MAX)
            .map(|(x, y)| view.unproject_at(*x, *y, anchor))
            .collect();
        Guide::drawn_from_world(id, &world, anchor, view.back)
    }

    /// A drawn guide from a world-space section curve and an extrusion axis.
    pub fn drawn_from_world(id: u64, section: &[Vec3], center: Vec3, axis: Vec3) -> Result<Guide, String> {
        let section = sanitize_curve(section)?;
        let axis = axis.normalized();
        if axis == Vec3::ZERO || !center.is_finite() {
            return Err("the guide needs a direction".into());
        }
        let len = polyline_length(&section);
        let (lo, hi) = bbox(&section);
        let diag = (hi - lo).length();
        let gap = section.first().zip(section.last()).map_or(f32::INFINITY, |(a, b)| a.distance(*b));
        let closed = section.len() > 8 && gap < len * 0.08 && diag > 1e-4;
        let length = (diag * 1.5).max(2.0);
        let mut g = Guide {
            id,
            name: format!("Guide {id}"),
            recipe: Recipe::Drawn { section, closed, center, axis, length, bend: None },
            xform: Xform::default(),
            opacity: 0.35,
            saved: false,
            grids: Vec::new(),
        };
        g.rebuild()?;
        Ok(g)
    }

    /// Bend a drawn guide along a stroke drawn on screen from the current view: the stroke lands
    /// on the plane through the guide's centre facing the camera.
    pub fn bend(&mut self, view: &View, screen: &[(f32, f32)]) -> Result<(), String> {
        let Recipe::Drawn { center, .. } = &self.recipe else {
            return Err("only drawn guides bend".into());
        };
        let anchor = self.xform.apply(*center);
        let world: Vec<Vec3> = screen
            .iter()
            .filter(|(x, y)| x.is_finite() && y.is_finite())
            .take(CURVE_POINTS_MAX)
            .map(|(x, y)| view.unproject_at(*x, *y, anchor))
            .collect();
        self.bend_world(&world)
    }

    /// Bend along a world-space path.
    pub fn bend_world(&mut self, path: &[Vec3]) -> Result<(), String> {
        let path = sanitize_curve(path)?;
        // The path is in world space; the recipe is in the guide's local space.
        let inv = |p: Vec3| {
            let x = &self.xform;
            let local = x.rotation.conjugate().rotate(p - x.translation);
            let s = x.scale;
            v3(
                if s.x.abs() > 1e-9 { local.x / s.x } else { 0.0 },
                if s.y.abs() > 1e-9 { local.y / s.y } else { 0.0 },
                if s.z.abs() > 1e-9 { local.z / s.z } else { 0.0 },
            )
        };
        let local: Vec<Vec3> = path.iter().map(|p| inv(*p)).collect();
        let Recipe::Drawn { bend, .. } = &mut self.recipe else {
            return Err("only drawn guides bend".into());
        };
        let before = bend.replace(local);
        if let Err(e) = self.rebuild() {
            if let Recipe::Drawn { bend, .. } = &mut self.recipe {
                *bend = before;
            }
            self.rebuild()?;
            return Err(e);
        }
        Ok(())
    }

    /// A lofted guide through curves in order.
    pub fn loft(id: u64, curves: &[Vec<Vec3>], tension: f32) -> Result<Guide, String> {
        if curves.len() < 2 {
            return Err("loft needs two or more curves".into());
        }
        let curves = curves.iter().map(|c| sanitize_curve(c)).collect::<Result<Vec<_>, _>>()?;
        let mut g = Guide {
            id,
            name: format!("Loft {id}"),
            recipe: Recipe::Loft { curves, tension: if tension.is_finite() { tension.clamp(0.0, 1.0) } else { 0.5 } },
            xform: Xform::default(),
            opacity: 0.35,
            saved: false,
            grids: Vec::new(),
        };
        g.rebuild()?;
        Ok(g)
    }

    /// A primitive guide at the origin.
    pub fn primitive(id: u64, kind: Primitive, segments: u32) -> Result<Guide, String> {
        let mut g = Guide {
            id,
            name: format!("{} {id}", capitalised(kind.name())),
            recipe: Recipe::Primitive { kind, segments: segments.clamp(SEGMENTS_MIN, SEGMENTS_MAX) },
            xform: Xform::default(),
            opacity: 0.35,
            saved: false,
            grids: Vec::new(),
        };
        g.rebuild()?;
        Ok(g)
    }

    pub fn set_opacity(&mut self, o: f32) {
        if o.is_finite() {
            self.opacity = o.clamp(0.0, OPACITY_MAX);
        }
    }

    pub fn is_drawn(&self) -> bool {
        matches!(self.recipe, Recipe::Drawn { .. })
    }

    /// Rebuild the surface from the recipe and transform.
    pub fn rebuild(&mut self) -> Result<(), String> {
        let local = match &self.recipe {
            Recipe::Drawn { section, closed, center, axis, length, bend } => drawn_grid(section, *closed, *center, *axis, *length, bend.as_deref()),
            Recipe::Loft { curves, tension } => loft_grid(curves, *tension).into_iter().collect(),
            Recipe::Primitive { kind, segments } => primitive_grids(*kind, (*segments).clamp(SEGMENTS_MIN, SEGMENTS_MAX)),
        };
        let grids: Vec<Grid> = local.iter().filter_map(|g| g.transformed(&self.xform)).collect();
        if grids.is_empty() {
            return Err("the guide has no surface".into());
        }
        self.grids = grids;
        Ok(())
    }

    /// The nearest surface hit along a ray.
    pub fn raycast(&self, origin: Vec3, dir: Vec3) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        for g in &self.grids {
            let pad = v3(1e-4, 1e-4, 1e-4);
            if !ray_hits_box(origin, dir, g.lo - pad, g.hi + pad) {
                continue;
            }
            for [a, b, c] in g.triangles() {
                if let Some((t, _, _)) = ray_triangle(origin, dir, a, b, c)
                    && best.is_none_or(|h| t < h.t)
                {
                    let mut n = (b - a).cross(c - a).normalized();
                    if n.dot(dir) > 0.0 {
                        n = -n;
                    }
                    best = Some(Hit { t, point: origin + dir * t, normal: n });
                }
            }
        }
        best
    }

    /// Bounding box of the surface.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut it = self.grids.iter();
        let first = it.next()?;
        Some(it.fold((first.lo, first.hi), |(lo, hi), g| (lo.min(g.lo), hi.max(g.hi))))
    }

    /// The section lines drawn on the surface: some rows and columns of each grid.
    pub fn section_lines(&self) -> Vec<Vec<Vec3>> {
        let mut out = Vec::new();
        for g in &self.grids {
            let step_r = (g.rows / 12).max(1);
            let step_c = (g.cols / 12).max(1);
            for r in (0..g.rows).step_by(step_r).chain(std::iter::once(g.rows - 1)) {
                let mut line: Vec<Vec3> = (0..g.cols).filter_map(|c| g.at(c, r)).collect();
                if g.closed_u
                    && let Some(f) = line.first().copied()
                {
                    line.push(f);
                }
                out.push(line);
            }
            for c in (0..g.cols).step_by(step_c) {
                out.push((0..g.rows).filter_map(|r| g.at(c, r)).collect());
            }
        }
        out.dedup();
        out
    }

    /// The orange line: where the guide started (the first column of a drawn or lofted guide).
    pub fn start_line(&self) -> Option<Vec<Vec3>> {
        if matches!(self.recipe, Recipe::Primitive { .. }) {
            return None;
        }
        let g = self.grids.first()?;
        Some((0..g.rows).filter_map(|r| g.at(0, r)).collect())
    }

    /// The centre of the surface (for transforms and duplicates).
    pub fn centre(&self) -> Vec3 {
        self.bounds().map_or(self.xform.translation, |(lo, hi)| (lo + hi) * 0.5)
    }
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// Rotation-minimising frames along a path, starting from the rotation taking `axis` onto the
/// first tangent (double reflection, Wang et al. 2008).
fn sweep_rotations(path: &[Vec3], axis: Vec3) -> Vec<Quat> {
    let n = path.len();
    let tangent = |i: usize| -> Vec3 {
        let a = path[i.saturating_sub(1)];
        let b = path[(i + 1).min(n - 1)];
        (b - a).normalized()
    };
    let mut out = Vec::with_capacity(n);
    let mut q = Quat::from_to(axis, tangent(0));
    out.push(q);
    for i in 1..n {
        let (t0, t1) = (tangent(i - 1), tangent(i));
        if t0 != Vec3::ZERO && t1 != Vec3::ZERO {
            q = Quat::from_to(t0, t1) * q;
        }
        out.push(q);
    }
    out
}

fn drawn_grid(section: &[Vec3], closed: bool, center: Vec3, axis: Vec3, length: f32, bend: Option<&[Vec3]>) -> Vec<Grid> {
    let axis = axis.normalized();
    if axis == Vec3::ZERO {
        return Vec::new();
    }
    let mut sec = if closed {
        // Resample the loop without repeating the first point.
        let mut loop_pts = section.to_vec();
        if let Some(f) = loop_pts.first().copied() {
            loop_pts.push(f);
        }
        let mut s = resample(&loop_pts, SECTION_SAMPLES + 1);
        s.pop();
        s
    } else {
        resample(section, SECTION_SAMPLES)
    };
    // Keep the section flat on its plane (perpendicular to the axis through the centre).
    for p in &mut sec {
        let d = (*p - center).dot(axis);
        *p -= axis * d;
    }
    let cols = sec.len();
    let length = if length.is_finite() { length.clamp(1e-3, 1e5) } else { 2.0 };
    let mut points = Vec::with_capacity(cols * SWEEP_SAMPLES);
    match bend {
        Some(path) if path.len() >= 2 && polyline_length(path) > 1e-5 => {
            let path = resample(path, SWEEP_SAMPLES);
            let rots = sweep_rotations(&path, axis);
            for (p, q) in path.iter().zip(&rots) {
                for s in &sec {
                    points.push(*p + q.rotate(*s - center));
                }
            }
        }
        _ => {
            for r in 0..SWEEP_SAMPLES {
                let off = axis * (length * (r as f32 / (SWEEP_SAMPLES - 1) as f32 - 0.5));
                for s in &sec {
                    points.push(*s + off);
                }
            }
        }
    }
    Grid::new(cols, SWEEP_SAMPLES, points, closed).into_iter().collect()
}

fn loft_grid(curves: &[Vec<Vec3>], tension: f32) -> Option<Grid> {
    let mut rows: Vec<Vec<Vec3>> = Vec::new();
    for c in curves {
        let mut r = resample(c, LOFT_SAMPLES);
        if let Some(prev) = rows.last() {
            // Run each curve the same way as the one before it.
            let (a0, a1) = (*prev.first()?, *prev.last()?);
            let (b0, b1) = (*r.first()?, *r.last()?);
            if a0.distance(b1) + a1.distance(b0) < a0.distance(b0) + a1.distance(b1) {
                r.reverse();
            }
        }
        rows.push(r);
    }
    let m = rows.len();
    if m < 2 {
        return None;
    }
    let scale = if tension.is_finite() { tension.clamp(0.0, 1.0) } else { 0.5 };
    let mut points = Vec::new();
    let mut nrows = 0usize;
    for k in 0..m - 1 {
        let steps = if k == m - 2 { LOFT_ROWS_PER_SPAN + 1 } else { LOFT_ROWS_PER_SPAN };
        for s in 0..steps {
            let t = s as f32 / LOFT_ROWS_PER_SPAN as f32;
            #[allow(clippy::needless_range_loop)]
            for i in 0..LOFT_SAMPLES {
                let p = |j: usize| rows[j][i];
                let (p0, p1) = (p(k), p(k + 1));
                let m0 = if k > 0 { (p(k + 1) - p(k - 1)) * (0.5 * scale) } else { (p1 - p0) * scale };
                let m1 = if k + 2 < m { (p(k + 2) - p(k)) * (0.5 * scale) } else { (p1 - p0) * scale };
                // Cubic Hermite.
                let (t2, t3) = (t * t, t * t * t);
                let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
                let h10 = t3 - 2.0 * t2 + t;
                let h01 = -2.0 * t3 + 3.0 * t2;
                let h11 = t3 - t2;
                points.push(p0 * h00 + m0 * h10 + p1 * h01 + m1 * h11);
            }
            nrows += 1;
        }
    }
    Grid::new(LOFT_SAMPLES, nrows, points, false)
}

fn primitive_grids(kind: Primitive, segments: u32) -> Vec<Grid> {
    let n = segments as usize;
    let mut out = Vec::new();
    let tau = std::f32::consts::TAU;
    match kind {
        Primitive::Cube | Primitive::Plane => {
            let faces: &[(Vec3, Vec3, Vec3)] = if kind == Primitive::Plane {
                &[(Vec3::ZERO, Vec3::X, -Vec3::Z)]
            } else {
                &[
                    (Vec3::X, -Vec3::Z, Vec3::Y),
                    (-Vec3::X, Vec3::Z, Vec3::Y),
                    (Vec3::Y, Vec3::X, -Vec3::Z),
                    (-Vec3::Y, Vec3::X, Vec3::Z),
                    (Vec3::Z, Vec3::X, Vec3::Y),
                    (-Vec3::Z, -Vec3::X, Vec3::Y),
                ]
            };
            for (c, u, v) in faces {
                let mut pts = Vec::with_capacity((n + 1) * (n + 1));
                for r in 0..=n {
                    for col in 0..=n {
                        let a = col as f32 / n as f32 * 2.0 - 1.0;
                        let b = r as f32 / n as f32 * 2.0 - 1.0;
                        let size = if kind == Primitive::Plane { 2.0 } else { 1.0 };
                        pts.push((*c + *u * a + *v * b) * size);
                    }
                }
                out.extend(Grid::new(n + 1, n + 1, pts, false));
            }
        }
        Primitive::Sphere => {
            let lat = (n / 2).max(2);
            let mut pts = Vec::with_capacity(n * (lat + 1));
            for r in 0..=lat {
                let phi = std::f32::consts::PI * (r as f32 / lat as f32) - std::f32::consts::FRAC_PI_2;
                for c in 0..n {
                    let th = tau * c as f32 / n as f32;
                    pts.push(v3(phi.cos() * th.cos(), phi.sin(), phi.cos() * -th.sin()));
                }
            }
            out.extend(Grid::new(n, lat + 1, pts, true));
        }
        Primitive::Tube => {
            let rows = 9;
            let mut pts = Vec::with_capacity(n * rows);
            for r in 0..rows {
                let y = r as f32 / (rows - 1) as f32 * 2.0 - 1.0;
                for c in 0..n {
                    let th = tau * c as f32 / n as f32;
                    pts.push(v3(th.cos(), y, -th.sin()));
                }
            }
            out.extend(Grid::new(n, rows, pts, true));
        }
        Primitive::Pyramid => {
            let rows = 9;
            let mut side = Vec::with_capacity(n * rows);
            let mut base = Vec::with_capacity(n * rows);
            for r in 0..rows {
                let t = r as f32 / (rows - 1) as f32;
                for c in 0..n {
                    let th = tau * c as f32 / n as f32 + std::f32::consts::FRAC_PI_4;
                    let rim = v3(th.cos() * std::f32::consts::SQRT_2, -1.0, -th.sin() * std::f32::consts::SQRT_2);
                    side.push(rim.lerp(v3(0.0, 1.0, 0.0), t));
                    base.push(v3(0.0, -1.0, 0.0).lerp(rim, t));
                }
            }
            out.extend(Grid::new(n, rows, side, true));
            out.extend(Grid::new(n, rows, base, true));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{Camera, PerfectView};

    fn circle(n: usize, r: f32) -> Vec<(f32, f32)> {
        (0..=n).map(|i| {
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            (640.0 + r * a.cos(), 400.0 + r * a.sin())
        })
        .collect()
    }

    #[test]
    fn a_circle_from_the_front_makes_a_tube_and_a_bend_from_the_top_makes_a_doughnut() {
        let mut cam = Camera::default();
        cam.snap(PerfectView::Front);
        let v = cam.view();
        let mut g = Guide::drawn(1, &v, &circle(64, 60.0), Vec3::ZERO).expect("guide");
        let Recipe::Drawn { closed, axis, .. } = &g.recipe else { panic!() };
        assert!(*closed);
        assert!(axis.distance(Vec3::Z) < 1e-4, "extruded along the view");
        // A ray from the front through the circle's edge hits the tube's front rim… from the side
        // a ray at the tube's radius grazes it; a ray through the centre misses (it is hollow).
        let (o, d) = v.ray(640.0, 400.0);
        assert!(g.raycast(o, d).is_none(), "the tube is open along the view");
        let mut side = cam;
        side.snap(PerfectView::Right);
        let sv = side.view();
        let (o, d) = sv.ray(640.0, 400.0);
        let hit = g.raycast(o, d).expect("the tube's wall is hit from the side");
        let radius = v.world_per_px(cam.distance) * 60.0;
        assert!((hit.point.x - radius).abs() < radius * 0.1, "{hit:?} r={radius}");

        // Bend: a larger circle drawn from the top.
        let mut top = cam;
        top.snap(PerfectView::Top);
        g.bend(&top.view(), &circle(96, 200.0)).expect("bend");
        let (lo, hi) = g.bounds().expect("bounds");
        let big = top.view().world_per_px(top.distance) * 200.0;
        assert!(hi.x - lo.x > big * 2.0, "the doughnut spans the bend circle");
        assert!((hi.y - lo.y) < radius * 2.5, "and stays as thick as the tube");
    }

    #[test]
    fn primitives_build_and_take_rays() {
        for kind in Primitive::ALL {
            for seg in [0, 3, kind.default_segments(), 1000] {
                let g = Guide::primitive(1, kind, seg).expect("primitive");
                assert!(!g.grids.is_empty());
                let (o, d) = if kind == Primitive::Plane { (v3(0.05, 10.0, 0.03), -Vec3::Y) } else { (v3(0.05, 0.03, 10.0), -Vec3::Z) };
                let hit = g.raycast(o, d);
                assert!(hit.is_some(), "{kind:?} {seg}");
            }
        }
        let cube = Guide::primitive(1, Primitive::Cube, 4).expect("cube");
        let hit = cube.raycast(v3(0.2, 0.1, 10.0), -Vec3::Z).expect("hit");
        assert!((hit.point.z - 1.0).abs() < 1e-4 && hit.normal.z > 0.9);
    }

    #[test]
    fn loft_joins_curves_in_order() {
        let a: Vec<Vec3> = (0..10).map(|i| v3(i as f32, 0.0, 0.0)).collect();
        let b: Vec<Vec3> = (0..10).rev().map(|i| v3(i as f32, 0.0, 3.0)).collect();
        let c: Vec<Vec3> = (0..10).map(|i| v3(i as f32, 1.0, 6.0)).collect();
        let g = Guide::loft(2, &[a.clone(), b, c], 0.7).expect("loft");
        let hit = g.raycast(v3(4.5, 10.0, 1.5), -Vec3::Y).expect("between the first two curves");
        assert!(hit.point.y.abs() < 0.3);
        assert!(Guide::loft(2, &[a], 0.5).is_err());
        assert!(Guide::loft(2, &[vec![Vec3::ZERO], vec![Vec3::X]], 0.5).is_err());
    }

    #[test]
    fn bad_curves_are_errors() {
        let v = Camera::default().view();
        assert!(Guide::drawn(1, &v, &[], Vec3::ZERO).is_err());
        assert!(Guide::drawn(1, &v, &[(1.0, 1.0), (1.0, 1.0)], Vec3::ZERO).is_err());
        assert!(Guide::drawn(1, &v, &[(f32::NAN, 1.0), (3.0, f32::INFINITY)], Vec3::ZERO).is_err());
        let mut p = Guide::primitive(1, Primitive::Sphere, 12).expect("sphere");
        assert!(p.bend(&v, &circle(10, 50.0)).is_err());
        let mut g = Guide::drawn(1, &v, &[(10.0, 10.0), (300.0, 50.0)], Vec3::ZERO).expect("open guide");
        assert!(g.bend(&v, &[(5.0, 5.0)]).is_err());
        assert!(!g.grids.is_empty(), "a failed bend leaves the guide as it was");
        g.set_opacity(5.0);
        assert_eq!(g.opacity, OPACITY_MAX);
        g.set_opacity(f32::NAN);
        assert_eq!(g.opacity, OPACITY_MAX);
    }

    #[test]
    fn resample_is_uniform() {
        let r = resample(&[Vec3::ZERO, v3(1.0, 0.0, 0.0), v3(1.0, 3.0, 0.0)], 5);
        assert_eq!(r.len(), 5);
        assert!(r[2].distance(v3(1.0, 1.0, 0.0)) < 1e-4);
        assert!(resample(&[], 4).is_empty());
    }
}
