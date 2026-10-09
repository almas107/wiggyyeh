//! Operations on a note seen through a view: picking, erasing, selecting by area, liquify,
//! duplicates and mirror copies. All of them are total: bad input does nothing or errs.

use std::collections::HashSet;
use std::sync::Arc;

use crate::camera::View;
use crate::math::{Vec3, ray_hits_box, ray_triangle, v3};
use crate::model::{ResourceState, Scene, Stroke};
use crate::noise::hash;

/// Screen samples of a curve: (x, y, depth) per point (None behind the camera).
fn screen_points(view: &View, s: &Stroke) -> Vec<Option<(f32, f32, f32)>> {
    s.points.iter().map(|p| view.project(p.p).map(|q| (q.x, q.y, q.depth))).collect()
}

fn seg_dist(p: [f32; 2], a: (f32, f32), b: (f32, f32)) -> f32 {
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let l2 = abx * abx + aby * aby;
    let t = if l2 < 1e-9 { 0.0 } else { (((p[0] - a.0) * abx + (p[1] - a.1) * aby) / l2).clamp(0.0, 1.0) };
    ((p[0] - a.0 - abx * t).powi(2) + (p[1] - a.1 - aby * t).powi(2)).sqrt()
}

/// Whether a world point is hidden from the eye by the active guide (Feather's isolation:
/// curves covered by a guide cannot be selected or erased).
pub fn occluded_by_guide(scene: &Scene, view: &View, p: Vec3) -> bool {
    let Some(g) = scene.active_guide() else { return false };
    if g.opacity <= 0.0 {
        return false;
    }
    let (o, d) = if view.orthographic {
        let o = p + view.back * (view.half_height * 100.0);
        (o, -view.back)
    } else {
        let d = (p - view.eye).normalized();
        (view.eye, d)
    };
    let dist = (p - o).length();
    g.raycast(o, d).is_some_and(|h| h.t < dist - dist.max(1.0) * 2e-3)
}

/// Curves a stroke can touch: visible groups only (and the active group only, when asked).
fn candidates<'a>(scene: &'a Scene, active_only: bool) -> impl Iterator<Item = &'a Arc<Stroke>> + 'a {
    scene.strokes.iter().filter(move |s| scene.group_shown(s.group) && (!active_only || s.group == scene.active_group))
}

/// The nearest curve within `radius` pixels of a screen point (front-most when tied).
pub fn pick_stroke(scene: &Scene, view: &View, at: [f32; 2], radius: f32, active_only: bool) -> Option<u64> {
    let mut best: Option<(f32, f32, u64)> = None;
    for s in candidates(scene, active_only) {
        let sp = screen_points(view, s);
        let single = sp.len() == 1;
        for (i, w) in sp.windows(2).enumerate() {
            let (Some(a), Some(b)) = (w[0], w[1]) else { continue };
            let d = seg_dist(at, (a.0, a.1), (b.0, b.1));
            let depth = (a.2 + b.2) * 0.5;
            if d <= radius && best.is_none_or(|(bd, bz, _)| d < bd - 0.5 || (d < bd + 0.5 && depth < bz)) {
                let p = s.points.get(i).map_or(Vec3::ZERO, |p| p.p);
                if !occluded_by_guide(scene, view, p) {
                    best = Some((d, depth, s.id));
                }
            }
        }
        if single && let Some(Some(a)) = sp.first() {
            let d = ((at[0] - a.0).powi(2) + (at[1] - a.1).powi(2)).sqrt();
            if d <= radius && best.is_none_or(|(bd, _, _)| d < bd) {
                best = Some((d, a.2, s.id));
            }
        }
    }
    best.map(|(_, _, id)| id)
}

