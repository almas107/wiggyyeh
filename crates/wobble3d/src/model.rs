//! The 3D note: curves (strokes) in groups, resources (guides, reference images, 3D models), the
//! environment (background, lighting, effects), brush presets and camera shots.
//!
//! Units: one world unit is one metre (Feather: one grid square = 1000 mm). Brush sizes are in
//! millimetres (1–300).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::camera::Camera;
use crate::guide::Guide;
use crate::math::{Vec3, Xform, v3};

pub const SIZE_MIN_MM: f32 = 1.0;
pub const SIZE_MAX_MM: f32 = 300.0;
/// Most points a single curve keeps.
pub const STROKE_POINTS_MAX: usize = 50_000;
/// Most curves a note holds.
pub const STROKES_MAX: usize = 200_000;
pub const GROUPS_MAX: usize = 1000;
/// Feather allows two reference images per note.
pub const IMAGES_MAX: usize = 2;
pub const MODELS_MAX: usize = 32;
pub const PRESETS_MAX: usize = 200;
pub const SHOTS_MAX: usize = 500;

/// An sRGB colour with straight alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgba(pub [u8; 4]);

impl Rgba {
    pub const BLACK: Rgba = Rgba([0, 0, 0, 255]);
    pub const WHITE: Rgba = Rgba([255, 255, 255, 255]);

    pub fn rgb(r: u8, g: u8, b: u8) -> Rgba {
        Rgba([r, g, b, 255])
    }

    /// `#rrggbb` or `#rrggbbaa`.
    pub fn from_hex(s: &str) -> Option<Rgba> {
        let h = s.trim().trim_start_matches('#');
        if !h.is_ascii() {
            return None;
        }
        let byte = |i: usize| h.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok());
        match h.len() {
            6 => Some(Rgba([byte(0)?, byte(2)?, byte(4)?, 255])),
            8 => Some(Rgba([byte(0)?, byte(2)?, byte(4)?, byte(6)?])),
            _ => None,
        }
    }

    pub fn to_hex(self) -> String {
        let [r, g, b, a] = self.0;
        if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }
    }

    /// Linear-ish floats 0..1 (sRGB values, no gamma decode: shading here is stylised).
    pub fn to_f32(self) -> [f32; 4] {
        let [r, g, b, a] = self.0;
        [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0]
    }

    pub fn from_f32(c: [f32; 4]) -> Rgba {
        let q = |v: f32| (if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 } * 255.0 + 0.5) as u8;
        Rgba([q(c[0]), q(c[1]), q(c[2]), q(c[3])])
    }

    /// (hue degrees, saturation, value), all finite.
    pub fn to_hsv(self) -> (f32, f32, f32) {
        let [r, g, b, _] = self.to_f32();
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let d = max - min;
        let h = if d <= 1e-6 {
            0.0
        } else if max == r {
            60.0 * ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
        let s = if max <= 1e-6 { 0.0 } else { d / max };
        (h, s, max)
    }

    pub fn from_hsv(h: f32, s: f32, v: f32, a: u8) -> Rgba {
        let h = if h.is_finite() { h.rem_euclid(360.0) } else { 0.0 };
        let s = if s.is_finite() { s.clamp(0.0, 1.0) } else { 0.0 };
        let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        let c = v * s;
        let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
        let m = v - c;
        let (r, g, b) = match (h / 60.0) as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let mut out = Rgba::from_f32([r + m, g + m, b + m, 1.0]);
        out.0[3] = a;
        out
    }
}

/// How a curve's cross-section is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BrushKind {
    /// A round tube: shaded like a 3D wire. Feather's default look.
    Pen,
    /// A soft round marker: a flat band that always faces the viewer.
    Marker,
    /// A flat band lying on the guide surface it was drawn on (like tape).
    Flat,
    /// A square tube.
    Square,
    /// A calligraphy nib: width depends on the direction of travel.
    Nib,
    /// Painterly: an oil-paint band with bristle streaks and a ragged edge.
    Oil,
    /// Painterly: thick opaque gouache dabs overlapping along the stroke.
    Gouache,
    /// Painterly: a dry brush that breaks up into streaks.
    DryBrush,
    /// Painterly: chalk / crayon grain.
    Chalk,
    /// Painterly: an ink brush with a pointed, ragged, pressure-swelling line.
    Ink,
}

