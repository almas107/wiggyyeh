//! Render caches, compositing and GPU upload.
//!
//! Each layer keeps one rendered image per frame (its raster plus all its strokes), rebuilt only
//! when the layer's content version or the wiggle changes. The composited frames are kept too, so
//! playing the animation is just swapping which texture is shown: no per-tick work at all. While
//! drawing, only the touched rectangle is recomposited and uploaded.

use std::collections::HashMap;

use egui::{Color32, ColorImage, TextureHandle, TextureId, TextureOptions};

use crate::brush::Brushes;
use crate::model::{Doc, Layer, LayerId};
use crate::pixels::{IRect, Pixmap, blend};

/// A layer rendered for every frame.
pub struct LayerCache {
    ver: u64,
    key: CacheKey,
    pub frames: Vec<Pixmap>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct CacheKey {
    wiggle: u64,
    w: usize,
    h: usize,
    frames: usize,
}

fn key_of(doc: &Doc) -> CacheKey {
    CacheKey { wiggle: doc.wiggle.to_bits(), w: doc.w, h: doc.h, frames: doc.frames }
}

/// Render one layer for one frame from scratch.
pub fn render_layer_frame(brushes: &mut Brushes, l: &Layer, frame: usize, doc: &Doc) -> Pixmap {
    let mut px = match l.raster.get(frame).and_then(Option::as_ref) {
        Some(r) if r.w == doc.w && r.h == doc.h => (**r).clone(),
        Some(r) => r.resized(doc.w, doc.h),
        None => Pixmap::new(doc.w, doc.h),
    };
    for s in l.strokes.iter() {
        brushes.render(s, frame, doc.wiggle, &mut px, (0, 0));
    }
    px
}

/// Render a layer for every frame, in parallel on native targets. Empty layers get no images
/// at all (they cost nothing to keep or composite).
fn render_layer(brushes: &mut Brushes, l: &Layer, doc: &Doc) -> Vec<Pixmap> {
    if l.is_empty() {
        return Vec::new();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Threads only pay off when there's real work.
        if doc.frames > 1 && l.strokes.len() > 8 {
            let out = std::thread::scope(|sc| {
                let handles: Vec<_> = (0..doc.frames)
                    .map(|f| {
                        sc.spawn(move || {
                            let mut b = Brushes::default();
                            render_layer_frame(&mut b, l, f, doc)
                        })
                    })
                    .collect();
                handles.into_iter().map(|h| h.join().ok()).collect::<Option<Vec<_>>>()
            });
            if let Some(v) = out {
                return v;
            }
        }
    }
    (0..doc.frames).map(|f| render_layer_frame(brushes, l, f, doc)).collect()
}

/// Composite frame `frame` of `doc` into `out` (sized like the document), only inside `rect`.
pub fn composite_rect(doc: &Doc, caches: &HashMap<LayerId, LayerCache>, frame: usize, rect: IRect, out: &mut Pixmap, with_bg: bool) {
    let r = rect.clamp_to(out.w.min(doc.w), out.h.min(doc.h));
    if r.is_empty() {
        return;
    }
    let base = if with_bg && !doc.transparent { doc.bg } else { Color32::TRANSPARENT };
    let w = out.w;
    let (x0, x1) = (usize::try_from(r.x0).unwrap_or(0), usize::try_from(r.x1).unwrap_or(0));
    for y in r.y0..r.y1 {
        let row = usize::try_from(y).unwrap_or(0) * w;
        if let Some(s) = out.px.get_mut(row + x0..row + x1) {
            s.fill(base);
        }
    }
    for (li, l) in doc.layers.iter().enumerate() {
        if !l.visible || l.opacity <= 0.0 {
            continue;
        }
        let Some(cache) = caches.get(&l.id) else { continue };
        let Some(src) = cache.frames.get(frame) else { continue };
        // Clipping: show only where the base layer has paint in any frame (a still edge, so
        // clipped strokes keep their own wiggle without sizzling against a moving boundary).
        let clip_to = if l.clip && li > 0 {
            let Some(bi) = doc.clip_base(li) else { continue };
            let Some(b) = doc.layers.get(bi) else { continue };
            if !b.visible {
                continue;
            }
            match caches.get(&b.id) {
                Some(c) => Some(c),
                None => continue,
            }
        } else {
            None
        };
        let op = (l.opacity.clamp(0.0, 1.0) * 255.0).round() as u32;
        let mode = l.blend;
        for y in r.y0..r.y1 {
            let row = usize::try_from(y).unwrap_or(0) * w;
            let srow = usize::try_from(y).unwrap_or(0) * src.w;
            let (Some(dst), Some(s)) = (out.px.get_mut(row + x0..row + x1), src.px.get(srow + x0..srow + x1)) else { continue };
            for (k, (d, sc)) in dst.iter_mut().zip(s).enumerate() {
                if sc.a() == 0 {
                    continue;
                }
                if let Some(cb) = clip_to {
                    let i = srow + x0 + k;
                    if !cb.frames.iter().any(|f| f.px.get(i).is_some_and(|c| c.a() > 0)) {
                        continue;
                    }
                }
                *d = blend(*d, *sc, op, mode);
            }
        }
    }
}

const TEX: TextureOptions = TextureOptions {
    magnification: egui::TextureFilter::Nearest,
    minification: egui::TextureFilter::Linear,
    wrap_mode: egui::TextureWrapMode::ClampToEdge,
    mipmap_mode: None,
};

/// Most bytes of recently replaced layer caches kept for instant undo and redo.
const SPARE_BYTES: usize = 192 * 1024 * 1024;

#[derive(Default)]
pub struct Renderer {
    pub brushes: Brushes,
    caches: HashMap<LayerId, LayerCache>,
    /// Recently replaced caches, keyed by layer id and version: undo and redo find the version
    /// they go back to here instead of re-rendering the layer.
    spare: Vec<(LayerId, LayerCache)>,
    /// Composited frames (with the background unless the canvas is transparent).
    pub out: Vec<Pixmap>,
    textures: Vec<TextureHandle>,
    /// Area still to recomposite.
    dirty: IRect,
    /// Area composited but not yet uploaded.
    upload: IRect,
    full_upload: bool,
}

impl Renderer {
    /// Recomposite everything on the next sync.
    pub fn invalidate(&mut self) {
        self.dirty = IRect::new(0, 0, i32::MAX / 2, i32::MAX / 2);
    }