/// Curves with any part within `radius` of a screen point.
pub fn strokes_near(scene: &Scene, view: &View, at: [f32; 2], radius: f32) -> Vec<u64> {
    let mut out = Vec::new();
    for s in candidates(scene, false) {
        let sp = screen_points(view, s);
        let hit = if sp.len() == 1 {
            sp[0].is_some_and(|a| ((at[0] - a.0).powi(2) + (at[1] - a.1).powi(2)).sqrt() <= radius)
        } else {
            sp.windows(2).enumerate().any(|(i, w)| match (w[0], w[1]) {
                (Some(a), Some(b)) => {
                    seg_dist(at, (a.0, a.1), (b.0, b.1)) <= radius && !s.points.get(i).is_some_and(|p| occluded_by_guide(scene, view, p.p))
                }
                _ => false,
            })
        };
        if hit {
            out.push(s.id);
        }
    }
    out
}

fn inside(p: (f32, f32), poly: &[[f32; 2]]) -> bool {
    let mut c = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + n - 1) % n]);
        if (a[1] > p.1) != (b[1] > p.1) && p.0 < (b[0] - a[0]) * (p.1 - a[1]) / (b[1] - a[1]) + a[0] {
            c = !c;
        }
    }
    c
}

/// Curves with a point inside a screen polygon (box and lasso select).
pub fn strokes_in_polygon(scene: &Scene, view: &View, poly: &[[f32; 2]]) -> Vec<u64> {
    if poly.len() < 3 {
        return Vec::new();
    }
    candidates(scene, false)
        .filter(|s| {
            s.points.iter().any(|p| view.project(p.p).is_some_and(|q| inside((q.x, q.y), poly)) && !occluded_by_guide(scene, view, p.p))
        })
        .map(|s| s.id)
        .collect()
}

/// Feather's Erase: remove the curve points under the eraser (on the curve's centre line),
/// splitting curves where they break. Returns whether anything changed.
pub fn erase_points(scene: &mut Scene, view: &View, at: [f32; 2], radius: f32) -> bool {
    let mut changed = false;
    let mut out: Vec<Arc<Stroke>> = Vec::with_capacity(scene.strokes.len());
    let strokes = std::mem::take(&mut scene.strokes);
    for s in strokes {
        if !scene.group_shown(s.group) {
            out.push(s);
            continue;
        }
        let gone: Vec<bool> = s
            .points
            .iter()
            .map(|p| {
                view.project(p.p).is_some_and(|q| ((q.x - at[0]).powi(2) + (q.y - at[1]).powi(2)).sqrt() <= radius) && !occluded_by_guide(scene, view, p.p)
            })
            .collect();
        if !gone.iter().any(|g| *g) {
            out.push(s);
            continue;
        }
        changed = true;
        let mut piece: Vec<crate::model::Point> = Vec::new();
        let mut pieces: Vec<Vec<crate::model::Point>> = Vec::new();
        for (p, g) in s.points.iter().zip(&gone) {
            if *g {
                if !piece.is_empty() {
                    pieces.push(std::mem::take(&mut piece));
                }
            } else {
                piece.push(*p);
            }
        }
        if !piece.is_empty() {
            pieces.push(piece);
        }
        for (k, pts) in pieces.into_iter().enumerate() {
            let id = if k == 0 { s.id } else { scene.alloc_id() };
            out.push(Arc::new(Stroke { id, points: pts, seed: if k == 0 { s.seed } else { hash(s.seed ^ id as u32) }, ..(*s).clone() }));
        }
    }
    scene.strokes = out;
    changed
}

/// Feather's Vacuum: remove every curve the eraser touches.
pub fn vacuum(scene: &mut Scene, view: &View, at: [f32; 2], radius: f32) -> bool {
    let hit: HashSet<u64> = strokes_near(scene, view, at, radius).into_iter().collect();
    if hit.is_empty() {
        return false;
    }
    scene.strokes.retain(|s| !hit.contains(&s.id));
    true
}