impl BrushKind {
    pub const ALL: [BrushKind; 10] = [
        BrushKind::Pen,
        BrushKind::Marker,
        BrushKind::Flat,
        BrushKind::Square,
        BrushKind::Nib,
        BrushKind::Oil,
        BrushKind::Gouache,
        BrushKind::DryBrush,
        BrushKind::Chalk,
        BrushKind::Ink,
    ];

    pub fn name(self) -> &'static str {
        match self {
            BrushKind::Pen => "pen",
            BrushKind::Marker => "marker",
            BrushKind::Flat => "flat",
            BrushKind::Square => "square",
            BrushKind::Nib => "nib",
            BrushKind::Oil => "oil",
            BrushKind::Gouache => "gouache",
            BrushKind::DryBrush => "drybrush",
            BrushKind::Chalk => "chalk",
            BrushKind::Ink => "ink",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            BrushKind::Pen => "Pen",
            BrushKind::Marker => "Marker",
            BrushKind::Flat => "Flat",
            BrushKind::Square => "Square",
            BrushKind::Nib => "Nib",
            BrushKind::Oil => "Oil",
            BrushKind::Gouache => "Gouache",
            BrushKind::DryBrush => "Dry brush",
            BrushKind::Chalk => "Chalk",
            BrushKind::Ink => "Ink",
        }
    }

    pub fn parse(s: &str) -> Option<BrushKind> {
        BrushKind::ALL.into_iter().find(|k| k.name().eq_ignore_ascii_case(s) || k.label().eq_ignore_ascii_case(s))
    }

    /// Painterly brushes use a texture (streaks, grain, ragged edges).
    pub fn painterly(self) -> bool {
        matches!(self, BrushKind::Oil | BrushKind::Gouache | BrushKind::DryBrush | BrushKind::Chalk | BrushKind::Ink)
    }

    /// The painterly options a brush starts with (Shift+B cycles kinds and loads these).
    pub fn default_paint(self) -> Paint {
        let p = Paint::default();
        match self {
            BrushKind::Oil => Paint { roughness: 0.45, bristles: 0.7, dryness: 0.15, taper: 0.35, ..p },
            BrushKind::Gouache => Paint { roughness: 0.6, bristles: 0.25, dryness: 0.05, layers: 3, taper: 0.2, ..p },
            BrushKind::DryBrush => Paint { roughness: 0.6, bristles: 0.75, dryness: 0.4, taper: 0.5, ..p },
            BrushKind::Chalk => Paint { roughness: 0.45, bristles: 0.1, dryness: 0.2, grain: 0.55, taper: 0.15, ..p },
            BrushKind::Ink => Paint { roughness: 0.35, bristles: 0.2, dryness: 0.1, taper: 0.8, ..p },
            BrushKind::Nib => Paint { taper: 0.3, ..p },
            _ => p,
        }
    }
}

/// Feather's materials (shown accurately in render mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Material {
    /// Flat colour, no lighting or shadows; takes patterns.
    Shadeless,
    /// Lit and casts shadows; takes patterns. Toon shading and ground shadows apply to it.
    #[default]
    Shaded,
    /// Glows (no lighting, no patterns); intensity adjustable.
    Glow,
    /// Shows the background (colour or image) through: a hole in the drawing.
    Cutout,
}

impl Material {
    pub const ALL: [Material; 4] = [Material::Shadeless, Material::Shaded, Material::Glow, Material::Cutout];
    pub fn name(self) -> &'static str {
        match self {
            Material::Shadeless => "shadeless",
            Material::Shaded => "shaded",
            Material::Glow => "glow",
            Material::Cutout => "cutout",
        }
    }
    pub fn parse(s: &str) -> Option<Material> {
        Material::ALL.into_iter().find(|m| m.name().eq_ignore_ascii_case(s))
    }
}

