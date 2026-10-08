//! The document: layers of wobbly vector strokes plus per-frame raster paint.
//!
//! Strokes stay vectors so the whole drawing can re-boil when the wiggle changes, and so moved or
//! scaled strokes keep crisp, freshly wobbled edges. Fills and imported pictures are raster, one
//! image per animation frame (fills are computed per frame so they boil with their outline).
//!
//! Layers are cheap to clone (`Arc`s), which is what undo snapshots rely on.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use egui::Color32;
use serde::{Deserialize, Serialize};

use crate::pixels::{Blend, MAX_SIDE, MIN_SIDE, Pixmap};

/// Frames in a boil loop.
pub const MIN_FRAMES: usize = 2;
pub const MAX_FRAMES: usize = 8;
pub const DEFAULT_FRAMES: usize = 3;
/// Brush size range.
pub const MAX_SIZE: f64 = 200.0;
/// Most layers a document may have.
pub const MAX_LAYERS: usize = 64;
/// Most points a single stroke keeps (a very long scribble is still fine; this caps memory).
pub const MAX_POINTS: usize = 50_000;

static NEXT: AtomicU64 = AtomicU64::new(1);

/// A process-unique number, for layer ids and content versions.
pub fn next_id() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Brush kinds: how a stroke turns into marks. The `id` strings match the original
/// Wobbleworks `.wob` tool names so old files open unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Brush {
    #[default]
    Marker,
    Dither,
    Fuzz,
    Shaky,
    Rowdy,
    Sketch,
    Ribbon,
    Spray,
    Beads,
    Chalk,
    Nib,
    Blob,
    Steady,
    Eraser,
}

/// How a brush lays its marks down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Stamps walked along the path.
    Stamp,
    /// Three thin wobbly passes.
    Sketch,
    /// Width pulses along the path.
    Ribbon,
    /// Speckles scattered around each point.
    Spray,
    /// One randomly sized stamp per point.
    Beads,
    /// The path's outline, filled.
    Blob,
}

impl Brush {
    pub const ALL: [Brush; 14] = [
        Brush::Marker,
        Brush::Dither,
        Brush::Fuzz,
        Brush::Shaky,
        Brush::Rowdy,
        Brush::Sketch,
        Brush::Ribbon,
        Brush::Spray,
        Brush::Beads,
        Brush::Chalk,
        Brush::Nib,
        Brush::Blob,
        Brush::Steady,
        Brush::Eraser,
    ];