/// Remove curves by id.
pub fn delete_strokes(scene: &mut Scene, ids: &HashSet<u64>) -> usize {
    let before = scene.strokes.len();
    scene.strokes.retain(|s| !ids.contains(&s.id));
    before - scene.strokes.len()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiquifyKind {
    /// Push and pull naturally.
    Push,
    /// Pull towards the brush centre (or away with `inverse`): sharp extend and shrink.
    Pinch,
    /// Smooth and straighten with a rubbing motion.
    Comb,
}

impl LiquifyKind {
    pub fn parse(s: &str) -> Option<LiquifyKind> {
        match s.to_ascii_lowercase().as_str() {
            "push" => Some(LiquifyKind::Push),
            "pinch" => Some(LiquifyKind::Pinch),
            "comb" => Some(LiquifyKind::Comb),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            LiquifyKind::Push => "push",
            LiquifyKind::Pinch => "pinch",
            LiquifyKind::Comb => "comb",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Liquify {
    pub kind: LiquifyKind,
    /// Brush radius in screen pixels (the same on screen at any zoom).
    pub size: f32,
    /// The full-strength inner circle, as a fraction of the size, 0..1.
    pub range: f32,
    /// 0..1
    pub strength: f32,
    pub inverse: bool,
}

impl Default for Liquify {
    fn default() -> Self {
        Liquify { kind: LiquifyKind::Push, size: 80.0, range: 0.4, strength: 0.6, inverse: false }
    }
}

impl Liquify {
    pub fn sanitize(&mut self) {
        self.size = if self.size.is_finite() { self.size.clamp(4.0, 1000.0) } else { 80.0 };
        self.range = if self.range.is_finite() { self.range.clamp(0.0, 1.0) } else { 0.4 };
        self.strength = if self.strength.is_finite() { self.strength.clamp(0.0, 1.0) } else { 0.6 };
    }

    fn falloff(&self, d: f32) -> f32 {
        let inner = self.size * self.range;
        if d <= inner {
            1.0
        } else if d >= self.size {
            0.0
        } else {
            let t = (d - inner) / (self.size - inner).max(1e-6);
            let s = 1.0 - t;
            s * s * (3.0 - 2.0 * s)
        }
    }
}

/// One liquify step on the given curves: the brush at `at`, moved by `delta` pixels.
pub fn liquify(scene: &mut Scene, view: &View, ids: &HashSet<u64>, at: [f32; 2], delta: [f32; 2], settings: &Liquify) -> bool {
    let mut st = *settings;
    st.sanitize();
    if !(at[0].is_finite() && at[1].is_finite() && delta[0].is_finite() && delta[1].is_finite()) {
        return false;
    }
    let mut changed = false;
    for s in scene.strokes.iter_mut().filter(|s| ids.contains(&s.id)) {
        let weights: Vec<Option<(f32, f32, f32, f32)>> = s
            .points
            .iter()
            .map(|p| {
                view.project(p.p).and_then(|q| {
                    let d = ((q.x - at[0]).powi(2) + (q.y - at[1]).powi(2)).sqrt();
                    let w = st.falloff(d) * st.strength;
                    (w > 0.0).then_some((w, q.x, q.y, q.depth))
                })
            })
            .collect();
        if weights.iter().all(Option::is_none) {
            continue;
        }
        changed = true;
        let s = Arc::make_mut(s);
        let before: Vec<Vec3> = s.points.iter().map(|p| p.p).collect();
        let n = before.len();
        for (i, w) in weights.iter().enumerate() {
            let Some((w, x, y, depth)) = *w else { continue };
            let wpp = view.world_per_px(depth);
            let shift = match st.kind {
                LiquifyKind::Push => (view.right * delta[0] - view.up * delta[1]) * (wpp * w),
                LiquifyKind::Pinch => {
                    let sign = if st.inverse { -1.0 } else { 1.0 };
                    let move_px = ((delta[0] * delta[0] + delta[1] * delta[1]).sqrt()).max(1.0);
                    let (dx, dy) = (at[0] - x, at[1] - y);
                    let k = (move_px * 0.02).min(0.5) * w * sign;
                    (view.right * dx - view.up * dy) * (wpp * k)
                }
                LiquifyKind::Comb => {
                    if i == 0 || i + 1 >= n {
                        Vec3::ZERO
                    } else {
                        let avg = (before[i - 1] + before[i + 1]) * 0.5;
                        (avg - before[i]) * (w * 0.6)
                    }
                }
            };
            if let Some(p) = s.points.get_mut(i)
                && shift.is_finite()
            {
                p.p += shift;
            }
        }
    }
    changed
}

/// Copies of curves with new ids, each point mapped by `f` (identity for an in-place copy).
pub fn duplicate(scene: &mut Scene, ids: &HashSet<u64>, f: impl Fn(Vec3) -> Vec3, nf: impl Fn(Vec3) -> Vec3) -> Vec<u64> {
    let src: Vec<Arc<Stroke>> = scene.strokes.iter().filter(|s| ids.contains(&s.id)).cloned().collect();
    let mut new_ids = Vec::new();
    for s in src {
        let id = scene.alloc_id();
        let mut c = (*s).clone();
        c.id = id;
        c.seed = hash(s.seed ^ (id as u32).wrapping_mul(0x9e37_79b9));
        for p in &mut c.points {
            p.p = f(p.p);
            p.n = nf(p.n);
        }
        scene.strokes.push(Arc::new(c));
        new_ids.push(id);
    }
    new_ids
}

/// Feather's "duplicate symmetrically by view": mirror across the vertical plane through the
/// orbit point, as seen from the current view.
pub fn view_mirror(view: &View, centre: Vec3) -> (impl Fn(Vec3) -> Vec3 + use<>, impl Fn(Vec3) -> Vec3 + use<>) {
    let n = view.right;
    let f = move |p: Vec3| p - n * (2.0 * (p - centre).dot(n));
    let nf = move |v: Vec3| v - n * (2.0 * v.dot(n));
    (f, nf)
}

/// How a closed curve is filled.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fill {
    /// Distance between strokes (world units).
    pub spacing: f32,
    /// Degrees, measured on the curve's plane from the view's horizontal.
    pub angle: f32,
    /// One back-and-forth stroke instead of separate hatches.
    pub zigzag: bool,
    /// Random wander of each stroke, as a fraction of the spacing, 0..1.
    pub jitter: f32,
}

/// Most strokes one fill makes.
pub const FILL_STROKES_MAX: usize = 4000;

/// Fill the inside of a closed curve with strokes lying on its best-fit plane. Returns the
/// curves as point lists (with the plane's normal on every point).
pub fn fill_curve(points: &[Vec3], view: &View, f: &Fill, seed: u32) -> Result<Vec<Vec<crate::model::Point>>, String> {
    let pts: Vec<Vec3> = points.iter().copied().filter(|p| p.is_finite()).collect();
    if pts.len() < 3 {
        return Err("fill needs a closed curve (draw a loop)".into());
    }
    let len: f32 = pts.windows(2).map(|w| w[0].distance(w[1])).sum();
    let (first, last) = (pts[0], pts[pts.len() - 1]);
    if first.distance(last) > len * 0.2 {
        return Err("the curve is not closed: draw a loop that ends near where it started".into());
    }
    // Newell's normal and the centroid.
    let mut n = Vec3::ZERO;
    let mut c = Vec3::ZERO;
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        n += v3((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
        c += a;
    }
    let n = n.normalized();
    if n == Vec3::ZERO {
        return Err("the curve is too flat to fill (it encloses no area)".into());
    }
    let c = c / pts.len() as f32;
    // In-plane axes: the view's horizontal laid on the plane, turned by the angle.
    let mut u = (view.right - n * view.right.dot(n)).normalized();
    if u == Vec3::ZERO {
        u = n.any_perpendicular();
    }
    let rot = crate::math::Quat::from_axis_angle(n, if f.angle.is_finite() { f.angle.to_radians() } else { 0.0 });
    let u = rot.rotate(u).normalized();
    let v = n.cross(u).normalized();
    let poly: Vec<(f32, f32)> = pts.iter().map(|p| ((*p - c).dot(u), (*p - c).dot(v))).collect();
    let spacing = if f.spacing.is_finite() && f.spacing > 0.0 { f.spacing } else { return Err("spacing must be above zero".into()) };
    let (ymin, ymax) = poly.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
    let rows = ((ymax - ymin) / spacing).ceil();
    if !rows.is_finite() || rows as usize > FILL_STROKES_MAX {
        return Err("that would be too many strokes: make the brush bigger".into());
    }
    let jitter = if f.jitter.is_finite() { f.jitter.clamp(0.0, 1.0) } else { 0.0 };
    let mut segments: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut k = 0u32;
    let mut y = ymin + spacing * 0.5;
    while y < ymax {
        let mut xs: Vec<f32> = Vec::new();
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            if (a.1 > y) != (b.1 > y) {
                let t = (y - a.1) / (b.1 - a.1);
                xs.push(a.0 + (b.0 - a.0) * t);
            }
        }
        xs.sort_by(f32::total_cmp);
        for pair in xs.chunks_exact(2) {
            let (x0, x1) = (pair[0], pair[1]);
            if x1 - x0 < spacing * 0.2 {
                continue;
            }
            let steps = (((x1 - x0) / (spacing * 0.35)).ceil() as usize).clamp(2, 400);
            let line: Vec<(f32, f32)> = (0..=steps)
                .map(|i| {
                    let t = i as f32 / steps as f32;
                    k = k.wrapping_add(1);
                    let wob = crate::noise::value1(seed ^ 0x51f1, k as f32 * 0.37) * jitter * spacing * 0.6;
                    (x0 + (x1 - x0) * t, y + wob)
                })
                .collect();
            segments.push(line);
        }
        y += spacing;
    }
    if segments.is_empty() {
        return Err("nothing inside the curve to fill".into());
    }
    let to3 = |(x, y): (f32, f32)| crate::model::Point { p: c + u * x + v * y, pressure: 1.0, n };
    if f.zigzag {
        // One stroke, back and forth.
        let mut all: Vec<crate::model::Point> = Vec::new();
        for (i, seg) in segments.iter().enumerate() {
            if i % 2 == 0 {
                all.extend(seg.iter().map(|p| to3(*p)));
            } else {
                all.extend(seg.iter().rev().map(|p| to3(*p)));
            }
        }
        return Ok(vec![all]);
    }
    Ok(segments.into_iter().map(|seg| seg.into_iter().map(to3).collect()).collect())
}

/// Draw-on targets other than guides: the first active image (a bounded flat guide) or model.
pub fn raycast_resources(scene: &Scene, origin: Vec3, dir: Vec3, only_active: bool) -> Option<(f32, Vec3, Vec3, u64)> {
    let mut best: Option<(f32, Vec3, Vec3, u64)> = None;
    let ok = |s: ResourceState| if only_active { s == ResourceState::Active } else { s != ResourceState::Hidden };
    for im in scene.images.iter().filter(|i| ok(i.state)) {
        let c = im.corners();
        for (a, b, cc) in [(c[0], c[1], c[2]), (c[0], c[2], c[3])] {
            if let Some((t, _, _)) = ray_triangle(origin, dir, a, b, cc)
                && best.is_none_or(|bb| t < bb.0)
            {
                let mut n = (b - a).cross(cc - a).normalized();
                if n.dot(dir) > 0.0 {
                    n = -n;
                }
                best = Some((t, origin + dir * t, n, im.id));
            }
        }
    }
    for m in scene.models.iter().filter(|m| ok(m.state)) {
        let world: Vec<Vec3> = m.positions.iter().map(|p| m.xform.apply(*p)).collect();
        let (mut lo, mut hi) = (v3(f32::INFINITY, f32::INFINITY, f32::INFINITY), v3(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY));
        for p in &world {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        if !ray_hits_box(origin, dir, lo, hi) {
            continue;
        }
        for t in &m.triangles {
            let (Some(a), Some(b), Some(c)) = (world.get(t[0] as usize), world.get(t[1] as usize), world.get(t[2] as usize)) else { continue };
            if let Some((tt, _, _)) = ray_triangle(origin, dir, *a, *b, *c)
                && best.is_none_or(|bb| tt < bb.0)
            {
                let mut n = (*b - *a).cross(*c - *a).normalized();
                if n.dot(dir) > 0.0 {
                    n = -n;
                }
                best = Some((tt, origin + dir * tt, n, m.id));
            }
        }
    }
    best
}

/// The resource (guide, image or model) under a screen point, nearest first.
pub fn pick_resource(scene: &Scene, view: &View, at: [f32; 2]) -> Option<u64> {
    let (o, d) = view.ray(at[0], at[1]);
    let mut best: Option<(f32, u64)> = raycast_resources(scene, o, d, false).map(|(t, _, _, id)| (t, id));
    for g in &scene.guides {
        if scene.guide_state(g.id) == ResourceState::Hidden {
            continue;
        }
        if let Some(h) = g.raycast(o, d)
            && best.is_none_or(|(t, _)| h.t < t)
        {
            best = Some((h.t, g.id));
        }
    }
    best.map(|(_, id)| id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{Camera, PerfectView};
    use crate::guide::{Guide, Primitive};
    use crate::model::{Brush, Point};

    fn front() -> View {
        let mut c = Camera::default();
        c.snap(PerfectView::Front);
        c.view()
    }

    fn scene_with_line() -> Scene {
        let mut s = Scene::default();
        s.strokes.push(Arc::new(Stroke {
            id: 10,
            group: 1,
            points: (0..21).map(|i| Point { p: v3(i as f32 * 0.1 - 1.0, 0.0, 0.0), pressure: 1.0, n: Vec3::ZERO }).collect(),
            brush: Brush::default(),
            seed: 3,
        }));
        s.next_id = 11;
        s
    }

    #[test]
    fn erase_splits_and_vacuum_removes() {
        let v = front();
        let mut s = scene_with_line();
        let c = v.project(Vec3::ZERO).expect("centre");
        assert!(erase_points(&mut s, &v, [c.x, c.y], 5.0));
        assert_eq!(s.strokes.len(), 2, "split in two");
        assert!(s.strokes.iter().all(|st| st.points.iter().all(|p| p.p.x.abs() > 0.01)));
        assert_ne!(s.strokes[0].id, s.strokes[1].id);
        assert!(!erase_points(&mut s, &v, [5.0, 5.0], 5.0));
        let left = v.project(v3(-0.8, 0.0, 0.0)).expect("left");
        assert!(vacuum(&mut s, &v, [left.x, left.y], 5.0));
        assert_eq!(s.strokes.len(), 1);
    }

    #[test]
    fn guides_protect_what_they_cover() {
        let v = front();
        let mut s = scene_with_line();
        // A cube guide in front of the line (between it and the camera).
        let mut g = Guide::primitive(50, Primitive::Plane, 4).expect("plane");
        g.xform.rotation = crate::math::Quat::from_axis_angle(Vec3::X, std::f32::consts::FRAC_PI_2);
        g.xform.translation = v3(0.0, 0.0, 1.0);
        g.rebuild().expect("rebuild");
        s.guides.push(Arc::new(g));
        s.active_guide = Some(50);
        let c = v.project(Vec3::ZERO).expect("centre");
        assert!(pick_stroke(&s, &v, [c.x, c.y], 6.0, false).is_none());
        assert!(!erase_points(&mut s, &v, [c.x, c.y], 6.0));
        // A transparent guide protects nothing.
        if let Some(g) = s.guides.first_mut() {
            Arc::make_mut(g).opacity = 0.0;
        }
        assert_eq!(pick_stroke(&s, &v, [c.x, c.y], 6.0, false), Some(10));
    }

    #[test]
    fn liquify_push_moves_only_what_is_under_the_brush() {
        let v = front();
        let mut s = scene_with_line();
        let ids: HashSet<u64> = [10].into();
        let c = v.project(Vec3::ZERO).expect("centre");
        let st = Liquify { size: 30.0, range: 0.5, strength: 1.0, ..Liquify::default() };
        assert!(liquify(&mut s, &v, &ids, [c.x, c.y], [0.0, -20.0], &st));
        let pts = &s.strokes[0].points;
        assert!(pts[10].p.y > 0.0, "the middle went up");
        assert_eq!(pts[0].p.y, 0.0, "the ends stayed");
        let mut comb = Liquify { kind: LiquifyKind::Comb, ..st };
        comb.size = 1e9;
        assert!(liquify(&mut s, &v, &ids, [c.x, c.y], [1.0, 0.0], &comb));
        assert!(!liquify(&mut s, &v, &ids, [f32::NAN, 0.0], [0.0, 0.0], &st));
    }

    #[test]
    fn duplicates_get_new_ids_and_mirror_by_view() {
        let v = front();
        let mut s = scene_with_line();
        for p in &mut Arc::make_mut(&mut s.strokes[0]).points {
            p.p.x += 3.0;
        }
        let ids: HashSet<u64> = [10].into();
        let (f, nf) = view_mirror(&v, Vec3::ZERO);
        let new = duplicate(&mut s, &ids, f, nf);
        assert_eq!(new.len(), 1);
        let copy = s.stroke(new[0]).expect("copy");
        assert!(copy.points.iter().all(|p| p.p.x < 0.0), "mirrored to the other side");
        assert_eq!(delete_strokes(&mut s, &ids), 1);
    }

    #[test]
    fn closed_curves_fill_with_strokes_on_their_plane() {
        let v = front();
        let square: Vec<Vec3> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0)]
            .iter()
            .flat_map(|(x, y)| std::iter::once(v3(*x, *y, 0.5)))
            .collect();
        let f = Fill { spacing: 0.2, angle: 0.0, zigzag: false, jitter: 0.0 };
        let made = fill_curve(&square, &v, &f, 1).expect("fill");
        assert_eq!(made.len(), 10);
        for c in &made {
            for p in c {
                assert!((p.p.z - 0.5).abs() < 1e-4 && p.p.x.abs() <= 1.0 + 1e-4 && p.n.z.abs() > 0.99);
            }
        }
        let zig = fill_curve(&square, &v, &Fill { zigzag: true, ..f }, 1).expect("zigzag");
        assert_eq!(zig.len(), 1);
        assert!(fill_curve(&square[..2], &v, &f, 1).is_err());
        let open: Vec<Vec3> = (0..10).map(|i| v3(i as f32, 0.0, 0.0)).collect();
        assert!(fill_curve(&open, &v, &f, 1).is_err());
        assert!(fill_curve(&square, &v, &Fill { spacing: 1e-9, ..f }, 1).is_err());
        assert!(fill_curve(&square, &v, &Fill { spacing: f32::NAN, ..f }, 1).is_err());
    }

    #[test]
    fn box_select_and_resource_pick() {
        let v = front();
        let mut s = scene_with_line();
        let c = v.project(Vec3::ZERO).expect("centre");
        let poly = [[c.x - 10.0, c.y - 10.0], [c.x + 10.0, c.y - 10.0], [c.x + 10.0, c.y + 10.0], [c.x - 10.0, c.y + 10.0]];
        assert_eq!(strokes_in_polygon(&s, &v, &poly), vec![10]);
        assert!(strokes_in_polygon(&s, &v, &poly[..2]).is_empty());
        let g = Guide::primitive(60, Primitive::Sphere, 12).expect("sphere");
        s.guides.push(Arc::new(g));
        s.set_guide_state(60, ResourceState::Visible);
        assert_eq!(pick_resource(&s, &v, [c.x, c.y]), Some(60));
    }
}
