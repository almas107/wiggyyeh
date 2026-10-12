//! Filled surfaces in the editor: the Fill (bucket), Lasso Fill and Shapes tools, filling
//! selected closed curves, and solid primitives. Geometry lives in [`crate::fill`].

use std::sync::Arc;

use serde_json::{Value, json};

use super::{Editor, Event, Mods};
use crate::assist::{mirror_maps, reflect};
use crate::camera::View;
use crate::fill;
use crate::math::Vec3;
use crate::model::{Point, Stroke};
use crate::noise::hash;

/// Screen pixels between a fill's corners on a curved surface.
const CURVED_SPACING: f32 = 18.0;

/// What the Shapes tool draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeKind {
    Rect,
    Ellipse,
    Polygon,
    Star,
    Line,
}

impl ShapeKind {
    pub const ALL: [ShapeKind; 5] = [ShapeKind::Rect, ShapeKind::Ellipse, ShapeKind::Polygon, ShapeKind::Star, ShapeKind::Line];
    pub fn name(self) -> &'static str {
        match self {
            ShapeKind::Rect => "rect",
            ShapeKind::Ellipse => "ellipse",
            ShapeKind::Polygon => "polygon",
            ShapeKind::Star => "star",
            ShapeKind::Line => "line",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ShapeKind::Rect => "Rectangle",
            ShapeKind::Ellipse => "Ellipse",
            ShapeKind::Polygon => "Polygon",
            ShapeKind::Star => "Star",
            ShapeKind::Line => "Line",
        }
    }
    pub fn parse(s: &str) -> Option<ShapeKind> {
        match s.to_ascii_lowercase().as_str() {
            "rect" | "rectangle" | "square" | "box" => Some(ShapeKind::Rect),
            "ellipse" | "oval" | "circle" => Some(ShapeKind::Ellipse),
            "polygon" | "poly" | "triangle" => Some(ShapeKind::Polygon),
            "star" => Some(ShapeKind::Star),
            "line" => Some(ShapeKind::Line),
            _ => None,
        }
    }
}

/// The Shapes tool's drag corners after Shift (equal sides) and Alt (from the centre).
pub fn shape_corners(down: [f32; 2], at: [f32; 2], kind: ShapeKind, mods: Mods) -> ([f32; 2], [f32; 2]) {
    let (mut dx, mut dy) = (at[0] - down[0], at[1] - down[1]);
    if mods.shift {
        if kind == ShapeKind::Line {
            // Snap the line to 45° steps.
            let a = dy.atan2(dx);
            let step = std::f32::consts::FRAC_PI_4;
            let a = (a / step).round() * step;
            let l = (dx * dx + dy * dy).sqrt();
            (dx, dy) = (l * a.cos(), l * a.sin());
        } else {
            let m = dx.abs().max(dy.abs());
            (dx, dy) = (m * dx.signum(), m * dy.signum());
        }
    }
    if mods.alt && kind != ShapeKind::Line { ([down[0] - dx, down[1] - dy], [down[0] + dx, down[1] + dy]) } else { (down, [down[0] + dx, down[1] + dy]) }
}

impl Editor {
    /// A shape's screen outline (closed), or a line's two ends.
    pub fn shape_outline(&self, a: [f32; 2], b: [f32; 2]) -> Result<Vec<[f32; 2]>, String> {
        match self.shape_kind {
            ShapeKind::Line => Ok(vec![a, b]),
            ShapeKind::Rect => fill::shape_outline("rect", a, b, 4),
            ShapeKind::Ellipse => fill::shape_outline("ellipse", a, b, 0),
            ShapeKind::Polygon => fill::shape_outline("polygon", a, b, self.shape_sides),
            ShapeKind::Star => fill::shape_outline("star", a, b, self.shape_sides),
        }
    }