/// Feather's procedural patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PatternKind {
    Dot,
    Line,
    Cross,
    Terrazzo,
    StippledDot,
}

impl PatternKind {
    pub const ALL: [PatternKind; 5] = [PatternKind::Dot, PatternKind::Line, PatternKind::Cross, PatternKind::Terrazzo, PatternKind::StippledDot];
    pub fn name(self) -> &'static str {
        match self {
            PatternKind::Dot => "dot",
            PatternKind::Line => "line",
            PatternKind::Cross => "cross",
            PatternKind::Terrazzo => "terrazzo",
            PatternKind::StippledDot => "stippled",
        }
    }
    pub fn parse(s: &str) -> Option<PatternKind> {
        PatternKind::ALL.into_iter().find(|m| m.name().eq_ignore_ascii_case(s))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pattern {
    pub kind: PatternKind,
    /// 0..1
    pub intensity: f32,
    /// Degrees.
    pub angle: f32,
    /// 0..1
    pub contrast: f32,
}

/// Painterly options (WobbleWorks): what makes strokes look painted and alive, the Metaphor:
/// ReFantazio text-box look, without node graphs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Paint {
    /// Ragged edge amount, 0..1.
    pub roughness: f32,
    /// Bristle streaks along the stroke, 0..1.
    pub bristles: f32,
    /// Dry-brush break-up: gaps that open where the brush runs out of paint, 0..1.
    pub dryness: f32,
    /// Paper/chalk grain, 0..1.
    pub grain: f32,
    /// How much the ends narrow to points, 0..1.
    pub taper: f32,
    /// Overlapping offset passes of the same stroke (1 = a single pass), 1..4.
    pub layers: u8,
    /// An echo: the same stroke again behind it, offset on screen and in another colour (the
    /// cut-paper drop shadow / outline of a painted UI).
    pub echo: Option<Echo>,
    /// How strongly this stroke boils, times the note's wiggle (0 holds it still), 0..4.
    pub boil: f32,
}

