//! Moving, rotating and scaling, the way Blender does it (so it feels familiar on a PC):
//!
//! - **G / R / S** start a modal transform that follows the mouse. Click or Enter confirms;
//!   right click or Esc cancels.
//! - **X / Y / Z** constrain to that axis (press again for the local axis, a third time to
//!   free it); **Shift+X/Y/Z** constrain to the plane without that axis.
//! - **Typing a number** sets the amount exactly (metres, degrees, or a factor); `-` flips it;
//!   Backspace edits.
//! - **Ctrl** snaps (0.1 m, 5°, 0.1×), **Shift** slows the mouse for precision.
//! - Unconstrained, a move slides in the view plane and a rotation turns about the view axis
//!   through the pivot: the same as Feather's view-oriented joystick, without the joystick.
//! - The **gizmo** (arrows, plane squares, rings and scale boxes) starts the same constrained
//!   transforms by dragging its handles.

use crate::camera::View;
use crate::math::{Quat, Vec3, Xform, ray_plane, v3};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Grab,
    Rotate,
    Scale,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s.to_ascii_lowercase().as_str() {
            "grab" | "move" | "translate" | "g" => Some(Mode::Grab),
            "rotate" | "r" => Some(Mode::Rotate),
            "scale" | "s" => Some(Mode::Scale),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Constraint {
    Free,
    /// Along one axis (0 = X, 1 = Y, 2 = Z).
    Axis {
        axis: usize,
        local: bool,
    },
    /// In the plane that leaves out one axis.
    Plane {
        axis: usize,
        local: bool,
    },
}

/// What a transform does, ready to apply to points, normals and placements.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Delta {
    Translate(Vec3),
    Rotate {
        pivot: Vec3,
        rot: Quat,
    },
    /// Per-axis factors in the frame `orient`, about `pivot`.
    Scale {
        pivot: Vec3,
        orient: Quat,
        factors: Vec3,
    },
}

impl Delta {
    pub fn identity() -> Delta {
        Delta::Translate(Vec3::ZERO)
    }

    pub fn point(&self, p: Vec3) -> Vec3 {
        match *self {
            Delta::Translate(d) => p + d,
            Delta::Rotate { pivot, rot } => pivot + rot.rotate(p - pivot),
            Delta::Scale { pivot, orient, factors } => {
                let local = orient.conjugate().rotate(p - pivot);
                pivot + orient.rotate(local.mul_elem(factors))
            }
        }
    }

    pub fn normal(&self, n: Vec3) -> Vec3 {
        if n == Vec3::ZERO {
            return n;
        }
        match *self {
            Delta::Translate(_) => n,
            Delta::Rotate { rot, .. } => rot.rotate(n).normalized(),
            Delta::Scale { orient, factors, .. } => {
                let inv = v3(recip(factors.x), recip(factors.y), recip(factors.z));
                orient.rotate(orient.conjugate().rotate(n).mul_elem(inv)).normalized()
            }
        }
    }

    /// A placement (guide, image, model) under this transform. Scaling along an axis that is not
    /// one of the placement's own axes is approximated by scaling its nearest own axis.
    pub fn xform(&self, x: &Xform) -> Xform {
        match *self {
            Delta::Translate(d) => Xform { translation: x.translation + d, ..*x },
            Delta::Rotate { pivot, rot } => Xform { translation: pivot + rot.rotate(x.translation - pivot), rotation: rot * x.rotation, scale: x.scale },
            Delta::Scale { pivot, orient, factors } => {
                let translation = self.point(x.translation);
                let _ = pivot;
                // Each world-frame factor goes to the placement axis most aligned with it.
                let mut s = x.scale;
                let axes = [Vec3::X, Vec3::Y, Vec3::Z];
                for (k, f) in [factors.x, factors.y, factors.z].into_iter().enumerate() {
                    if (f - 1.0).abs() < 1e-6 {
                        continue;
                    }
                    let dir = orient.rotate(axes[k]);
                    let local = x.rotation.conjugate().rotate(dir);
                    let (ax, ay, az) = (local.x.abs(), local.y.abs(), local.z.abs());
                    if ax >= ay && ax >= az {
                        s.x *= f;
                    } else if ay >= az {
                        s.y *= f;
                    } else {
                        s.z *= f;
                    }
                }
                Xform { translation, rotation: x.rotation, scale: s }
            }
        }
    }
}

