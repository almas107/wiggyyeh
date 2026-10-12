//! Filled surfaces: what Fill, Lasso Fill, the filled Shapes and solid primitives make.
//!
//! A fill is a [`Stroke`](crate::model::Stroke) with triangles: a patch of the surface it was
//! made on (a guide, a reference image or model, or the drawing plane), meshed finely enough to
//! follow a curved guide. It is built from a screen outline: triangulated on screen (ear
//! clipping), refined until no edge is longer than a few pixels on a curved surface (uniform
//! 1 → 4 splits, so neighbouring triangles always share their new corners and nothing cracks),
//! and every corner is cast onto the surface under it.
//!
//! The bucket fill finds its outline the way a 2D paint bucket does: the curves on screen are
//! drawn into a coarse mask, the empty region around the click is flooded, grown a little so
//! the paint tucks under the lines, and its outer edge is traced.

use crate::math::{Vec3, v3};
use crate::model::Point;

/// Most outline corners kept (longer outlines are thinned).
pub const OUTLINE_MAX: usize = 1200;
/// Most triangles one fill may have.
pub const TRIS_MAX: usize = 120_000;
/// Mask cell size for the bucket fill, in screen pixels.
pub const BUCKET_CELL: f32 = 3.0;
/// Biggest bucket mask side, in cells.
const BUCKET_SIDE_MAX: usize = 1400;

/// A screen polygon cleaned up for triangulation: finite, no repeated or closing points, at
/// most [`OUTLINE_MAX`] corners, counter-clockwise (in screen coordinates, y down).
pub fn clean_outline(raw: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut pts: Vec<[f32; 2]> = Vec::with_capacity(raw.len());
    for p in raw.iter().filter(|p| p[0].is_finite() && p[1].is_finite()) {
        if pts.last().is_none_or(|l| dist(*l, *p) >= 0.75) {
            pts.push(*p);
        }
    }
    while pts.len() > 2 && pts.first().zip(pts.last()).is_some_and(|(a, b)| dist(*a, *b) < 0.75) {
        pts.pop();
    }
    if pts.len() > OUTLINE_MAX {
        let step = pts.len().div_ceil(OUTLINE_MAX);
        pts = pts.into_iter().step_by(step).collect();
    }
    if signed_area(&pts) < 0.0 {
        pts.reverse();
    }
    pts
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Shoelace area (positive for the winding [`clean_outline`] keeps).
pub fn signed_area(p: &[[f32; 2]]) -> f32 {
    let n = p.len();
    if n < 3 {
        return 0.0;
    }
    let mut a = 0.0;
    for i in 0..n {
        let (q, r) = (p[i], p[(i + 1) % n]);
        a += q[0] * r[1] - r[0] * q[1];
    }
    a * 0.5
}

fn cross(o: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn in_triangle(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
    cross(a, b, p) >= 0.0 && cross(b, c, p) >= 0.0 && cross(c, a, p) >= 0.0
}

/// Ear clipping of a [`clean_outline`] polygon: triangles of indices into it. A polygon that
/// crosses itself (a scribbled lasso) still gives triangles: when no clean ear is left, the
/// flattest corner is cut anyway.
pub fn triangulate(p: &[[f32; 2]]) -> Vec<[u32; 3]> {
    let n = p.len();
    let mut out = Vec::with_capacity(n.saturating_sub(2));
    if n < 3 {
        return out;
    }
    let mut idx: Vec<usize> = (0..n).collect();
    let mut guard = 0usize;
    while idx.len() > 3 && guard < n * n + 8 {
        guard += 1;
        let m = idx.len();
        let mut cut = None;
        for k in 0..m {
            let (ia, ib, ic) = (idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]);
            let (a, b, c) = (p[ia], p[ib], p[ic]);
            if cross(a, b, c) <= 0.0 {
                continue;
            }
            let blocked = idx.iter().any(|&j| j != ia && j != ib && j != ic && in_triangle(p[j], a, b, c));
            if !blocked {
                cut = Some(k);
                break;
            }
        }
        let k = cut.unwrap_or_else(|| {
            // No clean ear (self-crossing outline): cut the most convex corner.
            (0..m)
                .max_by(|&x, &y| {
                    let f = |k: usize| cross(p[idx[(k + m - 1) % m]], p[idx[k]], p[idx[(k + 1) % m]]);
                    f(x).total_cmp(&f(y))
                })
                .unwrap_or(0)
        });
        let (ia, ib, ic) = (idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]);
        if cross(p[ia], p[ib], p[ic]).abs() > 1e-6 {
            out.push([ia as u32, ib as u32, ic as u32]);
        }
        idx.remove(k);
    }
    if idx.len() == 3 && cross(p[idx[0]], p[idx[1]], p[idx[2]]).abs() > 1e-6 {
        out.push([idx[0] as u32, idx[1] as u32, idx[2] as u32]);
    }
    out
}

