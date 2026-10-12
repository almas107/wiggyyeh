//! A CPU rasteriser for [`crate::render::Frame`]s: exports (PNG, GIF, turntables) and tests.
//! It draws what the GPU view draws, the same way: a depth prepass of the solid paint (pushed
//! back by [`crate::render::SURFACE_TOL`]), then everything under and on the nearest surface in
//! the frame's order (paint in drawing order, so on a shared surface the later curve wins, even
//! on curved guides), then the overlays (guides) where they are clearly in front. Then the
//! render-mode effects that need the whole picture (bloom, grain, pixelation).

use std::collections::HashMap;

use crate::model::{Effects, Rgba};
use crate::noise::{hash2, unit};
use crate::render::{Frame, Tex};
use crate::texture::Atlas;

/// Biggest picture a raster may make (each side).
pub const MAX_SIDE: u32 = 8192;

/// A straight-alpha RGBA8 picture.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Texture lookups for the raster: the atlas and image resources (straight RGBA8).
pub struct Textures<'a> {
    pub atlas: &'a Atlas,
    pub images: HashMap<u64, (u32, u32, &'a [u8])>,
}

struct Canvas {
    w: usize,
    h: usize,
    /// Premultiplied RGBA floats.
    px: Vec<[f32; 4]>,
    /// Reverse depth of the solid paint at each pixel (0 = nothing; larger is nearer).
    zbuf: Vec<f32>,
}

/// Paint at least this opaque counts as solid (depth-tested and written).
pub const SOLID_ALPHA: f32 = 0.5;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    /// Solid paint's depth, pushed back by the surface tolerance; no colour.
    Depth,
    /// Under-paint and paint, in order, where on or in front of the nearest surface.
    Paint,
    /// Overlays, where clearly in front of the nearest surface.
    Over,
}

/// A straight RGBA8 texture: width, height, pixels.
type Image<'a> = (u32, u32, &'a [u8]);

fn sample(tex: &Image<'_>, u: f32, v: f32) -> [f32; 4] {
    let (w, h, data) = (tex.0 as usize, tex.1 as usize, tex.2);
    if w == 0 || h == 0 || data.len() < w * h * 4 {
        return [1.0, 1.0, 1.0, 1.0];
    }
    let x = (u.clamp(0.0, 1.0) * w as f32 - 0.5).clamp(0.0, (w - 1) as f32);
    let y = (v.clamp(0.0, 1.0) * h as f32 - 0.5).clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let texel = |xx: usize, yy: usize| -> [f32; 4] {
        let i = (yy * w + xx) * 4;
        match data.get(i..i + 4) {
            Some(p) => {
                let a = p[3] as f32 / 255.0;
                [p[0] as f32 / 255.0 * a, p[1] as f32 / 255.0 * a, p[2] as f32 / 255.0 * a, a]
            }
            None => [0.0; 4],
        }
    };
    let (a, b, c, d) = (texel(x0, y0), texel(x1, y0), texel(x0, y1), texel(x1, y1));
    let mut out = [0.0; 4];
    for k in 0..4 {
        let top = a[k] + (b[k] - a[k]) * fx;
        let bot = c[k] + (d[k] - c[k]) * fx;
        out[k] = top + (bot - top) * fy;
    }
    out
}