    /// Store a fill (and its mirror copies) as one undo step; `under` puts it just below that
    /// curve in drawing order (the bucket fills under the lines around it). Returns the ids.
    fn commit_fill(&mut self, points: Vec<Point>, tris: Vec<[u32; 3]>, label: &str, under: Option<u64>) -> Vec<u64> {
        if points.is_empty() || tris.is_empty() {
            return Vec::new();
        }
        self.checkpoint(label);
        let maps = if self.mirror_on { mirror_maps(self.mirror_axes) } else { Vec::new() };
        let mut made: Vec<Stroke> = Vec::new();
        for m in &maps {
            let copy: Vec<Point> = points.iter().map(|p| Point { p: reflect(p.p, *m), pressure: p.pressure, n: reflect(p.n, *m) }).collect();
            made.push(self.new_fill(copy, tris.clone()));
        }
        made.push(self.new_fill(points, tris));
        let ids: Vec<u64> = made.iter().map(|s| s.id).collect();
        let at = under.and_then(|id| self.scene.strokes.iter().position(|s| s.id == id)).unwrap_or(self.scene.strokes.len());
        for (k, s) in made.into_iter().enumerate() {
            self.scene.strokes.insert((at + k).min(self.scene.strokes.len()), Arc::new(s));
        }
        self.events.push(Event::Filled);
        self.changed();
        ids
    }

    fn new_fill(&mut self, points: Vec<Point>, tris: Vec<[u32; 3]>) -> Stroke {
        let id = self.scene.alloc_id();
        let mut st = Stroke { id, group: self.scene.active_group, points, brush: self.brush, seed: hash(id as u32 ^ 0x5eed_0f11), tris };
        st.sanitize();
        st
    }

    /// Fill a screen outline on what the pen would draw on (the active guide, an active image
    /// or model, or the drawing plane).
    pub fn fill_outline(&mut self, outline: &[[f32; 2]], label: &str) -> Result<Vec<u64>, String> {
        if !self.can_fill() {
            return Err("nothing to fill on: draw a 3D Guide first (or turn on drawing in the air)".into());
        }
        let view = self.view();
        let flat = self.target_is_flat(&view, outline);
        let (points, tris) = self.patch_on_target(&view, outline, flat)?;
        Ok(self.commit_fill(points, tris, label, None))
    }

    /// Whether a fill made on the current target can skip refinement (a plane: the drawing
    /// plane, a reference image or a flat guide).
    fn target_is_flat(&self, view: &View, outline: &[[f32; 2]]) -> bool {
        let normals: Vec<Vec3> = outline.iter().step_by((outline.len() / 24).max(1)).filter_map(|p| self.target_hit(view, p[0], p[1]).map(|h| h.1)).collect();
        normals.windows(2).all(|w| w[0].dot(w[1]) > 0.9999)
    }

    fn patch_on_target(&self, view: &View, outline: &[[f32; 2]], flat: bool) -> Result<(Vec<Point>, Vec<[u32; 3]>), String> {
        let cleaned = fill::clean_outline(outline);
        let levels = if flat {
            0
        } else {
            let tris = fill::triangulate(&cleaned);
            fill::levels_for(&cleaned, &tris, CURVED_SPACING)
        };
        fill::patch(&cleaned, levels, |x, y| self.target_hit(view, x, y))
    }

    fn can_fill(&self) -> bool {
        self.scene.active_guide().is_some()
            || self.draw_in_air
            || self.scene.images.iter().any(|i| i.state == crate::model::ResourceState::Active)
            || self.scene.models.iter().any(|m| m.state == crate::model::ResourceState::Active)
    }