fn recip(v: f32) -> f32 {
    if v.abs() < 1e-9 { 0.0 } else { 1.0 / v }
}

const AXES: [Vec3; 3] = [Vec3::X, Vec3::Y, Vec3::Z];
pub const AXIS_NAMES: [&str; 3] = ["X", "Y", "Z"];

/// A modal transform in progress.
#[derive(Debug, Clone, PartialEq)]
pub struct Modal {
    pub mode: Mode,
    pub constraint: Constraint,
    pub pivot: Vec3,
    /// The local frame (the selection's own axes; identity for curves).
    pub orient: Quat,
    pub start: [f32; 2],
    pub mouse: [f32; 2],
    /// Typed amount.
    pub numeric: String,
    pub snap: bool,
    pub precise: bool,
}

impl Modal {
    pub fn new(mode: Mode, pivot: Vec3, orient: Quat, mouse: [f32; 2]) -> Modal {
        Modal { mode, constraint: Constraint::Free, pivot, orient, start: mouse, mouse, numeric: String::new(), snap: false, precise: false }
    }

    pub fn with_constraint(mut self, c: Constraint) -> Modal {
        self.constraint = c;
        self
    }

    /// X / Y / Z pressed (plane with Shift): global, then local, then free (Blender).
    pub fn press_axis(&mut self, axis: usize, plane: bool) {
        let axis = axis.min(2);
        self.constraint = match (self.constraint, plane) {
            (Constraint::Axis { axis: a, local: false }, false) if a == axis => Constraint::Axis { axis, local: true },
            (Constraint::Axis { axis: a, local: true }, false) if a == axis => Constraint::Free,
            (Constraint::Plane { axis: a, local: false }, true) if a == axis => Constraint::Plane { axis, local: true },
            (Constraint::Plane { axis: a, local: true }, true) if a == axis => Constraint::Free,
            (_, false) => Constraint::Axis { axis, local: false },
            (_, true) => Constraint::Plane { axis, local: false },
        };
    }

    /// A typed character: digits, '.', '-' (flip), or backspace ('\u{8}').
    pub fn type_char(&mut self, c: char) {
        match c {
            '0'..='9' | '.' if self.numeric.len() < 24 => self.numeric.push(c),
            '-' => {
                if let Some(rest) = self.numeric.strip_prefix('-') {
                    self.numeric = rest.to_string();
                } else {
                    self.numeric.insert(0, '-');
                }
            }
            '\u{8}' => {
                self.numeric.pop();
            }
            _ => {}
        }
    }

    fn typed(&self) -> Option<f32> {
        let s = self.numeric.as_str();
        if s.is_empty() || s == "-" || s == "." || s == "-." {
            return None;
        }
        s.parse::<f32>().ok().filter(|v| v.is_finite())
    }

    fn axis_vec(&self, axis: usize, local: bool) -> Vec3 {
        let a = AXES[axis.min(2)];
        if local { self.orient.rotate(a).normalized() } else { a }
    }

    /// The mouse movement, slowed when precise.
    fn mouse_delta(&self) -> [f32; 2] {
        let k = if self.precise { 0.1 } else { 1.0 };
        [(self.mouse[0] - self.start[0]) * k, (self.mouse[1] - self.start[1]) * k]
    }

