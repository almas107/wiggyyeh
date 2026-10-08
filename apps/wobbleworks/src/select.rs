//! Floating selections: lassoed or imported content you can move, scale, turn, flip and recolour
//! before baking it back into a layer.
//!
//! The floating content is shown as its own textures drawn through a transformed quad, so
//! dragging and turning it costs nothing on the CPU. Strokes stay vectors: when baked, their
//! points are transformed and they re-wobble at their new size.

use std::sync::Arc;

use egui::{Color32, ColorImage, Mesh, Pos2, TextureHandle, TextureOptions};

use crate::brush::Brushes;
use crate::geom;
use crate::model::{Doc, LayerId, Stroke};
use crate::pixels::{IRect, Pixmap, blend, mul255, side};

/// Move, scale, turn and flip about the selection's centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xform {
    pub dx: f64,
    pub dy: f64,
    pub scale: f64,
    /// Degrees.
    pub rot: f64,
    pub fh: bool,
    pub fv: bool,
    pub cx: f64,
    pub cy: f64,
}

impl Xform {
    pub fn about(cx: f64, cy: f64) -> Self {
        Xform { dx: 0.0, dy: 0.0, scale: 1.0, rot: 0.0, fh: false, fv: false, cx, cy }
    }

    pub fn reset(&mut self) {
        *self = Xform::about(self.cx, self.cy);
    }

    fn sx(&self) -> f64 {
        self.scale * if self.fh { -1.0 } else { 1.0 }
    }

    fn sy(&self) -> f64 {
        self.scale * if self.fv { -1.0 } else { 1.0 }
    }

    pub fn apply(&self, (x, y): (f64, f64)) -> (f64, f64) {
        let (x, y) = ((x - self.cx) * self.sx(), (y - self.cy) * self.sy());
        let (s, c) = self.rot.to_radians().sin_cos();
        (self.cx + self.dx + x * c - y * s, self.cy + self.dy + x * s + y * c)
    }

    pub fn inverse(&self, (x, y): (f64, f64)) -> (f64, f64) {
        let (x, y) = (x - self.cx - self.dx, y - self.cy - self.dy);
        let (s, c) = (-self.rot).to_radians().sin_cos();
        let (x, y) = (x * c - y * s, x * s + y * c);
        let (sx, sy) = (self.sx(), self.sy());
        if sx == 0.0 || sy == 0.0 || !sx.is_finite() || !sy.is_finite() {
            return (f64::NAN, f64::NAN);
        }
        (self.cx + x / sx, self.cy + y / sy)
    }
}

pub const MIN_SCALE: f64 = 0.05;
pub const MAX_SCALE: f64 = 8.0;

pub struct Floating {
    /// The layer it came from (and goes back to).
    pub layer: LayerId,
    pub poly: Vec<(f64, f64)>,
    pub strokes: Vec<Stroke>,
    /// Raster content per frame, `org`-relative.
    pub raster: Vec<Option<Pixmap>>,
    pub org: (i32, i32),
    pub xf: Xform,
    view: Vec<Pixmap>,
    view_org: (i32, i32),
    view_key: Option<(u64, usize)>,
    textures: Vec<TextureHandle>,
    textures_stale: bool,
}

impl Floating {
    fn new(layer: LayerId, poly: Vec<(f64, f64)>, strokes: Vec<Stroke>, raster: Vec<Option<Pixmap>>, org: (i32, i32)) -> Self {
        let b = geom::bbox(&poly);
        let (cx, cy) = (f64::from(b.x0 + b.x1) / 2.0, f64::from(b.y0 + b.y1) / 2.0);
        Floating {
            layer,
            poly,
            strokes,
            raster,
            org,
            xf: Xform::about(cx, cy),
            view: Vec::new(),
            view_org: (0, 0),
            view_key: None,
            textures: Vec::new(),
            textures_stale: true,
        }
    }