    pub fn invalidate_rect(&mut self, r: IRect) {
        self.dirty = self.dirty.union(r);
    }

    /// Rebuild stale layer caches for `doc`. Returns true if any were rebuilt.
    pub fn ensure_caches(&mut self, doc: &Doc) -> bool {
        let key = key_of(doc);
        let mut rebuilt = false;
        for l in &doc.layers {
            let stale = self.caches.get(&l.id).is_none_or(|c| c.ver != l.ver || c.key != key);
            if stale {
                let fresh = match self.spare.iter().position(|(id, c)| *id == l.id && c.ver == l.ver && c.key == key) {
                    Some(i) => self.spare.swap_remove(i).1,
                    None => LayerCache { ver: l.ver, key, frames: render_layer(&mut self.brushes, l, doc) },
                };
                if let Some(old) = self.caches.insert(l.id, fresh)
                    && old.key == key
                {
                    self.keep_spare(l.id, old);
                }
                rebuilt = true;
            }
        }
        if self.caches.len() > doc.layers.len() {
            self.caches.retain(|id, _| doc.layers.iter().any(|l| l.id == *id));
        }
        if rebuilt {
            self.invalidate();
        }
        rebuilt
    }

    fn keep_spare(&mut self, id: LayerId, c: LayerCache) {
        if c.frames.is_empty() {
            return;
        }
        self.spare.retain(|(i, s)| !(*i == id && s.ver == c.ver));
        self.spare.push((id, c));
        let bytes = |c: &LayerCache| c.frames.iter().map(|p| p.px.len() * 4).sum::<usize>();
        while self.spare.len() > 1 && self.spare.iter().map(|(_, c)| bytes(c)).sum::<usize>() > SPARE_BYTES {
            self.spare.remove(0);
        }
    }