impl Canvas {
    /// `screen`: Cutout paint takes its colour from this image at the pixel's place on screen
    /// (with the frame's cover rectangle), its shape from `tex`.
    #[allow(clippy::too_many_arguments)]
    fn tri(
        &mut self,
        pass: Pass,
        p: [[f32; 2]; 3],
        z: [f32; 3],
        zb: [f32; 3],
        solid: bool,
        uv: [[f32; 2]; 3],
        col: [[f32; 4]; 3],
        tex: Option<&(u32, u32, &[u8])>,
        screen: Option<(&Image<'_>, [f32; 4])>,
    ) {
        let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
        if area.abs() < 1e-9 || !area.is_finite() {
            return;
        }
        let minx = p.iter().map(|q| q[0]).fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
        let maxx = p.iter().map(|q| q[0]).fold(f32::NEG_INFINITY, f32::max).ceil().min(self.w as f32) as usize;
        let miny = p.iter().map(|q| q[1]).fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
        let maxy = p.iter().map(|q| q[1]).fold(f32::NEG_INFINITY, f32::max).ceil().min(self.h as f32) as usize;
        for y in miny..maxy {
            for x in minx..maxx {
                let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((p[1][0] - cx) * (p[2][1] - cy) - (p[2][0] - cx) * (p[1][1] - cy)) / area;
                let w1 = ((p[2][0] - cx) * (p[0][1] - cy) - (p[0][0] - cx) * (p[2][1] - cy)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < -1e-5 || w1 < -1e-5 || w2 < -1e-5 {
                    continue;
                }
                let mut c = [0.0f32; 4];
                for k in 0..4 {
                    c[k] = col[0][k] * w0 + col[1][k] * w1 + col[2][k] * w2;
                }
                if let Some(t) = tex {
                    let u = uv[0][0] * w0 + uv[1][0] * w1 + uv[2][0] * w2;
                    let v = uv[0][1] * w0 + uv[1][1] * w1 + uv[2][1] * w2;
                    let s = sample(t, u, v);
                    for k in 0..4 {
                        c[k] *= s[k];
                    }
                }
                if let Some((img, [u0, v0, u1, v1])) = screen {
                    let (fx, fy) = (cx / self.w as f32, cy / self.h as f32);
                    let s = sample(img, u0 + (u1 - u0) * fx, v0 + (v1 - v0) * fy);
                    c = [s[0] * c[3], s[1] * c[3], s[2] * c[3], s[3] * c[3]];
                }
                let zz = z[0] * w0 + z[1] * w1 + z[2] * w2;
                let i = y * self.w + x;
                let (Some(d), Some(zbuf)) = (self.px.get_mut(i), self.zbuf.get_mut(i)) else { continue };
                let strong = solid && c[3] >= SOLID_ALPHA;
                match pass {
                    Pass::Depth => {
                        let back = zb[0] * w0 + zb[1] * w1 + zb[2] * w2;
                        if strong && back > *zbuf {
                            *zbuf = back;
                        }
                    }
                    Pass::Paint | Pass::Over => {
                        // Paint: on the nearest surface (within the tolerance) or in front.
                        // Overlays: `zb` holds their depth pushed back twice as far, so they
                        // show only clearly in front of it.
                        let test = if pass == Pass::Over { zb[0] * w0 + zb[1] * w1 + zb[2] * w2 } else { zz };
                        if test < *zbuf {
                            continue;
                        }
                        if strong {
                            let a = c[3].max(1e-6);
                            *d = [(c[0] / a).clamp(0.0, 1.0), (c[1] / a).clamp(0.0, 1.0), (c[2] / a).clamp(0.0, 1.0), 1.0];
                            continue;
                        }
                        let inv = 1.0 - c[3].clamp(0.0, 1.0);
                        for k in 0..4 {
                            d[k] = (c[k] + d[k] * inv).clamp(0.0, 1.0);
                        }
                    }
                }
            }
        }
    }
}

/// Fill a frame into a picture over a background.
pub fn rasterize(
    frame: &Frame,
    textures: &Textures<'_>,
    width: u32,
    height: u32,
    background: Option<Rgba>,
    effects: Option<&Effects>,
) -> Result<Picture, String> {
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(format!("picture size must be 1–{MAX_SIDE} pixels a side"));
    }
    let (w, h) = (width as usize, height as usize);
    let bg = background.map_or([0.0; 4], |c| {
        let f = c.to_f32();
        [f[0] * f[3], f[1] * f[3], f[2] * f[3], f[3]]
    });
    let mut cv = Canvas { w, h, px: vec![bg; w * h], zbuf: vec![0.0; w * h] };
    let atlas = (textures.atlas.width() as u32, textures.atlas.height() as u32, textures.atlas.pixels.as_slice());
    for pass in [Pass::Depth, Pass::Paint, Pass::Over] {
        for b in &frame.batches {
            let over = b.layer == crate::render::layer::OVER;
            if (pass == Pass::Paint && over) || (pass == Pass::Over && !over) {
                continue;
            }
            let (tex, screen) = match b.tex {
                Tex::Atlas => (Some(&atlas), None),
                Tex::Image(id) => (textures.images.get(&id), None),
                Tex::Cutout(id) => (Some(&atlas), textures.images.get(&id).map(|img| (img, frame.cover))),
            };
            for t in b.indices.chunks_exact(3) {
                let (Some(a), Some(bb), Some(c)) = (b.vertices.get(t[0] as usize), b.vertices.get(t[1] as usize), b.vertices.get(t[2] as usize)) else {
                    continue;
                };
                let solid = a.solid && bb.solid && c.solid;
                if pass == Pass::Depth && !solid {
                    continue;
                }
                let f = |v: [u8; 4]| [v[0] as f32 / 255.0, v[1] as f32 / 255.0, v[2] as f32 / 255.0, v[3] as f32 / 255.0];
                let zb = if pass == Pass::Over { [a.zo, bb.zo, c.zo] } else { [a.zb, bb.zb, c.zb] };
                cv.tri(pass, [a.pos, bb.pos, c.pos], [a.z, bb.z, c.z], zb, solid, [a.uv, bb.uv, c.uv], [f(a.color), f(bb.color), f(c.color)], tex, screen);
            }
        }
    }
    if let Some(e) = effects {
        if e.dof > 0.0 {
            depth_of_field(&mut cv, e.dof, frame.focus_z);
        }
        post(&mut cv, e);
    }
    let mut rgba = Vec::with_capacity(w * h * 4);
    for p in &cv.px {
        let a = p[3];
        let un = |v: f32| if a > 1e-6 { (v / a).clamp(0.0, 1.0) } else { 0.0 };
        rgba.extend_from_slice(&[(un(p[0]) * 255.0 + 0.5) as u8, (un(p[1]) * 255.0 + 0.5) as u8, (un(p[2]) * 255.0 + 0.5) as u8, (a * 255.0 + 0.5) as u8]);
    }
    Ok(Picture { width, height, rgba })
}

fn box_blur(px: &[[f32; 4]], w: usize, h: usize, r: usize) -> Vec<[f32; 4]> {
    let mut tmp = vec![[0.0f32; 4]; w * h];
    let mut out = vec![[0.0f32; 4]; w * h];
    let r = r.max(1);
    for y in 0..h {
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(r), (x + r).min(w - 1));
            let mut s = [0.0f32; 4];
            for xx in x0..=x1 {
                for k in 0..4 {
                    s[k] += px[y * w + xx][k];
                }
            }
            let n = (x1 - x0 + 1) as f32;
            tmp[y * w + x] = [s[0] / n, s[1] / n, s[2] / n, s[3] / n];
        }
    }
    for y in 0..h {
        for x in 0..w {
            let (y0, y1) = (y.saturating_sub(r), (y + r).min(h - 1));
            let mut s = [0.0f32; 4];
            for yy in y0..=y1 {
                for k in 0..4 {
                    s[k] += tmp[yy * w + x][k];
                }
            }
            let n = (y1 - y0 + 1) as f32;
            out[y * w + x] = [s[0] / n, s[1] / n, s[2] / n, s[3] / n];
        }
    }
    out
}