    /// The transform for the current mouse, keys and typed number.
    pub fn delta(&self, view: &View) -> Delta {
        match self.mode {
            Mode::Grab => Delta::Translate(self.translation(view)),
            Mode::Rotate => {
                let (axis, mut angle) = self.rotation(view);
                if let Some(t) = self.typed() {
                    angle = t.to_radians();
                } else if self.snap {
                    let step = if self.precise { 1f32 } else { 5.0 }.to_radians();
                    angle = (angle / step).round() * step;
                }
                Delta::Rotate { pivot: self.pivot, rot: Quat::from_axis_angle(axis, angle) }
            }
            Mode::Scale => {
                let mut f = self.scale_factor(view);
                if let Some(t) = self.typed() {
                    f = t;
                } else if self.snap {
                    let step = if self.precise { 0.01 } else { 0.1 };
                    f = (f / step).round() * step;
                }
                let (orient, factors) = match self.constraint {
                    Constraint::Free => (Quat::IDENTITY, v3(f, f, f)),
                    Constraint::Axis { axis, local } => {
                        let mut v = v3(1.0, 1.0, 1.0);
                        set(&mut v, axis, f);
                        (if local { self.orient } else { Quat::IDENTITY }, v)
                    }
                    Constraint::Plane { axis, local } => {
                        let mut v = v3(f, f, f);
                        set(&mut v, axis, 1.0);
                        (if local { self.orient } else { Quat::IDENTITY }, v)
                    }
                };
                Delta::Scale { pivot: self.pivot, orient, factors }
            }
        }
    }

    fn translation(&self, view: &View) -> Vec3 {
        let typed = self.typed();
        let d = match self.constraint {
            Constraint::Free => {
                if let Some(t) = typed {
                    return view.right * t;
                }
                let [dx, dy] = self.mouse_delta();
                let depth = view.project(self.pivot).map_or(1.0, |p| p.depth);
                let wpp = view.world_per_px(depth);
                view.right * (dx * wpp) - view.up * (dy * wpp)
            }
            Constraint::Axis { axis, local } => {
                let a = self.axis_vec(axis, local);
                if let Some(t) = typed {
                    return a * t;
                }
                a * self.along_axis(view, a)
            }
            Constraint::Plane { axis, local } => {
                let n = self.axis_vec(axis, local);
                if let Some(t) = typed {
                    // A typed number moves along the plane's first other axis.
                    let other = self.axis_vec((axis + 1) % 3, local);
                    return other * t;
                }
                let k = if self.precise { 0.1 } else { 1.0 };
                let hit = |m: [f32; 2]| {
                    let (o, dir) = view.ray(m[0], m[1]);
                    ray_plane(o, dir, self.pivot, n).map(|t| o + dir * t).or_else(|| ray_plane(o, -dir, self.pivot, n).map(|t| o - dir * t))
                };
                match (hit(self.start), hit(self.mouse)) {
                    (Some(a), Some(b)) => {
                        let d = (b - a) * k;
                        d - n * d.dot(n)
                    }
                    _ => Vec3::ZERO,
                }
            }
        };
        if self.snap {
            let step = if self.precise { 0.01 } else { 0.1 };
            v3((d.x / step).round() * step, (d.y / step).round() * step, (d.z / step).round() * step)
        } else {
            d
        }
    }

    /// World units moved along `a` for the mouse movement (projected onto the axis on screen).
    fn along_axis(&self, view: &View, a: Vec3) -> f32 {
        let Some(p0) = view.project(self.pivot) else { return 0.0 };
        let unit = view.world_per_px(p0.depth) * 100.0;
        let Some(p1) = view.project(self.pivot + a * unit) else { return 0.0 };
        let (sx, sy) = ((p1.x - p0.x) / unit, (p1.y - p0.y) / unit);
        let l2 = sx * sx + sy * sy;
        if l2 < 1e-6 {
            return 0.0;
        }
        let [dx, dy] = self.mouse_delta();
        (dx * sx + dy * sy) / l2
    }