/// Split every triangle into four (shared edges split once, at the same new corner) `levels`
/// times.
pub fn refine(verts: &mut Vec<[f32; 2]>, tris: &mut Vec<[u32; 3]>, levels: u32) {
    for _ in 0..levels {
        if tris.len().saturating_mul(4) > TRIS_MAX {
            return;
        }
        let mut mids: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
        let mut mid = |a: u32, b: u32, verts: &mut Vec<[f32; 2]>| -> u32 {
            let key = (a.min(b), a.max(b));
            *mids.entry(key).or_insert_with(|| {
                let (pa, pb) = (verts[a as usize], verts[b as usize]);
                verts.push([(pa[0] + pb[0]) * 0.5, (pa[1] + pb[1]) * 0.5]);
                (verts.len() - 1) as u32
            })
        };
        let mut next = Vec::with_capacity(tris.len() * 4);
        for t in tris.iter() {
            let [a, b, c] = *t;
            let (ab, bc, ca) = (mid(a, b, verts), mid(b, c, verts), mid(c, a, verts));
            next.extend_from_slice(&[[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]);
        }
        *tris = next;
    }
}

/// How many 1 → 4 splits bring the longest edge under `spacing` pixels (at most 4).
pub fn levels_for(verts: &[[f32; 2]], tris: &[[u32; 3]], spacing: f32) -> u32 {
    let longest = tris
        .iter()
        .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
        .filter_map(|(a, b)| Some(dist(*verts.get(a as usize)?, *verts.get(b as usize)?)))
        .fold(0.0f32, f32::max);
    let spacing = if spacing.is_finite() { spacing.max(2.0) } else { 24.0 };
    let mut levels = 0;
    let mut l = longest;
    while l > spacing && levels < 4 {
        l *= 0.5;
        levels += 1;
    }
    levels
}

/// A filled patch from a screen outline: triangulated, refined `levels` times, and every corner
/// cast onto the surface with `cast` (screen point → point and normal). Corners that miss
/// land on the plane that fits the ones that hit. Errors when nothing hits.
pub fn patch(
    outline: &[[f32; 2]],
    levels: u32,
    cast: impl Fn(f32, f32) -> Option<(Vec3, Vec3)>,
    ray: impl Fn(f32, f32) -> (Vec3, Vec3),
) -> Result<(Vec<Point>, Vec<[u32; 3]>), String> {
    let mut verts = clean_outline(outline);
    if verts.len() < 3 || signed_area(&verts).abs() < 4.0 {
        return Err("the outline is too small to fill".into());
    }
    let mut tris = triangulate(&verts);
    if tris.is_empty() {
        return Err("the outline has no area to fill".into());
    }
    refine(&mut verts, &mut tris, levels);
    let hits: Vec<Option<(Vec3, Vec3)>> = verts.iter().map(|v| cast(v[0], v[1])).collect();
    let (plane_p, plane_n) =
        fit_plane(&hits.iter().flatten().copied().collect::<Vec<_>>()).ok_or("nothing to fill on here: draw on a guide, or turn on drawing in the air")?;
    let points: Vec<Point> = verts
        .iter()
        .zip(&hits)
        .map(|(v, h)| match h {
            Some((p, n)) => Point { p: *p, pressure: 1.0, n: *n },
            None => {
                let (o, d) = ray(v[0], v[1]);
                let den = d.dot(plane_n);
                let t = if den.abs() > 1e-6 { (plane_p - o).dot(plane_n) / den } else { 0.0 };
                Point { p: o + d * t.max(0.0), pressure: 1.0, n: plane_n }
            }
        })
        .collect();
    if points.iter().any(|p| !p.p.is_finite()) {
        return Err("the fill could not be placed on the surface".into());
    }
    Ok((points, tris))
}

/// The plane through points with normals: their mean point and mean normal (the normals say
/// which way the surface faces; a curved patch gets its average plane).
pub fn fit_plane(hits: &[(Vec3, Vec3)]) -> Option<(Vec3, Vec3)> {
    if hits.is_empty() {
        return None;
    }
    let k = 1.0 / hits.len() as f32;
    let c = hits.iter().fold(Vec3::ZERO, |a, (p, _)| a + *p) * k;
    let n = hits.iter().fold(Vec3::ZERO, |a, (_, n)| a + *n);
    let n = if n.length() > 1e-6 { n.normalized() } else { v3(0.0, 0.0, 1.0) };
    (c.is_finite() && n.is_finite()).then_some((c, n))
}

/// The plane best fitting a set of points (no normals): through their centre, normal along
/// their least spread (power iteration on the covariance's inverse spread). `None` when they
/// are too few or lie on a line.
pub fn plane_of_points(pts: &[Vec3]) -> Option<(Vec3, Vec3)> {
    if pts.len() < 3 {
        return None;
    }
    let k = 1.0 / pts.len() as f32;
    let c = pts.iter().fold(Vec3::ZERO, |a, p| a + *p) * k;
    let mut m = [[0.0f32; 3]; 3];
    for p in pts {
        let d = (*p - c).to_array();
        for (i, row) in m.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v += d[i] * d[j];
            }
        }
    }
    // The normal is the eigenvector of the smallest eigenvalue: the largest of (trace·I − M).
    let tr = m[0][0] + m[1][1] + m[2][2];
    if !(tr.is_finite() && tr > 1e-12) {
        return None;
    }
    let a = [[tr - m[0][0], -m[0][1], -m[0][2]], [-m[1][0], tr - m[1][1], -m[1][2]], [-m[2][0], -m[2][1], tr - m[2][2]]];
    let mut v = [0.577f32, 0.577, 0.577];
    for _ in 0..64 {
        let w = [
            a[0][0] * v[0] + a[0][1] * v[1] + a[0][2] * v[2],
            a[1][0] * v[0] + a[1][1] * v[1] + a[1][2] * v[2],
            a[2][0] * v[0] + a[2][1] * v[1] + a[2][2] * v[2],
        ];
        let l = (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
        if !(l.is_finite() && l > 1e-20) {
            return None;
        }
        v = [w[0] / l, w[1] / l, w[2] / l];
    }
    let n = v3(v[0], v[1], v[2]);
    // Points along a line have no plane.
    let spread = |dir: Vec3| pts.iter().map(|p| (*p - c).dot(dir).powi(2)).sum::<f32>();
    let along = n.any_perpendicular();
    let across = n.cross(along).normalized();
    if spread(along).min(spread(across)) < 1e-10 {
        return None;
    }
    Some((c, n))
}