    /// File id (compatible with the original `.wob` format).
    pub fn id(self) -> &'static str {
        match self {
            Brush::Marker => "marker",
            Brush::Dither => "hilite",
            Brush::Fuzz => "fuzz",
            Brush::Shaky => "shaky",
            Brush::Rowdy => "rowdy",
            Brush::Sketch => "sketch",
            Brush::Ribbon => "ribbon",
            Brush::Spray => "spray",
            Brush::Beads => "beads",
            Brush::Chalk => "chalk",
            Brush::Nib => "nib",
            Brush::Blob => "blob",
            Brush::Steady => "steady",
            Brush::Eraser => "eraser",
        }
    }

    /// Unknown ids become a marker, so a file from a newer version still opens.
    pub fn from_id(s: &str) -> Brush {
        Brush::ALL.into_iter().find(|b| b.id() == s).unwrap_or(Brush::Marker)
    }

    pub fn label(self) -> &'static str {
        match self {
            Brush::Marker => "Marker",
            Brush::Dither => "Dither",
            Brush::Fuzz => "Fuzz",
            Brush::Shaky => "Shaky",
            Brush::Rowdy => "Rowdy",
            Brush::Sketch => "Sketch",
            Brush::Ribbon => "Ribbon",
            Brush::Spray => "Spray",
            Brush::Beads => "Beads",
            Brush::Chalk => "Chalk",
            Brush::Nib => "Nib",
            Brush::Blob => "Blob fill",
            Brush::Steady => "Steady",
            Brush::Eraser => "Eraser",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Brush::Marker => "A bold, gently boiling line.",
            Brush::Dither => "Checkerboard ink: a half-tone highlighter.",
            Brush::Fuzz => "Sparse dotted ink, like felt.",
            Brush::Shaky => "Nervous, jittery line.",
            Brush::Rowdy => "Wild line that can't sit still.",
            Brush::Sketch => "Three loose pencil passes.",
            Brush::Ribbon => "Width swells and pinches along the line.",
            Brush::Spray => "Spray-can speckles.",
            Brush::Beads => "A string of bouncy dots.",
            Brush::Chalk => "Grainy chalk with gaps that shimmer.",
            Brush::Nib => "Slanted calligraphy pen: thick and thin strokes.",
            Brush::Blob => "Draw a loop and it fills in, wobbling.",
            Brush::Steady => "No wobble at all.",
            Brush::Eraser => "Rubs out marks (also wobble-free).",
        }
    }

    /// How far (in pixels) the line jitters at 100% wiggle.
    pub fn amp(self) -> f64 {
        match self {
            Brush::Marker | Brush::Dither | Brush::Fuzz | Brush::Nib => 1.5,
            Brush::Shaky => 4.0,
            Brush::Rowdy => 8.0,
            Brush::Sketch | Brush::Spray => 2.5,
            Brush::Ribbon | Brush::Beads | Brush::Chalk | Brush::Blob => 2.0,
            Brush::Steady | Brush::Eraser => 0.0,
        }
    }

    pub fn kind(self) -> Kind {
        match self {
            Brush::Sketch => Kind::Sketch,
            Brush::Ribbon => Kind::Ribbon,
            Brush::Spray => Kind::Spray,
            Brush::Beads => Kind::Beads,
            Brush::Blob => Kind::Blob,
            _ => Kind::Stamp,
        }
    }

    /// Stamp fill pattern: 0 solid, 1 checkerboard, 2 sparse grid.
    pub fn pattern(self) -> u8 {
        match self {
            Brush::Dither => 1,
            Brush::Fuzz => 2,
            _ => 0,
        }
    }

    pub fn erases(self) -> bool {
        self == Brush::Eraser
    }
}

/// Brush tip shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Tip {
    #[default]
    Round,
    Square,
    Diamond,
    Star,
    Heart,
}

impl Tip {
    pub const ALL: [Tip; 5] = [Tip::Round, Tip::Square, Tip::Diamond, Tip::Star, Tip::Heart];

    pub fn id(self) -> &'static str {
        match self {
            Tip::Round => "round",
            Tip::Square => "square",
            Tip::Diamond => "diamond",
            Tip::Star => "star",
            Tip::Heart => "heart",
        }
    }

    pub fn from_id(s: &str) -> Tip {
        Tip::ALL.into_iter().find(|t| t.id() == s).unwrap_or_default()
    }

    pub fn label(self) -> &'static str {
        match self {
            Tip::Round => "Round",
            Tip::Square => "Square",
            Tip::Diamond => "Diamond",
            Tip::Star => "Star",
            Tip::Heart => "Heart",
        }
    }
}

/// A stroke point in canvas pixels, with a pressure-derived size factor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
    /// Size multiplier (1.0 without a pen).
    pub p: f32,
}