    /// The paint bucket: fill the closed shape around a screen point, under the curves that
    /// close it, on the surface they were drawn on (the active guide when the point is on it,
    /// else the plane through those curves).
    pub fn fill_at(&mut self, x: f32, y: f32) -> Result<Vec<u64>, String> {
        if !(x.is_finite() && y.is_finite()) {
            return Err("x and y must be numbers".into());
        }
        let view = self.view();
        let (vw, vh) = (view.viewport.width, view.viewport.height);
        if x < 0.0 || y < 0.0 || x >= vw || y >= vh {
            return Err("that point is outside the view".into());
        }
        let cell = fill::BUCKET_CELL;
        let (gw, gh) = fill::bucket_grid(vw, vh);
        let mut walls = vec![false; gw * gh];
        // Each visible curve, as thick lines on screen; remember which cells each touches.
        let curves: Vec<&Arc<Stroke>> = self.scene.strokes.iter().filter(|s| !s.is_fill() && self.scene.group_shown(s.group)).collect();
        let radius_world = |s: &Stroke| s.brush.radius();
        let mut screen: Vec<Vec<Option<(f32, f32, f32)>>> = Vec::with_capacity(curves.len());
        for s in &curves {
            let sp: Vec<Option<(f32, f32, f32)>> = s.points.iter().map(|p| view.project(p.p).map(|q| (q.x, q.y, q.scale * radius_world(s)))).collect();
            if sp.len() == 1
                && let Some(Some(a)) = sp.first()
            {
                fill::wall_segment(&mut walls, gw, gh, cell, [a.0, a.1], [a.0, a.1], a.2.max(1.0));
            }
            for w in sp.windows(2) {
                if let (Some(a), Some(b)) = (w[0], w[1]) {
                    fill::wall_segment(&mut walls, gw, gh, cell, [a.0, a.1], [b.0, b.1], (a.2 + b.2).max(2.0) * 0.5);
                }
            }
            screen.push(sp);
        }
        let seed = ((x / cell) as usize, (y / cell) as usize);
        if walls.get(seed.1 * gw + seed.0).copied().unwrap_or(true) {
            return Err("that is on a line: click inside a closed shape".into());
        }
        let region = fill::flood(&walls, gw, gh, seed).ok_or("that area isn't closed: close the gap, or use Lasso Fill")?;
        let grown = fill::grow(&region, gw, gh, 1);
        let outline: Vec<[f32; 2]> = fill::trace(&grown, gw, gh).into_iter().map(|p| [p[0] * cell, p[1] * cell]).collect();
        if outline.len() < 3 {
            return Err("that area is too small to fill".into());
        }
        // The curves around the region: points within a few cells of it.
        let near = fill::grow(&region, gw, gh, 3);
        let in_near = |sx: f32, sy: f32| {
            let (cx, cy) = ((sx / cell) as isize, (sy / cell) as isize);
            cx >= 0 && cy >= 0 && (cx as usize) < gw && (cy as usize) < gh && near[cy as usize * gw + cx as usize]
        };
        let mut border_pts: Vec<Vec3> = Vec::new();
        let mut border_normals = Vec3::ZERO;
        let mut first_border: Option<u64> = None;
        for (s, sp) in curves.iter().zip(&screen) {
            let mut touches = false;
            for (p, q) in s.points.iter().zip(sp) {
                if let Some(q) = q
                    && in_near(q.0, q.1)
                {
                    border_pts.push(p.p);
                    border_normals += if p.n.dot(view.back) < 0.0 { -p.n } else { p.n };
                    touches = true;
                }
            }
            if touches && first_border.is_none() {
                first_border = Some(s.id);
            }
        }
        // On the active guide when the click lands on it; else on the curves' own plane.
        let on_guide = self.scene.active_guide().is_some_and(|g| {
            let (o, d) = view.ray(x, y);
            g.raycast(o, d).is_some()
        });
        let (points, tris) = if on_guide || border_pts.len() < 3 {
            let flat = self.target_is_flat(&view, &outline);
            self.patch_on_target(&view, &outline, flat)?
        } else {
            let (c, mut n) = fill::plane_of_points(&border_pts).ok_or("the lines around that area don't lie on one surface; use Lasso Fill on a guide")?;
            if n.dot(view.back) < 0.0 {
                n = -n;
            }
            // Curves drawn on a surface carry its normal: prefer it when they agree.
            if border_normals.length() > 1e-3 && border_normals.normalized().dot(n) > 0.9 {
                n = border_normals.normalized();
            }
            let cast = |sx: f32, sy: f32| {
                let (o, d) = view.ray(sx, sy);
                let den = d.dot(n);
                if den.abs() < 1e-6 {
                    return None;
                }
                let t = (c - o).dot(n) / den;
                (t > 0.0 && t.is_finite()).then(|| (o + d * t, n))
            };
            fill::patch(&fill::clean_outline(&outline), 0, cast)?
        };
        Ok(self.commit_fill(points, tris, "Fill", first_border))
    }