/// The bucket's region: the cells reachable from `seed` without crossing a wall, or `None`
/// when the flood leaks out of the view (the shape is not closed).
pub fn flood(walls: &[bool], w: usize, h: usize, seed: (usize, usize)) -> Option<Vec<bool>> {
    if w == 0 || h == 0 || walls.len() < w * h || seed.0 >= w || seed.1 >= h {
        return None;
    }
    let mut inside = vec![false; w * h];
    let start = seed.1 * w + seed.0;
    if walls[start] {
        return None;
    }
    let mut stack = vec![start];
    inside[start] = true;
    while let Some(i) = stack.pop() {
        let (x, y) = (i % w, i / w);
        if x == 0 || y == 0 || x + 1 == w || y + 1 == h {
            return None;
        }
        for j in [i - 1, i + 1, i - w, i + w] {
            if !walls[j] && !inside[j] {
                inside[j] = true;
                stack.push(j);
            }
        }
    }
    Some(inside)
}

/// Grow a region by `r` cells (square), staying out of nothing: the paint reaches under the
/// lines around it.
pub fn grow(region: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    let mut out = region.to_vec();
    if r == 0 {
        return out;
    }
    for y in 0..h {
        for x in 0..w {
            if !region[y * w + x] {
                continue;
            }
            for yy in y.saturating_sub(r)..(y + r + 1).min(h) {
                for xx in x.saturating_sub(r)..(x + r + 1).min(w) {
                    out[yy * w + xx] = true;
                }
            }
        }
    }
    out
}