    /// Cut what's inside `poly` out of the current layer. Strokes come along when most of their
    /// points are inside. `None` if there is nothing there.
    pub fn cut(doc: &mut Doc, poly: Vec<(f64, f64)>) -> Option<Floating> {
        if poly.len() < 3 {
            return None;
        }
        let (w, h, frames) = (doc.w, doc.h, doc.frames);
        let layer = doc.layers.get_mut(doc.current)?;
        let (mut keep, mut take) = (Vec::new(), Vec::new());
        for s in layer.strokes.iter() {
            let inside = s.pts.iter().filter(|p| geom::in_poly(p.x, p.y, &poly)).count();
            if !s.pts.is_empty() && inside * 10 >= s.pts.len() * 6 { take.push(s.clone()) } else { keep.push(s.clone()) }
        }
        let area = geom::bbox(&poly).clamp_to(w, h);
        let mut raster: Vec<Option<Pixmap>> = vec![None; frames];
        let mut any_px = false;
        if !area.is_empty() {
            for (f, slot) in raster.iter_mut().enumerate() {
                let Some(src) = layer.raster.get_mut(f).and_then(Option::as_mut) else { continue };
                let src = Arc::make_mut(src);
                let mut part = Pixmap::new(area.width(), area.height());
                let mut got = false;
                geom::fill_spans(&poly, area, |y, x0, x1| {
                    for x in x0..x1 {
                        let c = src.get(x, y);
                        if c.a() > 0 {
                            part.set(x - area.x0, y - area.y0, c);
                            src.set(x, y, Color32::TRANSPARENT);
                            got = true;
                        }
                    }
                });
                if got {
                    *slot = Some(part);
                    any_px = true;
                }
            }
        }
        if take.is_empty() && !any_px {
            return None;
        }
        layer.strokes = Arc::new(keep);
        layer.touch();
        Some(Floating::new(layer.id, poly, take, raster, (area.x0, area.y0)))
    }

    /// An imported picture, scaled down to fit and centred, floating over the current layer.
    pub fn from_image(doc: &Doc, img: &Pixmap) -> Option<Floating> {
        let layer = doc.layer()?.id;
        let fit = (doc.w as f64 / img.w as f64).min(doc.h as f64 / img.h as f64).min(1.0);
        let (w, h) = (((img.w as f64 * fit).round() as usize).max(1), ((img.h as f64 * fit).round() as usize).max(1));
        let scaled = if (w, h) == (img.w, img.h) { img.clone() } else { downscale(img, w, h) };
        let (x, y) = ((side(doc.w) - side(w)) / 2, (side(doc.h) - side(h)) / 2);
        let (x0, y0, x1, y1) = (f64::from(x), f64::from(y), f64::from(x + side(w)), f64::from(y + side(h)));
        let poly = vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
        let raster = (0..doc.frames).map(|_| Some(scaled.clone())).collect();
        Some(Floating::new(layer, poly, Vec::new(), raster, (x, y)))
    }

