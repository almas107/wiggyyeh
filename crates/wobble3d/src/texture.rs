//! The brush texture atlas: procedural alpha masks for painterly strokes (ragged edges, bristle
//! streaks, dry-brush gaps, chalk grain), Feather's patterns, the glow halo and film grain.
//!
//! Rows are made on demand for each painterly setting in use (quantised), so every brush option
//! shows in the texture without node graphs. The UI uploads the atlas whenever its
//! [`Atlas::revision`] changes. Pixels are straight-alpha RGBA8.

use std::collections::HashMap;

use crate::model::{BrushKind, Paint, PatternKind};
use crate::noise::{hash2, periodic1, signed, unit, value1};

pub const WIDTH: usize = 512;
pub const ROW: usize = 32;
pub const MAX_ROWS: usize = 256;
/// One atlas tile spans this many stroke widths along the stroke.
pub const TILE_WIDTHS: f32 = 8.0;
/// Painterly variants: each boil frame shows a different one, so the paint itself shimmers.
pub const VARIANTS: u32 = 4;
/// Levels a painterly option is quantised to.
const LEVELS: f32 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowKey {
    Solid,
    Paint { kind: BrushKind, rough: u8, bristles: u8, dryness: u8, grain: u8, variant: u8 },
    Pattern { kind: PatternKind, angle: u8, contrast: u8 },
    Halo,
    Grain,
}

fn q(v: f32) -> u8 {
    (if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 } * (LEVELS - 1.0)).round() as u8
}

fn dq(v: u8) -> f32 {
    v as f32 / (LEVELS - 1.0)
}

impl RowKey {
    pub fn paint(kind: BrushKind, p: &Paint, variant: u32) -> RowKey {
        RowKey::Paint { kind, rough: q(p.roughness), bristles: q(p.bristles), dryness: q(p.dryness), grain: q(p.grain), variant: (variant % VARIANTS) as u8 }
    }
    pub fn pattern(kind: PatternKind, angle_deg: f32, contrast: f32) -> RowKey {
        let a = if angle_deg.is_finite() { angle_deg.rem_euclid(180.0) } else { 0.0 };
        RowKey::Pattern { kind, angle: ((a / 15.0).round() as u8) % 12, contrast: q(contrast) }
    }
}

#[derive(Debug, Clone)]
pub struct Atlas {
    keys: HashMap<RowKey, usize>,
    pub pixels: Vec<u8>,
    pub rows: usize,
    /// Bumped whenever pixels change.
    pub revision: u64,
}

impl Default for Atlas {
    fn default() -> Self {
        let mut a = Atlas { keys: HashMap::new(), pixels: Vec::new(), rows: 0, revision: 0 };
        a.row(RowKey::Solid);
        a.row(RowKey::Halo);
        a.row(RowKey::Grain);
        a
    }
}

impl Atlas {
    pub fn width(&self) -> usize {
        WIDTH
    }
    pub fn height(&self) -> usize {
        self.rows * ROW
    }

    /// The row index for a key, made if needed. When the atlas is full it starts over (the
    /// caller's frame then rebuilds the rows it uses).
    pub fn row(&mut self, key: RowKey) -> usize {
        if let Some(r) = self.keys.get(&key) {
            return *r;
        }
        if self.rows >= MAX_ROWS {
            self.keys.clear();
            self.pixels.clear();
            self.rows = 0;
            for k in [RowKey::Solid, RowKey::Halo, RowKey::Grain] {
                self.push(k);
            }
        }
        self.push(key)
    }

    fn push(&mut self, key: RowKey) -> usize {
        let r = self.rows;
        self.pixels.extend(bake(key));
        self.rows += 1;
        self.keys.insert(key, r);
        self.revision = self.revision.wrapping_add(1);
        r
    }

    /// Texture coordinates of (u along the tile 0..1, v across 0..1) in a row.
    pub fn uv(&self, row: usize, u: f32, v: f32) -> [f32; 2] {
        let h = self.height().max(1) as f32;
        let u = if u.is_finite() { u.clamp(0.0, 1.0) } else { 0.0 };
        let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.5 };
        let y = row as f32 * ROW as f32 + 1.5 + v * (ROW as f32 - 3.0);
        [(0.5 + u * (WIDTH as f32 - 1.0)) / WIDTH as f32, y / h]
    }

    /// A texel of the solid row (for untextured geometry).
    pub fn solid_uv(&self) -> [f32; 2] {
        self.uv(0, 0.5, 0.5)
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = if (e1 - e0).abs() < 1e-9 { if x >= e1 { 1.0 } else { 0.0 } } else { ((x - e0) / (e1 - e0)).clamp(0.0, 1.0) };
    t * t * (3.0 - 2.0 * t)
}