/// The outer edge of a region as a polygon of cell corners (counter-clockwise on screen),
/// traced with marching squares from its top-left cell, then thinned.
pub fn trace(region: &[bool], w: usize, h: usize) -> Vec<[f32; 2]> {
    let at = |x: isize, y: isize| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && region[y as usize * w + x as usize];
    let Some(first) = region.iter().position(|v| *v) else { return Vec::new() };
    // Corners are lattice points (x, y) between cells; walk with the region on the left.
    let (sx, sy) = ((first % w) as isize, (first / w) as isize);
    let mut out: Vec<[f32; 2]> = Vec::new();
    let (mut x, mut y) = (sx, sy);
    // Direction: 0 right, 1 down, 2 left, 3 up (screen). Start along the top edge going right
    // with the region below (on the right-hand side in screen terms, which is the left-hand
    // side for a y-up reader): keep the region on the walker's right.
    let mut dir = 0u8;
    let limit = (w + 1) * (h + 1) * 4;
    for _ in 0..limit {
        out.push([x as f32, y as f32]);
        // Cells around corner (x, y): tl (x-1,y-1), tr (x,y-1), bl (x-1,y), br (x,y).
        let (tl, tr, bl, br) = (at(x - 1, y - 1), at(x, y - 1), at(x - 1, y), at(x, y));
        // Turn so the region stays on the right of the direction of travel.
        let options: [u8; 3] = [(dir + 3) % 4, dir, (dir + 1) % 4];
        let open = |d: u8| match d {
            0 => br && !tr,
            1 => bl && !br,
            2 => tl && !bl,
            _ => tr && !tl,
        };
        let Some(d) = options.into_iter().find(|d| open(*d)).or_else(|| (0..4).find(|d| open(*d))) else { break };
        dir = d;
        match d {
            0 => x += 1,
            1 => y += 1,
            2 => x -= 1,
            _ => y -= 1,
        }
        if x == sx && y == sy {
            break;
        }
    }
    simplify_closed(&out, 0.8)
}

/// Douglas–Peucker on a closed polygon (split at its farthest pair).
fn simplify_closed(p: &[[f32; 2]], tol: f32) -> Vec<[f32; 2]> {
    if p.len() < 8 {
        return p.to_vec();
    }
    let far = (1..p.len()).max_by(|&a, &b| dist(p[0], p[a]).total_cmp(&dist(p[0], p[b]))).unwrap_or(p.len() / 2);
    let mut a = dp(&p[..=far], tol);
    let mut tail: Vec<[f32; 2]> = p[far..].to_vec();
    tail.push(p[0]);
    let b = dp(&tail, tol);
    a.pop();
    a.extend(b);
    a.pop();
    a
}

fn dp(p: &[[f32; 2]], tol: f32) -> Vec<[f32; 2]> {
    if p.len() < 3 {
        return p.to_vec();
    }
    let (a, b) = (p[0], p[p.len() - 1]);
    let seg = |q: [f32; 2]| {
        let (abx, aby) = (b[0] - a[0], b[1] - a[1]);
        let l2 = abx * abx + aby * aby;
        let t = if l2 > 1e-9 { (((q[0] - a[0]) * abx + (q[1] - a[1]) * aby) / l2).clamp(0.0, 1.0) } else { 0.0 };
        dist(q, [a[0] + abx * t, a[1] + aby * t])
    };
    let (k, d) = p.iter().enumerate().skip(1).take(p.len() - 2).map(|(i, q)| (i, seg(*q))).fold((0, 0.0f32), |acc, x| if x.1 > acc.1 { x } else { acc });
    if d <= tol {
        return vec![a, b];
    }
    let mut left = dp(&p[..=k], tol);
    let right = dp(&p[k..], tol);
    left.pop();
    left.extend(right);
    left
}