    /// Keep a copy of a layer's cache as it is now (before a live stroke draws into it), so
    /// undoing the stroke is instant.
    pub fn keep_copy(&mut self, id: LayerId) {
        if let Some(c) = self.caches.get(&id) {
            let copy = LayerCache { ver: c.ver, key: c.key, frames: c.frames.clone() };
            self.keep_spare(id, copy);
        }
    }

    /// Give an empty layer's cache real (blank) images so it can be drawn into.
    pub fn materialize(&mut self, doc: &Doc, id: LayerId) {
        let key = key_of(doc);
        if let Some(c) = self.caches.get_mut(&id)
            && c.key == key
            && c.frames.len() != doc.frames
        {
            c.frames = (0..doc.frames).map(|_| Pixmap::new(doc.w, doc.h)).collect();
        }
    }

    /// Re-render only `rect` of layer `idx` after its content changed there (a fill, a cut, an
    /// applied selection), instead of the whole layer.
    pub fn refresh_rect(&mut self, doc: &Doc, idx: usize, rect: IRect) {
        let Some(l) = doc.layers.get(idx) else { return };
        let key = key_of(doc);
        let r = rect.clamp_to(doc.w, doc.h);
        let fits = self.caches.get(&l.id).is_some_and(|c| c.key == key && (c.frames.len() == doc.frames || l.is_empty()));
        if !fits || r.is_empty() {
            return;
        }
        self.materialize(doc, l.id);
        let (brushes, Some(cache)) = (&mut self.brushes, self.caches.get_mut(&l.id)) else { return };
        let org = (r.x0, r.y0);
        let strokes: Vec<&crate::model::Stroke> = l.strokes.iter().filter(|s| !s.bounds(doc.wiggle).intersect(r).is_empty()).collect();
        for (f, frame) in cache.frames.iter_mut().enumerate() {
            let mut part = match l.raster.get(f).and_then(Option::as_ref) {
                Some(src) => src.crop(r),
                None => Pixmap::new(r.width(), r.height()),
            };
            for s in &strokes {
                brushes.render(s, f, doc.wiggle, &mut part, org);
            }
            frame.paste(&part, org);
        }
        cache.ver = l.ver;
        self.invalidate_rect(r);
    }

    /// The stamp cache and a layer's render cache, borrowed together.
    pub fn split(&mut self, id: LayerId) -> (&mut Brushes, Option<&mut LayerCache>) {
        (&mut self.brushes, self.caches.get_mut(&id))
    }

    /// Drop a layer's cache (it is rebuilt on the next sync).
    pub fn forget(&mut self, id: LayerId) {
        self.caches.remove(&id);
    }

    pub fn cache(&self, id: LayerId) -> Option<&LayerCache> {
        self.caches.get(&id)
    }

    /// After a live stroke is committed: the cache already shows it, so adopt the new version
    /// instead of rebuilding.
    pub fn adopt_version(&mut self, id: LayerId, ver: u64) {
        if let Some(c) = self.caches.get_mut(&id) {
            c.ver = ver;
        }
    }

    #[cfg(test)]
    pub fn caches(&self) -> &HashMap<LayerId, LayerCache> {
        &self.caches
    }