    /// Canvas area the content covers after its transform (generous: strokes re-wobble at their
    /// new size when baked).
    pub fn footprint(&self, wiggle: f64) -> IRect {
        let mut r = geom::bbox(&self.poly);
        let mut pad: f64 = 2.0;
        for s in &self.strokes {
            r = r.union(s.bounds(wiggle));
            pad = pad.max(s.size * 1.6 + s.brush.amp() * wiggle * 3.0 + 4.0);
        }
        for p in self.raster.iter().flatten() {
            r = r.union(IRect::new(self.org.0, self.org.1, self.org.0 + side(p.w), self.org.1 + side(p.h)));
        }
        if r.is_empty() {
            return r;
        }
        let corners = [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)].map(|(x, y)| self.xf.apply((f64::from(x), f64::from(y))));
        let pad = (pad * self.xf.scale.max(1.0)).ceil().clamp(2.0, 2000.0) as i32;
        geom::bbox(&corners).expand(pad)
    }

    pub fn contains(&self, p: (f64, f64)) -> bool {
        let q: Vec<(f64, f64)> = self.poly.iter().map(|p| self.xf.apply(*p)).collect();
        geom::in_poly(p.0, p.1, &q)
    }

    /// The outline after transforming, for drawing marching ants.
    pub fn outline(&self) -> Vec<(f64, f64)> {
        self.poly.iter().map(|p| self.xf.apply(*p)).collect()
    }

    pub fn recolor(&mut self, color: Color32) {
        for s in &mut self.strokes {
            s.color = color;
        }
        for p in self.raster.iter_mut().flatten() {
            for c in &mut p.px {
                let a = u32::from(c.a());
                if a > 0 {
                    let k = |v: u8| u8::try_from(mul255(u32::from(v), a)).unwrap_or(255);
                    *c = Color32::from_rgba_premultiplied(k(color.r()), k(color.g()), k(color.b()), c.a());
                }
            }
        }
        self.view_key = None;
    }

    /// Rebuild the preview images if the content or wiggle changed.
    fn ensure_view(&mut self, brushes: &mut Brushes, doc: &Doc) {
        let key = (doc.wiggle.to_bits(), doc.frames);
        if self.view_key == Some(key) {
            return;
        }
        let mut r = geom::bbox(&self.poly);
        for s in &self.strokes {
            r = r.union(s.bounds(doc.wiggle));
        }
        for p in self.raster.iter().flatten() {
            r = r.union(IRect::new(self.org.0, self.org.1, self.org.0 + side(p.w), self.org.1 + side(p.h)));
        }
        let r = r.intersect(IRect::new(-4096, -4096, 8192, 8192));
        self.view_org = (r.x0, r.y0);
        self.view = (0..doc.frames)
            .map(|f| {
                let mut px = Pixmap::new(r.width().max(1), r.height().max(1));
                if let Some(Some(src)) = self.raster.get(f).or_else(|| self.raster.last()) {
                    px.draw_over(src, self.org.0 - r.x0, self.org.1 - r.y0);
                }
                for s in &self.strokes {
                    brushes.render(s, f, doc.wiggle, &mut px, self.view_org);
                }
                px
            })
            .collect();
        self.view_key = Some(key);
        self.textures_stale = true;
    }

    /// Paint the floating content for `frame` through its transform.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        brushes: &mut Brushes,
        doc: &Doc,
        frame: usize,
        opacity: f32,
        to_screen: &dyn Fn((f64, f64)) -> Pos2,
    ) {
        self.ensure_view(brushes, doc);
        if self.textures_stale {
            let opts = TextureOptions { magnification: egui::TextureFilter::Nearest, minification: egui::TextureFilter::Linear, ..TextureOptions::NEAREST };
            self.textures = self
                .view
                .iter()
                .enumerate()
                .map(|(f, p)| ctx.load_texture(format!("wobble-float-{f}"), ColorImage::new([p.w, p.h], p.px.clone()), opts))
                .collect();
            self.textures_stale = false;
        }
        let (Some(tex), Some(px)) = (self.textures.get(frame).or_else(|| self.textures.first()), self.view.first()) else { return };
        let (x0, y0) = (f64::from(self.view_org.0), f64::from(self.view_org.1));
        let (x1, y1) = (x0 + px.w as f64, y0 + px.h as f64);
        let tint = Color32::from_white_alpha((opacity.clamp(0.0, 1.0) * 255.0) as u8);
        let mut mesh = Mesh::with_texture(tex.id());
        let corners = [((x0, y0), (0.0, 0.0)), ((x1, y0), (1.0, 0.0)), ((x1, y1), (1.0, 1.0)), ((x0, y1), (0.0, 1.0))];
        for (p, uv) in corners {
            let s = to_screen(self.xf.apply(p));
            mesh.vertices.push(egui::epaint::Vertex { pos: s, uv: Pos2::new(uv.0, uv.1), color: tint });
        }
        mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
        painter.add(mesh);
    }

    /// Put the content back into its layer (or the current one if that layer is gone), at its
    /// transformed position. Returns the layer index it went into.
    pub fn bake(self, doc: &mut Doc) -> Option<usize> {
        let idx = doc.layers.iter().position(|l| l.id == self.layer).unwrap_or(doc.current);
        let (w, h) = (doc.w, doc.h);
        let xf = self.xf;
        let layer = doc.layers.get_mut(idx)?;
        if !self.strokes.is_empty() {
            let strokes = Arc::make_mut(&mut layer.strokes);
            for mut s in self.strokes {
                for p in &mut s.pts {
                    let (x, y) = xf.apply((p.x, p.y));
                    p.x = geom::clamp_coord(x);
                    p.y = geom::clamp_coord(y);
                }
                s.size = (s.size * xf.scale).clamp(1.0, crate::model::MAX_SIZE);
                strokes.push(s);
            }
        }
        for (f, src) in self.raster.iter().enumerate() {
            let Some(src) = src else { continue };
            let (ox, oy) = (f64::from(self.org.0), f64::from(self.org.1));
            let (sw, sh) = (src.w as f64, src.h as f64);
            let corners = [(ox, oy), (ox + sw, oy), (ox + sw, oy + sh), (ox, oy + sh)].map(|p| xf.apply(p));
            let area = geom::bbox(&corners).expand(1).clamp_to(w, h);
            if area.is_empty() {
                continue;
            }
            let Some(dst) = layer.raster_mut(f, w, h) else { continue };
            for y in area.y0..area.y1 {
                for x in area.x0..area.x1 {
                    let (sx, sy) = xf.inverse((f64::from(x) + 0.5, f64::from(y) + 0.5));
                    if !(sx.is_finite() && sy.is_finite()) {
                        continue;
                    }
                    let c = src.get((sx - ox).floor() as i32, (sy - oy).floor() as i32);
                    if c.a() == 0 {
                        continue;
                    }
                    if let Some(i) = dst.index(x, y)
                        && let Some(d) = dst.px.get_mut(i)
                    {
                        *d = blend(*d, c, 255, crate::pixels::Blend::Normal);
                    }
                }
            }
        }
        layer.touch();
        Some(idx)
    }

    /// A copy of the content for "stamp": bake a duplicate and keep floating.
    pub fn duplicate(&self) -> Floating {
        let mut f = Floating::new(self.layer, self.poly.clone(), self.strokes.clone(), self.raster.clone(), self.org);
        for s in &mut f.strokes {
            s.seed = s.seed.wrapping_mul(2_654_435_761).wrapping_add(17);
        }
        f.xf = self.xf;
        f
    }
}