impl Pt {
    pub fn new(x: f64, y: f64) -> Self {
        Pt { x, y, p: 1.0 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub brush: Brush,
    pub tip: Tip,
    pub color: Color32,
    pub size: f64,
    pub seed: u32,
    /// Drawn with the layer's alpha lock on: lands only on existing pixels and holds still.
    pub lock: bool,
    pub pts: Vec<Pt>,
}

impl Stroke {
    /// Axis-aligned bounds of the stroke's marks, generously padded for wobble and spray.
    pub fn bounds(&self, wiggle: f64) -> crate::pixels::IRect {
        let mut r = crate::pixels::IRect::EMPTY;
        let pad = self.size * 1.6 + self.brush.amp() * wiggle * 3.0 + 4.0;
        for p in &self.pts {
            let pr = pad * f64::from(p.p.max(1.0));
            let q = crate::pixels::IRect::new((p.x - pr).floor() as i32, (p.y - pr).floor() as i32, (p.x + pr).ceil() as i32 + 1, (p.y + pr).ceil() as i32 + 1);
            r = r.union(q);
        }
        r
    }
}

pub type LayerId = u64;

#[derive(Clone, Debug)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    pub blend: Blend,
    /// Show only where the base layer below has paint.
    pub clip: bool,
    /// New paint lands only on pixels already there.
    pub alpha_lock: bool,
    pub strokes: Arc<Vec<Stroke>>,
    /// One image per frame; `None` is blank.
    pub raster: Vec<Option<Arc<Pixmap>>>,
    /// Bumped on every content change (strokes or raster); render caches key on it.
    pub ver: u64,
}

impl Layer {
    pub fn new(name: impl Into<String>, frames: usize) -> Self {
        Layer {
            id: next_id(),
            name: name.into(),
            visible: true,
            opacity: 1.0,
            blend: Blend::Normal,
            clip: false,
            alpha_lock: false,
            strokes: Arc::new(Vec::new()),
            raster: vec![None; frames],
            ver: next_id(),
        }
    }

    pub fn touch(&mut self) {
        self.ver = next_id();
    }

    /// The raster for `frame`, allocated (blank) on first write.
    pub fn raster_mut(&mut self, frame: usize, w: usize, h: usize) -> Option<&mut Pixmap> {
        let slot = self.raster.get_mut(frame)?;
        let arc = slot.get_or_insert_with(|| Arc::new(Pixmap::new(w, h)));
        Some(Arc::make_mut(arc))
    }

    pub fn is_empty(&self) -> bool {
        self.strokes.is_empty() && self.raster.iter().all(Option::is_none)
    }
}

/// How the frames play back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Playback {
    #[default]
    Loop,
    PingPong,
    Random,
}