    /// (axis, angle): the mouse's turn around the pivot on screen.
    fn rotation(&self, view: &View) -> (Vec3, f32) {
        let axis = match self.constraint {
            Constraint::Free => view.back,
            Constraint::Axis { axis, local } | Constraint::Plane { axis, local } => self.axis_vec(axis, local),
        };
        let Some(c) = view.project(self.pivot) else { return (axis, 0.0) };
        let ang = |m: [f32; 2]| (-(m[1] - c.y)).atan2(m[0] - c.x);
        let mut a = ang(self.mouse) - ang(self.start);
        // Keep it continuous across ±π.
        if a > std::f32::consts::PI {
            a -= std::f32::consts::TAU;
        } else if a < -std::f32::consts::PI {
            a += std::f32::consts::TAU;
        }
        if self.precise {
            a *= 0.1;
        }
        // Counter-clockwise on screen turns counter-clockwise as seen from the viewer.
        if axis.dot(view.back) < 0.0 {
            a = -a;
        }
        (axis, a)
    }

    fn scale_factor(&self, view: &View) -> f32 {
        let Some(c) = view.project(self.pivot) else { return 1.0 };
        let d = |m: [f32; 2]| ((m[0] - c.x).powi(2) + (m[1] - c.y).powi(2)).sqrt();
        let d0 = d(self.start);
        if d0 < 1.0 {
            return 1.0;
        }
        let f = d(self.mouse) / d0;
        if self.precise { 1.0 + (f - 1.0) * 0.1 } else { f }
    }

    /// Blender-style header text: what the transform is doing right now.
    pub fn header(&self, view: &View) -> String {
        let what = match self.constraint {
            Constraint::Free => String::new(),
            Constraint::Axis { axis, local } => format!(" along {}{}", if local { "local " } else { "" }, AXIS_NAMES[axis.min(2)]),
            Constraint::Plane { axis, local } => {
                let others: String = (0..3).filter(|k| *k != axis).map(|k| AXIS_NAMES[k]).collect();
                format!(" in {}{}", if local { "local " } else { "" }, others)
            }
        };
        let typed = if self.numeric.is_empty() { String::new() } else { format!(" [{}]", self.numeric) };
        match self.delta(view) {
            Delta::Translate(d) => format!("Move{what}: Dx {:.3}  Dy {:.3}  Dz {:.3} ({:.3} m){typed}", d.x, d.y, d.z, d.length()),
            Delta::Rotate { .. } => {
                let (_, a) = self.rotation(view);
                let a = self.typed().unwrap_or(a.to_degrees());
                format!("Rotate{what}: {a:.1}°{typed}")
            }
            Delta::Scale { factors, .. } => format!("Scale{what}: {:.3} {:.3} {:.3}{typed}", factors.x, factors.y, factors.z),
        }
    }
}

fn set(v: &mut Vec3, axis: usize, val: f32) {
    match axis {
        0 => v.x = val,
        1 => v.y = val,
        _ => v.z = val,
    }
}

/// Gizmo handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    Move(usize),
    MovePlane(usize),
    MoveView,
    Rotate(usize),
    RotateView,
    Scale(usize),
    ScaleUniform,
}