/// Draw a thick segment into a wall mask (cells of `cell` pixels).
pub fn wall_segment(walls: &mut [bool], w: usize, h: usize, cell: f32, a: [f32; 2], b: [f32; 2], radius_px: f32) {
    let r = (radius_px / cell).max(0.55);
    let (ax, ay, bx, by) = (a[0] / cell, a[1] / cell, b[0] / cell, b[1] / cell);
    if !(ax.is_finite() && ay.is_finite() && bx.is_finite() && by.is_finite()) {
        return;
    }
    let (lo_x, hi_x) = ((ax.min(bx) - r).floor().max(0.0), (ax.max(bx) + r).ceil().min(w as f32 - 1.0));
    let (lo_y, hi_y) = ((ay.min(by) - r).floor().max(0.0), (ay.max(by) + r).ceil().min(h as f32 - 1.0));
    if lo_x > hi_x || lo_y > hi_y {
        return;
    }
    let (abx, aby) = (bx - ax, by - ay);
    let l2 = abx * abx + aby * aby;
    for y in lo_y as usize..=hi_y as usize {
        for x in lo_x as usize..=hi_x as usize {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let t = if l2 > 1e-9 { (((px - ax) * abx + (py - ay) * aby) / l2).clamp(0.0, 1.0) } else { 0.0 };
            let d = ((px - ax - abx * t).powi(2) + (py - ay - aby * t).powi(2)).sqrt();
            if d <= r
                && let Some(c) = walls.get_mut(y * w + x)
            {
                *c = true;
            }
        }
    }
}

/// The bucket's mask size for a view, in cells.
pub fn bucket_grid(width: f32, height: f32) -> (usize, usize) {
    let f = |v: f32| if v.is_finite() { ((v / BUCKET_CELL).ceil() as usize).clamp(4, BUCKET_SIDE_MAX) } else { 4 };
    (f(width), f(height))
}