impl Default for Paint {
    fn default() -> Self {
        Paint { roughness: 0.0, bristles: 0.0, dryness: 0.0, grain: 0.0, taper: 0.0, layers: 1, echo: None, boil: 1.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Echo {
    pub color: Rgba,
    /// Screen offset in pixels.
    pub offset: [f32; 2],
    /// Width relative to the stroke (1 = same width).
    pub width: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Brush {
    pub kind: BrushKind,
    pub color: Rgba,
    /// Diameter in millimetres, SIZE_MIN_MM..=SIZE_MAX_MM.
    pub size_mm: f32,
    /// 0..1
    pub opacity: f32,
    /// Pen pressure changes the size.
    pub pressure: bool,
    pub material: Material,
    pub pattern: Option<Pattern>,
    /// Glow intensity for the Glow material, 0..1.
    pub glow: f32,
    #[serde(default)]
    pub paint: Paint,
}

impl Default for Brush {
    fn default() -> Self {
        Brush {
            kind: BrushKind::Pen,
            color: Rgba::rgb(0x23, 0x22, 0x2b),
            size_mm: 12.0,
            opacity: 1.0,
            pressure: true,
            material: Material::Shaded,
            pattern: None,
            glow: 0.6,
            paint: Paint::default(),
        }
    }
}

impl Brush {
    /// Clamp every field into range (never trusts loaded or agent-supplied values).
    pub fn sanitize(&mut self) {
        let unit = |v: f32, d: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { d };
        self.size_mm = if self.size_mm.is_finite() { self.size_mm.clamp(SIZE_MIN_MM, SIZE_MAX_MM) } else { 12.0 };
        self.opacity = unit(self.opacity, 1.0);
        self.glow = unit(self.glow, 0.6);
        if let Some(p) = &mut self.pattern {
            p.intensity = unit(p.intensity, 0.5);
            p.contrast = unit(p.contrast, 0.5);
            p.angle = if p.angle.is_finite() { p.angle.rem_euclid(360.0) } else { 0.0 };
        }
        let pa = &mut self.paint;
        pa.roughness = unit(pa.roughness, 0.0);
        pa.bristles = unit(pa.bristles, 0.0);
        pa.dryness = unit(pa.dryness, 0.0);
        pa.grain = unit(pa.grain, 0.0);
        pa.taper = unit(pa.taper, 0.0);
        pa.layers = pa.layers.clamp(1, 4);
        pa.boil = if pa.boil.is_finite() { pa.boil.clamp(0.0, 4.0) } else { 1.0 };
        if let Some(e) = &mut pa.echo {
            let f = |v: f32| if v.is_finite() { v.clamp(-200.0, 200.0) } else { 0.0 };
            e.offset = [f(e.offset[0]), f(e.offset[1])];
            e.width = if e.width.is_finite() { e.width.clamp(0.1, 4.0) } else { 1.0 };
        }
    }

    /// Radius in world units (metres).
    pub fn radius(&self) -> f32 {
        self.size_mm.clamp(SIZE_MIN_MM, SIZE_MAX_MM) / 2000.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub p: Vec3,
    /// 0..1
    pub pressure: f32,
    /// The surface normal where the point was drawn (zero when drawn in the air).
    pub n: Vec3,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub id: u64,
    pub group: u64,
    pub points: Vec<Point>,
    pub brush: Brush,
    /// Seeds the boil and painterly noise, so every stroke wiggles its own way.
    pub seed: u32,
}

impl Stroke {
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut it = self.points.iter();
        let f = it.next()?.p;
        Some(it.fold((f, f), |(lo, hi), p| (lo.min(p.p), hi.max(p.p))))
    }
    pub fn centre(&self) -> Vec3 {
        self.bounds().map_or(Vec3::ZERO, |(lo, hi)| (lo + hi) * 0.5)
    }
    /// Remove non-finite points and clamp pressures; keep at most STROKE_POINTS_MAX.
    pub fn sanitize(&mut self) {
        self.points.retain(|p| p.p.is_finite());
        self.points.truncate(STROKE_POINTS_MAX);
        for p in &mut self.points {
            p.pressure = if p.pressure.is_finite() { p.pressure.clamp(0.0, 1.0) } else { 1.0 };
            if !p.n.is_finite() {
                p.n = Vec3::ZERO;
            }
        }
        self.brush.sanitize();
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub id: u64,
    pub name: String,
    pub visible: bool,
}

/// Feather's resource states (the cube icon).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ResourceState {
    /// Visible and drawn on.
    Active,
    /// Visible, not drawn on.
    #[default]
    Visible,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageResource {
    pub id: u64,
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// Straight-alpha RGBA8, `width * height * 4` bytes.
    #[serde(with = "bytes_b64")]
    pub rgba: Arc<Vec<u8>>,
    /// Places the unit-height quad (aspect-correct, centred on the origin, facing +Z).
    pub xform: Xform,
    pub opacity: f32,
    pub state: ResourceState,
}

impl ImageResource {
    /// The quad's four corners in world space (counter-clockwise from bottom left).
    pub fn corners(&self) -> [Vec3; 4] {
        let aspect = if self.height > 0 { self.width as f32 / self.height as f32 } else { 1.0 };
        let (hw, hh) = (aspect * 0.5, 0.5);
        [v3(-hw, -hh, 0.0), v3(hw, -hh, 0.0), v3(hw, hh, 0.0), v3(-hw, hh, 0.0)].map(|p| self.xform.apply(p))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelResource {
    pub id: u64,
    pub name: String,
    /// Positions in model space.
    pub positions: Vec<Vec3>,
    pub triangles: Vec<[u32; 3]>,
    pub xform: Xform,
    pub color: Rgba,
    pub state: ResourceState,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Lighting {
    /// Degrees around the up axis.
    pub azimuth: f32,
    /// Degrees above the horizon.
    pub altitude: f32,
    pub color: Rgba,
    /// 0..2
    pub strength: f32,
    pub ground_shadow: bool,
    pub toon: bool,
}

impl Default for Lighting {
    fn default() -> Self {
        Lighting { azimuth: 35.0, altitude: 50.0, color: Rgba::WHITE, strength: 1.0, ground_shadow: false, toon: false }
    }
}

impl Lighting {
    /// Unit vector towards the light.
    pub fn direction(&self) -> Vec3 {
        let (az, al) = (self.azimuth.to_radians(), self.altitude.to_radians());
        v3(al.cos() * az.sin(), al.sin(), al.cos() * az.cos()).normalized()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Effects {
    /// Glow Area: halo size, 0 = off, up to 1.
    pub glow: f32,
    /// Depth of field: aperture f-stop, 0 = off (else 0.7..22).
    pub dof: f32,
    /// Film grain amount, 0..1.
    pub grain: f32,
    /// Pixelation block size in pixels, 0/1 = off, up to 32.
    pub pixelate: f32,
    /// Bloom amount, 0..1.
    pub bloom: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Environment {
    pub show_axes: bool,
    pub show_grid: bool,
    pub background: Rgba,
    /// A reference image filling the screen behind everything (index into `images` is not
    /// used: the background image is separate and not counted towards the two images).
    #[serde(default)]
    pub background_image: Option<u64>,
    pub fog: bool,
    pub lighting: Lighting,
    pub effects: Effects,
    /// Render mode: materials, lighting and effects shown accurately.
    pub render_mode: bool,
}

impl Default for Environment {
    fn default() -> Self {
        Environment {
            show_axes: false,
            show_grid: true,
            background: Rgba::rgb(0xf4, 0xf1, 0xea),
            background_image: None,
            fog: false,
            lighting: Lighting::default(),
            effects: Effects::default(),
            render_mode: false,
        }
    }
}

/// The boiling-line animation (WobbleWorks).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Boil {
    pub enabled: bool,
    /// How far lines wander, in screen pixels (or millimetres in world mode), 0..40.
    pub amount: f32,
    /// Frames in the loop, 2..=12.
    pub frames: u32,
    /// Frames per second, 1..=24.
    pub fps: f32,
    /// Wiggle in world space (lines nearer the camera wiggle more) instead of on screen.
    pub world_space: bool,
    /// How wavy the wiggle is along a line: wavelength in screen pixels, 4..400.
    pub wavelength: f32,
    /// Line thickness wobble, 0..1.
    pub thickness: f32,
}

impl Default for Boil {
    fn default() -> Self {
        Boil { enabled: true, amount: 2.2, frames: 3, fps: 8.0, world_space: false, wavelength: 42.0, thickness: 0.25 }
    }
}

impl Boil {
    pub fn sanitize(&mut self) {
        let d = Boil::default();
        self.amount = if self.amount.is_finite() { self.amount.clamp(0.0, 40.0) } else { d.amount };
        self.frames = self.frames.clamp(2, 12);
        self.fps = if self.fps.is_finite() { self.fps.clamp(1.0, 24.0) } else { d.fps };
        self.wavelength = if self.wavelength.is_finite() { self.wavelength.clamp(4.0, 400.0) } else { d.wavelength };
        self.thickness = if self.thickness.is_finite() { self.thickness.clamp(0.0, 1.0) } else { d.thickness };
    }

    /// The boil frame showing at time `t` seconds.
    pub fn frame_at(&self, t: f64) -> u32 {
        if !self.enabled || !t.is_finite() || t < 0.0 {
            return 0;
        }
        ((t * self.fps.clamp(1.0, 24.0) as f64) as u64 % self.frames.clamp(2, 12) as u64) as u32
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub id: u64,
    pub name: String,
    pub brush: Brush,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shot {
    pub id: u64,
    pub name: String,
    pub camera: Camera,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PlayMode {
    Once,
    #[default]
    Loop,
    Swing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sequence {
    pub shots: Vec<Shot>,
    /// 0.5, 1 or 2.
    pub speed: f32,
    pub mode: PlayMode,
    /// Seconds per shot transition at 1x.
    pub seconds_per_shot: f32,
}

impl Default for Sequence {
    fn default() -> Self {
        Sequence { shots: Vec::new(), speed: 1.0, mode: PlayMode::Loop, seconds_per_shot: 2.0 }
    }
}

/// The whole note. Strokes and guides sit behind `Arc`s, so undo snapshots share them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub strokes: Vec<Arc<Stroke>>,
    /// Bottom to top.
    pub groups: Vec<Group>,
    pub active_group: u64,
    /// A group shown alone (long-press on its eye), if any.
    #[serde(default)]
    pub isolated: Option<u64>,
    /// Saved guides ("Surface" resources) and the active guide.
    pub guides: Vec<Arc<Guide>>,
    /// The guide drawn on (one at a time).
    pub active_guide: Option<u64>,
    /// Resource state of each saved guide, by id (the active guide is always Active).
    #[serde(default)]
    pub guide_states: Vec<(u64, ResourceState)>,
    pub images: Vec<Arc<ImageResource>>,
    pub models: Vec<Arc<ModelResource>>,
    pub environment: Environment,
    pub boil: Boil,
    pub presets: Vec<Preset>,
    pub sequence: Sequence,
    pub next_id: u64,
}

impl Default for Scene {
    fn default() -> Self {
        Scene {
            strokes: Vec::new(),
            groups: vec![Group { id: 1, name: "Group 1".into(), visible: true }],
            active_group: 1,
            isolated: None,
            guides: Vec::new(),
            active_guide: None,
            guide_states: Vec::new(),
            images: Vec::new(),
            models: Vec::new(),
            environment: Environment::default(),
            boil: Boil::default(),
            presets: Vec::new(),
            sequence: Sequence::default(),
            next_id: 2,
        }
    }
}

impl Scene {
    pub fn alloc_id(&mut self) -> u64 {
        let id = self.next_id.max(1);
        self.next_id = id.saturating_add(1);
        id
    }

    pub fn group(&self, id: u64) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    /// Whether strokes in a group show (visibility and isolation).
    pub fn group_shown(&self, id: u64) -> bool {
        match self.isolated {
            Some(iso) => iso == id,
            None => self.group(id).is_some_and(|g| g.visible),
        }
    }

    pub fn stroke(&self, id: u64) -> Option<&Arc<Stroke>> {
        self.strokes.iter().find(|s| s.id == id)
    }

    pub fn guide(&self, id: u64) -> Option<&Arc<Guide>> {
        self.guides.iter().find(|g| g.id == id)
    }

    pub fn active_guide(&self) -> Option<&Arc<Guide>> {
        self.active_guide.and_then(|id| self.guide(id))
    }

    pub fn guide_state(&self, id: u64) -> ResourceState {
        if self.active_guide == Some(id) {
            return ResourceState::Active;
        }
        self.guide_states.iter().find(|(g, _)| *g == id).map_or(ResourceState::Hidden, |(_, s)| *s)
    }

    pub fn set_guide_state(&mut self, id: u64, state: ResourceState) {
        self.guide_states.retain(|(g, _)| *g != id);
        if state == ResourceState::Active {
            // One active guide at a time: the old one stays visible.
            if let Some(old) = self.active_guide
                && old != id
            {
                self.guide_states.push((old, ResourceState::Visible));
            }
            self.active_guide = Some(id);
        } else {
            if self.active_guide == Some(id) {
                self.active_guide = None;
            }
            self.guide_states.push((id, state));
        }
    }

    /// Repair anything a file or an agent could have broken: ids, groups, ranges, caches.
    pub fn repair(&mut self) {
        if self.groups.is_empty() {
            let id = self.alloc_id();
            self.groups.push(Group { id, name: "Group 1".into(), visible: true });
        }
        self.groups.truncate(GROUPS_MAX);
        let max_id = self
            .strokes
            .iter()
            .map(|s| s.id)
            .chain(self.groups.iter().map(|g| g.id))
            .chain(self.guides.iter().map(|g| g.id))
            .chain(self.images.iter().map(|g| g.id))
            .chain(self.models.iter().map(|g| g.id))
            .chain(self.presets.iter().map(|g| g.id))
            .chain(self.sequence.shots.iter().map(|g| g.id))
            .max()
            .unwrap_or(0);
        self.next_id = self.next_id.max(max_id.saturating_add(1));
        if self.group(self.active_group).is_none() {
            self.active_group = self.groups.last().map_or(1, |g| g.id);
        }
        let first_group = self.groups.first().map_or(1, |g| g.id);
        self.strokes.truncate(STROKES_MAX);
        let ids: Vec<u64> = self.groups.iter().map(|g| g.id).collect();
        for s in &mut self.strokes {
            let s = Arc::make_mut(s);
            s.sanitize();
            if !ids.contains(&s.group) {
                s.group = first_group;
            }
        }
        self.strokes.retain(|s| !s.points.is_empty());
        for g in &mut self.guides {
            let g = Arc::make_mut(g);
            if g.rebuild().is_err() {
                g.grids.clear();
            }
        }
        self.guides.retain(|g| !g.grids.is_empty());
        if self.active_guide.is_some_and(|id| self.guide(id).is_none()) {
            self.active_guide = None;
        }
        if self.isolated.is_some_and(|id| self.group(id).is_none()) {
            self.isolated = None;
        }
        self.images.retain(|i| i.rgba.len() as u64 == i.width as u64 * i.height as u64 * 4 && i.width > 0 && i.height > 0);
        self.images.truncate(IMAGES_MAX);
        self.models.truncate(MODELS_MAX);
        for m in &mut self.models {
            let n = m.positions.len() as u64;
            if m.triangles.iter().any(|t| t.iter().any(|i| *i as u64 >= n)) || m.positions.iter().any(|p| !p.is_finite()) {
                let mm = Arc::make_mut(m);
                mm.positions.retain(|p| p.is_finite());
                let n = mm.positions.len() as u64;
                mm.triangles.retain(|t| t.iter().all(|i| (*i as u64) < n));
            }
        }
        self.boil.sanitize();
        for p in &mut self.presets {
            p.brush.sanitize();
        }
        self.presets.truncate(PRESETS_MAX);
        self.sequence.shots.truncate(SHOTS_MAX);
        for s in &mut self.sequence.shots {
            s.camera.sanitize();
        }
        let sp = self.sequence.speed;
        self.sequence.speed = if sp.is_finite() { sp.clamp(0.25, 4.0) } else { 1.0 };
        let sps = self.sequence.seconds_per_shot;
        self.sequence.seconds_per_shot = if sps.is_finite() { sps.clamp(0.1, 60.0) } else { 2.0 };
        let l = &mut self.environment.lighting;
        l.strength = if l.strength.is_finite() { l.strength.clamp(0.0, 2.0) } else { 1.0 };
        l.azimuth = if l.azimuth.is_finite() { l.azimuth.rem_euclid(360.0) } else { 35.0 };
        l.altitude = if l.altitude.is_finite() { l.altitude.clamp(-90.0, 90.0) } else { 50.0 };
        let e = &mut self.environment.effects;
        let c = |v: f32, hi: f32| if v.is_finite() { v.clamp(0.0, hi) } else { 0.0 };
        e.glow = c(e.glow, 1.0);
        e.dof = c(e.dof, 22.0);
        e.grain = c(e.grain, 1.0);
        e.pixelate = c(e.pixelate, 32.0);
        e.bloom = c(e.bloom, 1.0);
    }

    /// Bounding box of every visible stroke.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        self.strokes.iter().filter(|s| self.group_shown(s.group)).filter_map(|s| s.bounds()).reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
    }
}

/// Base64 for byte buffers in JSON.
mod bytes_b64 {
    use std::sync::Arc;

    use serde::{Deserialize, Deserializer, Serializer};

    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(data: &[u8]) -> String {
        let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
        for chunk in data.chunks(3) {
            let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            for (i, shift) in [18u32, 12, 6, 0].iter().enumerate() {
                if i <= chunk.len() {
                    out.push(ALPHABET[((n >> shift) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    pub fn decode(s: &str) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(s.len() / 4 * 3);
        let mut buf = 0u32;
        let mut bits = 0u32;
        for c in s.bytes() {
            if c == b'=' {
                break;
            }
            let v = ALPHABET.iter().position(|a| *a == c)? as u32;
            buf = (buf << 6) | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buf >> bits) as u8);
                buf &= (1 << bits) - 1;
            }
        }
        Some(out)
    }

    pub fn serialize<S: Serializer>(v: &Arc<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&encode(v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<Vec<u8>>, D::Error> {
        let s = String::deserialize(d)?;
        decode(&s).map(Arc::new).ok_or_else(|| serde::de::Error::custom("bad base64"))
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn round_trips() {
            for n in 0..10 {
                let data: Vec<u8> = (0..n).map(|i| (i * 37 + 5) as u8).collect();
                assert_eq!(super::decode(&super::encode(&data)).as_deref(), Some(&data[..]));
            }
            assert!(super::decode("!!").is_none());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_parse_and_convert() {
        assert_eq!(Rgba::from_hex("#ff8000"), Some(Rgba([255, 128, 0, 255])));
        assert_eq!(Rgba::from_hex("ff800080"), Some(Rgba([255, 128, 0, 128])));
        assert!(Rgba::from_hex("#ff80").is_none());
        assert!(Rgba::from_hex("#ééé").is_none());
        let c = Rgba::rgb(40, 200, 120);
        let (h, s, v) = c.to_hsv();
        assert_eq!(Rgba::from_hsv(h, s, v, 255), c);
        assert_eq!(Rgba::from_hsv(f32::NAN, f32::NAN, f32::NAN, 255), Rgba::BLACK);
    }

    #[test]
    fn repair_fixes_broken_notes() {
        let mut s = Scene { groups: Vec::new(), active_group: 99, next_id: 0, ..Scene::default() };
        s.strokes.push(Arc::new(Stroke {
            id: 50,
            group: 7,
            points: vec![Point { p: v3(f32::NAN, 0.0, 0.0), pressure: 2.0, n: Vec3::ZERO }, Point { p: Vec3::ZERO, pressure: f32::NAN, n: Vec3::ZERO }],
            brush: Brush { size_mm: 1e9, opacity: -1.0, ..Brush::default() },
            seed: 1,
        }));
        s.boil.frames = 900;
        s.repair();
        assert_eq!(s.groups.len(), 1);
        assert!(s.next_id > 50);
        let st = &s.strokes[0];
        assert_eq!(st.points.len(), 1);
        assert_eq!(st.points[0].pressure, 1.0);
        assert_eq!(st.brush.size_mm, SIZE_MAX_MM);
        assert_eq!(st.group, s.groups[0].id);
        assert_eq!(s.boil.frames, 12);
    }

    #[test]
    fn boil_frames_cycle() {
        let b = Boil { fps: 4.0, frames: 3, ..Boil::default() };
        assert_eq!((0..6).map(|i| b.frame_at(i as f64 * 0.25)).collect::<Vec<_>>(), vec![0, 1, 2, 0, 1, 2]);
        assert_eq!(b.frame_at(f64::NAN), 0);
        assert_eq!(Boil { enabled: false, ..b }.frame_at(3.0), 0);
    }

    #[test]
    fn one_active_guide_at_a_time() {
        let mut s = Scene::default();
        s.set_guide_state(5, ResourceState::Active);
        s.set_guide_state(6, ResourceState::Active);
        assert_eq!(s.active_guide, Some(6));
        assert_eq!(s.guide_state(5), ResourceState::Visible);
        s.set_guide_state(6, ResourceState::Hidden);
        assert_eq!(s.active_guide, None);
        assert_eq!(s.guide_state(6), ResourceState::Hidden);
    }
}
