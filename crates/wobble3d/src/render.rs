//! Tessellation: the note seen through the camera at one boil frame, as depth-sorted screen
//! triangles (painter's algorithm). Any backend can draw the result: the egui shell turns the
//! batches into meshes, [`crate::raster`] fills them on the CPU for exports and tests.
//!
//! Curves become camera-facing tubes or ribbons in screen space (shaded with a normal that
//! turns across the width, so a band reads as a round wire), flat tape lying on the surface it
//! was drawn on, or textured painterly bands. The boil displaces every curve with smooth noise
//! that changes per frame, on screen (cartoon boil) or in the world.

use std::collections::HashSet;

use crate::camera::{Camera, View};
use crate::guide::Guide;
use crate::math::{Vec3, v3};
use crate::model::{Boil, BrushKind, Environment, ImageResource, Material, ModelResource, ResourceState, Rgba, Scene, Stroke};
use crate::noise::{Track, hash2, signed, unit};
use crate::texture::{Atlas, RowKey, TILE_WIDTHS};

/// A vertex: screen pixels, atlas or image texture coordinates, premultiplied RGBA, its depth
/// in the depth buffer's terms ([`Frame::z`]), and whether its paint is solid (drawn with the
/// depth test and depth writes; soft edges and see-through paint are blended over after).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vtx {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [u8; 4],
    /// Reverse depth: larger is nearer (near / depth), so float precision holds at every
    /// distance; nudged by drawing order so later curves win on a shared surface.
    pub z: f32,
    pub solid: bool,
}

/// How far behind a curve its echo and extra layered passes sit (relative depth). They are the
/// curve shifted on screen, so on a tilted surface their depth is off by a few thousandths: this
/// keeps them behind every curve on the same surface (a painted panel's shadow stays under all
/// of its paint).
const ECHO_BEHIND: f32 = 1.2e-2;
const LAYER_BEHIND: f32 = 4e-3;
/// How far in front of their curve scattered dabs sit.
const DABS_FRONT: f32 = 6e-3;

/// Depth nudge per curve in drawing order: on a shared surface (a guide), later curves cover
/// earlier ones; curves more than about a percent apart in depth are ordered by depth alone.
pub const ORDER_BIAS: f32 = 1.5e-6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tex {
    Atlas,
    Image(u64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    pub tex: Tex,
    pub vertices: Vec<Vtx>,
    pub indices: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Frame {
    /// Back to front: for a painter (each over the ones before), or for the blended pass after
    /// the solid pass of a depth-buffered renderer.
    pub batches: Vec<Batch>,
    pub triangles: usize,
    /// The depth-buffer value of the orbit point (depth of field focuses there).
    pub focus_z: f32,
}

/// View depth → reverse depth-buffer value in 0..1 (larger is nearer). Perspective uses
/// near / depth: interpolating it across a triangle on screen is exact, and floats keep their
/// relative precision at any distance (a drawing-order nudge of a millionth still counts).
/// Orthographic views see behind the eye too, so their depth is shifted to stay positive.
pub fn depth_to_z(depth: f32, orthographic: bool, near: f32, far: f32) -> f32 {
    let z = if orthographic {
        let shift = far * 0.5;
        near / (depth + shift).max(near) * (shift / near).min(1e6) * 1e-6
    } else {
        near / depth.max(near)
    };
    if z.is_finite() { z.clamp(0.0, 1.0) } else { 0.0 }
}

/// The selection highlight (Feather shows selected curves in green).
pub const SELECT_GREEN: Rgba = Rgba([0x2f, 0xc8, 0x62, 255]);
pub const GUIDE_TINT: Rgba = Rgba([0x8f, 0xb4, 0xff, 255]);
pub const GUIDE_ORANGE: Rgba = Rgba([0xff, 0x8a, 0x1f, 255]);
/// Most triangles drawn from one 3D model resource.
const MODEL_TRIS_MAX: usize = 400_000;
/// Segments per sorted chunk of a curve.
const CHUNK: usize = 6;

pub struct Options<'a> {
    pub frame: u32,
    /// Selected curves.
    pub selected: &'a HashSet<u64>,
    /// Selected resources (guides, images, models).
    pub selected_resources: &'a HashSet<u64>,
    /// Curves drawn on top of the note's own (the stroke being drawn, mirror previews).
    pub extra: &'a [Stroke],
    /// Curves not drawn (e.g. being previewed elsewhere).
    pub hide: &'a HashSet<u64>,
    /// Show guides (exports hide them).
    pub guides: bool,
    /// Grid, axes and the orbit point (exports hide them).
    pub overlays: bool,
    pub orbit_point: Option<Vec3>,
}

struct Item {
    depth: f32,
    tex: Tex,
    v0: usize,
    i0: usize,
    i1: usize,
}

struct Builder<'a> {
    view: View,
    atlas: &'a Atlas,
    env: &'a Environment,
    boil: Boil,
    frame: u32,
    verts: Vec<Vtx>,
    idx: Vec<u32>,
    items: Vec<Item>,
    /// Vertex index where the open item started.
    open: Option<(Tex, usize, usize)>,
    /// Lighting direction (world) used for shading.
    light: Vec3,
    /// Curves whose scattered dabs are still to be drawn: (curve, colour, alpha, order).
    pending_dabs: Vec<(Stroke, [f32; 3], f32, u32)>,
    /// The drawing order of the curve being built (0 for everything else).
    order: u32,
    /// The far end of the depth range.
    far: f32,
}

fn premul(c: [f32; 4]) -> [u8; 4] {
    let a = if c[3].is_finite() { c[3].clamp(0.0, 1.0) } else { 0.0 };
    let q = |v: f32| ((if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }) * a * 255.0 + 0.5) as u8;
    [q(c[0]), q(c[1]), q(c[2]), (a * 255.0 + 0.5) as u8]
}