impl Handle {
    /// The modal transform a drag on this handle starts.
    pub fn start(self, pivot: Vec3, orient: Quat, mouse: [f32; 2], local: bool) -> Modal {
        let (mode, c) = match self {
            Handle::Move(a) => (Mode::Grab, Constraint::Axis { axis: a, local }),
            Handle::MovePlane(a) => (Mode::Grab, Constraint::Plane { axis: a, local }),
            Handle::MoveView => (Mode::Grab, Constraint::Free),
            Handle::Rotate(a) => (Mode::Rotate, Constraint::Axis { axis: a, local }),
            Handle::RotateView => (Mode::Rotate, Constraint::Free),
            Handle::Scale(a) => (Mode::Scale, Constraint::Axis { axis: a, local }),
            Handle::ScaleUniform => (Mode::Scale, Constraint::Free),
        };
        Modal::new(mode, pivot, orient, mouse).with_constraint(c)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GizmoKind {
    Move,
    Rotate,
    Scale,
    /// All three, like Blender's Transform tool.
    All,
}

/// A drawable, clickable gizmo part: a screen polyline in an axis colour.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub handle: Handle,
    pub line: Vec<[f32; 2]>,
    /// 0 = X red, 1 = Y green, 2 = Z blue, 3 = view (white).
    pub axis: usize,
    /// Filled shape (plane squares, scale boxes, arrow tips) instead of a line.
    pub filled: bool,
}

/// The gizmo's parts on screen, `size` pixels across.
pub fn gizmo(view: &View, pivot: Vec3, orient: Quat, local: bool, kind: GizmoKind, size: f32) -> Vec<Part> {
    let Some(c) = view.project(pivot) else { return Vec::new() };
    let wpp = view.world_per_px(c.depth);
    let len = size * wpp;
    let axis = |k: usize| if local { orient.rotate(AXES[k]) } else { AXES[k] };
    let scr = |p: Vec3| view.project(p).map(|s| [s.x, s.y]);
    let mut out = Vec::new();
    let mv = matches!(kind, GizmoKind::Move | GizmoKind::All);
    let rot = matches!(kind, GizmoKind::Rotate | GizmoKind::All);
    let sc = matches!(kind, GizmoKind::Scale | GizmoKind::All);
    for k in 0..3 {
        let a = axis(k);
        // Hide axes pointing straight at the viewer (Blender fades them).
        let facing = a.dot(view.back).abs() > 0.97;
        if mv && !facing {
            if let (Some(p0), Some(p1)) = (scr(pivot + a * (len * 0.2)), scr(pivot + a * len)) {
                out.push(Part { handle: Handle::Move(k), line: vec![p0, p1], axis: k, filled: false });
                // Arrow tip.
                let (dx, dy) = (p1[0] - p0[0], p1[1] - p0[1]);
                let l = (dx * dx + dy * dy).sqrt().max(1e-3);
                let (ux, uy) = (dx / l, dy / l);
                let tip = [p1[0] + ux * 12.0, p1[1] + uy * 12.0];
                out.push(Part {
                    handle: Handle::Move(k),
                    line: vec![tip, [p1[0] - uy * 5.0, p1[1] + ux * 5.0], [p1[0] + uy * 5.0, p1[1] - ux * 5.0]],
                    axis: k,
                    filled: true,
                });
            }
            // Plane square between the other two axes.
            let (b, cc) = (axis((k + 1) % 3), axis((k + 2) % 3));
            let o = pivot + (b + cc) * (len * 0.3);
            let s = len * 0.08;
            let corners = [o - b * s - cc * s, o + b * s - cc * s, o + b * s + cc * s, o - b * s + cc * s];
            let pts: Vec<[f32; 2]> = corners.iter().filter_map(|p| scr(*p)).collect();
            if pts.len() == 4 && a.dot(view.back).abs() > 0.15 {
                out.push(Part { handle: Handle::MovePlane(k), line: pts, axis: k, filled: true });
            }
        }
        if sc && !facing {
            let at = pivot + a * (len * if mv { 0.62 } else { 1.0 });
            if let (Some(p0), Some(p1)) = (scr(pivot), scr(at)) {
                if !mv {
                    out.push(Part { handle: Handle::Scale(k), line: vec![p0, p1], axis: k, filled: false });
                }
                let h = 5.0;
                out.push(Part {
                    handle: Handle::Scale(k),
                    line: vec![[p1[0] - h, p1[1] - h], [p1[0] + h, p1[1] - h], [p1[0] + h, p1[1] + h], [p1[0] - h, p1[1] + h]],
                    axis: k,
                    filled: true,
                });
            }
        }
        if rot {
            let (b, cc) = (axis((k + 1) % 3), axis((k + 2) % 3));
            let r = len * 0.85;
            let ring: Vec<[f32; 2]> = (0..=48)
                .filter_map(|i| {
                    let t = std::f32::consts::TAU * i as f32 / 48.0;
                    let p = pivot + b * (r * t.cos()) + cc * (r * t.sin());
                    // Only the half facing the viewer, as Blender draws it.
                    ((p - pivot).dot(view.back) >= -len * 0.05 || a.dot(view.back).abs() > 0.9).then(|| scr(p)).flatten()
                })
                .collect();
            if ring.len() > 2 {
                out.push(Part { handle: Handle::Rotate(k), line: ring, axis: k, filled: false });
            }
        }
    }
    if rot {
        let r = size * 1.05;
        let ring: Vec<[f32; 2]> = (0..=64)
            .map(|i| {
                let t = std::f32::consts::TAU * i as f32 / 64.0;
                [c.x + r * t.cos(), c.y + r * t.sin()]
            })
            .collect();
        out.push(Part { handle: Handle::RotateView, line: ring, axis: 3, filled: false });
    }
    if sc && !mv {
        let r = size * 1.2;
        let ring: Vec<[f32; 2]> = (0..=64)
            .map(|i| {
                let t = std::f32::consts::TAU * i as f32 / 64.0;
                [c.x + r * t.cos(), c.y + r * t.sin()]
            })
            .collect();
        out.push(Part { handle: Handle::ScaleUniform, line: ring, axis: 3, filled: false });
    }
    if mv {
        let r = 7.0;
        let ring: Vec<[f32; 2]> = (0..=24)
            .map(|i| {
                let t = std::f32::consts::TAU * i as f32 / 24.0;
                [c.x + r * t.cos(), c.y + r * t.sin()]
            })
            .collect();
        out.push(Part { handle: Handle::MoveView, line: ring, axis: 3, filled: false });
    }
    out
}

fn seg_dist(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (abx, aby) = (b[0] - a[0], b[1] - a[1]);
    let l2 = abx * abx + aby * aby;
    let t = if l2 < 1e-9 { 0.0 } else { (((p[0] - a[0]) * abx + (p[1] - a[1]) * aby) / l2).clamp(0.0, 1.0) };
    ((p[0] - a[0] - abx * t).powi(2) + (p[1] - a[1] - aby * t).powi(2)).sqrt()
}

fn inside(p: [f32; 2], poly: &[[f32; 2]]) -> bool {
    let mut c = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + n - 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            c = !c;
        }
    }
    c
}