    /// Bring caches and composited frames up to date.
    pub fn sync(&mut self, doc: &Doc) {
        self.ensure_caches(doc);
        let sized = self.out.len() == doc.frames && self.out.first().is_some_and(|p| p.w == doc.w && p.h == doc.h);
        if !sized {
            self.out = (0..doc.frames).map(|_| Pixmap::new(doc.w, doc.h)).collect();
            self.invalidate();
            self.full_upload = true;
        }
        let r = self.dirty.clamp_to(doc.w, doc.h);
        self.dirty = IRect::EMPTY;
        if r.is_empty() {
            return;
        }
        let caches = &self.caches;
        #[cfg(not(target_arch = "wasm32"))]
        {
            // Big areas: one thread per frame.
            if r.width() * r.height() > 160_000 && self.out.len() > 1 {
                std::thread::scope(|sc| {
                    for (f, out) in self.out.iter_mut().enumerate() {
                        sc.spawn(move || composite_rect(doc, caches, f, r, out, true));
                    }
                });
                self.upload = self.upload.union(r);
                return;
            }
        }
        for (f, out) in self.out.iter_mut().enumerate() {
            composite_rect(doc, caches, f, r, out, true);
        }
        self.upload = self.upload.union(r);
    }

    /// Push composited changes to the GPU (partial uploads while drawing).
    pub fn upload(&mut self, ctx: &egui::Context) {
        let need_new = self.textures.len() != self.out.len() || self.textures.iter().zip(&self.out).any(|(t, p)| t.size() != [p.w, p.h]);
        if need_new || self.full_upload {
            self.textures = self
                .out
                .iter()
                .enumerate()
                .map(|(f, p)| ctx.load_texture(format!("wobble-frame-{f}"), ColorImage::new([p.w, p.h], p.px.clone()), TEX))
                .collect();
            self.full_upload = false;
            self.upload = IRect::EMPTY;
            return;
        }
        let Some(first) = self.out.first() else { return };
        let r = self.upload.clamp_to(first.w, first.h);
        self.upload = IRect::EMPTY;
        if r.is_empty() {
            return;
        }
        for (t, p) in self.textures.iter_mut().zip(&self.out) {
            let part = p.crop(r);
            let pos = [usize::try_from(r.x0).unwrap_or(0), usize::try_from(r.y0).unwrap_or(0)];
            t.set_partial(pos, ColorImage::new([part.w, part.h], part.px), TEX);
        }
    }

    pub fn texture(&self, frame: usize) -> Option<TextureId> {
        self.textures.get(frame).or_else(|| self.textures.first()).map(TextureHandle::id)
    }