/// Premultiplied colour with zero alpha: adds light (glow).
fn additive(c: [f32; 3], amount: f32) -> [u8; 4] {
    let q = |v: f32| ((v * amount).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    [q(c[0]), q(c[1]), q(c[2]), 0]
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn rgb(c: Rgba) -> [f32; 3] {
    let f = c.to_f32();
    [f[0], f[1], f[2]]
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = if (e1 - e0).abs() < 1e-9 { if x >= e1 { 1.0 } else { 0.0 } } else { ((x - e0) / (e1 - e0)).clamp(0.0, 1.0) };
    t * t * (3.0 - 2.0 * t)
}

/// One projected sample along a curve.
#[derive(Debug, Clone, Copy)]
struct Sample {
    x: f32,
    y: f32,
    depth: f32,
    /// Half width in pixels.
    hw: f32,
    alpha: f32,
    /// Texture u (fraction of a tile).
    u: f32,
    /// Joins the previous sample.
    connect: bool,
    /// World position and surface normal (for flat brushes).
    w: Vec3,
    n: Vec3,
    /// World half width.
    whw: f32,
}

/// A vertex across a profile: offset in half widths (-1..1, beyond for the fringe), the
/// shading angle, alpha factor and v.
#[derive(Debug, Clone, Copy)]
struct Across {
    off: f32,
    theta: f32,
    alpha: f32,
    v: f32,
    /// Extra pixels outwards, away from the centre line (the anti-aliasing fringe).
    px: f32,
}

const fn ac(off: f32, theta: f32, alpha: f32, v: f32, px: f32) -> Across {
    Across { off, theta, alpha, v, px }
}

/// A tube: five shaded vertices across plus fringes.
const TUBE: [Across; 7] = [
    ac(-1.0, -1.5, 0.0, 0.0, 1.0),
    ac(-1.0, -1.5, 1.0, 0.0, 0.0),
    ac(-std::f32::consts::FRAC_1_SQRT_2, -0.785, 1.0, 0.15, 0.0),
    ac(0.0, 0.0, 1.0, 0.5, 0.0),
    ac(std::f32::consts::FRAC_1_SQRT_2, 0.785, 1.0, 0.85, 0.0),
    ac(1.0, 1.5, 1.0, 1.0, 0.0),
    ac(1.0, 1.5, 0.0, 1.0, 1.0),
];
const BAND: [Across; 4] = [ac(-1.0, 0.0, 0.0, 0.0, 1.0), ac(-1.0, 0.0, 1.0, 0.0, 0.0), ac(1.0, 0.0, 1.0, 1.0, 0.0), ac(1.0, 0.0, 0.0, 1.0, 1.0)];
const TEXTURED: [Across; 2] = [ac(-1.0, 0.0, 1.0, 0.0, 0.0), ac(1.0, 0.0, 1.0, 1.0, 0.0)];
const SQUARE_L: [Across; 3] = [ac(-1.0, -0.7, 0.0, 0.0, 1.0), ac(-1.0, -0.7, 1.0, 0.0, 0.0), ac(0.0, -0.7, 1.0, 0.5, 0.0)];
const SQUARE_R: [Across; 3] = [ac(0.0, 0.7, 1.0, 0.5, 0.0), ac(1.0, 0.7, 1.0, 1.0, 0.0), ac(1.0, 0.7, 0.0, 1.0, 1.0)];

/// How a pass colours its vertices.
#[derive(Clone, Copy)]
enum Paint {
    /// Shaded with the stroke's material.
    Shade { base: [f32; 3], alpha: f32, material: Material, lit: bool },
    /// One flat colour (echoes, shadows, pattern marks).
    Flat([f32; 4]),
    /// Added light (glow halos).
    Add([f32; 3], f32),
}

impl<'a> Builder<'a> {
    fn new(view: View, atlas: &'a Atlas, env: &'a Environment, boil: Boil, frame: u32, light: Vec3) -> Builder<'a> {
        let far = (view.half_height * view.f * 400.0).max(view.near * 10.0);
        Builder {
            view,
            atlas,
            env,
            boil,
            frame,
            verts: Vec::new(),
            idx: Vec::new(),
            items: Vec::new(),
            open: None,
            light,
            pending_dabs: Vec::new(),
            order: 0,
            far,
        }
    }

    /// A texture row made before the frame (see [`stroke_rows`]); the solid row if missing.
    fn row(&self, key: RowKey) -> usize {
        self.atlas.find(&key).unwrap_or(0)
    }

    fn flush_dabs(&mut self) {
        for (s, base, alpha, order) in std::mem::take(&mut self.pending_dabs) {
            self.order = order;
            self.dabs(&s, base, alpha);
        }
        self.order = 0;
    }

    /// Take another builder's geometry (built in parallel) into this one.
    fn absorb(&mut self, verts: Vec<Vtx>, idx: Vec<u32>, items: Vec<Item>) {
        let (v_off, i_off) = (self.verts.len(), self.idx.len());
        self.verts.extend(verts);
        self.idx.extend(idx);
        self.items.extend(items.into_iter().map(|it| Item { v0: it.v0 + v_off, i0: it.i0 + i_off, i1: it.i1 + i_off, ..it }));
    }

    fn begin(&mut self, tex: Tex) {
        self.open = Some((tex, self.verts.len(), self.idx.len()));
    }

    /// Close the open item: its sort key is `depth` nudged by `bias` (relative) and by the
    /// current curve's drawing order; its vertices' depths get the same nudge.
    fn end(&mut self, depth: f32, bias: f32) {
        let Some((tex, v0, i0)) = self.open.take() else { return };
        if self.idx.len() <= i0 {
            self.verts.truncate(v0);
            return;
        }
        let factor = (1.0 + bias) * (1.0 - self.order as f32 * ORDER_BIAS);
        let depth = if depth.is_finite() { depth * factor } else { 0.0 };
        let mut max_alpha = 0u8;
        let (ortho, near, far) = (self.view.orthographic, self.view.near, self.far);
        if let Some(vs) = self.verts.get_mut(v0..) {
            for v in vs.iter_mut() {
                max_alpha = max_alpha.max(v.color[3]);
                v.z = depth_to_z(v.z * factor, ortho, near, far);
            }
            // Solid when fully opaque somewhere and not added light.
            let solid = max_alpha == 255;
            for v in vs.iter_mut() {
                v.solid = solid;
            }
        }
        self.items.push(Item { depth, tex, v0, i0, i1: self.idx.len() });
    }

    /// An item drawn over everything (the orbit point).
    fn end_on_top(&mut self) {
        let Some((tex, v0, i0)) = self.open.take() else { return };
        if let Some(vs) = self.verts.get_mut(v0..) {
            for v in vs.iter_mut() {
                v.z = 1.0;
                v.solid = false;
            }
        }
        if self.idx.len() > i0 {
            self.items.push(Item { depth: -1.0, tex, v0, i0, i1: self.idx.len() });
        }
    }

    /// A vertex at view depth `depth` (turned into a depth-buffer value when the item closes).
    fn vtx(&mut self, x: f32, y: f32, depth: f32, uv: [f32; 2], color: [u8; 4]) -> u32 {
        let base = self.open.map_or(0, |(_, v0, _)| v0);
        self.verts.push(Vtx { pos: [x, y], uv, color, z: depth, solid: false });
        (self.verts.len() - 1 - base) as u32
    }

    fn tri(&mut self, a: u32, b: u32, c: u32) {
        self.idx.extend_from_slice(&[a, b, c]);
    }

    fn fog(&self, c: [f32; 3], depth: f32, distance: f32) -> [f32; 3] {
        if !self.env.fog {
            return c;
        }
        let f = smoothstep(distance * 0.4, distance * 4.0, depth) * 0.92;
        mix3(c, rgb(self.env.background), f)
    }

    /// Shade a base colour for a surface normal.
    fn shade(&self, base: [f32; 3], n: Vec3, material: Material, lit: bool) -> [f32; 3] {
        let render = self.env.render_mode;
        match (render, material) {
            (true, Material::Shadeless) => return base,
            (true, Material::Glow) => return mix3(base, [1.0, 1.0, 1.0], 0.35),
            (true, Material::Cutout) => return rgb(self.env.background),
            _ => {}
        }
        if !lit {
            return base;
        }
        let l = self.light;
        let ndl = n.dot(l);
        if render {
            let li = &self.env.lighting;
            let mut lambert = ndl.max(0.0);
            if li.toon {
                lambert = if lambert > 0.6 {
                    1.0
                } else if lambert > 0.15 {
                    0.55
                } else {
                    0.1
                };
            }
            let lc = rgb(li.color);
            let s = li.strength;
            let amb = 0.38;
            let h = (l + self.view.back).normalized();
            let spec = if li.toon { 0.0 } else { n.dot(h).max(0.0).powi(32) * 0.25 * s };
            [
                base[0] * (amb + lambert * s * lc[0] * 0.75) + spec,
                base[1] * (amb + lambert * s * lc[1] * 0.75) + spec,
                base[2] * (amb + lambert * s * lc[2] * 0.75) + spec,
            ]
        } else {
            // Sketch mode: a soft studio light from the upper left of the view.
            let k = 0.66 + 0.34 * ndl.max(-0.3);
            let h = (l + self.view.back).normalized();
            let spec = n.dot(h).max(0.0).powi(24) * 0.12;
            [base[0] * k + spec, base[1] * k + spec, base[2] * k + spec]
        }
    }

    /// The depth of the surface a sample was drawn on, at screen point (x, y): strokes on the
    /// same surface get the same depth at every pixel (so drawing order can decide between
    /// them), however their ribbons are laid out on screen. Samples without a surface keep
    /// their own depth.
    fn surface_depth(&self, s: &Sample, x: f32, y: f32) -> f32 {
        if s.n == Vec3::ZERO {
            return s.depth;
        }
        let v = &self.view;
        let nc = (s.n.dot(v.right), s.n.dot(v.up), s.n.dot(v.back));
        let rel = s.w - v.eye;
        let p0 = (rel.dot(v.right), rel.dot(v.up), rel.dot(v.back));
        let k = nc.0 * p0.0 + nc.1 * p0.1 + nc.2 * p0.2;
        let (hw, hh) = (v.viewport.width * 0.5, v.viewport.height * 0.5);
        let d = if v.orthographic {
            if nc.2.abs() < 1e-3 {
                return s.depth;
            }
            let sx = (x - hw) / hh * v.half_height;
            let sy = (hh - y) / hh * v.half_height;
            (nc.0 * sx + nc.1 * sy - k) / nc.2
        } else {
            let sx = (x - hw) / hh / v.f;
            let sy = (hh - y) / hh / v.f;
            let den = nc.0 * sx + nc.1 * sy - nc.2;
            if den.abs() < 1e-6 {
                return s.depth;
            }
            k / den
        };
        // Grazing surfaces: stay near the sample.
        if d.is_finite() { d.clamp(s.depth - s.whw * 4.0 - 1e-4, s.depth + s.whw * 4.0 + 1e-4) } else { s.depth }
    }

    fn colour(&self, paint: &Paint, n: Vec3, alpha: f32, depth: f32, distance: f32) -> [u8; 4] {
        match *paint {
            Paint::Shade { base, alpha: a, material, lit } => {
                let c = self.shade(base, n, material, lit);
                let c = self.fog(c, depth, distance);
                premul([c[0], c[1], c[2], a * alpha])
            }
            Paint::Flat(c) => {
                let f = self.fog([c[0], c[1], c[2]], depth, distance);
                premul([f[0], f[1], f[2], c[3] * alpha])
            }
            Paint::Add(c, amt) => additive(c, amt * alpha),
        }
    }

    /// Emit strips across a run of samples, chunked for sorting.
    #[allow(clippy::too_many_arguments)]
    fn strips(&mut self, run: &[Sample], profiles: &[&[Across]], paint: Paint, row: usize, offset: [f32; 2], width: f32, bias: f32, distance: f32) {
        if run.len() < 2 {
            return;
        }
        let back = self.view.back;
        let (right, up) = (self.view.right, self.view.up);
        let n = run.len();
        // Screen normals per sample.
        let normals: Vec<[f32; 2]> = (0..n)
            .map(|i| {
                let a = run[i.saturating_sub(1)];
                let b = run[(i + 1).min(n - 1)];
                let (dx, dy) = (b.x - a.x, b.y - a.y);
                let l = (dx * dx + dy * dy).sqrt();
                if l > 1e-6 { [-dy / l, dx / l] } else { [0.0, 1.0] }
            })
            .collect();
        let mut start = 0usize;
        while start + 1 < n {
            let end = (start + CHUNK).min(n - 1);
            self.begin(Tex::Atlas);
            let mut depth = 0.0;
            for prof in profiles {
                let cols = prof.len() as u32;
                let mut first: Option<u32> = None;
                for (k, s) in run.iter().enumerate().take(end + 1).skip(start) {
                    let [nx, ny] = normals[k];
                    let hw = s.hw * width;
                    let across_w = (right * nx - up * ny).normalized();
                    for a in prof.iter() {
                        let d = a.off * hw + a.px * a.off.signum();
                        let (x, y) = (s.x + nx * d + offset[0], s.y + ny * d + offset[1]);
                        let normal = back * a.theta.cos() + across_w * a.theta.sin();
                        let col = self.colour(&paint, normal, s.alpha * a.alpha, s.depth, distance);
                        let uv = self.atlas.uv(row, s.u, a.v);
                        let d = self.surface_depth(s, x, y);
                        let id = self.vtx(x, y, d, uv, col);
                        first.get_or_insert(id);
                    }
                }
                let Some(f) = first else { continue };
                for (j, k) in (start..end).enumerate() {
                    if !run[k + 1].connect {
                        continue;
                    }
                    let r0 = f + j as u32 * cols;
                    let r1 = r0 + cols;
                    for c in 0..cols.saturating_sub(1) {
                        self.tri(r0 + c, r0 + c + 1, r1 + c + 1);
                        self.tri(r0 + c, r1 + c + 1, r1 + c);
                    }
                }
            }
            for s in run.iter().take(end + 1).skip(start) {
                depth += s.depth;
            }
            let d = depth / (end + 1 - start) as f32;
            self.end(d, bias);
            start = end;
        }
    }

    /// A round end cap (a shaded disc).
    fn cap(&mut self, s: &Sample, paint: Paint, offset: [f32; 2], width: f32, bias: f32, distance: f32) {
        let r = s.hw * width;
        if r < 0.3 {
            return;
        }
        let back = self.view.back;
        let (right, up) = (self.view.right, self.view.up);
        self.begin(Tex::Atlas);
        let uv = self.atlas.solid_uv();
        let (cx, cy) = (s.x + offset[0], s.y + offset[1]);
        let centre = self.colour(&paint, back, s.alpha, s.depth, distance);
        let c0 = self.vtx(cx, cy, self.surface_depth(s, cx, cy), uv, centre);
        let segs = ((r * 0.8) as usize).clamp(8, 28);
        let mut ring = Vec::with_capacity(segs);
        let mut fringe = Vec::with_capacity(segs);
        for i in 0..segs {
            let a = std::f32::consts::TAU * i as f32 / segs as f32;
            let (ca, sa) = (a.cos(), a.sin());
            let nrm = (back * 0.35 + (right * ca - up * sa)).normalized();
            let col = self.colour(&paint, nrm, s.alpha, s.depth, distance);
            let (rx, ry) = (cx + ca * r, cy + sa * r);
            ring.push(self.vtx(rx, ry, self.surface_depth(s, rx, ry), uv, col));
            let (fx, fy) = (cx + ca * (r + 1.0), cy + sa * (r + 1.0));
            fringe.push(self.vtx(fx, fy, self.surface_depth(s, fx, fy), uv, [0, 0, 0, 0]));
        }
        for i in 0..segs {
            let j = (i + 1) % segs;
            self.tri(c0, ring[i], ring[j]);
            self.tri(ring[i], fringe[i], fringe[j]);
            self.tri(ring[i], fringe[j], ring[j]);
        }
        self.end(s.depth, bias);
    }

    /// A screen-space line through world points, split so each piece sorts on its own depth.
    fn line(&mut self, pts: &[Vec3], width: f32, color: [f32; 4], bias: f32) {
        let uv = self.atlas.solid_uv();
        for w in pts.windows(2) {
            let (Some(a), Some(b)) = (self.clip_project(w[0], w[1]), self.clip_project(w[1], w[0])) else { continue };
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let l = (dx * dx + dy * dy).sqrt();
            if l < 1e-4 {
                continue;
            }
            let (nx, ny) = (-dy / l * width * 0.5, dx / l * width * 0.5);
            let (fx, fy) = (-dy / l, dx / l);
            self.begin(Tex::Atlas);
            let c = premul(color);
            let z = [0, 0, 0, 0];
            let p = [
                self.vtx(a.0 - nx - fx, a.1 - ny - fy, a.2, uv, z),
                self.vtx(a.0 - nx, a.1 - ny, a.2, uv, c),
                self.vtx(a.0 + nx, a.1 + ny, a.2, uv, c),
                self.vtx(a.0 + nx + fx, a.1 + ny + fy, a.2, uv, z),
                self.vtx(b.0 - nx - fx, b.1 - ny - fy, b.2, uv, z),
                self.vtx(b.0 - nx, b.1 - ny, b.2, uv, c),
                self.vtx(b.0 + nx, b.1 + ny, b.2, uv, c),
                self.vtx(b.0 + nx + fx, b.1 + ny + fy, b.2, uv, z),
            ];
            for i in 0..3 {
                self.tri(p[i], p[i + 1], p[i + 5]);
                self.tri(p[i], p[i + 5], p[i + 4]);
            }
            self.end((a.2 + b.2) * 0.5, bias);
        }
    }

    /// Project `p`, moving it towards `other` onto the near plane when it is behind the camera.
    fn clip_project(&self, p: Vec3, other: Vec3) -> Option<(f32, f32, f32)> {
        if let Some(s) = self.view.project(p) {
            return Some((s.x, s.y, s.depth));
        }
        let depth = |q: Vec3| -(q - self.view.eye).dot(self.view.back);
        let (dp, dq) = (depth(p), depth(other));
        let near = self.view.near * 1.01;
        if dq <= near || (dq - dp).abs() < 1e-9 {
            return None;
        }
        let t = (near - dp) / (dq - dp);
        let q = p.lerp(other, t);
        self.view.project(q).map(|s| (s.x, s.y, s.depth))
    }

    fn triangle3(&mut self, tex: Tex, p: [Vec3; 3], uv: [[f32; 2]; 3], cols: [[u8; 4]; 3], bias: f32) {
        let (Some(a), Some(b), Some(c)) = (self.view.project(p[0]), self.view.project(p[1]), self.view.project(p[2])) else { return };
        self.begin(tex);
        let ia = self.vtx(a.x, a.y, a.depth, uv[0], cols[0]);
        let ib = self.vtx(b.x, b.y, b.depth, uv[1], cols[1]);
        let ic = self.vtx(c.x, c.y, c.depth, uv[2], cols[2]);
        self.tri(ia, ib, ic);
        self.end((a.depth + b.depth + c.depth) / 3.0, bias);
    }

    fn finish(mut self) -> Frame {
        self.flush_dabs();
        self.items.sort_by(|a, b| b.depth.total_cmp(&a.depth));
        // Atlas coordinates were row-space while rows were being added.
        let mut is_atlas = vec![false; self.verts.len()];
        for it in &self.items {
            if it.tex == Tex::Atlas
                && let Some(idx) = self.idx.get(it.i0..it.i1)
            {
                for i in idx {
                    if let Some(f) = is_atlas.get_mut(it.v0 + *i as usize) {
                        *f = true;
                    }
                }
            }
        }
        for (v, a) in self.verts.iter_mut().zip(is_atlas) {
            if a {
                v.uv = self.atlas.normalize(v.uv);
            }
        }
        let mut frame = Frame::default();
        for it in &self.items {
            let need_new = frame.batches.last().is_none_or(|b: &Batch| b.tex != it.tex || b.vertices.len() > 60_000);
            if need_new {
                frame.batches.push(Batch { tex: it.tex, vertices: Vec::new(), indices: Vec::new() });
            }
            let Some(batch) = frame.batches.last_mut() else { continue };
            let base = batch.vertices.len() as u32;
            let (Some(idx), true) = (self.idx.get(it.i0..it.i1), it.v0 <= self.verts.len()) else { continue };
            let max_local = idx.iter().copied().max().unwrap_or(0) as usize;
            let Some(vs) = self.verts.get(it.v0..=it.v0 + max_local) else { continue };
            batch.vertices.extend_from_slice(vs);
            batch.indices.extend(idx.iter().map(|i| i + base));
            frame.triangles += idx.len() / 3;
        }
        frame
    }
}

/// Everything one curve needs from the boil for one frame.
struct BoilAt {
    amp: f32,
    world: bool,
    wavelength: f32,
    thickness: f32,
    seed: u32,
}

impl<'a> Builder<'a> {
    /// The curve's samples on screen, with the boil applied, split into visible runs.
    fn runs(&self, s: &Stroke, textured_scale: Option<f32>) -> Vec<Vec<Sample>> {
        let b = &self.boil;
        let active = b.enabled && b.amount > 0.0 && s.brush.paint.boil > 0.0;
        let ba = BoilAt {
            amp: if active { b.amount * s.brush.paint.boil } else { 0.0 },
            world: b.world_space,
            wavelength: b.wavelength.max(4.0),
            thickness: if active { b.thickness } else { 0.0 },
            seed: hash2(s.seed, self.frame.wrapping_mul(0x9e37_79b9).wrapping_add(17)),
        };
        let radius = s.brush.radius();
        let pressure = s.brush.pressure;
        let pts = &s.points;
        // World arc length (stable texture coordinates and world boil).
        let mut arc = Vec::with_capacity(pts.len());
        let mut acc = 0.0f32;
        for (i, p) in pts.iter().enumerate() {
            if i > 0 {
                acc += p.p.distance(pts[i - 1].p);
            }
            arc.push(acc);
        }
        let total_w = acc.max(1e-9);
        let mut runs: Vec<Vec<Sample>> = Vec::new();
        let mut cur: Vec<Sample> = Vec::new();
        let mut arc_px = 0.0f32;
        let mut last_px: Option<(f32, f32)> = None;
        let taper = s.brush.paint.taper;
        let world_amp = ba.amp * 0.004;
        let world_wave = ba.wavelength * 0.004;
        let mut tracks = [Track::new(ba.seed ^ 1), Track::new(ba.seed ^ 2), Track::new(ba.seed ^ 3)];
        let mut wtracks = [Track::new(ba.seed ^ 1), Track::new(ba.seed ^ 2), Track::new(ba.seed ^ 3)];
        for (i, p) in pts.iter().enumerate() {
            let mut w = p.p;
            if ba.world && ba.amp > 0.0 {
                let t = arc[i] / world_wave;
                let [a, b, c] = &mut wtracks;
                w += v3(a.at(t), b.at(t), c.at(t)) * world_amp;
            }
            let Some(pr) = self.view.project(w) else {
                if !cur.is_empty() {
                    runs.push(std::mem::take(&mut cur));
                }
                last_px = None;
                continue;
            };
            let (mut x, mut y) = (pr.x, pr.y);
            if let Some((lx, ly)) = last_px {
                arc_px += ((x - lx).powi(2) + (y - ly).powi(2)).sqrt();
            }
            last_px = Some((x, y));
            let t = arc_px / ba.wavelength;
            if !ba.world && ba.amp > 0.0 {
                x += tracks[0].at(t) * ba.amp;
                y += tracks[1].at(t) * ba.amp;
            }
            let pf = if pressure { 0.15 + 0.85 * p.pressure } else { 1.0 };
            let frac = arc[i] / total_w;
            let tf = if taper > 0.0 {
                let r = taper * 0.5;
                (smoothstep(0.0, r, frac) * smoothstep(0.0, r, 1.0 - frac)).max(0.06)
            } else {
                1.0
            };
            let wob = if ba.thickness > 0.0 { 1.0 + ba.thickness * tracks[2].at(t * 1.7) * 0.6 } else { 1.0 };
            let mut hw = radius * pr.scale * pf * tf * wob;
            let mut alpha = 1.0;
            if hw < 0.5 {
                alpha = (hw / 0.5).max(0.05);
                hw = 0.5;
            }
            let u = textured_scale.map_or(0.5, |k| arc[i] / (2.0 * radius) / TILE_WIDTHS * k);
            cur.push(Sample { x, y, depth: pr.depth, hw, alpha, u, connect: true, w, n: p.n, whw: radius * pf * tf * wob });
        }
        if !cur.is_empty() {
            runs.push(cur);
        }
        for run in &mut runs {
            // Only the samples the picture needs: drop those within a third of a pixel of the
            // straight run between their neighbours (and of its width).
            if run.len() > 2 {
                *run = simplify(run, 0.33);
            }
            if textured_scale.is_some() {
                *run = wrap_u(run);
            }
        }
        runs
    }
}

/// Greedy screen-space simplification: keep a sample when skipping it would move the line (or
/// its edge) more than `tol` pixels. Each step checks the newest skipped sample and the worst one
/// so far (constant work per sample); runs of at most 48 samples are bridged at a time.
fn simplify(run: &[Sample], tol: f32) -> Vec<Sample> {
    let n = run.len();
    let mut out = Vec::with_capacity(n / 2 + 2);
    let Some(first) = run.first() else { return out };
    out.push(*first);
    let error = |a: &Sample, b: &Sample, m: &Sample, f: f32| -> f32 {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let len2 = dx * dx + dy * dy;
        let t = if len2 > 1e-9 { (((m.x - a.x) * dx + (m.y - a.y) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
        let (px, py) = (a.x + dx * t, a.y + dy * t);
        let off = ((m.x - px).powi(2) + (m.y - py).powi(2)).sqrt();
        let hw = a.hw + (b.hw - a.hw) * f;
        let alpha_err = ((m.alpha - (a.alpha + (b.alpha - a.alpha) * f)).abs() - 0.04).max(0.0) * 100.0;
        off + (m.hw - hw).abs() + alpha_err + if m.connect { 0.0 } else { 1e6 }
    };
    let mut anchor = 0usize;
    let mut worst = 1usize;
    let mut j = 2usize;
    while j < n {
        let a = run[anchor];
        let b = run[j];
        let span = (j - anchor) as f32;
        let check = |k: usize| error(&a, &b, &run[k], (k - anchor) as f32 / span);
        let newest = j - 1;
        let (e_new, e_worst) = (check(newest), if worst > anchor && worst < j { check(worst) } else { 0.0 });
        if j - anchor > 48 || e_new > tol || e_worst > tol {
            anchor = j - 1;
            worst = anchor + 1;
            out.push(run[anchor]);
        } else if e_new > e_worst {
            worst = newest;
        }
        j += 1;
    }
    if let Some(last) = run.last()
        && anchor != n - 1
    {
        out.push(*last);
    }
    out
}

/// Insert seam samples where the texture coordinate crosses a tile edge, so no quad spans it.
fn wrap_u(run: &[Sample]) -> Vec<Sample> {
    let mut out = Vec::with_capacity(run.len() + 8);
    for (i, s) in run.iter().enumerate() {
        if i == 0 {
            out.push(Sample { u: s.u - s.u.floor(), ..*s });
            continue;
        }
        let prev = run[i - 1];
        let (a, b) = (prev.u, s.u);
        let (fa, fb) = (a.floor(), b.floor());
        if fb > fa && b > a && fb - fa < 64.0 {
            let mut edge = fa + 1.0;
            while edge <= fb {
                let t = ((edge - a) / (b - a)).clamp(0.0, 1.0);
                let lerp = |p: f32, q: f32| p + (q - p) * t;
                let m = Sample {
                    x: lerp(prev.x, s.x),
                    y: lerp(prev.y, s.y),
                    depth: lerp(prev.depth, s.depth),
                    hw: lerp(prev.hw, s.hw),
                    alpha: lerp(prev.alpha, s.alpha),
                    u: 1.0,
                    connect: true,
                    w: prev.w.lerp(s.w, t),
                    n: prev.n,
                    whw: lerp(prev.whw, s.whw),
                };
                out.push(m);
                out.push(Sample { u: 0.0, connect: false, ..m });
                edge += 1.0;
            }
        }
        out.push(Sample { u: s.u - s.u.floor(), ..*s });
    }
    out
}

impl<'a> Builder<'a> {
    fn stroke(&mut self, s: &Stroke, selected: bool) {
        if s.points.is_empty() {
            return;
        }
        let br = &s.brush;
        let distance = self.cam_distance();
        let mut base = rgb(br.color);
        if selected {
            base = mix3(base, rgb(SELECT_GREEN), 0.75);
        }
        let alpha = br.opacity;
        let kind = br.kind;
        let render = self.env.render_mode;
        let material = br.material;
        let lit = matches!(kind, BrushKind::Pen | BrushKind::Square | BrushKind::Flat);
        let shade = Paint::Shade { base, alpha, material, lit };

        // Ground shadow (render mode, shaded curves).
        if render && self.env.lighting.ground_shadow && material == Material::Shaded {
            self.ground_shadow(s, alpha);
        }

        if br.paint.scatter > 0.0 {
            // Drawn after (in front of) the stroke itself; see `dabs`.
            self.pending_dabs.push((s.clone(), base, alpha, self.order));
        }
        if kind == BrushKind::Flat {
            self.flat(s, shade, distance);
        } else if kind.painterly() {
            self.painterly(s, base, alpha, material, selected, distance);
        } else {
            let runs = self.runs(s, None);
            let solid = 0usize;
            // Echo: the same curve again, offset and recoloured, behind.
            if let Some(e) = br.paint.echo {
                let c = rgb(e.color);
                for run in &runs {
                    self.strips(run, &[&BAND], Paint::Flat([c[0], c[1], c[2], alpha * e.color.to_f32()[3]]), solid, e.offset, e.width, ECHO_BEHIND, distance);
                }
            }
            if render && material == Material::Glow {
                self.halo(&runs, base, br.glow, distance);
            }
            for run in &runs {
                match kind {
                    BrushKind::Pen => {
                        if run.len() == 1 {
                            self.cap(&run[0], shade, [0.0, 0.0], 1.0, 0.0, distance);
                        } else {
                            // Hairlines don't need five shaded columns.
                            let thin = run.iter().map(|s| s.hw).fold(0.0f32, f32::max) < 1.6;
                            self.strips(run, &[if thin { &BAND } else { &TUBE }], shade, solid, [0.0, 0.0], 1.0, 0.0, distance);
                            if let (Some(a), Some(b)) = (run.first().copied(), run.last().copied()) {
                                self.cap(&a, shade, [0.0, 0.0], 1.0, 0.0, distance);
                                self.cap(&b, shade, [0.0, 0.0], 1.0, 0.0, distance);
                            }
                        }
                    }
                    BrushKind::Square => self.strips(run, &[&SQUARE_L, &SQUARE_R], shade, solid, [0.0, 0.0], 1.0, 0.0, distance),
                    BrushKind::Nib => {
                        let mut r = run.clone();
                        nib_widths(&mut r);
                        self.strips(&r, &[&BAND], shade, solid, [0.0, 0.0], 1.0, 0.0, distance);
                    }
                    _ => {
                        self.strips(run, &[&BAND], shade, solid, [0.0, 0.0], 1.0, 0.0, distance);
                        if (run.len() == 1 || kind == BrushKind::Marker)
                            && let (Some(a), Some(b)) = (run.first().copied(), run.last().copied())
                        {
                            self.cap(&a, shade, [0.0, 0.0], 1.0, 0.0, distance);
                            self.cap(&b, shade, [0.0, 0.0], 1.0, 0.0, distance);
                        }
                    }
                }
            }
            if let Some(p) = br.pattern
                && (!render || matches!(material, Material::Shadeless | Material::Shaded))
            {
                let runs_t = self.runs(s, Some(1.0));
                let row = self.row(RowKey::pattern(p.kind, p.angle, p.contrast));
                let mark = [base[0] * 0.3, base[1] * 0.3, base[2] * 0.3, alpha * p.intensity];
                for run in &runs_t {
                    self.strips(run, &[&TEXTURED], Paint::Flat(mark), row, [0.0, 0.0], 0.92, -2e-4, distance);
                }
            }
        }
    }

    fn cam_distance(&self) -> f32 {
        self.view.half_height * self.view.f
    }

    fn halo(&mut self, runs: &[Vec<Sample>], base: [f32; 3], glow: f32, distance: f32) {
        let row = self.row(RowKey::Halo);
        let size = 2.5 + glow * 4.0 + self.env.effects.glow * 6.0;
        for run in runs {
            let r = wrap_u(run);
            self.strips(&r, &[&TEXTURED], Paint::Add(base, 0.35 + glow * 0.6), row, [0.0, 0.0], size, 3e-4, distance);
        }
    }

    fn painterly(&mut self, s: &Stroke, base: [f32; 3], alpha: f32, material: Material, selected: bool, distance: f32) {
        let br = &s.brush;
        let kind = br.kind;
        let variant = if self.boil.enabled && br.paint.boil > 0.0 { self.frame } else { 0 };
        let runs = self.runs(s, Some(1.0));
        let render = self.env.render_mode;
        let color = |c: [f32; 3]| -> [f32; 3] { if render && material == Material::Cutout { rgb(self.env.background) } else { c } };
        let dry_ends = br.paint.roughness > 0.0 || br.paint.dryness > 0.0;
        let (start_paint, tail_paint) = dry_paints(&br.paint);
        if let Some(e) = br.paint.echo {
            let v = variant.wrapping_add(2);
            let rows =
                [self.row(RowKey::paint(kind, &start_paint, v)), self.row(RowKey::paint(kind, &br.paint, v)), self.row(RowKey::paint(kind, &tail_paint, v))];
            let c = color(rgb(e.color));
            let paint = Paint::Flat([c[0], c[1], c[2], alpha * e.color.to_f32()[3]]);
            for run in &runs {
                self.dry_ended(run, dry_ends, rows, paint, e.offset, e.width, ECHO_BEHIND, distance);
            }
        }
        if render && material == Material::Glow {
            self.halo(&runs, base, br.glow, distance);
        }
        let layers = br.paint.layers.clamp(1, 4) as u32;
        for layer in (0..layers).rev() {
            let row = self.row(RowKey::paint(kind, &br.paint, variant.wrapping_add(layer)));
            let start_row = self.row(RowKey::paint(kind, &start_paint, variant.wrapping_add(layer)));
            let tail_row = self.row(RowKey::paint(kind, &tail_paint, variant.wrapping_add(layer)));
            let h = hash2(s.seed, layer.wrapping_add(self.frame.wrapping_mul(31)));
            let (off, width, tint) = if layer == 0 {
                ([0.0, 0.0], 1.0, 1.0)
            } else {
                let mag = runs.first().and_then(|r| r.first()).map_or(1.0, |f| f.hw) * 0.45;
                ([signed(h) * mag, signed(h ^ 9) * mag], 0.8 + 0.3 * unit(h ^ 3), 0.9 + 0.2 * unit(h ^ 5))
            };
            let mut c = color(base);
            if !selected {
                c = [c[0] * tint, c[1] * tint, c[2] * tint];
                if layer > 0 {
                    c = jitter_colour(c, br.paint.color_jitter, h ^ 0x77);
                }
            }
            let c = if render && material == Material::Glow { mix3(c, [1.0, 1.0, 1.0], 0.35) } else { c };
            let paint = Paint::Flat([c[0], c[1], c[2], alpha]);
            for run in &runs {
                let bias = layer as f32 * LAYER_BEHIND;
                self.dry_ended(run, dry_ends, [start_row, row, tail_row], paint, off, width, bias, distance);
            }
        }
    }

    /// Scattered brush dabs along a curve (geometry-nodes style brushstroke instances): each dab
    /// is a textured card turned along the stroke, with random size, offset, turn and colour,
    /// re-rolled every boil frame.
    fn dabs(&mut self, s: &Stroke, base: [f32; 3], alpha: f32) {
        let pa = s.brush.paint;
        let row = self.row(RowKey::Dabs { variant: (self.frame % crate::texture::VARIANTS) as u8 });
        let distance = self.cam_distance();
        let runs = self.runs(s, None);
        let frame_seed = if self.boil.enabled && pa.boil > 0.0 { self.frame } else { 0 };
        let mut k: u32 = 0;
        for run in &runs {
            if run.len() < 2 {
                continue;
            }
            let mut carry = 0.0f32;
            for w in run.windows(2) {
                let (a, b) = (w[0], w[1]);
                let (dx, dy) = (b.x - a.x, b.y - a.y);
                let seg = (dx * dx + dy * dy).sqrt();
                if seg < 1e-4 {
                    continue;
                }
                let hw = (a.hw + b.hw) * 0.5;
                let spacing = (hw * 2.0 * (1.1 - pa.scatter * 0.9)).max(2.0);
                let mut d = spacing - carry;
                while d <= seg {
                    let t = d / seg;
                    let (x, y) = (a.x + dx * t, a.y + dy * t);
                    let depth = a.depth + (b.depth - a.depth) * t;
                    let h = hash2(s.seed ^ k.wrapping_mul(0x9e37_79b9), frame_seed.wrapping_mul(0x85eb_ca6b));
                    k = k.wrapping_add(1);
                    let ang = dy.atan2(dx) + signed(h) * pa.jitter * 1.1;
                    let (ca, sa) = (ang.cos(), ang.sin());
                    let (nx, ny) = (-dy / seg, dx / seg);
                    let off_n = signed(h ^ 1) * pa.jitter * hw * 1.2;
                    let off_t = signed(h ^ 2) * pa.jitter * hw * 0.8;
                    let (cx, cy) = (x + nx * off_n + dx / seg * off_t, y + ny * off_n + dy / seg * off_t);
                    let len = hw * 2.4 * pa.dab_size * (0.7 + 0.6 * unit(h ^ 3));
                    let wid = hw * 1.15 * pa.dab_size * (0.7 + 0.6 * unit(h ^ 4));
                    let shape = (h >> 9) % crate::texture::DABS;
                    let (u0, u1) = (shape as f32 / crate::texture::DABS as f32, (shape + 1) as f32 / crate::texture::DABS as f32 - 1e-3);
                    let c = jitter_colour(base, pa.color_jitter, h ^ 5);
                    let col = self.colour(&Paint::Flat([c[0], c[1], c[2], alpha * (0.75 + 0.25 * unit(h ^ 6))]), self.view.back, 1.0, depth, distance);
                    self.begin(Tex::Atlas);
                    let corner = |su: f32, sv: f32| (cx + ca * len * 0.5 * su - sa * wid * 0.5 * sv, cy + sa * len * 0.5 * su + ca * wid * 0.5 * sv);
                    let p00 = corner(-1.0, -1.0);
                    let p10 = corner(1.0, -1.0);
                    let p11 = corner(1.0, 1.0);
                    let p01 = corner(-1.0, 1.0);
                    let uv = |u: f32, v: f32| self.atlas.uv(row, u, v);
                    let (q00, q10, q11, q01) = (uv(u0, 0.0), uv(u1, 0.0), uv(u1, 1.0), uv(u0, 1.0));
                    let at = Sample { x: cx, y: cy, depth, ..a };
                    let i0 = self.vtx(p00.0, p00.1, self.surface_depth(&at, p00.0, p00.1), q00, col);
                    let i1 = self.vtx(p10.0, p10.1, self.surface_depth(&at, p10.0, p10.1), q10, col);
                    let i2 = self.vtx(p11.0, p11.1, self.surface_depth(&at, p11.0, p11.1), q11, col);
                    let i3 = self.vtx(p01.0, p01.1, self.surface_depth(&at, p01.0, p01.1), q01, col);
                    self.tri(i0, i1, i2);
                    self.tri(i0, i2, i3);
                    self.end(depth, -DABS_FRONT - (k % 7) as f32 * 1e-6);
                    d += spacing * (0.8 + 0.4 * unit(h ^ 7));
                }
                carry = seg - (d - spacing).max(0.0);
                carry = carry.clamp(0.0, spacing);
            }
        }
    }

    /// A painterly run with dry-brush ends: a stroke starts a little dry and runs out of paint at
    /// its tail, where it breaks into bristle streaks instead of a smooth point. The dry texture
    /// cross-fades in over the end stretches, so there is no seam. `rows` = [start, body, tail].
    #[allow(clippy::too_many_arguments)]
    fn dry_ended(&mut self, run: &[Sample], dry: bool, rows: [usize; 3], paint: Paint, off: [f32; 2], width: f32, bias: f32, distance: f32) {
        let n = run.len();
        if !dry || n < 6 {
            self.strips(run, &[&TEXTURED], paint, rows[1], off, width, bias, distance);
            return;
        }
        let (a, b) = end_split(run);
        let ease = |t: f32| {
            let t = t.clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        // The body, fading out over both end stretches.
        let mut body = run.to_vec();
        for (i, s) in body.iter_mut().enumerate() {
            if a > 0 && i < a {
                s.alpha *= ease(i as f32 / a as f32);
            }
            if i > b && n - 1 > b {
                s.alpha *= 1.0 - ease((i - b) as f32 / (n - 1 - b) as f32);
            }
        }
        self.strips(&body, &[&TEXTURED], paint, rows[1], off, width, bias, distance);
        if a > 0 {
            let mut start: Vec<Sample> = run[..=a].to_vec();
            for (i, s) in start.iter_mut().enumerate() {
                s.alpha *= 1.0 - ease(i as f32 / a as f32);
            }
            self.strips(&start, &[&TEXTURED], paint, rows[0], off, width, bias - 1e-5, distance);
        }
        if b + 1 < n {
            let mut tail: Vec<Sample> = run[b..].to_vec();
            let len = (n - 1 - b).max(1) as f32;
            for (i, s) in tail.iter_mut().enumerate() {
                s.alpha *= ease(i as f32 / len);
            }
            self.strips(&tail, &[&TEXTURED], paint, rows[2], off, width, bias - 1e-5, distance);
        }
    }

    /// Tape lying on the surface it was drawn on.
    fn flat(&mut self, s: &Stroke, paint: Paint, distance: f32) {
        let runs = self.runs(s, None);
        let uv = self.atlas.solid_uv();
        let back = self.view.back;
        for run in &runs {
            if run.len() < 2 {
                continue;
            }
            let n = run.len();
            let mut start = 0;
            while start + 1 < n {
                let end = (start + CHUNK).min(n - 1);
                self.begin(Tex::Atlas);
                let mut ids = Vec::with_capacity((end - start + 1) * 2);
                let mut depth = 0.0;
                for k in start..=end {
                    let sm = run[k];
                    let a = run[k.saturating_sub(1)].w;
                    let b = run[(k + 1).min(n - 1)].w;
                    let t = (b - a).normalized();
                    let normal = if sm.n == Vec3::ZERO { back } else { sm.n };
                    let side = t.cross(normal).normalized() * sm.whw;
                    let (p0, p1) = (self.view.project(sm.w - side), self.view.project(sm.w + side));
                    let (Some(p0), Some(p1)) = (p0, p1) else {
                        ids.push(None);
                        continue;
                    };
                    // Keep the screen offset of the boil.
                    let (ox, oy) = (sm.x - (p0.x + p1.x) * 0.5, sm.y - (p0.y + p1.y) * 0.5);
                    let nn = if normal.dot(back) < 0.0 { -normal } else { normal };
                    let col = self.colour(&paint, nn, sm.alpha, sm.depth, distance);
                    let a = self.vtx(p0.x + ox, p0.y + oy, p0.depth, uv, col);
                    let b = self.vtx(p1.x + ox, p1.y + oy, p1.depth, uv, col);
                    ids.push(Some((a, b)));
                    depth += sm.depth;
                }
                for w in ids.windows(2) {
                    if let (Some((a0, b0)), Some((a1, b1))) = (w[0], w[1]) {
                        self.tri(a0, b0, b1);
                        self.tri(a0, b1, a1);
                    }
                }
                self.end(depth / (end + 1 - start) as f32, 0.0);
                start = end;
            }
        }
    }

    fn ground_shadow(&mut self, s: &Stroke, alpha: f32) {
        let l = self.light;
        if l.y < 0.05 {
            return;
        }
        let pts: Vec<crate::model::Point> =
            s.points.iter().filter(|p| p.p.y >= -1e-4).map(|p| crate::model::Point { p: p.p - l * (p.p.y / l.y), pressure: p.pressure, n: Vec3::Y }).collect();
        if pts.len() < 2 {
            return;
        }
        let mut shadow = Stroke { points: pts, ..s.clone() };
        shadow.brush.kind = BrushKind::Flat;
        shadow.brush.paint.boil = s.brush.paint.boil;
        let distance = self.cam_distance();
        self.flat(&shadow, Paint::Flat([0.0, 0.0, 0.0, 0.22 * alpha]), distance);
    }

    fn guide(&mut self, g: &Guide, state: ResourceState, selected: bool) {
        if state == ResourceState::Hidden {
            return;
        }
        let active = state == ResourceState::Active;
        let op = if active { g.opacity } else { g.opacity * 0.5 };
        let tint = rgb(if selected { SELECT_GREEN } else { GUIDE_TINT });
        let back = self.view.back;
        let uv = self.atlas.solid_uv();
        if op > 0.0 {
            for grid in &g.grids {
                for [a, b, c] in grid.triangles() {
                    let n = (b - a).cross(c - a).normalized();
                    let facing = n.dot(back).abs();
                    let k = 0.78 + 0.22 * facing;
                    let col = premul([tint[0] * k, tint[1] * k, tint[2] * k, op * 0.5]);
                    self.triangle3(Tex::Atlas, [a, b, c], [uv; 3], [col; 3], 0.0);
                }
            }
        }
        let line_a = (op * 1.4).clamp(0.0, 0.9);
        if line_a > 0.01 {
            let lc = mix3(tint, [0.1, 0.15, 0.3], 0.45);
            for l in g.section_lines() {
                self.line(&l, 1.0, [lc[0], lc[1], lc[2], line_a], -2e-4);
            }
        }
        if let Some(l) = g.start_line() {
            let o = rgb(GUIDE_ORANGE);
            self.line(&l, 2.2, [o[0], o[1], o[2], (op * 2.0 + 0.35).min(1.0)], -3e-4);
        }
    }

    fn image(&mut self, im: &ImageResource, selected: bool) {
        if im.state == ResourceState::Hidden {
            return;
        }
        let c = im.corners();
        let a = im.opacity.clamp(0.0, 1.0).max(0.04);
        let tint = if selected { mix3([1.0; 3], rgb(SELECT_GREEN), 0.35) } else { [1.0; 3] };
        let col = premul([tint[0], tint[1], tint[2], a]);
        let tex = Tex::Image(im.id);
        // Split into a grid so the quad sorts well against curves.
        let n = 4;
        for j in 0..n {
            for i in 0..n {
                let f = |u: f32, v: f32| {
                    let bottom = c[0].lerp(c[1], u);
                    let top = c[3].lerp(c[2], u);
                    bottom.lerp(top, v)
                };
                let (u0, u1, v0, v1) = (i as f32 / n as f32, (i + 1) as f32 / n as f32, j as f32 / n as f32, (j + 1) as f32 / n as f32);
                let uv = |u: f32, v: f32| [u, 1.0 - v];
                self.triangle3(tex, [f(u0, v0), f(u1, v0), f(u1, v1)], [uv(u0, v0), uv(u1, v0), uv(u1, v1)], [col; 3], 1e-4);
                self.triangle3(tex, [f(u0, v0), f(u1, v1), f(u0, v1)], [uv(u0, v0), uv(u1, v1), uv(u0, v1)], [col; 3], 1e-4);
            }
        }
    }

    fn model(&mut self, m: &ModelResource, selected: bool) {
        if m.state == ResourceState::Hidden {
            return;
        }
        let uv = self.atlas.solid_uv();
        let base = if selected { mix3(rgb(m.color), rgb(SELECT_GREEN), 0.6) } else { rgb(m.color) };
        // Models are solid (they hide what's behind them), a little lighter when not drawn on.
        let alpha = 1.0;
        let base = if m.state == ResourceState::Active { base } else { mix3(base, [1.0, 1.0, 1.0], 0.25) };
        let back = self.view.back;
        let world: Vec<Vec3> = m.positions.iter().map(|p| m.xform.apply(*p)).collect();
        for t in m.triangles.iter().take(MODEL_TRIS_MAX) {
            let (Some(a), Some(b), Some(c)) = (world.get(t[0] as usize), world.get(t[1] as usize), world.get(t[2] as usize)) else { continue };
            let mut n = (*b - *a).cross(*c - *a).normalized();
            if n.dot(back) < 0.0 {
                n = -n;
            }
            let col = self.shade(base, n, Material::Shaded, true);
            let c4 = premul([col[0], col[1], col[2], alpha]);
            self.triangle3(Tex::Atlas, [*a, *b, *c], [uv; 3], [c4; 3], 2e-4);
        }
    }

    /// The background image, filling the view (cover) behind everything.
    fn backdrop(&mut self, im: &ImageResource) {
        let (w, h) = (self.view.viewport.width, self.view.viewport.height);
        if im.width == 0 || im.height == 0 {
            return;
        }
        let (ia, va) = (im.width as f32 / im.height as f32, w / h);
        // Cover: crop the image's longer side.
        let (u0, u1, v0, v1) = if ia > va {
            let k = va / ia;
            (0.5 - k * 0.5, 0.5 + k * 0.5, 0.0, 1.0)
        } else {
            let k = ia / va;
            (0.0, 1.0, 0.5 - k * 0.5, 0.5 + k * 0.5)
        };
        self.begin(Tex::Image(im.id));
        let c = [255, 255, 255, 255];
        let far = self.far;
        let a = self.vtx(0.0, 0.0, far, [u0, v0], c);
        let b2 = self.vtx(w, 0.0, far, [u1, v0], c);
        let c2 = self.vtx(w, h, far, [u1, v1], c);
        let d = self.vtx(0.0, h, far, [u0, v1], c);
        self.tri(a, b2, c2);
        self.tri(a, c2, d);
        self.end(far, 0.0);
    }

    fn grid(&mut self) {
        let bg = rgb(self.env.background);
        let ink = if bg.iter().sum::<f32>() > 1.5 { [0.1, 0.1, 0.15] } else { [0.9, 0.9, 0.95] };
        let c = mix3(bg, ink, 0.22);
        let extent = 10i32;
        for i in -extent..=extent {
            let major = i == 0;
            for (axis_x, k) in [(true, i), (false, i)] {
                let mut pts = Vec::new();
                for j in -extent..=extent {
                    let (x, z) = if axis_x { (j as f32, k as f32) } else { (k as f32, j as f32) };
                    pts.push(v3(x, 0.0, z));
                }
                let color = if major { if axis_x { [0.9, 0.3, 0.3, 0.6] } else { [0.3, 0.45, 0.95, 0.6] } } else { [c[0], c[1], c[2], 0.55] };
                self.line(&pts, if major { 1.4 } else { 1.0 }, color, 1e-4);
            }
        }
    }

    fn axes(&mut self) {
        let len = 50.0;
        for (d, c) in [(Vec3::X, [0.92, 0.25, 0.25, 0.9]), (Vec3::Y, [0.25, 0.75, 0.3, 0.9]), (Vec3::Z, [0.25, 0.4, 0.95, 0.9])] {
            let pts: Vec<Vec3> = (-25..=25).map(|i| d * (i as f32 * len / 25.0)).collect();
            self.line(&pts, 1.6, c, -1e-4);
        }
    }

    fn orbit_point(&mut self, p: Vec3) {
        let Some(s) = self.view.project(p) else { return };
        let uv = self.atlas.solid_uv();
        self.begin(Tex::Atlas);
        let segs = 16;
        let col = premul([1.0, 0.55, 0.12, 0.95]);
        let ring: Vec<(u32, u32)> = (0..segs)
            .map(|i| {
                let a = std::f32::consts::TAU * i as f32 / segs as f32;
                let (c, sn) = (a.cos(), a.sin());
                (self.vtx(s.x + c * 4.0, s.y + sn * 4.0, 0.0, uv, col), self.vtx(s.x + c * 6.0, s.y + sn * 6.0, 0.0, uv, col))
            })
            .collect();
        for i in 0..segs {
            let j = (i + 1) % segs;
            self.tri(ring[i].0, ring[i].1, ring[j].1);
            self.tri(ring[i].0, ring[j].1, ring[j].0);
        }
        // Always on top.
        self.end_on_top();
    }
}

/// The paint of a painterly stroke's dry start and dry tail.
fn dry_paints(p: &crate::model::Paint) -> (crate::model::Paint, crate::model::Paint) {
    let mut start = *p;
    start.dryness = start.dryness.max(0.5);
    let mut tail = *p;
    tail.dryness = tail.dryness.max(0.9);
    tail.bristles = tail.bristles.max(0.8);
    (start, tail)
}

/// Every texture row a curve needs at a boil frame (`frame` is 0 when the boil is off). The
/// rows are made before the frame is built, so curves can be built in parallel.
fn stroke_rows(s: &Stroke, frame: u32, boil_on: bool, out: &mut Vec<RowKey>) {
    let br = &s.brush;
    let p = &br.paint;
    if br.kind.painterly() {
        let variant = if boil_on && p.boil > 0.0 { frame } else { 0 };
        let (start, tail) = dry_paints(p);
        let mut add = |v: u32| {
            out.push(RowKey::paint(br.kind, p, v));
            out.push(RowKey::paint(br.kind, &start, v));
            out.push(RowKey::paint(br.kind, &tail, v));
        };
        if p.echo.is_some() {
            add(variant.wrapping_add(2));
        }
        for layer in 0..u32::from(p.layers.clamp(1, 4)) {
            add(variant.wrapping_add(layer));
        }
    } else if let Some(pat) = br.pattern {
        out.push(RowKey::pattern(pat.kind, pat.angle, pat.contrast));
    }
    if p.scatter > 0.0 {
        out.push(RowKey::Dabs { variant: (frame % crate::texture::VARIANTS) as u8 });
    }
}

/// A colour nudged in hue, saturation and value by up to `amount`.
fn jitter_colour(c: [f32; 3], amount: f32, h: u32) -> [f32; 3] {
    if amount <= 0.0 {
        return c;
    }
    let rgba = Rgba::from_f32([c[0], c[1], c[2], 1.0]);
    let (hue, sat, val) = rgba.to_hsv();
    let out = Rgba::from_hsv(hue + signed(h) * amount * 28.0, sat + signed(h ^ 0x11) * amount * 0.15, val + signed(h ^ 0x22) * amount * 0.18, 255).to_f32();
    [out[0], out[1], out[2]]
}

/// Where a painterly run's dry start ends and its dry tail begins: (a, b) with the start piece
/// `..=a`, the body `a..=b` and the tail `b..` (all valid indices, a ≤ b).
fn end_split(run: &[Sample]) -> (usize, usize) {
    let n = run.len();
    if n < 6 {
        return (0, n.saturating_sub(1));
    }
    let mut arc = Vec::with_capacity(n);
    let mut acc = 0.0f32;
    for (i, s) in run.iter().enumerate() {
        if i > 0 {
            acc += ((s.x - run[i - 1].x).powi(2) + (s.y - run[i - 1].y).powi(2)).sqrt();
        }
        arc.push(acc);
    }
    let hw = run.iter().map(|s| s.hw).sum::<f32>() / n as f32;
    let start_len = (hw * 1.2).min(acc * 0.15);
    let tail_len = (hw * 4.0).min(acc * 0.35);
    let a = arc.iter().position(|d| *d >= start_len).unwrap_or(0).min(n - 1);
    let b = arc.iter().rposition(|d| *d <= acc - tail_len).unwrap_or(n - 1).max(a);
    (a, b)
}

/// Nib: the width depends on the direction of travel on screen (a 45° calligraphy nib).
fn nib_widths(run: &mut [Sample]) {
    let n = run.len();
    let dirs: Vec<(f32, f32)> = (0..n)
        .map(|i| {
            let a = run[i.saturating_sub(1)];
            let b = run[(i + 1).min(n - 1)];
            (b.x - a.x, b.y - a.y)
        })
        .collect();
    for (s, (dx, dy)) in run.iter_mut().zip(dirs) {
        let l = (dx * dx + dy * dy).sqrt();
        if l > 1e-6 {
            let (ux, uy) = (dx / l, dy / l);
            // Nib edge at 45°: width ∝ |cross(dir, nib)|.
            let k = (ux * std::f32::consts::FRAC_1_SQRT_2 + uy * std::f32::consts::FRAC_1_SQRT_2).abs();
            s.hw *= 0.18 + 0.82 * k;
        }
    }
}

/// Render the note as seen by the camera at a boil frame.
pub fn render(scene: &Scene, camera: &Camera, atlas: &mut Atlas, opts: &Options<'_>) -> Frame {
    let generation = atlas.generation;
    let frame = render_once(scene, camera, atlas, opts);
    if atlas.generation == generation {
        return frame;
    }
    // The atlas filled up and started over mid-frame: its rows now all exist, so once more.
    render_once(scene, camera, atlas, opts)
}

fn render_once(scene: &Scene, camera: &Camera, atlas: &mut Atlas, opts: &Options<'_>) -> Frame {
    let mut cam = *camera;
    cam.sanitize();
    let view = cam.view();
    let env = &scene.environment;
    let light = if env.render_mode { env.lighting.direction() } else { (view.back * 0.75 + view.up * 0.5 - view.right * 0.35).normalized() };
    let mut boil = scene.boil;
    boil.sanitize();
    let frame = if boil.enabled { opts.frame } else { 0 };
    let strokes: Vec<(&Stroke, bool)> = scene
        .strokes
        .iter()
        .filter(|s| !opts.hide.contains(&s.id) && scene.group_shown(s.group))
        .map(|s| (&**s, opts.selected.contains(&s.id)))
        .chain(opts.extra.iter().map(|s| (s, false)))
        .collect();
    // Make every texture row first (twice if the atlas filled up and started over).
    let mut keys = Vec::new();
    for (s, _) in &strokes {
        stroke_rows(s, frame, boil.enabled, &mut keys);
    }
    for _ in 0..2 {
        let generation = atlas.generation;
        for k in &keys {
            atlas.row(*k);
        }
        if atlas.generation == generation {
            break;
        }
    }
    let atlas: &Atlas = atlas;
    let mut b = Builder::new(view, atlas, env, boil, frame, light);
    if let Some(bg) = &env.background_image {
        b.backdrop(bg);
    }
    if opts.overlays && env.show_grid {
        b.grid();
    }
    if opts.overlays && env.show_axes {
        b.axes();
    }
    for im in &scene.images {
        b.image(im, opts.selected_resources.contains(&im.id));
    }
    for m in &scene.models {
        b.model(m, opts.selected_resources.contains(&m.id));
    }
    build_strokes(&mut b, &strokes, view, atlas, env, boil, frame, light);
    if opts.guides {
        for g in &scene.guides {
            b.guide(g, scene.guide_state(g.id), opts.selected_resources.contains(&g.id));
        }
    }
    if let Some(p) = opts.orbit_point
        && opts.overlays
    {
        b.orbit_point(p);
    }
    let focus = cam.distance;
    let mut f = b.finish();
    f.focus_z = depth_to_z(focus, view.orthographic, view.near, (view.half_height * view.f * 400.0).max(view.near * 10.0));
    f
}

/// Curves, built on every core (native) in chunks, then gathered for sorting.
#[allow(clippy::too_many_arguments)]
fn build_strokes(b: &mut Builder<'_>, strokes: &[(&Stroke, bool)], view: View, atlas: &Atlas, env: &Environment, boil: Boil, frame: u32, light: Vec3) {
    #[cfg(not(target_arch = "wasm32"))]
    if strokes.len() > 48 {
        use rayon::prelude::*;
        let parts: Vec<(Vec<Vtx>, Vec<u32>, Vec<Item>)> = strokes
            .par_chunks(32)
            .enumerate()
            .map(|(c, chunk)| {
                let mut part = Builder::new(view, atlas, env, boil, frame, light);
                for (i, (s, sel)) in chunk.iter().enumerate() {
                    part.order = (c * 32 + i + 1) as u32;
                    part.stroke(s, *sel);
                }
                part.order = 0;
                part.flush_dabs();
                (part.verts, part.idx, part.items)
            })
            .collect();
        for (v, i, it) in parts {
            b.absorb(v, i, it);
        }
        return;
    }
    let _ = (view, atlas, env, boil, frame, light);
    for (i, (s, sel)) in strokes.iter().enumerate() {
        b.order = (i + 1) as u32;
        b.stroke(s, *sel);
    }
    b.flush_dabs();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Brush, Echo, Point};
    use std::sync::Arc;

    fn line_stroke(kind: BrushKind) -> Stroke {
        Stroke {
            id: 9,
            group: 1,
            points: (0..40).map(|i| Point { p: v3(i as f32 * 0.05 - 1.0, 0.3, 0.0), pressure: 0.8, n: Vec3::Z }).collect(),
            brush: Brush { kind, size_mm: 40.0, paint: kind.default_paint(), ..Brush::default() },
            seed: 5,
        }
    }

    fn opts<'a>(e: &'a HashSet<u64>, frame: u32) -> Options<'a> {
        Options { frame, selected: e, selected_resources: e, extra: &[], hide: e, guides: true, overlays: true, orbit_point: None }
    }

    #[test]
    fn every_brush_kind_renders_valid_triangles() {
        let empty = HashSet::new();
        for kind in BrushKind::ALL {
            for render_mode in [false, true] {
                let mut scene = Scene::default();
                scene.environment.render_mode = render_mode;
                scene.environment.lighting.ground_shadow = true;
                scene.environment.fog = true;
                let mut s = line_stroke(kind);
                s.brush.paint.echo = Some(Echo { color: Rgba::rgb(255, 0, 0), offset: [4.0, 3.0], width: 1.2 });
                s.brush.pattern = Some(crate::model::Pattern { kind: crate::model::PatternKind::Dot, intensity: 0.5, angle: 30.0, contrast: 0.5 });
                scene.strokes.push(Arc::new(s));
                let mut atlas = Atlas::default();
                let f = render(&scene, &Camera::default(), &mut atlas, &opts(&empty, 1));
                assert!(f.triangles > 0, "{kind:?}");
                for b in &f.batches {
                    assert_eq!(b.indices.len() % 3, 0);
                    assert!(b.indices.iter().all(|i| (*i as usize) < b.vertices.len()), "{kind:?}");
                    assert!(b.vertices.iter().all(|v| v.pos[0].is_finite() && v.pos[1].is_finite() && v.uv[0].is_finite()));
                }
            }
        }
    }

    #[test]
    fn the_boil_changes_lines_between_frames_and_holds_when_off() {
        let empty = HashSet::new();
        let mut scene = Scene::default();
        scene.environment.show_grid = false;
        scene.strokes.push(Arc::new(line_stroke(BrushKind::Marker)));
        let mut atlas = Atlas::default();
        let cam = Camera::default();
        let a = render(&scene, &cam, &mut atlas, &opts(&empty, 0));
        let b = render(&scene, &cam, &mut atlas, &opts(&empty, 1));
        let a2 = render(&scene, &cam, &mut atlas, &opts(&empty, 0));
        assert_ne!(a, b, "frames differ");
        assert_eq!(a, a2, "the same frame is the same picture");
        scene.boil.enabled = false;
        let c = render(&scene, &cam, &mut atlas, &opts(&empty, 0));
        let d = render(&scene, &cam, &mut atlas, &opts(&empty, 1));
        assert_eq!(c, d);
        scene.boil.enabled = true;
        scene.boil.world_space = true;
        let e = render(&scene, &cam, &mut atlas, &opts(&empty, 1));
        assert_ne!(e, b);
    }

    #[test]
    fn scattered_dabs_add_cards_that_reroll_each_frame() {
        let empty = HashSet::new();
        let mut scene = Scene::default();
        scene.environment.show_grid = false;
        let mut s = line_stroke(BrushKind::Oil);
        scene.strokes.push(Arc::new(s.clone()));
        let mut atlas = Atlas::default();
        let cam = Camera::default();
        let plain = render(&scene, &cam, &mut atlas, &opts(&empty, 0)).triangles;
        s.brush.paint.scatter = 0.8;
        s.brush.paint.jitter = 0.6;
        s.brush.paint.color_jitter = 0.5;
        scene.strokes[0] = Arc::new(s);
        let a = render(&scene, &cam, &mut atlas, &opts(&empty, 0));
        let b = render(&scene, &cam, &mut atlas, &opts(&empty, 1));
        assert!(a.triangles > plain + 20, "{} vs {plain}", a.triangles);
        assert_ne!(a, b);
        // Held still when the boil is off.
        scene.boil.enabled = false;
        let c = render(&scene, &cam, &mut atlas, &opts(&empty, 0));
        let d = render(&scene, &cam, &mut atlas, &opts(&empty, 1));
        assert_eq!(c, d);
    }

    #[test]
    fn far_things_draw_first() {
        let empty = HashSet::new();
        let mut scene = Scene::default();
        scene.environment.show_grid = false;
        scene.boil.enabled = false;
        let mut near = line_stroke(BrushKind::Marker);
        near.brush.color = Rgba::rgb(255, 0, 0);
        for p in &mut near.points {
            p.p.z = 2.0;
        }
        let mut far = line_stroke(BrushKind::Marker);
        far.id = 10;
        far.brush.color = Rgba::rgb(0, 0, 255);
        for p in &mut far.points {
            p.p.z = -2.0;
        }
        // Insert near first: sorting must still draw the far one first.
        scene.strokes.push(Arc::new(near));
        scene.strokes.push(Arc::new(far));
        let mut atlas = Atlas::default();
        let cam = Camera { yaw: 0.0, pitch: 0.0, ..Camera::default() };
        let f = render(&scene, &cam, &mut atlas, &opts(&empty, 0));
        let colours: Vec<[u8; 4]> = f.batches.iter().flat_map(|b| b.vertices.iter().map(|v| v.color)).filter(|c| c[3] == 255).collect();
        let first_red = colours.iter().position(|c| c[0] > 200 && c[2] < 50);
        let first_blue = colours.iter().position(|c| c[2] > 200 && c[0] < 50);
        assert!(first_blue < first_red, "{first_blue:?} {first_red:?}");
    }

    #[test]
    fn strokes_behind_the_camera_are_clipped_not_crashing() {
        let empty = HashSet::new();
        let mut scene = Scene::default();
        let mut s = line_stroke(BrushKind::Pen);
        for (i, p) in s.points.iter_mut().enumerate() {
            p.p = v3(0.0, 0.0, i as f32 - 20.0) * 1.0;
        }
        scene.strokes.push(Arc::new(s));
        let cam = Camera { yaw: 0.0, pitch: 0.0, distance: 2.0, ..Camera::default() };
        let mut atlas = Atlas::default();
        let f = render(&scene, &cam, &mut atlas, &opts(&empty, 0));
        for b in &f.batches {
            assert!(b.vertices.iter().all(|v| v.pos[0].is_finite() && v.pos[1].is_finite()));
        }
    }

    #[test]
    fn guides_images_and_models_render() {
        let empty = HashSet::new();
        let mut scene = Scene::default();
        let g = Guide::primitive(3, crate::guide::Primitive::Sphere, 12).expect("sphere");
        scene.guides.push(Arc::new(g));
        scene.active_guide = Some(3);
        scene.images.push(Arc::new(ImageResource {
            id: 4,
            name: "ref".into(),
            width: 2,
            height: 2,
            rgba: Arc::new(vec![255; 16]),
            xform: crate::math::Xform::default(),
            opacity: 0.5,
            state: ResourceState::Visible,
        }));
        scene.models.push(Arc::new(ModelResource {
            id: 5,
            name: "m".into(),
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
            triangles: vec![[0, 1, 2], [0, 1, 99]],
            xform: crate::math::Xform::default(),
            color: Rgba::WHITE,
            state: ResourceState::Visible,
        }));
        scene.environment.show_axes = true;
        let mut atlas = Atlas::default();
        let f = render(&scene, &Camera::default(), &mut atlas, &Options { orbit_point: Some(Vec3::ZERO), ..opts(&empty, 0) });
        assert!(f.batches.iter().any(|b| b.tex == Tex::Image(4)));
        assert!(f.triangles > 500);
    }
}