    /// Fill each selected closed curve with a solid surface (on top of nothing: under the curve).
    pub fn fill_selection(&mut self) -> Result<Vec<u64>, String> {
        let view = self.view();
        let mut ids: Vec<u64> = self.selection.iter().copied().collect();
        ids.sort_unstable();
        let mut made = Vec::new();
        let mut pending: Vec<(Vec<Point>, Vec<[u32; 3]>, u64)> = Vec::new();
        for id in ids {
            let Some(s) = self.scene.stroke(id).cloned() else { continue };
            if s.is_fill() || s.points.len() < 3 {
                continue;
            }
            let outline: Vec<[f32; 2]> = s.points.iter().filter_map(|p| view.project(p.p).map(|q| [q.x, q.y])).collect();
            let pts: Vec<Vec3> = s.points.iter().map(|p| p.p).collect();
            let Some((c, mut n)) = fill::plane_of_points(&pts) else { continue };
            if n.dot(view.back) < 0.0 {
                n = -n;
            }
            let cast = |sx: f32, sy: f32| {
                let (o, d) = view.ray(sx, sy);
                let den = d.dot(n);
                if den.abs() < 1e-6 {
                    return None;
                }
                let t = (c - o).dot(n) / den;
                (t > 0.0 && t.is_finite()).then(|| (o + d * t, n))
            };
            if let Ok((p, t)) = fill::patch(&outline, 0, cast) {
                pending.push((p, t, id));
            }
        }
        if pending.is_empty() {
            return Err("select closed curves to fill (draw a loop, then Select it)".into());
        }
        for (p, t, under) in pending {
            made.extend(self.commit_fill(p, t, "Fill Selection", Some(under)));
        }
        Ok(made)
    }

    /// A solid, painted copy of a guide's surface in the brush colour (a primitive becomes a
    /// coloured object you can draw on and move like any curve).
    pub fn solid_from_guide(&mut self, id: Option<u64>) -> Result<Vec<u64>, String> {
        let gid = id
            .or(self.scene.active_guide)
            .or_else(|| self.selected_resources.iter().copied().find(|r| self.scene.guide(*r).is_some()))
            .ok_or("no guide to make solid: place a primitive or draw a guide first")?;
        let g = self.scene.guide(gid).cloned().ok_or("that guide is gone")?;
        let mut points: Vec<Point> = Vec::new();
        let mut tris: Vec<[u32; 3]> = Vec::new();
        for grid in &g.grids {
            let base = points.len() as u32;
            let (cols, rows) = (grid.cols, grid.rows);
            if cols < 2 || rows < 2 || grid.points.len() < cols * rows {
                continue;
            }
            let normal_at = |c: usize, r: usize| -> Vec3 {
                let at = |c: usize, r: usize| grid.at(c, r);
                let cc = if grid.closed_u { (c + 1) % cols } else { (c + 1).min(cols - 1) };
                let cp = if grid.closed_u { (c + cols - 1) % cols } else { c.saturating_sub(1) };
                let (rn, rp) = ((r + 1).min(rows - 1), r.saturating_sub(1));
                match (at(cc, r), at(cp, r), at(c, rn), at(c, rp)) {
                    (Some(a), Some(b), Some(d), Some(e)) => (a - b).cross(d - e).normalized(),
                    _ => Vec3::ZERO,
                }
            };
            for r in 0..rows {
                for c in 0..cols {
                    let p = grid.at(c, r).unwrap_or(Vec3::ZERO);
                    let n = normal_at(c, r);
                    points.push(Point { p, pressure: 1.0, n: if n.is_finite() { n } else { Vec3::ZERO } });
                }
            }
            let quads_u = if grid.closed_u { cols } else { cols - 1 };
            for r in 0..rows - 1 {
                for c in 0..quads_u {
                    let c1 = (c + 1) % cols;
                    let i = |c: usize, r: usize| base + (r * cols + c) as u32;
                    tris.push([i(c, r), i(c1, r), i(c1, r + 1)]);
                    tris.push([i(c, r), i(c1, r + 1), i(c, r + 1)]);
                }
            }
        }
        if tris.is_empty() {
            return Err("that guide has no surface".into());
        }
        Ok(self.commit_fill(points, tris, "Solid", None))
    }