/// Depth of field: each pixel is blurred by its distance from the focus (the orbit point) in
/// depth, more for a smaller f-stop. Variable box blur through a summed-area table.
fn depth_of_field(cv: &mut Canvas, fstop: f32, focus_z: f32) {
    let (w, h) = (cv.w, cv.h);
    if focus_z <= 0.0 || !focus_z.is_finite() || w * h > 8192 * 8192 {
        return;
    }
    let fstop = fstop.clamp(0.7, 22.0);
    let max_r = (w.min(h) as f32 * 0.02 * (2.8 / fstop)).clamp(0.0, 40.0);
    // Summed-area table of premultiplied colour.
    let mut sat = vec![[0.0f64; 4]; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = [0.0f64; 4];
        for x in 0..w {
            for k in 0..4 {
                row[k] += f64::from(cv.px[y * w + x][k]);
                sat[(y + 1) * (w + 1) + x + 1][k] = sat[y * (w + 1) + x + 1][k] + row[k];
            }
        }
    }
    for y in 0..h {
        for x in 0..w {
            let z = cv.zbuf[y * w + x];
            // Empty pixels take the background's (far) blur.
            let off = if z > 0.0 { (1.0 - z / focus_z).abs() } else { 1.0 };
            let r = (max_r * off.min(1.0)).round() as usize;
            if r == 0 {
                continue;
            }
            let (x0, x1, y0, y1) = (x.saturating_sub(r), (x + r + 1).min(w), y.saturating_sub(r), (y + r + 1).min(h));
            let n = ((x1 - x0) * (y1 - y0)) as f64;
            let mut c = [0.0f32; 4];
            for (k, ck) in c.iter_mut().enumerate() {
                let s = sat[y1 * (w + 1) + x1][k] - sat[y0 * (w + 1) + x1][k] - sat[y1 * (w + 1) + x0][k] + sat[y0 * (w + 1) + x0][k];
                *ck = (s / n) as f32;
            }
            cv.px[y * w + x] = c;
        }
    }
}