/// One row's pixels.
fn bake(key: RowKey) -> Vec<u8> {
    let mut out = vec![0u8; WIDTH * ROW * 4];
    for y in 0..ROW {
        // v across the stroke from the padded row.
        let v = ((y as f32 - 1.5) / (ROW as f32 - 3.0)).clamp(0.0, 1.0);
        for x in 0..WIDTH {
            let u = x as f32 / WIDTH as f32;
            let (lum, a) = texel(key, u, v);
            let i = (y * WIDTH + x) * 4;
            let l = (lum.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            if let Some(px) = out.get_mut(i..i + 4) {
                px.copy_from_slice(&[l, l, l, (a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8]);
            }
        }
    }
    out
}

/// (luminance, alpha) of a texel.
fn texel(key: RowKey, u: f32, v: f32) -> (f32, f32) {
    let y = v * 2.0 - 1.0;
    match key {
        RowKey::Solid => (1.0, 1.0),
        RowKey::Halo => {
            let a = (-(y * y) * 4.0).exp() * (1.0 - y.abs());
            (1.0, a)
        }
        RowKey::Grain => {
            let px = (u * WIDTH as f32) as u32;
            let py = (v * ROW as f32) as u32;
            (unit(hash2(px, py ^ 0xa5a5)), 1.0)
        }
        RowKey::Paint { kind, rough, bristles, dryness, grain, variant } => {
            paint_texel(kind, dq(rough), dq(bristles), dq(dryness), dq(grain), variant as u32, u, y)
        }
        RowKey::Pattern { kind, angle, contrast } => (1.0, pattern_texel(kind, angle as f32 * 15.0, dq(contrast), u, v)),
    }
}

/// A painted stroke's texel: `u` along (periodic), `y` across in -1..1.
fn paint_texel(kind: BrushKind, rough: f32, bristles: f32, dryness: f32, grain: f32, variant: u32, u: f32, y: f32) -> (f32, f32) {
    let seed = hash2(kind as u32 + 11, variant);
    // Along the tile in stroke widths.
    let along = u * TILE_WIDTHS;
    let p = TILE_WIDTHS as u32;
    // Ragged edges: each side wanders on its own.
    let (edge_freq, edge_amp) = match kind {
        BrushKind::Gouache => (0.9, 0.42),
        BrushKind::Ink => (2.2, 0.28),
        BrushKind::DryBrush => (1.6, 0.4),
        BrushKind::Chalk => (3.0, 0.25),
        _ => (1.3, 0.34),
    };
    let side = if y >= 0.0 { 0x51u32 } else { 0x77 };
    let wobble = |s: u32| {
        let a = periodic1(seed ^ s, along * edge_freq, (p as f32 * edge_freq).round().max(1.0) as u32);
        let b = periodic1(seed ^ s ^ 0x3c, along * edge_freq * 4.0, (p as f32 * edge_freq * 4.0).round().max(1.0) as u32);
        (a * 0.7 + b * 0.3) * 0.5 + 0.5
    };
    let edge = 1.0 - rough * edge_amp * wobble(side);
    let soft = match kind {
        BrushKind::Ink => 0.03,
        BrushKind::Chalk => 0.12,
        _ => 0.06,
    };
    let mut a = smoothstep(edge, edge - soft, y.abs());
    // Bristle streaks: lines along the stroke, a little wavy.
    let streak_freq = match kind {
        BrushKind::DryBrush => 34.0,
        BrushKind::Oil => 26.0,
        _ => 18.0,
    };
    let drift = periodic1(seed ^ 0x99, along * 0.5, (p / 2).max(1)) * 0.02;
    let streak = value1(seed ^ 0x1234, (y + drift) * streak_freq) * 0.5 + 0.5;
    let streak_cut = bristles * 0.85 * (1.0 - streak).powi(2) * 2.0;
    a *= (1.0 - streak_cut).clamp(0.0, 1.0);
    // Dry brush: streaks break into gaps along the stroke.
    if dryness > 0.0 {
        let lane = (y * streak_freq * 0.5).floor() as i32 as u32;
        let along_noise = periodic1(seed ^ hash2(lane, 0xd7), along * 1.7, (p as f32 * 1.7).round().max(1.0) as u32) * 0.5 + 0.5;
        let dry = smoothstep(dryness * 0.75, dryness * 0.75 + 0.15, along_noise);
        a *= 1.0 - dryness * (1.0 - dry);
    }
    // Grain: tiny holes.
    if grain > 0.0 {
        let gx = (u * WIDTH as f32) as u32;
        let gy = ((y * 0.5 + 0.5) * ROW as f32) as u32;
        let g = unit(hash2(hash2(gx / 2, gy), seed));
        a *= 1.0 - grain * smoothstep(0.45, 0.9, g);
    }
    // Paint thickness: streaks a little lighter and darker (the colour's own texture).
    let lum = match kind {
        BrushKind::Oil | BrushKind::Gouache => 0.86 + 0.14 * streak + 0.05 * signed(hash2(seed, (along * 3.0) as u32)),
        BrushKind::DryBrush => 0.9 + 0.1 * streak,
        _ => 1.0,
    };
    (lum.clamp(0.0, 1.0), a.clamp(0.0, 1.0))
}

/// A pattern mark's alpha (1 = mark).
fn pattern_texel(kind: PatternKind, angle_deg: f32, contrast: f32, u: f32, v: f32) -> f32 {
    // Work in stroke widths: the tile is TILE_WIDTHS long and one width across.
    let (x0, y0) = (u * TILE_WIDTHS, v);
    let (s, c) = angle_deg.to_radians().sin_cos();
    let (x, y) = (x0 * c - y0 * s, x0 * s + y0 * c);
    let soft = 0.06 * (1.0 - contrast * 0.85) + 0.004;
    let spacing = 0.25;
    let cell = |t: f32| t / spacing - (t / spacing).floor();
    let mark = match kind {
        PatternKind::Dot => {
            let (dx, dy) = (cell(x) - 0.5, cell(y) - 0.5);
            let d = (dx * dx + dy * dy).sqrt() * spacing;
            smoothstep(0.075 + soft, 0.075 - soft, d)
        }
        PatternKind::Line => {
            let d = (cell(y) - 0.5).abs() * spacing;
            smoothstep(0.05 + soft, 0.05 - soft, d)
        }
        PatternKind::Cross => {
            let d = (cell(y) - 0.5).abs().min((cell(x) - 0.5).abs()) * spacing;
            smoothstep(0.035 + soft, 0.035 - soft, d)
        }
        PatternKind::Terrazzo => {
            // Chips: random blobs in a jittered grid.
            let s2 = 0.35;
            let (gx, gy) = ((x / s2).floor(), (y / s2).floor());
            let mut best = 0.0f32;
            for oy in -1..=1 {
                for ox in -1..=1 {
                    let (cx, cy) = (gx + ox as f32, gy + oy as f32);
                    let h = hash2(cx as i64 as u32, cy as i64 as u32);
                    if unit(h) < 0.35 {
                        continue;
                    }
                    let px = (cx + unit(h ^ 1)) * s2;
                    let py = (cy + unit(h ^ 2)) * s2;
                    let r = s2 * (0.12 + 0.2 * unit(h ^ 3));
                    let ang = (y - py).atan2(x - px);
                    let rr = r * (1.0 + 0.35 * (ang * 3.0 + unit(h ^ 4) * 6.0).sin());
                    let d = ((x - px).powi(2) + (y - py).powi(2)).sqrt();
                    best = best.max(smoothstep(rr + soft, rr - soft, d));
                }
            }
            best
        }
        PatternKind::StippledDot => {
            let s2 = 0.12;
            let (gx, gy) = ((x / s2).floor(), (y / s2).floor());
            let h = hash2(gx as i64 as u32, gy as i64 as u32 ^ 0x77);
            if unit(h) < 0.45 {
                0.0
            } else {
                let px = (gx + 0.2 + 0.6 * unit(h ^ 1)) * s2;
                let py = (gy + 0.2 + 0.6 * unit(h ^ 2)) * s2;
                let r = s2 * (0.12 + 0.12 * unit(h ^ 3));
                let d = ((x - px).powi(2) + (y - py).powi(2)).sqrt();
                smoothstep(r + soft, r - soft, d)
            }
        }
    };
    mark * (0.55 + 0.45 * contrast)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_made_once_and_mapped() {
        let mut a = Atlas::default();
        let rev = a.revision;
        let p = BrushKind::Oil.default_paint();
        let r1 = a.row(RowKey::paint(BrushKind::Oil, &p, 0));
        let r2 = a.row(RowKey::paint(BrushKind::Oil, &p, 0));
        assert_eq!(r1, r2);
        assert!(a.revision > rev);
        assert_eq!(a.pixels.len(), a.width() * a.height() * 4);
        let uv = a.uv(r1, 0.5, 0.5);
        assert!(uv[1] > 0.0 && uv[1] < 1.0);
    }

    #[test]
    fn a_full_atlas_starts_over() {
        let mut a = Atlas::default();
        for i in 0..(MAX_ROWS + 10) {
            let k = RowKey::Pattern { kind: PatternKind::Dot, angle: (i % 12) as u8, contrast: (i / 12) as u8 };
            let r = a.row(k);
            assert!(r < MAX_ROWS);
        }
        assert!(a.rows <= MAX_ROWS);
        assert_eq!(a.pixels.len(), a.width() * a.height() * 4);
    }

    #[test]
    fn painterly_masks_have_ragged_edges_and_streaks() {
        let p = Paint { roughness: 1.0, bristles: 1.0, dryness: 0.6, ..Paint::default() };
        let px = bake(RowKey::paint(BrushKind::DryBrush, &p, 0));
        let alpha = |x: usize, y: usize| px[(y * WIDTH + x) * 4 + 3];
        // The centre is mostly paint, the very edge mostly not.
        let centre: u32 = (0..WIDTH).map(|x| alpha(x, ROW / 2) as u32).sum();
        let edge: u32 = (0..WIDTH).map(|x| alpha(x, 1) as u32).sum();
        assert!(centre > edge * 2, "centre {centre} edge {edge}");
        // Gaps appear: not every centre texel is solid.
        assert!((0..WIDTH).any(|x| alpha(x, ROW / 2) < 128));
        let solid = bake(RowKey::Solid);
        assert!(solid.chunks(4).all(|c| c == [255, 255, 255, 255]));
    }
}