    /// The composited frame (for the eyedropper and fills).
    pub fn frame(&self, f: usize) -> Option<&Pixmap> {
        self.out.get(f)
    }
}

/// Composite every frame of `doc` from scratch (the reference the live path is checked against).
#[cfg(test)]
pub fn export_frames(doc: &Doc) -> Vec<Pixmap> {
    let mut r = Renderer::default();
    r.ensure_caches(doc);
    (0..doc.frames)
        .map(|f| {
            let mut out = Pixmap::new(doc.w, doc.h);
            composite_rect(doc, &r.caches, f, IRect::full(doc.w, doc.h), &mut out, true);
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Brush, Pt, Stroke, Tip};
    use std::sync::Arc;

    fn dot(x: f64, y: f64, c: Color32) -> Stroke {
        Stroke { brush: Brush::Steady, tip: Tip::Square, color: c, size: 4.0, seed: 1, lock: false, pts: vec![Pt::new(x, y)] }
    }

    #[test]
    fn composite_respects_visibility_opacity_and_background() {
        let mut d = Doc::new(32, 32);
        Arc::make_mut(&mut d.layers[0].strokes).push(dot(10.0, 10.0, Color32::RED));
        let f = export_frames(&d);
        assert_eq!(f.len(), 3);
        assert_eq!(f[0].get(10, 10), Color32::RED);
        assert_eq!(f[0].get(30, 30), Color32::WHITE);
        d.layers[0].opacity = 0.0;
        assert_eq!(export_frames(&d)[0].get(10, 10), Color32::WHITE);
        d.layers[0].opacity = 1.0;
        d.layers[0].visible = false;
        d.transparent = true;
        assert_eq!(export_frames(&d)[0].get(10, 10), Color32::TRANSPARENT);
    }

    #[test]
    fn clipped_layer_shows_only_over_its_base() {
        let mut d = Doc::new(32, 32);
        d.transparent = true;
        Arc::make_mut(&mut d.layers[0].strokes).push(dot(10.0, 10.0, Color32::RED));
        let mut top = Layer::new("top", 3);
        Arc::make_mut(&mut top.strokes).push(dot(10.0, 10.0, Color32::BLUE));
        Arc::make_mut(&mut top.strokes).push(dot(25.0, 25.0, Color32::BLUE));
        top.clip = true;
        d.layers.push(top);
        let f = export_frames(&d);
        assert_eq!(f[0].get(10, 10), Color32::BLUE);
        assert_eq!(f[0].get(25, 25), Color32::TRANSPARENT);
    }

    #[test]
    fn caches_rebuild_only_when_stale() {
        let mut d = Doc::new(32, 32);
        let mut r = Renderer::default();
        assert!(r.ensure_caches(&d));
        assert!(!r.ensure_caches(&d));
        d.wiggle = 2.0;
        assert!(r.ensure_caches(&d));
        d.layers[0].touch();
        assert!(r.ensure_caches(&d));
        d.layers.push(Layer::new("b", 3));
        d.layers.remove(0);
        r.ensure_caches(&d);
        assert_eq!(r.caches().len(), 1);
    }

    #[test]
    fn spare_caches_make_undo_free_and_refresh_rect_matches_full_render() {
        let mut d = Doc::new(64, 64);
        let mut r = Renderer::default();
        Arc::make_mut(&mut d.layers[0].strokes).push(dot(10.0, 10.0, Color32::RED));
        d.layers[0].touch();
        r.ensure_caches(&d);
        let v1 = d.layers[0].clone();
        Arc::make_mut(&mut d.layers[0].strokes).push(dot(30.0, 30.0, Color32::BLUE));
        d.layers[0].touch();
        r.ensure_caches(&d);
        // "Undo": the old version comes back from the spare list, not a re-render.
        d.layers[0] = v1;
        let before = r.spare.len();
        r.ensure_caches(&d);
        assert_eq!(r.spare.len(), before, "one taken, one kept");
        assert!(r.caches[&d.layers[0].id].frames == export_layer(&d));
        // A raster change re-rendered only in a rectangle equals a full render.
        for f in 0..3 {
            d.layers[0].raster_mut(f, 64, 64).unwrap().set(11, 11, Color32::GREEN);
        }
        d.layers[0].touch();
        r.refresh_rect(&d, 0, IRect::new(5, 5, 20, 20));
        assert!(!r.ensure_caches(&d), "refresh_rect brought the cache up to date");
        assert!(r.caches[&d.layers[0].id].frames == export_layer(&d));
    }

    fn export_layer(d: &Doc) -> Vec<Pixmap> {
        let mut b = Brushes::default();
        (0..d.frames).map(|f| render_layer_frame(&mut b, &d.layers[0], f, d)).collect()
    }

    #[test]
    fn parallel_and_serial_layer_renders_agree() {
        let mut d = Doc::new(64, 64);
        let strokes: Vec<Stroke> = (0..20)
            .map(|i| Stroke {
                brush: Brush::Shaky,
                seed: i,
                pts: vec![Pt::new(5.0, f64::from(i) * 3.0), Pt::new(60.0, f64::from(i) * 3.0)],
                ..dot(0.0, 0.0, Color32::BLACK)
            })
            .collect();
        d.layers[0].strokes = Arc::new(strokes);
        let mut b = Brushes::default();
        let par = render_layer(&mut b, &d.layers[0], &d);
        for (f, p) in par.iter().enumerate() {
            assert!(*p == render_layer_frame(&mut b, &d.layers[0], f, &d));
        }
    }
}