    /// The fill commands (dispatched from [`Editor::run`]).
    pub(super) fn run_fill(&mut self, cmd: &str, p: &Value) -> Option<Result<Value, String>> {
        let num = |k: &str| p.get(k).and_then(Value::as_f64).map(|v| v as f32);
        let out = match cmd {
            "fill.at" => (|| {
                let (x, y) = (num("x").ok_or("x is required")?, num("y").ok_or("y is required")?);
                self.fill_at(x, y).map(|ids| json!(ids))
            })(),
            "fill.lasso" => (|| {
                let pts = super::screen_points(p, "points")?;
                let outline: Vec<[f32; 2]> = pts.iter().map(|q| [q[0], q[1]]).collect();
                self.fill_outline(&outline, "Lasso Fill").map(|ids| json!(ids))
            })(),
            "fill.shape" => (|| {
                if let Some(k) = p.get("kind").and_then(Value::as_str) {
                    self.shape_kind = ShapeKind::parse(k).ok_or_else(|| format!("unknown shape {k:?} (rect, ellipse, polygon, star, line)"))?;
                }
                if let Some(n) = p.get("sides").and_then(Value::as_u64) {
                    self.shape_sides = (n as u32).clamp(3, 32);
                }
                let filled = p.get("filled").and_then(Value::as_bool).unwrap_or(self.shape_filled);
                let a = [num("x0").ok_or("x0 is required")?, num("y0").ok_or("y0 is required")?];
                let b = [num("x1").ok_or("x1 is required")?, num("y1").ok_or("y1 is required")?];
                self.make_shape(a, b, filled).map(|ids| json!(ids))
            })(),
            "shape.set" => (|| {
                if let Some(k) = p.get("kind").and_then(Value::as_str) {
                    self.shape_kind = ShapeKind::parse(k).ok_or_else(|| format!("unknown shape {k:?} (rect, ellipse, polygon, star, line)"))?;
                }
                if let Some(n) = p.get("sides") {
                    let n = n.as_u64().ok_or("sides must be a whole number")?;
                    self.shape_sides = (n.min(32) as u32).max(3);
                }
                if let Some(f) = p.get("filled") {
                    self.shape_filled = f.as_bool().ok_or("filled must be true or false")?;
                }
                self.touch();
                Ok(json!({"kind": self.shape_kind.name(), "sides": self.shape_sides, "filled": self.shape_filled}))
            })(),
            "fill.selection" => self.fill_selection().map(|ids| json!(ids)),
            "guide.solid" => (|| {
                let id = match p.get("id") {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(v.as_u64().ok_or("id must be a guide id")?),
                };
                self.solid_from_guide(id).map(|ids| json!(ids))
            })(),
            _ => return None,
        };
        Some(out)
    }

    /// Finish a Shapes drag: a filled patch, or the outline as a curve in the current brush.
    pub fn make_shape(&mut self, a: [f32; 2], b: [f32; 2], filled: bool) -> Result<Vec<u64>, String> {
        let outline = self.shape_outline(a, b)?;
        if filled && self.shape_kind != ShapeKind::Line {
            return self.fill_outline(&outline, "Shape");
        }
        // The outline as a curve: a point every 3 pixels, cast on the target.
        let view = self.view();
        let mut ring = outline.clone();
        if self.shape_kind != ShapeKind::Line
            && let Some(f) = outline.first()
        {
            ring.push(*f);
        }
        let mut pts = Vec::new();
        for w in ring.windows(2) {
            let (p, q) = (w[0], w[1]);
            let n = ((((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt() / 3.0).ceil() as usize).clamp(1, 2000);
            for i in 0..n {
                let t = i as f32 / n as f32;
                pts.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
            }
        }
        if let Some(l) = ring.last() {
            pts.push(*l);
        }
        let curve: Vec<Point> = pts.iter().filter_map(|q| self.target_hit(&view, q[0], q[1]).map(|(p, n)| Point { p, pressure: 1.0, n })).collect();
        if curve.len() < 2 {
            return Err("nothing to draw on there: draw a 3D Guide first (or turn on drawing in the air)".into());
        }
        Ok(self.commit_curves(vec![curve], "Shape"))
    }
}