impl Playback {
    pub const ALL: [Playback; 3] = [Playback::Loop, Playback::PingPong, Playback::Random];
    pub fn label(self) -> &'static str {
        match self {
            Playback::Loop => "Loop",
            Playback::PingPong => "Ping-pong",
            Playback::Random => "Jumble",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Doc {
    pub w: usize,
    pub h: usize,
    pub frames: usize,
    pub layers: Vec<Layer>,
    pub current: usize,
    pub bg: Color32,
    pub transparent: bool,
    /// Wiggle multiplier (1.0 = 100%).
    pub wiggle: f64,
    /// Milliseconds per frame.
    pub speed_ms: u32,
    pub playback: Playback,
}

impl Doc {
    pub fn new(w: usize, h: usize) -> Self {
        let frames = DEFAULT_FRAMES;
        Doc {
            w: w.clamp(MIN_SIDE, MAX_SIDE),
            h: h.clamp(MIN_SIDE, MAX_SIDE),
            frames,
            layers: vec![Layer::new("Layer 1", frames)],
            current: 0,
            bg: Color32::WHITE,
            transparent: false,
            wiggle: 1.0,
            speed_ms: 120,
            playback: Playback::Loop,
        }
    }

    pub fn layer(&self) -> Option<&Layer> {
        self.layers.get(self.current)
    }

    pub fn layer_mut(&mut self) -> Option<&mut Layer> {
        self.layers.get_mut(self.current)
    }

    /// Keep `current` in range and every layer's raster list matching `frames`.
    pub fn normalize(&mut self) {
        self.frames = self.frames.clamp(MIN_FRAMES, MAX_FRAMES);
        self.w = self.w.clamp(MIN_SIDE, MAX_SIDE);
        self.h = self.h.clamp(MIN_SIDE, MAX_SIDE);
        if self.layers.is_empty() {
            self.layers.push(Layer::new("Layer 1", self.frames));
        }
        self.layers.truncate(MAX_LAYERS);
        let (fr, w, h) = (self.frames, self.w, self.h);
        for l in &mut self.layers {
            if l.raster.len() != fr {
                l.raster.resize(fr, None);
                l.touch();
            }
            let mut changed = false;
            for slot in &mut l.raster {
                if let Some(p) = slot
                    && (p.w != w || p.h != h)
                {
                    *slot = Some(Arc::new(p.resized(w, h)));
                    changed = true;
                }
            }
            if changed {
                l.touch();
            }
            if !l.opacity.is_finite() {
                l.opacity = 1.0;
            }
            l.opacity = l.opacity.clamp(0.0, 1.0);
        }
        self.current = self.current.min(self.layers.len().saturating_sub(1));
        if !self.wiggle.is_finite() {
            self.wiggle = 1.0;
        }
        self.wiggle = self.wiggle.clamp(0.0, 4.0);
        self.speed_ms = self.speed_ms.clamp(20, 2000);
    }

    /// Change the number of frames. New frames copy the last existing raster frame so fills and
    /// imports don't flicker out.
    pub fn set_frames(&mut self, n: usize) {
        let n = n.clamp(MIN_FRAMES, MAX_FRAMES);
        if n == self.frames {
            return;
        }
        for l in &mut self.layers {
            let last = l.raster.last().cloned().flatten();
            l.raster.resize(n, None);
            for slot in l.raster.iter_mut().skip(self.frames) {
                *slot = last.clone();
            }
            l.touch();
        }
        self.frames = n;
    }

    /// Resize the canvas (crop or extend at the bottom-right; strokes keep their coordinates).
    pub fn resize(&mut self, w: usize, h: usize) {
        self.w = w.clamp(MIN_SIDE, MAX_SIDE);
        self.h = h.clamp(MIN_SIDE, MAX_SIDE);
        self.normalize();
    }

    /// The nearest layer at or below `i` that isn't clipped (the clipping base for `i`).
    pub fn clip_base(&self, i: usize) -> Option<usize> {
        let mut j = i;
        loop {
            let l = self.layers.get(j)?;
            if !l.clip || j == 0 {
                return Some(j);
            }
            j -= 1;
        }
    }

    pub fn unique_layer_name(&self) -> String {
        let mut n = self.layers.len() + 1;
        loop {
            let name = format!("Layer {n}");
            if !self.layers.iter().any(|l| l.name == name) {
                return name;
            }
            n += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brush_ids_round_trip_and_unknowns_fall_back() {
        for b in Brush::ALL {
            assert_eq!(Brush::from_id(b.id()), b);
        }
        assert_eq!(Brush::from_id("hilite"), Brush::Dither);
        assert_eq!(Brush::from_id("¯\\_(ツ)_/¯"), Brush::Marker);
        assert_eq!(Tip::from_id("nope"), Tip::Round);
    }

    #[test]
    fn frames_grow_by_copying_the_last_raster() {
        let mut d = Doc::new(32, 32);
        d.layers[0].raster_mut(2, 32, 32).unwrap().set(1, 1, Color32::RED);
        d.set_frames(5);
        assert_eq!(d.layers[0].raster.len(), 5);
        assert_eq!(d.layers[0].raster[4].as_ref().unwrap().get(1, 1), Color32::RED);
        d.set_frames(0);
        assert_eq!(d.frames, MIN_FRAMES);
        assert_eq!(d.layers[0].raster.len(), MIN_FRAMES);
    }

    #[test]
    fn normalize_repairs_hostile_state() {
        let mut d = Doc::new(32, 32);
        d.layers.clear();
        d.current = 99;
        d.wiggle = f64::NAN;
        d.frames = 1000;
        d.w = 0;
        d.normalize();
        assert_eq!(d.layers.len(), 1);
        assert_eq!(d.current, 0);
        assert_eq!(d.wiggle, 1.0);
        assert_eq!(d.frames, MAX_FRAMES);
        assert_eq!(d.w, MIN_SIDE);
    }

    #[test]
    fn clip_base_skips_clipped_layers() {
        let mut d = Doc::new(32, 32);
        for _ in 0..3 {
            d.layers.push(Layer::new("x", 3));
        }
        d.layers[2].clip = true;
        d.layers[3].clip = true;
        assert_eq!(d.clip_base(3), Some(1));
        assert_eq!(d.clip_base(1), Some(1));
        assert_eq!(d.clip_base(9), None);
    }
}