/// Rectangle, ellipse or regular polygon outlines for the Shapes tool, on screen.
pub fn shape_outline(kind: &str, a: [f32; 2], b: [f32; 2], sides: u32) -> Result<Vec<[f32; 2]>, String> {
    if !(a.iter().chain(&b).all(|v| v.is_finite())) {
        return Err("shape corners must be numbers".into());
    }
    let (x0, y0, x1, y1) = (a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1]));
    if x1 - x0 < 2.0 || y1 - y0 < 2.0 {
        return Err("drag out a bigger shape".into());
    }
    let (cx, cy, rx, ry) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5, (x1 - x0) * 0.5, (y1 - y0) * 0.5);
    match kind {
        "rect" | "rectangle" | "square" => Ok(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]),
        "ellipse" | "oval" | "circle" => {
            let n = ((rx.max(ry) * std::f32::consts::TAU / 4.0).ceil() as usize).clamp(24, 360);
            Ok((0..n)
                .map(|i| {
                    let t = std::f32::consts::TAU * i as f32 / n as f32;
                    [cx + rx * t.cos(), cy + ry * t.sin()]
                })
                .collect())
        }
        "polygon" | "triangle" | "star" => {
            let sides = if kind == "triangle" { 3 } else { sides.clamp(3, 32) } as usize;
            let star = kind == "star";
            let n = if star { sides * 2 } else { sides };
            Ok((0..n)
                .map(|i| {
                    let t = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * i as f32 / n as f32;
                    let k = if star && i % 2 == 1 { 0.45 } else { 1.0 };
                    [cx + rx * k * t.cos(), cy + ry * k * t.sin()]
                })
                .collect())
        }
        _ => Err(format!("unknown shape {kind:?} (rect, ellipse, polygon, triangle, star)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area_of(p: &[[f32; 2]], tris: &[[u32; 3]]) -> f32 {
        tris.iter().map(|t| cross(p[t[0] as usize], p[t[1] as usize], p[t[2] as usize]).abs() * 0.5).sum()
    }

    #[test]
    fn concave_outlines_triangulate_to_their_area() {
        // An L shape (concave), either winding.
        let l = vec![[0.0, 0.0], [100.0, 0.0], [100.0, 40.0], [40.0, 40.0], [40.0, 100.0], [0.0, 100.0]];
        for raw in [l.clone(), l.iter().rev().copied().collect()] {
            let p = clean_outline(&raw);
            let t = triangulate(&p);
            assert_eq!(t.len(), 4);
            assert!((area_of(&p, &t) - 100.0 * 40.0 - 40.0 * 60.0).abs() < 1e-2);
        }
        // A bow tie (crossing itself) still gives triangles and never loops forever.
        let bow = clean_outline(&[[0.0, 0.0], [100.0, 100.0], [100.0, 0.0], [0.0, 100.0]]);
        assert!(!triangulate(&bow).is_empty());
        assert!(triangulate(&[]).is_empty());
        assert!(clean_outline(&[[f32::NAN, 0.0], [1.0, 1.0]]).len() < 3);
    }

    #[test]
    fn refining_keeps_the_area_and_shares_corners() {
        let mut v = clean_outline(&[[0.0, 0.0], [90.0, 0.0], [90.0, 60.0], [0.0, 60.0]]);
        let mut t = triangulate(&v);
        let levels = levels_for(&v, &t, 20.0);
        assert_eq!(levels, 3);
        refine(&mut v, &mut t, levels);
        assert_eq!(t.len(), 2 * 64);
        assert!((area_of(&v, &t) - 5400.0).abs() < 0.5);
        // Conforming: the grid of a split square has (2^3 + 1)^2 corners, nothing duplicated.
        assert_eq!(v.len(), 81);
    }

    #[test]
    fn a_patch_lands_on_the_surface_and_falls_back_to_its_plane() {
        // A flat floor at y = 0 under a camera looking straight down; the right half misses.
        let ray = |x: f32, y: f32| (v3(x, 10.0, y), v3(0.0, -1.0, 0.0));
        let cast = |x: f32, y: f32| (x < 50.0).then_some((v3(x, 0.0, y), v3(0.0, 1.0, 0.0)));
        let (pts, tris) = patch(&[[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]], 2, cast, ray).expect("patch");
        assert!(!tris.is_empty());
        assert!(pts.iter().all(|p| p.p.y.abs() < 1e-4), "every corner on y = 0");
        assert!(patch(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], 0, cast, ray).is_err(), "too small");
        assert!(patch(&[[60.0, 0.0], [100.0, 0.0], [100.0, 100.0]], 0, cast, ray).is_err(), "nothing hit");
    }

    #[test]
    fn the_bucket_floods_closed_shapes_only() {
        let (w, h) = (40, 30);
        let mut walls = vec![false; w * h];
        let cell = 1.0;
        for (a, b) in [([5.0, 5.0], [30.0, 5.0]), ([30.0, 5.0], [30.0, 25.0]), ([30.0, 25.0], [5.0, 25.0]), ([5.0, 25.0], [5.0, 5.0])] {
            wall_segment(&mut walls, w, h, cell, a, b, 0.5);
        }
        let region = flood(&walls, w, h, (15, 15)).expect("closed box");
        let n = region.iter().filter(|v| **v).count();
        assert!((300..=500).contains(&n), "{n} cells");
        assert!(flood(&walls, w, h, (1, 1)).is_none(), "outside leaks to the border");
        let grown = grow(&region, w, h, 1);
        let outline = trace(&grown, w, h);
        assert!(outline.len() >= 4, "{outline:?}");
        let a = signed_area(&clean_outline(&outline));
        assert!(a > 400.0 && a < 700.0, "{a}");
        // Open the box: the flood leaks.
        let mut open = vec![false; w * h];
        wall_segment(&mut open, w, h, cell, [5.0, 5.0], [30.0, 5.0], 0.5);
        assert!(flood(&open, w, h, (15, 15)).is_none());
        assert!(flood(&[], 0, 0, (0, 0)).is_none());
    }

    #[test]
    fn shapes_have_outlines_and_bad_ones_are_errors() {
        assert_eq!(shape_outline("rect", [10.0, 10.0], [50.0, 30.0], 0).expect("rect").len(), 4);
        assert!(shape_outline("ellipse", [0.0, 0.0], [100.0, 60.0], 0).expect("ellipse").len() >= 24);
        assert_eq!(shape_outline("star", [0.0, 0.0], [100.0, 100.0], 5).expect("star").len(), 10);
        assert_eq!(shape_outline("polygon", [0.0, 0.0], [100.0, 100.0], 999).expect("poly").len(), 32);
        assert!(shape_outline("rect", [0.0, 0.0], [1.0, 1.0], 0).is_err());
        assert!(shape_outline("blob", [0.0, 0.0], [10.0, 10.0], 0).is_err());
        assert!(shape_outline("rect", [f32::NAN, 0.0], [10.0, 10.0], 0).is_err());
    }

    #[test]
    fn planes_fit_points() {
        let pts: Vec<Vec3> = (0..20).map(|i| v3((i % 5) as f32, 2.0, (i / 5) as f32)).collect();
        let (c, n) = plane_of_points(&pts).expect("plane");
        assert!((c.y - 2.0).abs() < 1e-4 && n.y.abs() > 0.999, "{n:?}");
        assert!(plane_of_points(&[v3(0.0, 0.0, 0.0), v3(1.0, 0.0, 0.0), v3(2.0, 0.0, 0.0)]).is_none(), "a line");
    }
}