fn post(cv: &mut Canvas, e: &Effects) {
    let (w, h) = (cv.w, cv.h);
    if e.bloom > 0.0 && w * h <= 4096 * 4096 {
        let bright: Vec<[f32; 4]> = cv
            .px
            .iter()
            .map(|p| {
                let l = (p[0] + p[1] + p[2]) / 3.0;
                let k = ((l - 0.65) / 0.35).clamp(0.0, 1.0);
                [p[0] * k, p[1] * k, p[2] * k, 0.0]
            })
            .collect();
        let r = ((w.min(h) as f32) * 0.012).max(2.0) as usize;
        let blur = box_blur(&box_blur(&bright, w, h, r), w, h, r);
        for (p, b) in cv.px.iter_mut().zip(blur) {
            for k in 0..3 {
                p[k] = (p[k] + b[k] * e.bloom * 1.5).min(p[3].max(b[k]).min(1.0));
            }
        }
    }
    if e.pixelate > 1.0 {
        let s = e.pixelate.clamp(1.0, 32.0) as usize;
        for by in (0..h).step_by(s) {
            for bx in (0..w).step_by(s) {
                let mut acc = [0.0f32; 4];
                let mut n = 0.0;
                for y in by..(by + s).min(h) {
                    for x in bx..(bx + s).min(w) {
                        for (a, v) in acc.iter_mut().zip(cv.px[y * w + x]) {
                            *a += v;
                        }
                        n += 1.0;
                    }
                }
                let avg = [acc[0] / n, acc[1] / n, acc[2] / n, acc[3] / n];
                for y in by..(by + s).min(h) {
                    for x in bx..(bx + s).min(w) {
                        cv.px[y * w + x] = avg;
                    }
                }
            }
        }
    }
    if e.grain > 0.0 {
        for (i, p) in cv.px.iter_mut().enumerate() {
            let n = (unit(hash2(i as u32, 0x6a11)) - 0.5) * e.grain * 0.25;
            for k in 0..3 {
                p[k] = (p[k] + n * p[3]).clamp(0.0, p[3]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{Batch, Vtx};

    #[test]
    fn a_triangle_fills_with_premultiplied_over() {
        let atlas = Atlas::default();
        let uv = atlas.solid_uv();
        let v = |x: f32, y: f32| Vtx { pos: [x, y], uv, color: [128, 0, 0, 128], z: 0.5, zb: 0.5, zo: 0.5, solid: false };
        let frame = Frame {
            batches: vec![Batch { tex: Tex::Atlas, layer: 1, vertices: vec![v(0.0, 0.0), v(10.0, 0.0), v(0.0, 10.0)], indices: vec![0, 1, 2] }],
            triangles: 1,
            ..Frame::default()
        };
        let tex = Textures { atlas: &atlas, images: HashMap::new() };
        let pic = rasterize(&frame, &tex, 10, 10, Some(Rgba::WHITE), None).expect("raster");
        let px = &pic.rgba[(2 * 10 + 2) * 4..(2 * 10 + 2) * 4 + 4];
        // Half red over white: (255, 127, 127).
        assert!(px[0] > 250 && (120..136).contains(&px[1]) && px[3] == 255, "{px:?}");
        assert_eq!(&pic.rgba[(9 * 10 + 9) * 4..(9 * 10 + 9) * 4 + 4], &[255, 255, 255, 255]);
    }

    #[test]
    fn bad_sizes_and_indices_are_handled() {
        let atlas = Atlas::default();
        let tex = Textures { atlas: &atlas, images: HashMap::new() };
        assert!(rasterize(&Frame::default(), &tex, 0, 10, None, None).is_err());
        assert!(rasterize(&Frame::default(), &tex, 10, MAX_SIDE + 1, None, None).is_err());
        let frame =
            Frame { batches: vec![Batch { tex: Tex::Image(77), layer: 1, vertices: vec![], indices: vec![0, 5, 9] }], triangles: 1, ..Frame::default() };
        let pic =
            rasterize(&frame, &tex, 4, 4, None, Some(&Effects { grain: 1.0, pixelate: 2.0, bloom: 1.0, dof: 2.0, ..Effects::default() })).expect("raster");
        assert_eq!(pic.rgba.len(), 64);
    }
}

#[cfg(test)]
mod order_tests {
    use super::*;
    use crate::camera::Camera;
    use crate::editor::Editor;
    use serde_json::json;

    /// Two crossing curves on the same plane: the one drawn later covers the earlier one, for
    /// every brush kind and from a tilted view.
    #[test]
    fn later_curves_cover_earlier_ones_on_a_shared_surface() {
        for kind in crate::model::BrushKind::ALL {
            let mut e = Editor::new();
            e.set_viewport(200.0, 200.0);
            e.run("env.set", &json!({"grid": false})).expect("env");
            e.run("boil.set", &json!({"enabled": false})).expect("boil");
            e.run("camera.view", &json!({"view": "front"})).expect("view");
            e.run("brush.set", &json!({"kind": kind.name(), "color": "#ff0000", "size": 60, "pressure": false})).expect("red");
            e.run("stroke.draw", &json!({"points": (0..30).map(|i| [20.0 + i as f32 * 5.5, 100.0, 1.0]).collect::<Vec<_>>()})).expect("red");
            e.run("brush.set", &json!({"color": "#0000ff"})).expect("blue");
            e.run("stroke.draw", &json!({"points": (0..30).map(|i| [100.0, 20.0 + i as f32 * 5.5, 1.0]).collect::<Vec<_>>()})).expect("blue");
            e.camera = Camera { yaw: 25.0, pitch: 15.0, viewport: e.camera.viewport, ..e.camera };
            e.camera.orthographic = false;
            let f = e.render(0, false);
            let tex = Textures { atlas: &e.atlas, images: HashMap::new() };
            let pic = rasterize(&f, &tex, 200, 200, Some(crate::model::Rgba::WHITE), None).expect("raster");
            // Where they cross (the target point, at the centre).
            let i = (100 * 200 + 100) * 4;
            let p = &pic.rgba[i..i + 4];
            let red = p[0] > 150 && p[2] < 100;
            let blue = p[2] > 150 && p[0] < 100;
            assert!(!red, "{kind:?}: the earlier red curve shows through ({p:?})");
            assert!(blue || kind.painterly(), "{kind:?}: the later blue curve is on top ({p:?})");
        }
    }

    /// Paint laid over paint on one guide (a plane and a sphere, seen at an angle) never shows
    /// the earlier colour through the later one, on any boil frame. The flat brush used to:
    /// its vertices took the depth of the point before the boil moved it on screen, so tape
    /// boiling over tape on a tilted guide z-fought in speckles (Feather users report the same).
    #[test]
    fn paint_over_paint_on_a_guide_never_shows_through_while_boiling() {
        for guide in ["plane", "sphere"] {
            for kind in [crate::model::BrushKind::Flat, crate::model::BrushKind::Marker, crate::model::BrushKind::Pen] {
                let mut e = Editor::new();
                e.set_viewport(240.0, 240.0);
                e.run("env.set", &json!({"grid": false})).expect("env");
                e.run("boil.set", &json!({"enabled": true, "amount": 3.0})).expect("boil");
                e.run("guide.primitive", &json!({"kind": guide})).expect("guide");
                e.run("guide.done", &json!({})).expect("done");
                e.run("tool.set", &json!({"tool": "draw"})).expect("draw");
                e.camera = Camera { yaw: 35.0, pitch: 50.0, distance: 3.0, viewport: e.camera.viewport, ..e.camera };
                e.camera.orthographic = false;
                e.run("brush.set", &json!({"kind": kind.name(), "color": "#ff0000", "size": 120, "pressure": false})).expect("red");
                e.run("stroke.draw", &json!({"points": (0..40).map(|i| [60.0 + i as f32 * 3.0, 120.0, 1.0]).collect::<Vec<_>>()})).expect("red");
                e.run("brush.set", &json!({"color": "#0000ff", "size": 60})).expect("blue");
                e.run("stroke.draw", &json!({"points": (0..40).map(|i| [120.0 + (i % 2) as f32, 60.0 + i as f32 * 3.0, 1.0]).collect::<Vec<_>>()}))
                    .expect("blue");
                assert_eq!(e.scene.strokes.len(), 2, "{guide} {kind:?}: both strokes landed on the guide");
                for frame in 0..3 {
                    let f = e.render(frame, false);
                    let tex = Textures { atlas: &e.atlas, images: HashMap::new() };
                    let pic = rasterize(&f, &tex, 240, 240, Some(crate::model::Rgba::WHITE), None).expect("raster");
                    // Hide the red curve: where blue is solid on its own, it must be solid with red under it too.
                    let red_id = e.scene.strokes[0].id;
                    let hide: std::collections::HashSet<u64> = [red_id].into();
                    let none = std::collections::HashSet::new();
                    let opts = crate::render::Options {
                        frame,
                        selected: &none,
                        selected_resources: &none,
                        extra: &[],
                        hide: &hide,
                        guides: false,
                        overlays: false,
                        orbit_point: None,
                    };
                    let alone = crate::render::render(&e.scene, &e.camera, &mut e.atlas, &opts);
                    let tex = Textures { atlas: &e.atlas, images: HashMap::new() };
                    let blue = rasterize(&alone, &tex, 240, 240, Some(crate::model::Rgba::WHITE), None).expect("raster");
                    let is_blue = |p: &[u8]| p[2] > 200 && p[0] < 60;
                    let (mut inside, mut bled) = (0, 0);
                    for y in 2..238usize {
                        for x in 2..238usize {
                            // Interior of the blue curve only (its soft edge blends by design).
                            let solid = (-2i32..=2).all(|dy| {
                                (-2i32..=2).all(|dx| {
                                    let i = ((y as i32 + dy) as usize * 240 + (x as i32 + dx) as usize) * 4;
                                    is_blue(&blue.rgba[i..i + 4])
                                })
                            });
                            if !solid {
                                continue;
                            }
                            inside += 1;
                            let i = (y * 240 + x) * 4;
                            if !is_blue(&pic.rgba[i..i + 4]) {
                                bled += 1;
                            }
                        }
                    }
                    if let Ok(dir) = std::env::var("WOBBLE3D_DEBUG_DIR") {
                        let _ = std::fs::write(format!("{dir}/{guide}-{kind:?}-{frame}.rgba"), &pic.rgba);
                    }
                    assert!(inside > 60, "{guide} {kind:?} frame {frame}: blue is drawn ({inside} px)");
                    assert_eq!(bled, 0, "{guide} {kind:?} frame {frame}: red shows through blue at {bled} of {inside} px");
                }
            }
        }
    }

    /// Cutout paint in Render mode shows the background image under it (Feather: a cutout
    /// "shows the background"), not the background colour and not the brush colour.
    #[test]
    fn cutout_paint_shows_the_background_image() {
        let mut e = Editor::new();
        e.set_viewport(200.0, 200.0);
        e.run("env.set", &json!({"grid": false, "render": true})).expect("env");
        e.run("boil.set", &json!({"enabled": false})).expect("boil");
        e.run("camera.view", &json!({"view": "front"})).expect("view");
        // A 2 × 1 image: red on the left, blue on the right.
        let rgba = vec![255, 0, 0, 255, 0, 0, 255, 255];
        e.scene.environment.background_image = Some(std::sync::Arc::new(crate::model::ImageResource {
            id: 77,
            name: "bg".into(),
            width: 2,
            height: 1,
            rgba: std::sync::Arc::new(rgba.clone()),
            xform: crate::math::Xform::default(),
            opacity: 1.0,
            state: crate::model::ResourceState::Visible,
        }));
        e.run("brush.set", &json!({"kind": "pen", "color": "#00ff00", "size": 60, "pressure": false, "material": "cutout"})).expect("brush");
        e.run("stroke.draw", &json!({"points": (0..30).map(|i| [20.0 + i as f32 * 5.5, 100.0, 1.0]).collect::<Vec<_>>()})).expect("draw");
        let f = e.render(0, false);
        assert!(f.batches.iter().any(|b| b.tex == Tex::Cutout(77)), "cutout paint is marked");
        let mut images = HashMap::new();
        images.insert(77, (2u32, 1u32, rgba.as_slice()));
        let tex = Textures { atlas: &e.atlas, images };
        let pic = rasterize(&f, &tex, 200, 200, Some(crate::model::Rgba::WHITE), None).expect("raster");
        let at = |x: usize, y: usize| pic.rgba[(y * 200 + x) * 4..(y * 200 + x) * 4 + 4].to_vec();
        let (left, right) = (at(30, 100), at(170, 100));
        assert!(left[0] > 200 && left[1] < 60 && left[2] < 60, "left shows red: {left:?}");
        assert!(right[2] > 200 && right[1] < 60 && right[0] < 60, "right shows blue: {right:?}");
        // Without Render mode the same curve is plain paint (no image lookup).
        e.run("env.set", &json!({"render": false})).expect("env");
        let f = e.render(0, false);
        assert!(f.batches.iter().all(|b| !matches!(b.tex, Tex::Cutout(_))));
    }
}