/// Box-filter downscale for imported pictures.
pub fn downscale(src: &Pixmap, w: usize, h: usize) -> Pixmap {
    let mut out = Pixmap::new(w, h);
    let (w, h) = (out.w, out.h);
    for y in 0..h {
        let (sy0, sy1) = (y * src.h / h, ((y + 1) * src.h / h).max(y * src.h / h + 1).min(src.h));
        for x in 0..w {
            let (sx0, sx1) = (x * src.w / w, ((x + 1) * src.w / w).max(x * src.w / w + 1).min(src.w));
            let (mut r, mut g, mut b, mut a, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
            for yy in sy0..sy1 {
                for xx in sx0..sx1 {
                    if let Some(c) = src.px.get(yy * src.w + xx) {
                        r += u32::from(c.r());
                        g += u32::from(c.g());
                        b += u32::from(c.b());
                        a += u32::from(c.a());
                        n += 1;
                    }
                }
            }
            if n > 0 {
                let q = |v: u32| u8::try_from(v / n).unwrap_or(255);
                if let Some(p) = out.px.get_mut(y * w + x) {
                    *p = Color32::from_rgba_premultiplied(q(r), q(g), q(b), q(a));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Brush, Pt, Tip};

    #[test]
    fn xform_inverse_round_trips() {
        let mut x = Xform::about(50.0, 40.0);
        x.dx = 7.0;
        x.dy = -3.0;
        x.scale = 1.7;
        x.rot = 33.0;
        x.fh = true;
        let p = (12.0, 99.0);
        let q = x.inverse(x.apply(p));
        assert!((q.0 - p.0).abs() < 1e-9 && (q.1 - p.1).abs() < 1e-9);
        x.scale = 0.0;
        assert!(x.inverse((1.0, 1.0)).0.is_nan());
    }

    fn doc_with_dot() -> Doc {
        let mut d = Doc::new(64, 64);
        Arc::make_mut(&mut d.layers[0].strokes).push(Stroke {
            brush: Brush::Steady,
            tip: Tip::Square,
            color: Color32::BLACK,
            size: 4.0,
            seed: 3,
            lock: false,
            pts: vec![Pt::new(10.0, 10.0), Pt::new(12.0, 10.0)],
        });
        for f in 0..3 {
            d.layers[0].raster_mut(f, 64, 64).unwrap().set(11, 11, Color32::RED);
        }
        d
    }

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<(f64, f64)> {
        vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    }

    #[test]
    fn cut_move_and_bake() {
        let mut d = doc_with_dot();
        let mut fl = Floating::cut(&mut d, square(5.0, 5.0, 20.0, 20.0)).unwrap();
        assert!(d.layers[0].strokes.is_empty());
        assert_eq!(d.layers[0].raster[0].as_ref().unwrap().get(11, 11), Color32::TRANSPARENT);
        assert!(fl.contains((11.0, 11.0)));
        fl.xf.dx = 30.0;
        assert!(fl.contains((41.0, 11.0)));
        fl.bake(&mut d).unwrap();
        assert_eq!(d.layers[0].strokes[0].pts[0].x, 40.0);
        assert_eq!(d.layers[0].raster[1].as_ref().unwrap().get(41, 11), Color32::RED);
    }

    #[test]
    fn cut_of_nothing_is_none() {
        let mut d = doc_with_dot();
        assert!(Floating::cut(&mut d, square(40.0, 40.0, 50.0, 50.0)).is_none());
        assert!(Floating::cut(&mut d, vec![(0.0, 0.0), (1.0, 1.0)]).is_none());
        assert_eq!(d.layers[0].strokes.len(), 1);
    }

    #[test]
    fn imported_images_fit_the_canvas() {
        let d = Doc::new(64, 32);
        let img = Pixmap::filled(200, 100, Color32::RED);
        let f = Floating::from_image(&d, &img).unwrap();
        let p = f.raster[0].as_ref().unwrap();
        assert_eq!((p.w, p.h), (64, 32));
        assert_eq!(p.get(10, 10), Color32::RED);
    }

    #[test]
    fn recolor_keeps_coverage() {
        let mut d = doc_with_dot();
        let mut f = Floating::cut(&mut d, square(0.0, 0.0, 30.0, 30.0)).unwrap();
        f.recolor(Color32::BLUE);
        assert_eq!(f.strokes[0].color, Color32::BLUE);
        let px = f.raster[0].as_ref().unwrap();
        assert!(px.px.contains(&Color32::BLUE));
    }
}
