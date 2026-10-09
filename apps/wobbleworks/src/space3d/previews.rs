//! Brush preview tiles: each brush type drawn by the 3D renderer as a short S-curve in the
//! current colour, so the brush panel shows what every type looks like.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::TextureHandle;
use wobbleworks_3d::camera::{Camera, PerfectView, Viewport};
use wobbleworks_3d::math::{Vec3, v3};
use wobbleworks_3d::model::{Brush, BrushKind, Point, Scene, Stroke};
use wobbleworks_3d::raster::{Textures, rasterize};
use wobbleworks_3d::render::{Options, render};
use wobbleworks_3d::texture::Atlas;

pub const SIZE: [usize; 2] = [92, 30];

#[derive(Default)]
pub struct Previews {
    atlas: Atlas,
    tiles: HashMap<u64, TextureHandle>,
}

fn key(kind: BrushKind, b: &Brush) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    kind.hash(&mut h);
    b.color.0.hash(&mut h);
    ((b.opacity * 100.0) as u32).hash(&mut h);
    (b.material as u8).hash(&mut h);
    h.finish()
}

/// The sample picture (straight RGBA8, `SIZE`).
pub fn picture(atlas: &mut Atlas, kind: BrushKind, b: &Brush) -> Option<Vec<u8>> {
    let mut brush = Brush { kind, paint: kind.default_paint(), size_mm: 260.0, pressure: true, ..*b };
    brush.pattern = None;
    brush.sanitize();
    let points: Vec<Point> = (0..40)
        .map(|i| {
            let t = i as f32 / 39.0;
            Point { p: v3(-0.95 + t * 1.9, (t * std::f32::consts::TAU).sin() * 0.12, 0.0), pressure: 0.35 + 0.65 * (t * std::f32::consts::PI).sin(), n: Vec3::Z }
        })
        .collect();
    let mut scene = Scene::default();
    scene.environment.show_grid = false;
    scene.boil.enabled = false;
    scene.strokes.push(Arc::new(Stroke { id: 1, group: 1, points, brush, seed: 7 }));
    let mut cam = Camera { viewport: Viewport { width: SIZE[0] as f32, height: SIZE[1] as f32 }, distance: 1.5, ..Camera::default() };
    cam.snap(PerfectView::Front);
    let none = HashSet::new();
    let opts = Options { frame: 0, selected: &none, selected_resources: &none, extra: &[], hide: &none, guides: false, overlays: false, orbit_point: None };
    let frame = render(&scene, &cam, atlas, &opts);
    let tex = Textures { atlas, images: HashMap::new() };
    rasterize(&frame, &tex, SIZE[0] as u32, SIZE[1] as u32, None, None).ok().map(|p| p.rgba)
}

impl Previews {
    /// The tile for a brush type in the current brush's colour.
    pub fn tile(&mut self, ctx: &egui::Context, kind: BrushKind, b: &Brush) -> Option<egui::TextureId> {
        let k = key(kind, b);
        if let Some(t) = self.tiles.get(&k) {
            return Some(t.id());
        }
        if self.tiles.len() > 200 {
            self.tiles.clear();
        }
        let rgba = picture(&mut self.atlas, kind, b)?;
        let img = egui::ColorImage::from_rgba_unmultiplied(SIZE, &rgba);
        let handle = ctx.load_texture(format!("w3d-brush-{k}"), img, egui::TextureOptions::LINEAR);
        let id = handle.id();
        self.tiles.insert(k, handle);
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_brush_type_draws_a_sample() {
        let mut atlas = Atlas::default();
        for kind in BrushKind::ALL {
            let px = picture(&mut atlas, kind, &Brush::default()).expect("picture");
            let inked = px.chunks(4).filter(|p| p[3] > 128).count();
            assert!(inked > 150, "{kind:?}: {inked} pixels");
        }
    }
}