/// The handle under the mouse (filled parts first, then the nearest line within 8 px).
pub fn pick(parts: &[Part], mouse: [f32; 2]) -> Option<Handle> {
    if let Some(p) = parts.iter().find(|p| p.filled && p.line.len() >= 3 && inside(mouse, &p.line)) {
        return Some(p.handle);
    }
    let mut best: Option<(f32, Handle)> = None;
    for p in parts.iter().filter(|p| !p.filled || p.line.len() < 3) {
        for w in p.line.windows(2) {
            let d = seg_dist(mouse, w[0], w[1]);
            if d < 8.0 && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, p.handle));
            }
        }
    }
    best.map(|(_, h)| h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{Camera, PerfectView};

    fn front() -> View {
        let mut c = Camera::default();
        c.snap(PerfectView::Front);
        c.view()
    }

    #[test]
    fn free_grab_follows_the_mouse_in_the_view_plane() {
        let v = front();
        let mut m = Modal::new(Mode::Grab, Vec3::ZERO, Quat::IDENTITY, [640.0, 400.0]);
        m.mouse = [740.0, 400.0];
        let Delta::Translate(d) = m.delta(&v) else { panic!() };
        assert!(d.x > 0.0 && d.y.abs() < 1e-5 && d.z.abs() < 1e-5);
        let s = v.project(d).expect("on screen");
        assert!((s.x - 740.0).abs() < 0.5, "the object stays under the mouse");
    }

    #[test]
    fn axis_keys_cycle_global_local_free_and_typed_numbers_win() {
        let v = front();
        let mut m = Modal::new(Mode::Grab, Vec3::ZERO, Quat::from_axis_angle(Vec3::Y, 0.5), [640.0, 400.0]);
        m.press_axis(0, false);
        assert_eq!(m.constraint, Constraint::Axis { axis: 0, local: false });
        m.press_axis(0, false);
        assert_eq!(m.constraint, Constraint::Axis { axis: 0, local: true });
        m.press_axis(0, false);
        assert_eq!(m.constraint, Constraint::Free);
        m.press_axis(1, false);
        m.mouse = [700.0, 300.0];
        let Delta::Translate(d) = m.delta(&v) else { panic!() };
        assert!(d.x.abs() < 1e-6 && d.y > 0.0, "{d:?}");
        for c in "2.5".chars() {
            m.type_char(c);
        }
        m.type_char('-');
        assert_eq!(m.delta(&v), Delta::Translate(v3(0.0, -2.5, 0.0)));
        m.type_char('\u{8}');
        m.type_char('x');
        assert_eq!(m.numeric, "-2.");
    }

    #[test]
    fn rotation_turns_counter_clockwise_with_the_mouse_and_snaps() {
        let v = front();
        let mut m = Modal::new(Mode::Rotate, Vec3::ZERO, Quat::IDENTITY, [740.0, 400.0]);
        m.mouse = [640.0, 300.0]; // a quarter turn counter-clockwise on screen
        let d = m.delta(&v);
        let p = d.point(Vec3::X);
        assert!(p.distance(Vec3::Y) < 1e-3, "{p:?}");
        m.mouse = [740.0, 397.0];
        m.snap = true;
        let d = m.delta(&v);
        assert!(d.point(Vec3::X).distance(Vec3::X) < 1e-4, "snapped to 0°");
    }

    #[test]
    fn scale_by_distance_axis_and_plane() {
        let v = front();
        let mut m = Modal::new(Mode::Scale, Vec3::ZERO, Quat::IDENTITY, [740.0, 400.0]);
        m.mouse = [840.0, 400.0];
        assert!(m.delta(&v).point(v3(1.0, 1.0, 1.0)).distance(v3(2.0, 2.0, 2.0)) < 1e-3);
        m.press_axis(0, false);
        assert!(m.delta(&v).point(v3(1.0, 1.0, 1.0)).distance(v3(2.0, 1.0, 1.0)) < 1e-3);
        m.press_axis(0, true);
        assert!(m.delta(&v).point(v3(1.0, 1.0, 1.0)).distance(v3(1.0, 2.0, 2.0)) < 1e-3);
        // Degenerate start at the pivot: no scale, no NaN.
        let mut z = Modal::new(Mode::Scale, Vec3::ZERO, Quat::IDENTITY, [640.0, 400.0]);
        z.mouse = [900.0, 100.0];
        assert_eq!(z.delta(&v).point(Vec3::X), Vec3::X);
    }

    #[test]
    fn gizmo_handles_can_be_picked() {
        let v = Camera::default().view();
        let parts = gizmo(&v, Vec3::ZERO, Quat::IDENTITY, false, GizmoKind::All, 80.0);
        assert!(parts.iter().any(|p| p.handle == Handle::Move(0)));
        let tip = parts.iter().find(|p| p.handle == Handle::Move(1) && !p.filled).and_then(|p| p.line.last().copied()).expect("y arrow");
        assert_eq!(pick(&parts, tip), Some(Handle::Move(1)));
        assert_eq!(pick(&parts, [5.0, 5.0]), None);
        let m = Handle::Rotate(2).start(Vec3::ZERO, Quat::IDENTITY, [0.0, 0.0], false);
        assert_eq!(m.constraint, Constraint::Axis { axis: 2, local: false });
    }

    #[test]
    fn deltas_move_placements() {
        let x = Xform::default();
        let r = Delta::Rotate { pivot: v3(1.0, 0.0, 0.0), rot: Quat::from_axis_angle(Vec3::Z, std::f32::consts::PI) };
        let moved = r.xform(&x);
        assert!(moved.translation.distance(v3(2.0, 0.0, 0.0)) < 1e-4);
        let s = Delta::Scale { pivot: Vec3::ZERO, orient: Quat::IDENTITY, factors: v3(2.0, 1.0, 1.0) };
        assert_eq!(s.xform(&x).scale, v3(2.0, 1.0, 1.0));
        assert_eq!(s.normal(Vec3::ZERO), Vec3::ZERO);
    }
}
