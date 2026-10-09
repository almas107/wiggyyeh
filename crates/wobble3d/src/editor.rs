//! The editor: the note, the camera, the current tool and brush, selection, sessions (guide
//! drawing, loft, primitives, liquify, modal transforms) and undo. A UI feeds it pointer events
//! and runs commands by id; it never needs to touch the note directly.

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::assist::{Shape, Stabilizer, mirror_maps, recognize, reflect};
use crate::camera::{Camera, PerfectView, View, Viewport};
use crate::guide::{Guide, Primitive};
use crate::keys::Keymap;
use crate::math::{Quat, Vec3, Xform, v3};
use crate::model::{
    Brush, BrushKind, Group, ImageResource, Material, ModelResource, Pattern, PatternKind, PlayMode, Point, Preset, ResourceState, Rgba, Scene, Shot, Stroke,
};
use crate::noise::hash;
use crate::ops::{self, Liquify, LiquifyKind};
use crate::render::{self, Frame, Options};
use crate::texture::Atlas;
use crate::transform::{self, Delta, GizmoKind, Handle, Modal, Mode, Part};

pub const UNDO_MAX: usize = 256;
/// How long the pen must rest for Draw Shape's hold-to-adjust (seconds).
pub const HOLD_SECONDS: f64 = 0.45;
/// Most points in one command's point list.
pub const PARAM_POINTS_MAX: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Draw,
    DrawShape,
    Erase,
    Vacuum,
    Select,
    Deselect,
    /// Draw a 3D Guide.
    Guide,
    /// Bend the active guide.
    Bend,
    Loft,
    Primitive,
    Liquify,
    Injector,
    Eyedropper,
    /// Feather's Stamp: each click (or step of a drag) drops a copy of the selection on the
    /// guide under the pointer.
    Stamp,
}

impl Tool {
    pub const ALL: [Tool; 14] = [
        Tool::Draw,
        Tool::DrawShape,
        Tool::Erase,
        Tool::Vacuum,
        Tool::Select,
        Tool::Deselect,
        Tool::Guide,
        Tool::Bend,
        Tool::Loft,
        Tool::Primitive,
        Tool::Liquify,
        Tool::Injector,
        Tool::Eyedropper,
        Tool::Stamp,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Tool::Draw => "draw",
            Tool::DrawShape => "shape",
            Tool::Erase => "erase",
            Tool::Vacuum => "vacuum",
            Tool::Select => "select",
            Tool::Deselect => "deselect",
            Tool::Guide => "guide",
            Tool::Bend => "bend",
            Tool::Loft => "loft",
            Tool::Primitive => "primitive",
            Tool::Liquify => "liquify",
            Tool::Injector => "injector",
            Tool::Eyedropper => "eyedropper",
            Tool::Stamp => "stamp",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::Draw => "Draw",
            Tool::DrawShape => "Draw Shape",
            Tool::Erase => "Erase",
            Tool::Vacuum => "Vacuum",
            Tool::Select => "Select",
            Tool::Deselect => "Deselect",
            Tool::Guide => "Draw 3D Guide",
            Tool::Bend => "Bend 3D Guide",
            Tool::Loft => "Loft",
            Tool::Primitive => "Primitives",
            Tool::Liquify => "Liquify",
            Tool::Injector => "Injector",
            Tool::Eyedropper => "Eyedropper",
            Tool::Stamp => "Stamp",
        }
    }

    pub fn parse(s: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.name().eq_ignore_ascii_case(s))
    }

    /// Tools that draw or erase (Feather: choosing them drops the selection).
    fn paints(self) -> bool {
        matches!(self, Tool::Draw | Tool::DrawShape | Tool::Erase | Tool::Vacuum)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectMode {
    /// Drag over curves (Feather).
    Brush,
    Box,
    Circle,
    Lasso,
}

impl SelectMode {
    pub fn parse(s: &str) -> Option<SelectMode> {
        match s.to_ascii_lowercase().as_str() {
            "brush" | "tweak" => Some(SelectMode::Brush),
            "box" => Some(SelectMode::Box),
            "circle" => Some(SelectMode::Circle),
            "lasso" => Some(SelectMode::Lasso),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            SelectMode::Brush => "brush",
            SelectMode::Box => "box",
            SelectMode::Circle => "circle",
            SelectMode::Lasso => "lasso",
        }
    }
    fn next(self) -> SelectMode {
        match self {
            SelectMode::Brush => SelectMode::Box,
            SelectMode::Box => SelectMode::Circle,
            SelectMode::Circle => SelectMode::Lasso,
            SelectMode::Lasso => SelectMode::Brush,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pivot {
    /// The middle of the selected things' centres (Blender's median point).
    Median,
    /// The centre of their bounding box.
    Bounds,
    /// The orbit point (like Blender's 3D cursor; Feather's screen-centre pivot).
    OrbitPoint,
}

impl Pivot {
    pub fn parse(s: &str) -> Option<Pivot> {
        match s.to_ascii_lowercase().as_str() {
            "median" => Some(Pivot::Median),
            "bounds" => Some(Pivot::Bounds),
            "orbit" | "cursor" | "orbitpoint" => Some(Pivot::OrbitPoint),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

/// What happened, for sounds and effects in the shell.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Stroke,
    Erased,
    Undo,
    Redo,
    Deleted,
    Duplicated,
    GuideMade,
    GuideClosed,
    GuideSaved,
    Selected(usize),
    Transformed,
    Sampled,
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveKind {
    Draw,
    Guide,
    Bend,
    Erase,
    Select,
    Liquify,
}

#[derive(Debug, Clone)]
struct Live {
    kind: LiveKind,
    /// Stabilised screen samples (x, y, pressure).
    raw: Vec<[f32; 3]>,
    stab: Stabilizer,
    /// The curve being drawn and pieces already finished (where the pen left the guide).
    stroke: Vec<Point>,
    pieces: Vec<Vec<Point>>,
    down: [f32; 2],
    last: [f32; 2],
    last_move_t: f64,
    shape: Option<Shape>,
    adjusting: bool,
    mods: Mods,
    moved: bool,
}

#[derive(Debug, Clone)]
struct ModalState {
    modal: Modal,
    base: Scene,
}

#[derive(Debug, Clone)]
struct LoftState {
    curves: Vec<u64>,
    tension: f32,
    guide: Option<u64>,
    base: Scene,
}

#[derive(Debug, Clone)]
struct PrimitiveState {
    kind: Primitive,
    segments: u32,
    guide: u64,
    base: Scene,
}

/// A session in progress, for the context bar.
#[derive(Debug, Clone, PartialEq)]
pub enum Session {
    None,
    Loft { curves: usize, tension: f32 },
    Primitive { kind: Primitive, segments: u32 },
    Liquify,
}

/// What the shell draws over the 3D view.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overlay {
    /// The guide or bend stroke being drawn, the lasso.
    pub polyline: Vec<[f32; 2]>,
    /// The box being dragged.
    pub rect: Option<[f32; 4]>,
    /// A brush circle at the mouse (eraser, circle select, liquify): (radius, inner radius).
    pub circle: Option<(f32, f32)>,
    pub gizmo: Vec<Part>,
    /// The modal transform's header line.
    pub header: Option<String>,
    /// The axis line of a constrained transform: two screen points and the axis (0..2).
    pub axis_line: Option<([f32; 2], [f32; 2], usize)>,
    /// Draw Shape is being adjusted.
    pub adjusting: bool,
}

pub struct Editor {
    pub scene: Scene,
    pub camera: Camera,
    pub brush: Brush,
    pub tool: Tool,
    pub select_mode: SelectMode,
    pub selection: HashSet<u64>,
    pub selected_resources: HashSet<u64>,
    pub mirror_on: bool,
    pub mirror_axes: [bool; 3],
    /// Stable Stroke amount, 0..1 (0 = off).
    pub stable: f32,
    /// Draw Shape also corrects guide and bend strokes.
    pub guide_shape: bool,
    /// With no guide, draw on the plane through the orbit point facing the view (Feather needs a
    /// guide; WobbleWorks lets you sketch straight away).
    pub draw_in_air: bool,
    pub pin_orbit: bool,
    pub show_orbit: bool,
    pub liquify: Liquify,
    /// Eraser and circle-select radius in pixels.
    pub eraser_size: f32,
    pub pivot: Pivot,
    pub gizmo: Option<GizmoKind>,
    /// Gizmo on local axes.
    pub gizmo_local: bool,
    pub keymap: Keymap,
    pub atlas: Atlas,
    pub status: String,
    pub events: Vec<Event>,
    /// Bumped on every visible change (render caches key on it).
    pub revision: u64,
    pub dirty: bool,
    pub mouse: [f32; 2],
    pub time: f64,
    /// Liquify's Compare: show the note as it was when liquify started.
    pub compare: bool,
    undo: Vec<(String, Scene)>,
    redo: Vec<(String, Scene)>,
    live: Option<Live>,
    modal: Option<ModalState>,
    liquify_before: Option<Scene>,
    loft: Option<LoftState>,
    primitive: Option<PrimitiveState>,
    recent_guide: Option<Arc<Guide>>,
    /// Where the last stamp went while dragging the Stamp tool.
    stamp_from: Option<[f32; 2]>,
}

impl Default for Editor {
    fn default() -> Self {
        Editor::new()
    }
}

fn f(params: &Value, key: &str) -> Option<f32> {
    params.get(key).and_then(Value::as_f64).map(|v| v as f32).filter(|v| v.is_finite())
}

fn b(params: &Value, key: &str) -> Option<bool> {
    params.get(key).and_then(Value::as_bool)
}

fn s<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str)
}

fn id(params: &Value, key: &str) -> Option<u64> {
    params.get(key).and_then(Value::as_u64)
}

fn ids(params: &Value, key: &str) -> Result<HashSet<u64>, String> {
    match params.get(key) {
        Some(Value::Array(a)) => a.iter().take(PARAM_POINTS_MAX).map(|v| v.as_u64().ok_or_else(|| format!("{key}: ids are whole numbers"))).collect(),
        Some(_) => Err(format!("{key} must be a list of ids")),
        None => Err(format!("missing {key}")),
    }
}

fn need_f(params: &Value, key: &str) -> Result<f32, String> {
    f(params, key).ok_or_else(|| format!("missing or bad number {key:?}"))
}

/// Screen points: [[x, y], …] or [[x, y, pressure], …].
fn screen_points(params: &Value, key: &str) -> Result<Vec<[f32; 3]>, String> {
    let a = params.get(key).and_then(Value::as_array).ok_or_else(|| format!("{key} must be a list of [x, y] points"))?;
    if a.len() > PARAM_POINTS_MAX {
        return Err(format!("{key}: at most {PARAM_POINTS_MAX} points"));
    }
    a.iter()
        .map(|p| {
            let c = p.as_array().ok_or("each point is [x, y] or [x, y, pressure]")?;
            let g = |i: usize| c.get(i).and_then(Value::as_f64).map(|v| v as f32).filter(|v| v.is_finite());
            match (g(0), g(1)) {
                (Some(x), Some(y)) => Ok([x, y, g(2).unwrap_or(1.0).clamp(0.0, 1.0)]),
                _ => Err("each point is [x, y] or [x, y, pressure] with finite numbers".to_string()),
            }
        })
        .collect()
}

fn vec3(params: &Value, key: &str) -> Option<Vec3> {
    let a = params.get(key)?.as_array()?;
    let v: Vec<f64> = a.iter().filter_map(Value::as_f64).collect();
    Vec3::from_slice(&v)
}

impl Editor {
    pub fn new() -> Editor {
        Editor {
            scene: Scene::default(),
            camera: Camera::default(),
            brush: Brush::default(),
            tool: Tool::Draw,
            select_mode: SelectMode::Brush,
            selection: HashSet::new(),
            selected_resources: HashSet::new(),
            mirror_on: false,
            mirror_axes: [true, false, false],
            stable: 0.3,
            guide_shape: true,
            draw_in_air: true,
            pin_orbit: false,
            show_orbit: true,
            liquify: Liquify::default(),
            eraser_size: 18.0,
            pivot: Pivot::Median,
            gizmo: Some(GizmoKind::Move),
            gizmo_local: false,
            keymap: Keymap::default(),
            atlas: Atlas::default(),
            status: String::new(),
            events: Vec::new(),
            revision: 1,
            dirty: false,
            mouse: [0.0, 0.0],
            time: 0.0,
            compare: false,
            undo: Vec::new(),
            redo: Vec::new(),
            live: None,
            modal: None,
            liquify_before: None,
            loft: None,
            primitive: None,
            recent_guide: None,
            stamp_from: None,
        }
    }

    pub fn view(&self) -> View {
        self.camera.view()
    }

    fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn changed(&mut self) {
        self.touch();
        self.dirty = true;
    }

    /// Remember the note before a change (one undo step).
    pub fn checkpoint(&mut self, label: &str) {
        self.undo.push((label.to_string(), self.scene.clone()));
        if self.undo.len() > UNDO_MAX {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|(l, _)| l.as_str())
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|(l, _)| l.as_str())
    }
    /// The history list, oldest first, plus how many steps are undone (for a History panel).
    pub fn history(&self) -> (Vec<String>, Vec<String>) {
        (self.undo.iter().map(|(l, _)| l.clone()).collect(), self.redo.iter().rev().map(|(l, _)| l.clone()).collect())
    }

    pub fn undo(&mut self) -> bool {
        self.end_sessions(false);
        let Some((label, prev)) = self.undo.pop() else { return false };
        let cur = std::mem::replace(&mut self.scene, prev);
        self.redo.push((label.clone(), cur));
        self.prune_selection();
        self.status = format!("Undo {label}");
        self.events.push(Event::Undo);
        self.changed();
        true
    }

    pub fn redo(&mut self) -> bool {
        self.end_sessions(false);
        let Some((label, next)) = self.redo.pop() else { return false };
        let cur = std::mem::replace(&mut self.scene, next);
        self.undo.push((label.clone(), cur));
        self.prune_selection();
        self.status = format!("Redo {label}");
        self.events.push(Event::Redo);
        self.changed();
        true
    }

    fn prune_selection(&mut self) {
        let alive: HashSet<u64> = self.scene.strokes.iter().map(|s| s.id).collect();
        self.selection.retain(|i| alive.contains(i));
        let res: HashSet<u64> =
            self.scene.guides.iter().map(|g| g.id).chain(self.scene.images.iter().map(|i| i.id)).chain(self.scene.models.iter().map(|m| m.id)).collect();
        self.selected_resources.retain(|i| res.contains(i));
    }

    fn error(&mut self, e: impl Into<String>) -> String {
        let e = e.into();
        self.status = e.clone();
        self.events.push(Event::Error(e.clone()));
        e
    }

    /// Start over with an empty note.
    pub fn new_note(&mut self) {
        self.end_sessions(false);
        self.scene = Scene::default();
        self.camera = Camera { viewport: self.camera.viewport, ..Camera::default() };
        self.selection.clear();
        self.selected_resources.clear();
        self.undo.clear();
        self.redo.clear();
        self.dirty = false;
        self.touch();
    }

    /// Replace the note (after loading a file).
    pub fn open(&mut self, scene: Scene, camera: Camera) {
        self.end_sessions(false);
        self.scene = scene;
        self.scene.repair();
        let vp = self.camera.viewport;
        self.camera = camera;
        self.camera.viewport = vp;
        self.camera.sanitize();
        self.selection.clear();
        self.selected_resources.clear();
        self.undo.clear();
        self.redo.clear();
        self.dirty = false;
        self.touch();
    }

    pub fn set_viewport(&mut self, width: f32, height: f32) {
        let vp = Viewport { width, height }.sane();
        if vp != self.camera.viewport {
            self.camera.viewport = vp;
            self.touch();
        }
    }

    // ---------------------------------------------------------------- tools

    pub fn set_tool(&mut self, tool: Tool) {
        if tool == self.tool {
            return;
        }
        self.end_sessions(true);
        if tool.paints() {
            self.selection.clear();
            self.selected_resources.clear();
        }
        if tool == Tool::Primitive {
            self.tool = tool;
            if let Err(e) = self.begin_primitive(Primitive::Sphere, Primitive::Sphere.default_segments()) {
                self.error(e);
            }
            return;
        }
        if tool == Tool::Loft {
            self.loft = Some(LoftState { curves: Vec::new(), tension: 0.5, guide: None, base: self.scene.clone() });
            self.selection.clear();
        }
        if tool == Tool::Liquify {
            if self.selection.is_empty() {
                self.error("Select curves first, then Liquify");
                return;
            }
            self.liquify_before = Some(self.scene.clone());
        }
        if tool == Tool::Stamp && self.selection.is_empty() {
            self.error("Select curves first, then Stamp");
            return;
        }
        if tool == Tool::Bend && !self.scene.active_guide().is_some_and(|g| g.is_drawn()) {
            self.error("Bend needs an active drawn 3D Guide");
            return;
        }
        self.tool = tool;
        self.status = tool.label().to_string();
        self.touch();
    }

    /// Finish (or cancel) loft, primitive and liquify sessions and any modal transform.
    /// Switching tools confirms them (Feather 1.1.0).
    pub fn end_sessions(&mut self, confirm: bool) {
        if self.modal.is_some() {
            if confirm {
                self.modal_confirm();
            } else {
                self.modal_cancel();
            }
        }
        self.live = None;
        if self.loft.is_some() {
            if confirm {
                let _ = self.loft_done();
            } else {
                self.loft_cancel();
            }
        }
        if self.primitive.is_some() {
            if confirm {
                let _ = self.primitive_done();
            } else {
                self.primitive_cancel();
            }
        }
        if self.liquify_before.is_some() {
            self.liquify_before = None;
            self.compare = false;
            if self.tool == Tool::Liquify {
                self.tool = Tool::Select;
            }
        }
    }

    // ---------------------------------------------------------------- drawing targets

    /// Where a screen point lands: on the active guide, an active image or model, or (when
    /// allowed) the plane through the orbit point facing the view. (point, surface normal)
    pub fn target_hit(&self, view: &View, x: f32, y: f32) -> Option<(Vec3, Vec3)> {
        let (o, d) = view.ray(x, y);
        if let Some(g) = self.scene.active_guide() {
            return g.raycast(o, d).map(|h| (h.point, h.normal));
        }
        if let Some((_, p, n, _)) = ops::raycast_resources(&self.scene, o, d, true) {
            return Some((p, n));
        }
        if self.draw_in_air {
            // The plane through the orbit point facing the view: its normal is kept with the
            // point (flat tape lies on it, and strokes on it layer in drawing order).
            return Some((view.unproject_at(x, y, self.camera.target), view.back));
        }
        None
    }

    fn can_draw(&mut self) -> bool {
        if !self.scene.group_shown(self.scene.active_group) {
            self.error("The active group is hidden: show it to draw into it");
            return false;
        }
        if self.scene.active_guide().is_none() && !self.draw_in_air && !self.scene.images.iter().any(|i| i.state == ResourceState::Active)
            && !self.scene.models.iter().any(|m| m.state == ResourceState::Active)
        {
            self.error("Draw a 3D Guide first (Q), or turn on drawing in the air");
            return false;
        }
        true
    }

    fn new_stroke(&mut self, points: Vec<Point>) -> Stroke {
        let id = self.scene.alloc_id();
        let mut st = Stroke { id, group: self.scene.active_group, points, brush: self.brush, seed: hash(id as u32 ^ 0x5eed_0001) };
        st.sanitize();
        st
    }

    /// Add finished curves (with their mirror copies) as one undo step.
    fn commit_curves(&mut self, curves: Vec<Vec<Point>>, label: &str) -> Vec<u64> {
        let curves: Vec<Vec<Point>> = curves.into_iter().filter(|c| !c.is_empty()).collect();
        if curves.is_empty() {
            return Vec::new();
        }
        self.checkpoint(label);
        let mut out = Vec::new();
        let maps = if self.mirror_on { mirror_maps(self.mirror_axes) } else { Vec::new() };
        for c in curves {
            for m in &maps {
                let copy: Vec<Point> = c.iter().map(|p| Point { p: reflect(p.p, *m), pressure: p.pressure, n: reflect(p.n, *m) }).collect();
                let st = self.new_stroke(copy);
                out.push(st.id);
                self.scene.strokes.push(Arc::new(st));
            }
            let st = self.new_stroke(c);
            out.push(st.id);
            self.scene.strokes.push(Arc::new(st));
        }
        self.events.push(Event::Stroke);
        self.changed();
        out
    }

    /// Curves drawn so far in the live stroke, with mirror previews (render them as extras).
    pub fn preview_strokes(&self) -> Vec<Stroke> {
        let Some(live) = &self.live else { return Vec::new() };
        if live.kind != LiveKind::Draw {
            return Vec::new();
        }
        let mut curves: Vec<&Vec<Point>> = live.pieces.iter().collect();
        curves.push(&live.stroke);
        let maps = if self.mirror_on { mirror_maps(self.mirror_axes) } else { Vec::new() };
        let mut out = Vec::new();
        for (k, c) in curves.into_iter().filter(|c| !c.is_empty()).enumerate() {
            let base = Stroke { id: u64::MAX - k as u64, group: self.scene.active_group, points: c.clone(), brush: self.brush, seed: 0x1234 + k as u32 };
            for (j, m) in maps.iter().enumerate() {
                let pts = c.iter().map(|p| Point { p: reflect(p.p, *m), pressure: p.pressure, n: reflect(p.n, *m) }).collect();
                out.push(Stroke { id: u64::MAX - 100 - (k * 8 + j) as u64, points: pts, ..base.clone() });
            }
            out.push(base);
        }
        out
    }

    /// Re-project the live screen samples (after Draw Shape changes them).
    fn reproject_live(&mut self) {
        let view = self.view();
        let Some(live) = &self.live else { return };
        let pts: Vec<[f32; 3]> = match live.shape {
            Some(sh) => {
                let p = live.raw.last().map_or(1.0, |r| r[2]);
                sh.points().into_iter().map(|q| [q[0], q[1], p]).collect()
            }
            None => live.raw.clone(),
        };
        let mut pieces = Vec::new();
        let mut cur = Vec::new();
        for q in &pts {
            match self.target_hit(&view, q[0], q[1]) {
                Some((p, n)) => cur.push(Point { p, pressure: q[2], n }),
                None => {
                    if !cur.is_empty() {
                        pieces.push(std::mem::take(&mut cur));
                    }
                }
            }
        }
        if let Some(live) = &mut self.live {
            live.pieces = pieces;
            live.stroke = cur;
        }
        self.touch();
    }

    fn add_sample(&mut self, q: [f32; 3]) {
        let view = self.view();
        let hit = self.target_hit(&view, q[0], q[1]);
        let Some(live) = &mut self.live else { return };
        live.raw.push(q);
        if live.kind != LiveKind::Draw {
            return;
        }
        match hit {
            Some((p, n)) => live.stroke.push(Point { p, pressure: q[2], n }),
            None => {
                if !live.stroke.is_empty() {
                    let s = std::mem::take(&mut live.stroke);
                    live.pieces.push(s);
                }
            }
        }
        self.touch();
    }

    // ---------------------------------------------------------------- pointer

    /// The primary button (pen / left mouse) went down.
    pub fn pointer_down(&mut self, x: f32, y: f32, pressure: f32, t: f64, mods: Mods) {
        if !(x.is_finite() && y.is_finite()) {
            return;
        }
        self.mouse = [x, y];
        self.time = t;
        let pressure = if pressure.is_finite() { pressure.clamp(0.0, 1.0) } else { 1.0 };
        if self.modal.is_some() {
            self.modal_confirm();
            return;
        }
        // Gizmo handles first.
        if let Some(h) = self.gizmo_parts().and_then(|parts| transform::pick(&parts, [x, y])) {
            self.start_modal_from_handle(h, [x, y]);
            return;
        }
        let kind = match self.tool {
            Tool::Draw | Tool::DrawShape => {
                if !self.can_draw() {
                    return;
                }
                LiveKind::Draw
            }
            Tool::Guide => LiveKind::Guide,
            Tool::Bend => LiveKind::Bend,
            Tool::Erase | Tool::Vacuum => {
                self.checkpoint(if self.tool == Tool::Erase { "Erase" } else { "Vacuum" });
                LiveKind::Erase
            }
            Tool::Select | Tool::Deselect => LiveKind::Select,
            Tool::Liquify => {
                self.checkpoint("Liquify");
                LiveKind::Liquify
            }
            Tool::Loft => {
                self.loft_click(x, y);
                return;
            }
            Tool::Primitive => {
                // Clicking a resource selects it so G / R / S move the primitive.
                return;
            }
            Tool::Injector | Tool::Eyedropper => {
                self.sample_at(x, y, self.tool == Tool::Injector);
                return;
            }
            Tool::Stamp => {
                self.checkpoint("Stamp");
                if !self.stamp_at(x, y) {
                    self.undo.pop();
                }
                self.stamp_from = Some([x, y]);
                return;
            }
        };
        let stab = Stabilizer::new(if kind == LiveKind::Draw || kind == LiveKind::Guide || kind == LiveKind::Bend { self.stable } else { 0.0 });
        self.live = Some(Live {
            kind,
            raw: Vec::new(),
            stab,
            stroke: Vec::new(),
            pieces: Vec::new(),
            down: [x, y],
            last: [x, y],
            last_move_t: t,
            shape: None,
            adjusting: false,
            mods,
            moved: false,
        });
        match kind {
            LiveKind::Draw | LiveKind::Guide | LiveKind::Bend => {
                let q = self.live.as_mut().and_then(|l| l.stab.push(x, y, pressure));
                if let Some(q) = q {
                    self.add_sample(q);
                }
            }
            LiveKind::Erase => self.erase_at(x, y),
            LiveKind::Select => {
                if self.select_mode == SelectMode::Brush || self.select_mode == SelectMode::Circle {
                    self.select_paint(x, y, mods);
                }
                if let Some(l) = &mut self.live {
                    l.raw.push([x, y, 1.0]);
                }
            }
            LiveKind::Liquify => {}
        }
        self.touch();
    }

    pub fn pointer_move(&mut self, x: f32, y: f32, pressure: f32, t: f64, mods: Mods) {
        if !(x.is_finite() && y.is_finite()) {
            return;
        }
        if let Some(from) = self.stamp_from {
            // Dragging the stamp: a copy every selection-width along the drag.
            let step = self.stamp_spacing();
            if ((x - from[0]).powi(2) + (y - from[1]).powi(2)).sqrt() >= step {
                self.checkpoint("Stamp");
                if !self.stamp_at(x, y) {
                    self.undo.pop();
                }
                self.stamp_from = Some([x, y]);
            }
        }
        let prev = self.mouse;
        self.mouse = [x, y];
        self.time = t;
        let pressure = if pressure.is_finite() { pressure.clamp(0.0, 1.0) } else { 1.0 };
        if let Some(m) = &mut self.modal {
            m.modal.mouse = [x, y];
            m.modal.snap = mods.ctrl;
            m.modal.precise = mods.shift;
            self.apply_modal();
            return;
        }
        let Some(live) = &mut self.live else {
            // Hover: brush circles follow the mouse.
            if matches!(self.tool, Tool::Erase | Tool::Vacuum | Tool::Liquify) || (self.tool == Tool::Select && self.select_mode == SelectMode::Circle) {
                self.touch();
            }
            return;
        };
        let moved = ((x - live.last[0]).powi(2) + (y - live.last[1]).powi(2)).sqrt() > 1.5;
        if moved {
            live.last_move_t = t;
            live.moved = true;
        }
        live.last = [x, y];
        match live.kind {
            LiveKind::Draw | LiveKind::Guide | LiveKind::Bend => {
                if live.adjusting {
                    if let Some(sh) = &mut live.shape {
                        sh.adjust([x, y]);
                    }
                    if live.kind == LiveKind::Draw {
                        self.reproject_live();
                    }
                    self.touch();
                } else if let Some(q) = live.stab.push(x, y, pressure) {
                    self.add_sample(q);
                }
            }
            LiveKind::Erase => self.erase_at(x, y),
            LiveKind::Select => {
                let mode = self.select_mode;
                if let Some(l) = &mut self.live
                    && mode == SelectMode::Lasso
                {
                    l.raw.push([x, y, 1.0]);
                }
                if matches!(mode, SelectMode::Brush | SelectMode::Circle) {
                    self.select_paint(x, y, mods);
                }
                self.touch();
            }
            LiveKind::Liquify => {
                let view = self.view();
                let delta = [x - prev[0], y - prev[1]];
                let mut st = self.liquify;
                st.inverse ^= mods.alt;
                if ops::liquify(&mut self.scene, &view, &self.selection, [x, y], delta, &st) {
                    self.changed();
                }
            }
        }
    }

    /// Call every frame while the pointer is down: Draw Shape's hold-to-adjust.
    pub fn tick(&mut self, t: f64) {
        self.time = t;
        let shape_tool = self.tool == Tool::DrawShape;
        let guide_shape = self.guide_shape;
        let Some(live) = &mut self.live else { return };
        let applies = match live.kind {
            LiveKind::Draw => shape_tool,
            LiveKind::Guide | LiveKind::Bend => shape_tool || guide_shape,
            _ => false,
        };
        if !applies || live.adjusting || t - live.last_move_t < HOLD_SECONDS {
            return;
        }
        let pts: Vec<[f32; 2]> = live.raw.iter().map(|q| [q[0], q[1]]).collect();
        let short = pts.windows(2).map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt()).sum::<f32>() < 6.0;
        live.shape = if short && !live.moved {
            // Press, hold, then drag: a circle sized by the drag.
            Some(Shape::Circle { c: live.down, r: 1.0, start: 0.0 })
        } else {
            recognize(&pts)
        };
        if live.shape.is_some() {
            live.adjusting = true;
            if live.kind == LiveKind::Draw {
                self.reproject_live();
            }
            self.touch();
        }
    }

    pub fn pointer_up(&mut self, x: f32, y: f32, t: f64, mods: Mods) {
        self.time = t;
        self.stamp_from = None;
        let (x, y) = if x.is_finite() && y.is_finite() { (x, y) } else { (self.mouse[0], self.mouse[1]) };
        let Some(mut live) = self.live.take() else { return };
        match live.kind {
            LiveKind::Draw | LiveKind::Guide | LiveKind::Bend => {
                if !live.adjusting {
                    let p = live.raw.last().map_or(1.0, |q| q[2]);
                    let tail = live.stab.finish(x, y, p);
                    self.live = Some(live);
                    for q in tail {
                        self.add_sample(q);
                    }
                    let Some(l) = self.live.take() else { return };
                    live = l;
                    let correct = match live.kind {
                        LiveKind::Draw => self.tool == Tool::DrawShape,
                        _ => self.tool == Tool::DrawShape,
                    };
                    if correct {
                        let pts: Vec<[f32; 2]> = live.raw.iter().map(|q| [q[0], q[1]]).collect();
                        live.shape = recognize(&pts);
                    }
                }
                let screen: Vec<[f32; 3]> = match live.shape {
                    Some(sh) => {
                        let p = live.raw.last().map_or(1.0, |q| q[2]);
                        sh.points().into_iter().map(|q| [q[0], q[1], p]).collect()
                    }
                    None => live.raw.clone(),
                };
                match live.kind {
                    LiveKind::Draw => {
                        if live.shape.is_some() {
                            self.live = Some(live);
                            self.reproject_live();
                            let Some(l) = self.live.take() else { return };
                            live = l;
                        }
                        let mut curves = live.pieces;
                        curves.push(live.stroke);
                        self.commit_curves(curves, "Draw");
                    }
                    LiveKind::Guide => {
                        let pts: Vec<(f32, f32)> = screen.iter().map(|q| (q[0], q[1])).collect();
                        if let Err(e) = self.make_guide(&pts) {
                            self.error(e);
                        }
                    }
                    LiveKind::Bend => {
                        let pts: Vec<(f32, f32)> = screen.iter().map(|q| (q[0], q[1])).collect();
                        if let Err(e) = self.bend_guide(&pts) {
                            self.error(e);
                        }
                    }
                    _ => {}
                }
            }
            LiveKind::Erase => {
                // Drop the undo step if nothing was erased.
                if self.undo.last().is_some_and(|(_, s)| s.strokes == self.scene.strokes) {
                    self.undo.pop();
                }
            }
            LiveKind::Select => self.finish_select(&live, x, y, mods),
            LiveKind::Liquify => {
                if self.undo.last().is_some_and(|(_, s)| s.strokes == self.scene.strokes) {
                    self.undo.pop();
                }
            }
        }
        self.touch();
    }

    /// Right click / Esc: cancel whatever is in progress. Returns whether anything was cancelled.
    pub fn cancel(&mut self) -> bool {
        if self.modal.is_some() {
            self.modal_cancel();
            return true;
        }
        if self.live.take().is_some() {
            self.touch();
            return true;
        }
        if self.loft.is_some() {
            self.loft_cancel();
            return true;
        }
        if self.primitive.is_some() {
            self.primitive_cancel();
            return true;
        }
        false
    }

    pub fn session(&self) -> Session {
        if let Some(p) = &self.primitive {
            return Session::Primitive { kind: p.kind, segments: p.segments };
        }
        if let Some(l) = &self.loft {
            return Session::Loft { curves: l.curves.len(), tension: l.tension };
        }
        if self.liquify_before.is_some() {
            return Session::Liquify;
        }
        Session::None
    }

    /// Undo back to (and including) the step `index` from the oldest (History panel).
    pub fn undo_to(&mut self, index: usize) {
        while self.undo.len() > index {
            if !self.undo() {
                break;
            }
        }
    }

    pub fn is_busy(&self) -> bool {
        self.live.is_some() || self.modal.is_some()
    }

    pub fn in_modal(&self) -> bool {
        self.modal.is_some()
    }

    fn erase_at(&mut self, x: f32, y: f32) {
        let view = self.view();
        let r = self.eraser_size;
        let did = if self.tool == Tool::Vacuum { ops::vacuum(&mut self.scene, &view, [x, y], r) } else { ops::erase_points(&mut self.scene, &view, [x, y], r) };
        if did {
            self.prune_selection();
            self.events.push(Event::Erased);
            self.changed();
        }
    }

    fn select_paint(&mut self, x: f32, y: f32, mods: Mods) {
        let view = self.view();
        let r = if self.select_mode == SelectMode::Circle { self.eraser_size } else { 6.0 };
        let hits = ops::strokes_near(&self.scene, &view, [x, y], r);
        let remove = self.tool == Tool::Deselect || mods.ctrl;
        let before = self.selection.len();
        for h in hits {
            if remove {
                self.selection.remove(&h);
            } else {
                self.selection.insert(h);
            }
        }
        if self.selection.len() != before {
            self.events.push(Event::Selected(self.selection.len()));
            self.touch();
        }
    }

    fn finish_select(&mut self, live: &Live, x: f32, y: f32, mods: Mods) {
        let view = self.view();
        let remove = self.tool == Tool::Deselect || mods.ctrl;
        let extend = mods.shift || live.mods.shift;
        let mut changed = false;
        if !live.moved {
            // A click: pick one thing (Blender), Shift toggles, a click on nothing clears.
            let hit = ops::pick_stroke(&self.scene, &view, [x, y], 6.0, false);
            if !extend && !remove {
                changed = !self.selection.is_empty() || !self.selected_resources.is_empty();
                self.selection.clear();
                self.selected_resources.clear();
            }
            match hit {
                Some(h) => {
                    if remove || (extend && self.selection.contains(&h)) {
                        self.selection.remove(&h);
                    } else {
                        self.selection.insert(h);
                    }
                    changed = true;
                }
                None => {
                    if let Some(r) = ops::pick_resource(&self.scene, &view, [x, y]) {
                        if remove || (extend && self.selected_resources.contains(&r)) {
                            self.selected_resources.remove(&r);
                        } else {
                            self.selected_resources.insert(r);
                        }
                        changed = true;
                    }
                }
            }
        } else if matches!(self.select_mode, SelectMode::Box | SelectMode::Lasso) {
            let poly: Vec<[f32; 2]> = if self.select_mode == SelectMode::Box {
                let (a, c) = (live.down, [x, y]);
                vec![a, [c[0], a[1]], c, [a[0], c[1]]]
            } else {
                live.raw.iter().map(|q| [q[0], q[1]]).collect()
            };
            let hits = ops::strokes_in_polygon(&self.scene, &view, &poly);
            if !extend && !remove {
                self.selection.clear();
            }
            for h in hits {
                if remove {
                    self.selection.remove(&h);
                } else {
                    self.selection.insert(h);
                }
            }
            changed = true;
        }
        if changed {
            self.events.push(Event::Selected(self.selection.len() + self.selected_resources.len()));
        }
    }

    /// Screen distance between stamps while dragging: the selection's size on screen.
    fn stamp_spacing(&self) -> f32 {
        let view = self.view();
        let mut lo = [f32::INFINITY; 2];
        let mut hi = [f32::NEG_INFINITY; 2];
        for st in self.scene.strokes.iter().filter(|s| self.selection.contains(&s.id)) {
            for p in &st.points {
                if let Some(q) = view.project(p.p) {
                    lo = [lo[0].min(q.x), lo[1].min(q.y)];
                    hi = [hi[0].max(q.x), hi[1].max(q.y)];
                }
            }
        }
        let size = (hi[0] - lo[0]).max(hi[1] - lo[1]);
        if size.is_finite() { size.clamp(8.0, 2000.0) } else { 40.0 }
    }

    /// Drop a copy of the selection centred on what's under (x, y). Returns whether it did.
    fn stamp_at(&mut self, x: f32, y: f32) -> bool {
        let view = self.view();
        let (Some(pivot), Some((hit, _))) = (self.pivot_point(), self.target_hit(&view, x, y)) else { return false };
        let d = hit - pivot;
        let sel = self.selection.clone();
        let made = ops::duplicate(&mut self.scene, &sel, move |q| q + d, |n| n);
        if made.is_empty() {
            return false;
        }
        self.events.push(Event::Duplicated);
        self.changed();
        true
    }

    fn sample_at(&mut self, x: f32, y: f32, whole_brush: bool) {
        let view = self.view();
        if let Some(sid) = ops::pick_stroke(&self.scene, &view, [x, y], 8.0, true)
            && let Some(st) = self.scene.stroke(sid)
        {
            if whole_brush {
                self.brush = st.brush;
                self.status = format!("Sampled a {} brush", st.brush.kind.label());
            } else {
                self.brush.color = st.brush.color;
                self.status = format!("Sampled {}", st.brush.color.to_hex());
            }
            self.events.push(Event::Sampled);
            self.touch();
            return;
        }
        if !whole_brush {
            let (o, d) = view.ray(x, y);
            if let Some((_, p, _, rid)) = ops::raycast_resources(&self.scene, o, d, false)
                && let Some(im) = self.scene.images.iter().find(|i| i.id == rid)
                && let Some(c) = image_colour(im, p)
            {
                self.brush.color = c;
                self.status = format!("Sampled {}", c.to_hex());
                self.events.push(Event::Sampled);
                self.touch();
                return;
            }
        }
        self.status = "Nothing to sample there (only the active group is sampled)".into();
    }

    // ---------------------------------------------------------------- guides

    fn make_guide(&mut self, screen: &[(f32, f32)]) -> Result<u64, String> {
        let view = self.view();
        let gid = self.scene.alloc_id();
        let mut g = Guide::drawn(gid, &view, screen, self.camera.target)?;
        if let Some(prev) = self.scene.active_guide() {
            g.opacity = prev.opacity;
        }
        self.checkpoint("Draw 3D Guide");
        self.replace_active_guide();
        self.scene.guides.push(Arc::new(g));
        self.scene.set_guide_state(gid, ResourceState::Active);
        self.events.push(Event::GuideMade);
        self.status = "3D Guide: draw on it (D), bend it (Ctrl+B), close it (Esc)".into();
        if self.tool == Tool::Guide {
            self.tool = Tool::Draw;
        }
        self.changed();
        Ok(gid)
    }

    /// Make room for a new active guide: an unsaved one is removed (and can be recalled), a
    /// saved one goes back to the Resource tab.
    fn replace_active_guide(&mut self) {
        if let Some(old) = self.scene.active_guide().cloned() {
            if old.saved {
                self.scene.set_guide_state(old.id, ResourceState::Hidden);
            } else {
                self.scene.guides.retain(|g| g.id != old.id);
                self.scene.active_guide = None;
                self.recent_guide = Some(old);
            }
        }
    }

    fn bend_guide(&mut self, screen: &[(f32, f32)]) -> Result<(), String> {
        let view = self.view();
        let gid = self.scene.active_guide.ok_or("no active 3D Guide to bend")?;
        let mut g = (**self.scene.guide(gid).ok_or("no active 3D Guide to bend")?).clone();
        g.bend(&view, screen)?;
        self.checkpoint("Bend 3D Guide");
        if let Some(slot) = self.scene.guides.iter_mut().find(|x| x.id == gid) {
            *slot = Arc::new(g);
        }
        self.events.push(Event::GuideMade);
        self.changed();
        Ok(())
    }

    /// Close the active guide (Esc): unsaved guides are removed (recall brings them back).
    pub fn close_guide(&mut self) -> bool {
        if self.cancel() {
            return true;
        }
        let Some(g) = self.scene.active_guide().cloned() else { return false };
        self.checkpoint("Close 3D Guide");
        if g.saved {
            self.scene.set_guide_state(g.id, ResourceState::Hidden);
        } else {
            self.scene.guides.retain(|x| x.id != g.id);
            self.scene.active_guide = None;
            self.recent_guide = Some(g);
        }
        if self.tool == Tool::Bend {
            self.tool = Tool::Draw;
        }
        self.events.push(Event::GuideClosed);
        self.changed();
        true
    }

    fn save_guide(&mut self) -> Result<(), String> {
        let gid = self.scene.active_guide.ok_or("no active 3D Guide to save")?;
        self.checkpoint("Save 3D Guide");
        if let Some(slot) = self.scene.guides.iter_mut().find(|x| x.id == gid) {
            Arc::make_mut(slot).saved = true;
        }
        self.scene.set_guide_state(gid, ResourceState::Hidden);
        self.events.push(Event::GuideSaved);
        self.status = "Saved to Resources as a Surface".into();
        self.changed();
        Ok(())
    }

    fn recall_guide(&mut self) -> Result<(), String> {
        let g = self.recent_guide.take().ok_or("no recently closed guide")?;
        self.checkpoint("Recall 3D Guide");
        self.replace_active_guide();
        let gid = g.id;
        if self.scene.guide(gid).is_none() {
            self.scene.guides.push(g);
        }
        self.scene.set_guide_state(gid, ResourceState::Active);
        self.changed();
        Ok(())
    }

    fn begin_primitive(&mut self, kind: Primitive, segments: u32) -> Result<(), String> {
        let base = self.scene.clone();
        let gid = self.scene.alloc_id();
        let g = Guide::primitive(gid, kind, segments)?;
        self.replace_active_guide();
        self.scene.guides.push(Arc::new(g));
        self.scene.set_guide_state(gid, ResourceState::Active);
        self.selected_resources = [gid].into();
        self.selection.clear();
        self.primitive = Some(PrimitiveState { kind, segments, guide: gid, base });
        self.status = "Primitive: G / S / R to place it, then Done (Enter)".into();
        self.changed();
        Ok(())
    }

    fn primitive_update(&mut self, kind: Option<Primitive>, segments: Option<u32>) -> Result<(), String> {
        let Some(st) = &mut self.primitive else { return Err("no primitive in progress".into()) };
        if let Some(k) = kind {
            st.kind = k;
        }
        if let Some(n) = segments {
            st.segments = n.clamp(crate::guide::SEGMENTS_MIN, crate::guide::SEGMENTS_MAX);
        }
        let (gid, kind, seg) = (st.guide, st.kind, st.segments);
        let slot = self.scene.guides.iter_mut().find(|g| g.id == gid).ok_or("the primitive is gone")?;
        let mut g = Guide::primitive(gid, kind, seg)?;
        g.xform = slot.xform;
        g.opacity = slot.opacity;
        g.rebuild()?;
        *slot = Arc::new(g);
        self.changed();
        Ok(())
    }

    fn primitive_done(&mut self) -> Result<(), String> {
        let st = self.primitive.take().ok_or("no primitive in progress")?;
        let made = std::mem::replace(&mut self.scene, st.base);
        self.checkpoint("Primitive 3D Guide");
        self.scene = made;
        self.events.push(Event::GuideMade);
        if self.tool == Tool::Primitive {
            self.tool = Tool::Draw;
        }
        self.selected_resources.clear();
        self.changed();
        Ok(())
    }

    fn primitive_cancel(&mut self) {
        if let Some(st) = self.primitive.take() {
            self.scene = st.base;
            self.selected_resources.clear();
            if self.tool == Tool::Primitive {
                self.tool = Tool::Draw;
            }
            self.touch();
        }
    }

    fn loft_click(&mut self, x: f32, y: f32) {
        let view = self.view();
        let Some(sid) = ops::pick_stroke(&self.scene, &view, [x, y], 8.0, false) else { return };
        let Some(st) = &mut self.loft else { return };
        if let Some(pos) = st.curves.iter().position(|c| *c == sid) {
            st.curves.remove(pos);
        } else {
            st.curves.push(sid);
        }
        self.selection = st.curves.iter().copied().collect();
        if let Err(e) = self.loft_update() {
            self.status = e;
        }
    }

    fn loft_update(&mut self) -> Result<(), String> {
        let Some(st) = &self.loft else { return Err("no loft in progress".into()) };
        let curves: Vec<Vec<Vec3>> = st.curves.iter().filter_map(|c| st.base.stroke(*c)).map(|s| s.points.iter().map(|p| p.p).collect()).collect();
        let tension = st.tension;
        let old = st.guide;
        // Rebuild from the state before the loft each time.
        if let Some(gid) = old {
            self.scene.guides.retain(|g| g.id != gid);
            if self.scene.active_guide == Some(gid) {
                self.scene.active_guide = None;
            }
        }
        if curves.len() < 2 {
            if let Some(st) = &mut self.loft {
                st.guide = None;
            }
            self.touch();
            return Err("Loft: pick two or more curves in order".into());
        }
        let gid = match old {
            Some(g) => g,
            None => self.scene.alloc_id(),
        };
        let g = Guide::loft(gid, &curves, tension)?;
        self.replace_active_guide();
        self.scene.guides.push(Arc::new(g));
        self.scene.set_guide_state(gid, ResourceState::Active);
        if let Some(st) = &mut self.loft {
            st.guide = Some(gid);
        }
        self.touch();
        Ok(())
    }

    fn loft_done(&mut self) -> Result<(), String> {
        let st = self.loft.take().ok_or("no loft in progress")?;
        self.selection.clear();
        if st.guide.is_none() {
            self.scene = st.base;
            if self.tool == Tool::Loft {
                self.tool = Tool::Draw;
            }
            self.touch();
            return Err("Loft needs two or more curves".into());
        }
        let made = std::mem::replace(&mut self.scene, st.base);
        self.checkpoint("Loft 3D Guide");
        self.scene = made;
        self.events.push(Event::GuideMade);
        if self.tool == Tool::Loft {
            self.tool = Tool::Draw;
        }
        self.changed();
        Ok(())
    }

    fn loft_cancel(&mut self) {
        if let Some(st) = self.loft.take() {
            self.scene = st.base;
            self.selection.clear();
            if self.tool == Tool::Loft {
                self.tool = Tool::Draw;
            }
            self.touch();
        }
    }

    // ---------------------------------------------------------------- transforms

    /// The centre transforms turn about.
    pub fn pivot_point(&self) -> Option<Vec3> {
        let mut centres: Vec<Vec3> = Vec::new();
        let mut lo = v3(f32::INFINITY, f32::INFINITY, f32::INFINITY);
        let mut hi = -lo;
        for st in self.scene.strokes.iter().filter(|s| self.selection.contains(&s.id)) {
            if let Some((a, b2)) = st.bounds() {
                centres.push((a + b2) * 0.5);
                lo = lo.min(a);
                hi = hi.max(b2);
            }
        }
        for g in self.scene.guides.iter().filter(|g| self.selected_resources.contains(&g.id)) {
            if let Some((a, b2)) = g.bounds() {
                centres.push((a + b2) * 0.5);
                lo = lo.min(a);
                hi = hi.max(b2);
            }
        }
        for im in self.scene.images.iter().filter(|i| self.selected_resources.contains(&i.id)) {
            centres.push(im.xform.translation);
            lo = lo.min(im.xform.translation);
            hi = hi.max(im.xform.translation);
        }
        for m in self.scene.models.iter().filter(|m| self.selected_resources.contains(&m.id)) {
            centres.push(m.xform.translation);
            lo = lo.min(m.xform.translation);
            hi = hi.max(m.xform.translation);
        }
        if centres.is_empty() {
            return None;
        }
        Some(match self.pivot {
            Pivot::Median => centres.iter().fold(Vec3::ZERO, |a, c| a + *c) / centres.len() as f32,
            Pivot::Bounds => (lo + hi) * 0.5,
            Pivot::OrbitPoint => self.camera.target,
        })
    }

    /// The local frame: a single selected resource's rotation.
    fn orient(&self) -> Quat {
        if !self.selection.is_empty() || self.selected_resources.len() != 1 {
            return Quat::IDENTITY;
        }
        let Some(id) = self.selected_resources.iter().next().copied() else { return Quat::IDENTITY };
        self.scene
            .guide(id)
            .map(|g| g.xform.rotation)
            .or_else(|| self.scene.images.iter().find(|i| i.id == id).map(|i| i.xform.rotation))
            .or_else(|| self.scene.models.iter().find(|m| m.id == id).map(|m| m.xform.rotation))
            .unwrap_or(Quat::IDENTITY)
    }

    pub fn has_selection(&self) -> bool {
        !self.selection.is_empty() || !self.selected_resources.is_empty()
    }

    /// The gizmo's parts, when it shows (a selection, a select or primitive tool, no modal).
    pub fn gizmo_parts(&self) -> Option<Vec<Part>> {
        let kind = self.gizmo?;
        if self.modal.is_some() || !self.has_selection() || !matches!(self.tool, Tool::Select | Tool::Deselect | Tool::Primitive) {
            return None;
        }
        let pivot = self.pivot_point()?;
        Some(transform::gizmo(&self.view(), pivot, self.orient(), self.gizmo_local, kind, 80.0))
    }

    pub fn start_modal(&mut self, mode: Mode, mouse: [f32; 2]) -> Result<(), String> {
        if self.modal.is_some() {
            return Err("a transform is already running".into());
        }
        let pivot = self.pivot_point().ok_or("select something to transform first")?;
        self.live = None;
        let modal = Modal::new(mode, pivot, self.orient(), mouse);
        self.modal = Some(ModalState { modal, base: self.scene.clone() });
        self.touch();
        Ok(())
    }

    fn start_modal_from_handle(&mut self, h: Handle, mouse: [f32; 2]) {
        let Some(pivot) = self.pivot_point() else { return };
        let modal = h.start(pivot, self.orient(), mouse, self.gizmo_local);
        self.modal = Some(ModalState { modal, base: self.scene.clone() });
        self.touch();
    }

    /// The selection under a delta, built from `base`.
    fn transformed(&self, base: &Scene, d: &Delta) -> Scene {
        let mut scene = base.clone();
        for st in scene.strokes.iter_mut().filter(|s| self.selection.contains(&s.id)) {
            let m = Arc::make_mut(st);
            for p in &mut m.points {
                p.p = d.point(p.p);
                p.n = d.normal(p.n);
            }
        }
        for g in scene.guides.iter_mut().filter(|g| self.selected_resources.contains(&g.id)) {
            let m = Arc::make_mut(g);
            m.xform = d.xform(&m.xform);
            if m.rebuild().is_err() {
                // Keep the old surface (a zero scale cannot build one).
                if let Some(orig) = base.guide(m.id) {
                    *m = (**orig).clone();
                }
            }
        }
        for im in scene.images.iter_mut().filter(|i| self.selected_resources.contains(&i.id)) {
            let m = Arc::make_mut(im);
            m.xform = d.xform(&m.xform);
        }
        for md in scene.models.iter_mut().filter(|m| self.selected_resources.contains(&m.id)) {
            let m = Arc::make_mut(md);
            m.xform = d.xform(&m.xform);
        }
        scene
    }

    fn apply_modal(&mut self) {
        let view = self.view();
        let Some(m) = &self.modal else { return };
        let d = m.modal.delta(&view);
        let scene = self.transformed(&m.base, &d);
        self.scene = scene;
        self.status = m.modal.header(&view);
        self.touch();
    }

    pub fn modal_confirm(&mut self) {
        let Some(m) = self.modal.take() else { return };
        let label = match m.modal.mode {
            Mode::Grab => "Move",
            Mode::Rotate => "Rotate",
            Mode::Scale => "Scale",
        };
        let done = std::mem::replace(&mut self.scene, m.base);
        if done != self.scene {
            self.checkpoint(label);
            self.scene = done;
            self.events.push(Event::Transformed);
            self.changed();
        } else {
            self.scene = done;
            self.touch();
        }
    }

    pub fn modal_cancel(&mut self) {
        if let Some(m) = self.modal.take() {
            self.scene = m.base;
            self.status = "Cancelled".into();
            self.touch();
        }
    }

    /// One exact transform of the selection (agents and the N panel).
    pub fn transform_exact(&mut self, mode: Mode, axis: Option<usize>, plane: bool, amount: f32) -> Result<(), String> {
        if !amount.is_finite() {
            return Err("amount must be a number".into());
        }
        let pivot = self.pivot_point().ok_or("select something to transform first")?;
        let mut m = Modal::new(mode, pivot, self.orient(), [0.0, 0.0]);
        if let Some(a) = axis {
            m.press_axis(a.min(2), plane);
        }
        m.numeric = format!("{amount}");
        let d = m.delta(&self.view());
        let scene = self.transformed(&self.scene, &d);
        self.checkpoint(match mode {
            Mode::Grab => "Move",
            Mode::Rotate => "Rotate",
            Mode::Scale => "Scale",
        });
        self.scene = scene;
        self.events.push(Event::Transformed);
        self.changed();
        Ok(())
    }

    // ---------------------------------------------------------------- overlay and rendering

    pub fn overlay(&self) -> Overlay {
        let mut o = Overlay::default();
        if let Some(m) = &self.modal {
            let view = self.view();
            o.header = Some(m.modal.header(&view));
            let axis = match m.modal.constraint {
                transform::Constraint::Axis { axis, local } => Some((axis, local)),
                _ => None,
            };
            if let Some((axis, local)) = axis {
                let dir = if local { m.modal.orient.rotate([Vec3::X, Vec3::Y, Vec3::Z][axis.min(2)]) } else { [Vec3::X, Vec3::Y, Vec3::Z][axis.min(2)] };
                let far = view.half_height * 40.0;
                let cur = match m.modal.delta(&view) {
                    Delta::Translate(dd) => m.modal.pivot + dd,
                    _ => m.modal.pivot,
                };
                if let (Some(a), Some(c)) = (view.project(cur - dir * far), view.project(cur + dir * far)) {
                    o.axis_line = Some(([a.x, a.y], [c.x, c.y], axis));
                }
            }
            return o;
        }
        o.gizmo = self.gizmo_parts().unwrap_or_default();
        if let Some(live) = &self.live {
            o.adjusting = live.adjusting;
            match live.kind {
                LiveKind::Guide | LiveKind::Bend => {
                    o.polyline = match live.shape {
                        Some(sh) => sh.points(),
                        None => live.raw.iter().map(|q| [q[0], q[1]]).collect(),
                    }
                }
                LiveKind::Select if self.select_mode == SelectMode::Box && live.moved => {
                    o.rect = Some([live.down[0], live.down[1], live.last[0], live.last[1]]);
                }
                LiveKind::Select if self.select_mode == SelectMode::Lasso => {
                    o.polyline = live.raw.iter().map(|q| [q[0], q[1]]).collect();
                }
                _ => {}
            }
        }
        o.circle = match self.tool {
            Tool::Erase | Tool::Vacuum => Some((self.eraser_size, 0.0)),
            Tool::Select | Tool::Deselect if self.select_mode == SelectMode::Circle => Some((self.eraser_size, 0.0)),
            Tool::Liquify => Some((self.liquify.size, self.liquify.size * self.liquify.range)),
            _ => None,
        };
        o
    }

    /// The scene to show (Liquify's Compare shows the note as it was).
    pub fn shown_scene(&self) -> &Scene {
        match (&self.liquify_before, self.compare) {
            (Some(before), true) => before,
            _ => &self.scene,
        }
    }

    /// Render the view at a boil frame.
    pub fn render(&mut self, frame: u32, overlays: bool) -> Frame {
        let extra = self.preview_strokes();
        let empty = HashSet::new();
        let orbit = (self.show_orbit && overlays).then_some(self.camera.target);
        let selected = self.selection.clone();
        let resources = self.selected_resources.clone();
        let scene = match (&self.liquify_before, self.compare) {
            (Some(before), true) => before,
            _ => &self.scene,
        };
        let opts = Options { frame, selected: &selected, selected_resources: &resources, extra: &extra, hide: &empty, guides: overlays, overlays, orbit_point: orbit };
        render::render(scene, &self.camera, &mut self.atlas, &opts)
    }

    // ---------------------------------------------------------------- brush

    /// Change the brush; with a selection, the selected curves change too (one undo step).
    fn set_brush(&mut self, params: &Value) -> Result<(), String> {
        let mut nb = self.brush;
        let mut fields: Vec<&str> = Vec::new();
        if let Some(k) = s(params, "kind") {
            nb.kind = BrushKind::parse(k).ok_or_else(|| format!("unknown brush kind {k:?}"))?;
            if b(params, "keepPaint") != Some(true) && params.get("paint").is_none() {
                nb.paint = nb.kind.default_paint();
                fields.push("paint");
            }
            fields.push("kind");
        }
        if let Some(c) = s(params, "color") {
            nb.color = Rgba::from_hex(c).ok_or_else(|| format!("bad colour {c:?} (use #rrggbb)"))?;
            fields.push("color");
        }
        if let Some(v) = f(params, "size") {
            nb.size_mm = v;
            fields.push("size");
        }
        if let Some(v) = f(params, "opacity") {
            nb.opacity = v;
            fields.push("opacity");
        }
        if let Some(v) = b(params, "pressure") {
            nb.pressure = v;
            fields.push("pressure");
        }
        if let Some(m) = s(params, "material") {
            nb.material = Material::parse(m).ok_or_else(|| format!("unknown material {m:?}"))?;
            fields.push("material");
        }
        if let Some(v) = f(params, "glow") {
            nb.glow = v;
            fields.push("glow");
        }
        if let Some(p) = params.get("pattern") {
            if p.is_null() {
                nb.pattern = None;
            } else {
                let kind = s(p, "kind").and_then(PatternKind::parse).or(nb.pattern.map(|q| q.kind)).ok_or("pattern needs a kind (dot, line, cross, terrazzo, stippled)")?;
                let old = nb.pattern.unwrap_or(Pattern { kind, intensity: 0.5, angle: 0.0, contrast: 0.5 });
                nb.pattern = Some(Pattern {
                    kind,
                    intensity: f(p, "intensity").unwrap_or(old.intensity),
                    angle: f(p, "angle").unwrap_or(old.angle),
                    contrast: f(p, "contrast").unwrap_or(old.contrast),
                });
            }
            fields.push("pattern");
        }
        if let Some(p) = params.get("paint") {
            let pa = &mut nb.paint;
            for (k, slot) in [
                ("roughness", &mut pa.roughness),
                ("bristles", &mut pa.bristles),
                ("dryness", &mut pa.dryness),
                ("grain", &mut pa.grain),
                ("taper", &mut pa.taper),
                ("boil", &mut pa.boil),
                ("scatter", &mut pa.scatter),
                ("dabSize", &mut pa.dab_size),
                ("jitter", &mut pa.jitter),
                ("colorJitter", &mut pa.color_jitter),
            ] {
                if let Some(v) = f(p, k) {
                    *slot = v;
                }
            }
            if let Some(v) = p.get("layers").and_then(Value::as_u64) {
                pa.layers = v.min(4) as u8;
            }
            if let Some(e) = p.get("echo") {
                if e.is_null() {
                    pa.echo = None;
                } else {
                    let old = pa.echo.unwrap_or(crate::model::Echo { color: Rgba::BLACK, offset: [6.0, 5.0], width: 1.15 });
                    let color = match s(e, "color") {
                        Some(c) => Rgba::from_hex(c).ok_or_else(|| format!("bad echo colour {c:?}"))?,
                        None => old.color,
                    };
                    let offset = match e.get("offset").and_then(Value::as_array) {
                        Some(a) => [a.first().and_then(Value::as_f64).unwrap_or(0.0) as f32, a.get(1).and_then(Value::as_f64).unwrap_or(0.0) as f32],
                        None => old.offset,
                    };
                    pa.echo = Some(crate::model::Echo { color, offset, width: f(e, "width").unwrap_or(old.width) });
                }
            }
            fields.push("paint");
        }
        nb.sanitize();
        self.brush = nb;
        let apply = b(params, "applyToSelection").unwrap_or(true);
        if apply && !self.selection.is_empty() && !fields.is_empty() {
            self.checkpoint("Change curves");
            for st in self.scene.strokes.iter_mut().filter(|s| self.selection.contains(&s.id)) {
                let m = Arc::make_mut(st);
                for fld in &fields {
                    match *fld {
                        "kind" => m.brush.kind = nb.kind,
                        "color" => m.brush.color = nb.color,
                        "size" => m.brush.size_mm = nb.size_mm,
                        "opacity" => m.brush.opacity = nb.opacity,
                        "pressure" => m.brush.pressure = nb.pressure,
                        "material" => m.brush.material = nb.material,
                        "glow" => m.brush.glow = nb.glow,
                        "pattern" => m.brush.pattern = nb.pattern,
                        "paint" => m.brush.paint = nb.paint,
                        _ => {}
                    }
                }
            }
            self.changed();
        } else {
            self.touch();
        }
        Ok(())
    }

    // ---------------------------------------------------------------- groups

    fn group_index(&self, gid: u64) -> Result<usize, String> {
        self.scene.groups.iter().position(|g| g.id == gid).ok_or_else(|| format!("no group {gid}"))
    }

    fn new_group(&mut self, name: Option<&str>) -> Result<u64, String> {
        if self.scene.groups.len() >= crate::model::GROUPS_MAX {
            return Err("too many groups".into());
        }
        self.checkpoint("New group");
        let gid = self.scene.alloc_id();
        let at = self.scene.groups.iter().position(|g| g.id == self.scene.active_group).map_or(self.scene.groups.len(), |i| i + 1);
        let name = name.map(str::to_string).unwrap_or_else(|| format!("Group {}", self.scene.groups.len() + 1));
        self.scene.groups.insert(at, Group { id: gid, name, visible: true });
        self.scene.active_group = gid;
        self.changed();
        Ok(gid)
    }

    // ---------------------------------------------------------------- commands

    /// Every command id with a one-line description (for search, docs and agents).
    pub fn commands() -> Vec<(&'static str, &'static str)> {
        COMMANDS.to_vec()
    }

    /// Run a command by id. Never panics; bad params are errors.
    pub fn run(&mut self, cmd: &str, params: &Value) -> Result<Value, String> {
        let r = self.run_inner(cmd, params);
        if let Err(e) = &r {
            self.status = e.clone();
        }
        r
    }

    fn run_inner(&mut self, cmd: &str, p: &Value) -> Result<Value, String> {
        let ok = Ok(Value::Null);
        match cmd {
            // ---- camera
            "camera.view" => {
                let v = s(p, "view").ok_or("missing view")?;
                let pv = if v == "nearest" { self.camera.nearest_perfect_view() } else { PerfectView::parse(v).ok_or_else(|| format!("unknown view {v:?}"))? };
                self.camera.snap(pv);
                self.touch();
                Ok(json!(pv.name()))
            }
            "camera.orbit" => {
                self.camera.orbit(f(p, "dx").unwrap_or(0.0), f(p, "dy").unwrap_or(0.0));
                self.touch();
                ok
            }
            "camera.pan" => {
                self.camera.pan(f(p, "dx").unwrap_or(0.0), f(p, "dy").unwrap_or(0.0));
                self.touch();
                ok
            }
            "camera.zoom" => {
                self.camera.zoom(need_f(p, "factor")?);
                self.touch();
                ok
            }
            "camera.fov" => {
                let mm = match (f(p, "mm"), f(p, "by")) {
                    (Some(mm), _) => mm,
                    (None, Some(by)) => self.camera.focal_mm * (1.0 + by * 0.01),
                    _ => return Err("give mm or by".into()),
                };
                self.camera.set_focal(mm);
                self.touch();
                Ok(json!(self.camera.focal_mm))
            }
            "camera.toggleProjection" => {
                self.camera.toggle_projection();
                self.touch();
                Ok(json!(self.camera.orthographic))
            }
            "camera.reset" => {
                self.camera = Camera { viewport: self.camera.viewport, ..Camera::default() };
                self.touch();
                ok
            }
            "camera.frameAll" => {
                let (lo, hi) = self.scene.bounds().unwrap_or((v3(-1.0, -1.0, -1.0), v3(1.0, 1.0, 1.0)));
                self.camera.frame(lo, hi);
                self.touch();
                ok
            }
            "camera.frameSelected" => {
                let mut bb: Option<(Vec3, Vec3)> = None;
                for st in self.scene.strokes.iter().filter(|s| self.selection.contains(&s.id)) {
                    if let Some((a, c)) = st.bounds() {
                        bb = Some(bb.map_or((a, c), |(l, h)| (l.min(a), h.max(c))));
                    }
                }
                for g in self.scene.guides.iter().filter(|g| self.selected_resources.contains(&g.id)) {
                    if let Some((a, c)) = g.bounds() {
                        bb = Some(bb.map_or((a, c), |(l, h)| (l.min(a), h.max(c))));
                    }
                }
                let (lo, hi) = bb.ok_or("nothing selected")?;
                self.camera.frame(lo, hi);
                self.touch();
                ok
            }
            "camera.setOrbitPoint" => {
                let (x, y) = (need_f(p, "x")?, need_f(p, "y")?);
                let view = self.view();
                let (o, d) = view.ray(x, y);
                let hit = self
                    .scene
                    .active_guide()
                    .and_then(|g| g.raycast(o, d).map(|h| h.point))
                    .or_else(|| ops::pick_stroke(&self.scene, &view, [x, y], 8.0, false).and_then(|sid| self.scene.stroke(sid)).map(|st| {
                        st.points.iter().map(|q| q.p).min_by(|a, c| {
                            let da = view.project(*a).map_or(f32::INFINITY, |s| (s.x - x).powi(2) + (s.y - y).powi(2));
                            let dc = view.project(*c).map_or(f32::INFINITY, |s| (s.x - x).powi(2) + (s.y - y).powi(2));
                            da.total_cmp(&dc)
                        }).unwrap_or(st.centre())
                    }))
                    .or_else(|| crate::math::ray_plane(o, d, Vec3::ZERO, Vec3::Y).map(|t| o + d * t));
                match hit {
                    Some(pt) => {
                        self.camera.set_orbit_point(pt);
                        self.pin_orbit = true;
                    }
                    None => {
                        // Feather: hold on empty space unpins; when already unpinned, resets.
                        if self.pin_orbit {
                            self.pin_orbit = false;
                        } else {
                            self.camera = Camera { viewport: self.camera.viewport, ..Camera::default() };
                        }
                    }
                }
                self.touch();
                Ok(json!({"pinned": self.pin_orbit}))
            }
            "camera.set" => {
                if let Some(v) = f(p, "yaw") {
                    self.camera.yaw = v;
                }
                if let Some(v) = f(p, "pitch") {
                    self.camera.pitch = v;
                }
                if let Some(v) = f(p, "distance") {
                    self.camera.distance = v;
                }
                if let Some(v) = f(p, "focal") {
                    self.camera.focal_mm = v;
                }
                if let Some(v) = b(p, "orthographic") {
                    self.camera.orthographic = v;
                }
                if let Some(v) = vec3(p, "target") {
                    self.camera.target = v;
                }
                self.camera.sanitize();
                self.touch();
                ok
            }
            "camera.viewport" => {
                self.set_viewport(need_f(p, "width")?, need_f(p, "height")?);
                ok
            }
            // ---- tools
            "tool.set" => {
                let name = s(p, "tool").ok_or("missing tool")?;
                let mut t = Tool::parse(name).ok_or_else(|| format!("unknown tool {name:?}"))?;
                if b(p, "cycle") == Some(true) && (t == self.tool || (t == Tool::Draw && self.tool == Tool::DrawShape) || (t == Tool::Erase && self.tool == Tool::Vacuum)) {
                    t = match self.tool {
                        Tool::Draw => Tool::DrawShape,
                        Tool::DrawShape => Tool::Draw,
                        Tool::Erase => Tool::Vacuum,
                        Tool::Vacuum => Tool::Erase,
                        Tool::Select | Tool::Deselect => {
                            self.select_mode = self.select_mode.next();
                            self.status = format!("Select: {}", self.select_mode.name());
                            self.touch();
                            return Ok(json!(self.select_mode.name()));
                        }
                        other => other,
                    };
                }
                if let Some(m) = s(p, "mode") {
                    self.select_mode = SelectMode::parse(m).ok_or_else(|| format!("unknown select mode {m:?}"))?;
                }
                self.set_tool(t);
                if self.tool != t {
                    return Err(self.status.clone());
                }
                Ok(json!(self.tool.name()))
            }
            "tool.toggleMode" => {
                let t = if matches!(self.tool, Tool::Select | Tool::Deselect) { Tool::Draw } else { Tool::Select };
                self.set_tool(t);
                Ok(json!(self.tool.name()))
            }
            // ---- brush
            "brush.set" => {
                self.set_brush(p)?;
                Ok(json!(self.brush))
            }
            "brush.nudge" => {
                if let Some(d) = f(p, "size") {
                    let step = if self.brush.size_mm < 10.0 { 1.0 } else { (self.brush.size_mm * 0.1).round().max(1.0) };
                    let size = self.brush.size_mm + d.signum() * step;
                    self.set_brush(&json!({"size": size}))?;
                }
                if let Some(d) = f(p, "opacity") {
                    let o = ((self.brush.opacity * 10.0).round() + d.signum()) / 10.0;
                    self.set_brush(&json!({"opacity": o}))?;
                }
                Ok(json!(self.brush))
            }
            "brush.cycle" => {
                let i = BrushKind::ALL.iter().position(|k| *k == self.brush.kind).map_or(0, |i| (i + 1) % BrushKind::ALL.len());
                let k = BrushKind::ALL.get(i).copied().unwrap_or(BrushKind::Pen);
                self.set_brush(&json!({"kind": k.name()}))?;
                Ok(json!(k.name()))
            }
            "brush.sample" => {
                let (x, y) = (need_f(p, "x")?, need_f(p, "y")?);
                self.sample_at(x, y, b(p, "whole").unwrap_or(true));
                Ok(json!(self.brush))
            }
            "preset.add" => {
                if self.scene.presets.len() >= crate::model::PRESETS_MAX {
                    return Err("too many presets".into());
                }
                self.checkpoint("Add brush preset");
                let pid = self.scene.alloc_id();
                let name = s(p, "name").map(str::to_string).unwrap_or_else(|| format!("{} {}", self.brush.kind.label(), self.scene.presets.len() + 1));
                self.scene.presets.push(Preset { id: pid, name, brush: self.brush });
                self.changed();
                Ok(json!(pid))
            }
            "preset.load" => {
                let pid = id(p, "id").ok_or("missing id")?;
                let pr = self.scene.presets.iter().find(|x| x.id == pid).ok_or("no such preset")?;
                self.brush = pr.brush;
                self.touch();
                ok
            }
            "preset.delete" => {
                let set = ids(p, "ids")?;
                self.scene.presets.retain(|x| !set.contains(&x.id));
                self.changed();
                ok
            }
            // ---- curves
            "stroke.add" => {
                let a = p.get("points").and_then(Value::as_array).ok_or("points must be a list of [x, y, z] or [x, y, z, pressure]")?;
                if a.len() > PARAM_POINTS_MAX {
                    return Err("too many points".into());
                }
                let mut pts = Vec::with_capacity(a.len());
                for q in a {
                    let c: Vec<f64> = q.as_array().ok_or("each point is [x, y, z]")?.iter().filter_map(Value::as_f64).collect();
                    let pos = Vec3::from_slice(&c).ok_or("each point is [x, y, z] with finite numbers")?;
                    let pr = c.get(3).map_or(1.0, |v| *v as f32);
                    pts.push(Point { p: pos, pressure: if pr.is_finite() { pr.clamp(0.0, 1.0) } else { 1.0 }, n: Vec3::ZERO });
                }
                if pts.is_empty() {
                    return Err("a curve needs at least one point".into());
                }
                if !self.scene.group_shown(self.scene.active_group) {
                    return Err("the active group is hidden".into());
                }
                let made = self.commit_curves(vec![pts], "Draw");
                Ok(json!(made))
            }
            "stroke.draw" => {
                // Drive the current drawing tool with screen points (as a pen would).
                let pts = screen_points(p, "points")?;
                let (Some(first), Some(last)) = (pts.first().copied(), pts.last().copied()) else { return Err("no points".into()) };
                let before = self.scene.strokes.len();
                let t0 = self.time;
                self.pointer_down(first[0], first[1], first[2], t0, Mods::default());
                for (i, q) in pts.iter().enumerate().skip(1) {
                    self.pointer_move(q[0], q[1], q[2], t0 + i as f64 * 0.004, Mods::default());
                }
                self.pointer_up(last[0], last[1], t0 + pts.len() as f64 * 0.004, Mods::default());
                let made: Vec<u64> = self.scene.strokes.iter().skip(before).map(|s| s.id).collect();
                Ok(json!(made))
            }
            // ---- selection
            "select.all" => {
                self.selection = self.scene.strokes.iter().filter(|s| self.scene.group_shown(s.group)).map(|s| s.id).collect();
                self.events.push(Event::Selected(self.selection.len()));
                self.touch();
                Ok(json!(self.selection.len()))
            }
            "select.none" => {
                self.selection.clear();
                self.selected_resources.clear();
                self.touch();
                ok
            }
            "select.invert" => {
                let all: HashSet<u64> = self.scene.strokes.iter().filter(|s| self.scene.group_shown(s.group)).map(|s| s.id).collect();
                self.selection = all.difference(&self.selection).copied().collect();
                self.touch();
                Ok(json!(self.selection.len()))
            }
            "select.set" => {
                let set = ids(p, "ids")?;
                let alive: HashSet<u64> = self.scene.strokes.iter().map(|s| s.id).collect();
                self.selection = set.iter().copied().filter(|i| alive.contains(i)).collect();
                let res: HashSet<u64> = self.scene.guides.iter().map(|g| g.id).chain(self.scene.images.iter().map(|i| i.id)).chain(self.scene.models.iter().map(|m| m.id)).collect();
                self.selected_resources = set.into_iter().filter(|i| res.contains(i)).collect();
                self.touch();
                Ok(json!(self.selection.len() + self.selected_resources.len()))
            }
            "select.at" => {
                let (x, y) = (need_f(p, "x")?, need_f(p, "y")?);
                let mods = Mods { shift: b(p, "extend").unwrap_or(false), ctrl: b(p, "subtract").unwrap_or(false), alt: false };
                let live = Live {
                    kind: LiveKind::Select,
                    raw: vec![[x, y, 1.0]],
                    stab: Stabilizer::default(),
                    stroke: Vec::new(),
                    pieces: Vec::new(),
                    down: [x, y],
                    last: [x, y],
                    last_move_t: 0.0,
                    shape: None,
                    adjusting: false,
                    mods,
                    moved: false,
                };
                self.finish_select(&live, x, y, mods);
                self.touch();
                Ok(json!(self.selection.iter().chain(self.selected_resources.iter()).copied().collect::<Vec<u64>>()))
            }
            "select.box" => {
                let (x0, y0, x1, y1) = (need_f(p, "x0")?, need_f(p, "y0")?, need_f(p, "x1")?, need_f(p, "y1")?);
                let view = self.view();
                let hits = ops::strokes_in_polygon(&self.scene, &view, &[[x0, y0], [x1, y0], [x1, y1], [x0, y1]]);
                if b(p, "extend") != Some(true) {
                    self.selection.clear();
                }
                self.selection.extend(hits);
                self.touch();
                Ok(json!(self.selection.len()))
            }
            "select.group" => {
                let gid = id(p, "id").ok_or("missing id")?;
                self.group_index(gid)?;
                if b(p, "extend") != Some(true) {
                    self.selection.clear();
                }
                self.selection.extend(self.scene.strokes.iter().filter(|s| s.group == gid).map(|s| s.id));
                self.touch();
                Ok(json!(self.selection.len()))
            }
            "select.linkedUnderMouse" => {
                let view = self.view();
                let sid = ops::pick_stroke(&self.scene, &view, self.mouse, 8.0, false).ok_or("no curve under the mouse")?;
                let gid = self.scene.stroke(sid).map(|s| s.group).ok_or("no curve under the mouse")?;
                self.selection.extend(self.scene.strokes.iter().filter(|s| s.group == gid).map(|s| s.id));
                self.touch();
                Ok(json!(self.selection.len()))
            }
            // ---- edit
            "edit.undo" => Ok(json!(self.undo())),
            "edit.redo" => Ok(json!(self.redo())),
            "edit.delete" => {
                if !self.has_selection() {
                    return Err("nothing selected".into());
                }
                self.checkpoint("Delete");
                let sel = std::mem::take(&mut self.selection);
                let n = ops::delete_strokes(&mut self.scene, &sel);
                let res = std::mem::take(&mut self.selected_resources);
                self.scene.guides.retain(|g| !res.contains(&g.id));
                if self.scene.active_guide.is_some_and(|g| res.contains(&g)) {
                    self.scene.active_guide = None;
                }
                self.scene.guide_states.retain(|(g, _)| !res.contains(g));
                self.scene.images.retain(|i| !res.contains(&i.id));
                self.scene.models.retain(|m| !res.contains(&m.id));
                self.events.push(Event::Deleted);
                self.changed();
                Ok(json!(n))
            }
            "edit.duplicate" => {
                if self.selection.is_empty() {
                    return Err("select curves to duplicate".into());
                }
                let mode = s(p, "mode").unwrap_or("inplace");
                self.checkpoint("Duplicate");
                let sel = self.selection.clone();
                let made: Vec<u64> = match mode {
                    "inplace" | "move" => ops::duplicate(&mut self.scene, &sel, |q| q, |n| n),
                    "view" => {
                        let (fp, fnn) = ops::view_mirror(&self.view(), self.camera.target);
                        ops::duplicate(&mut self.scene, &sel, fp, fnn)
                    }
                    "mirror" => {
                        let maps = mirror_maps(self.mirror_axes);
                        if !self.mirror_on || maps.is_empty() {
                            self.undo.pop();
                            return Err("turn Mirror on (Shift+X) and pick its axes first".into());
                        }
                        let mut all = Vec::new();
                        for m in maps {
                            all.extend(ops::duplicate(&mut self.scene, &sel, move |q| reflect(q, m), move |n| reflect(n, m)));
                        }
                        all
                    }
                    other => {
                        self.undo.pop();
                        return Err(format!("unknown duplicate mode {other:?}"));
                    }
                };
                self.selection = made.iter().copied().collect();
                self.status = format!("Duplicated {} curves", made.len());
                self.events.push(Event::Duplicated);
                self.changed();
                if mode == "move" {
                    // Blender's Shift+D: the copies follow the mouse straight away.
                    let mouse = self.mouse;
                    let _ = self.start_modal(Mode::Grab, mouse);
                }
                Ok(json!(made))
            }
            "edit.fill" => {
                if self.selection.is_empty() {
                    return Err("select a closed curve to fill".into());
                }
                let view = self.view();
                let spacing = f(p, "spacing").unwrap_or(self.brush.radius() * 1.4);
                let fill = ops::Fill {
                    spacing,
                    angle: f(p, "angle").unwrap_or(0.0),
                    zigzag: b(p, "zigzag").unwrap_or(false),
                    jitter: f(p, "jitter").unwrap_or(0.3),
                };
                let sel: Vec<Arc<Stroke>> = self.scene.strokes.iter().filter(|s| self.selection.contains(&s.id)).cloned().collect();
                let mut made_curves: Vec<(u64, Vec<Point>)> = Vec::new();
                for st in &sel {
                    let pts: Vec<Vec3> = st.points.iter().map(|q| q.p).collect();
                    for c in ops::fill_curve(&pts, &view, &fill, st.seed)? {
                        made_curves.push((st.group, c));
                    }
                }
                if made_curves.len() > ops::FILL_STROKES_MAX {
                    return Err("that would be too many strokes: make the brush bigger".into());
                }
                self.checkpoint("Fill");
                let mut made = Vec::new();
                for (group, pts) in made_curves {
                    let mut st = self.new_stroke(pts);
                    st.group = group;
                    made.push(st.id);
                    self.scene.strokes.push(Arc::new(st));
                }
                self.selection = made.iter().copied().collect();
                self.events.push(Event::Stroke);
                self.changed();
                Ok(json!(made))
            }
            "edit.flip" => {
                let axis = id(p, "axis").map(|a| a as usize).or_else(|| s(p, "axis").and_then(|a| "xyz".find(a.to_ascii_lowercase().as_str()))).ok_or("axis is x, y or z")?;
                let pivot = self.pivot_point().ok_or("select something to mirror")?;
                let mut fac = v3(1.0, 1.0, 1.0);
                match axis.min(2) {
                    0 => fac.x = -1.0,
                    1 => fac.y = -1.0,
                    _ => fac.z = -1.0,
                }
                let d = Delta::Scale { pivot, orient: Quat::IDENTITY, factors: fac };
                let scene = self.transformed(&self.scene, &d);
                self.checkpoint("Mirror");
                self.scene = scene;
                self.changed();
                ok
            }
            // ---- transforms
            "transform.start" => {
                let mode = s(p, "mode").and_then(Mode::parse).ok_or("mode is grab, rotate or scale")?;
                let mouse = [f(p, "x").unwrap_or(self.mouse[0]), f(p, "y").unwrap_or(self.mouse[1])];
                self.start_modal(mode, mouse)?;
                ok
            }
            "transform.axis" => {
                let axis = id(p, "axis").map(|a| a as usize).or_else(|| s(p, "axis").and_then(|a| "xyz".find(a.to_ascii_lowercase().as_str()))).ok_or("axis is x, y or z")?;
                let plane = b(p, "plane").unwrap_or(false);
                let m = self.modal.as_mut().ok_or("no transform running")?;
                m.modal.press_axis(axis, plane);
                self.apply_modal();
                ok
            }
            "transform.type" => {
                let text = s(p, "text").ok_or("missing text")?;
                let m = self.modal.as_mut().ok_or("no transform running")?;
                for c in text.chars().take(32) {
                    m.modal.type_char(c);
                }
                self.apply_modal();
                ok
            }
            "transform.mouse" => {
                let (x, y) = (need_f(p, "x")?, need_f(p, "y")?);
                let m = self.modal.as_mut().ok_or("no transform running")?;
                m.modal.mouse = [x, y];
                m.modal.snap = b(p, "snap").unwrap_or(false);
                m.modal.precise = b(p, "precise").unwrap_or(false);
                self.apply_modal();
                ok
            }
            "transform.confirm" => {
                if self.modal.is_none() {
                    if self.primitive.is_some() {
                        return self.primitive_done().map(|_| Value::Null);
                    }
                    if self.loft.is_some() {
                        return self.loft_done().map(|_| Value::Null);
                    }
                    return Err("no transform running".into());
                }
                self.modal_confirm();
                ok
            }
            "transform.cancel" => {
                self.modal_cancel();
                ok
            }
            "transform.apply" => {
                let mode = s(p, "mode").and_then(Mode::parse).ok_or("mode is grab, rotate or scale")?;
                let axis = id(p, "axis").map(|a| a as usize).or_else(|| s(p, "axis").and_then(|a| "xyz".find(a.to_ascii_lowercase().as_str())));
                self.transform_exact(mode, axis, b(p, "plane").unwrap_or(false), need_f(p, "amount")?)?;
                ok
            }
            "transform.pivot" => {
                let v = s(p, "pivot").ok_or("missing pivot")?;
                self.pivot = Pivot::parse(v).ok_or_else(|| format!("unknown pivot {v:?}"))?;
                self.touch();
                ok
            }
            "transform.gizmo" => {
                self.gizmo = match s(p, "kind").unwrap_or("move") {
                    "none" | "off" => None,
                    "move" => Some(GizmoKind::Move),
                    "rotate" => Some(GizmoKind::Rotate),
                    "scale" => Some(GizmoKind::Scale),
                    "all" => Some(GizmoKind::All),
                    other => return Err(format!("unknown gizmo {other:?}")),
                };
                if let Some(l) = b(p, "local") {
                    self.gizmo_local = l;
                }
                self.touch();
                ok
            }
            // ---- guides
            "guide.draw" => {
                let pts = screen_points(p, "points")?;
                let pts: Vec<(f32, f32)> = pts.iter().map(|q| (q[0], q[1])).collect();
                let pts = if b(p, "shape") == Some(true) {
                    let raw: Vec<[f32; 2]> = pts.iter().map(|q| [q.0, q.1]).collect();
                    recognize(&raw).map_or(pts, |sh| sh.points().into_iter().map(|q| (q[0], q[1])).collect())
                } else {
                    pts
                };
                let gid = self.make_guide(&pts)?;
                Ok(json!(gid))
            }
            "guide.bend" => {
                let pts = screen_points(p, "points")?;
                let pts: Vec<(f32, f32)> = pts.iter().map(|q| (q[0], q[1])).collect();
                self.bend_guide(&pts)?;
                ok
            }
            "guide.close" => Ok(json!(self.close_guide())),
            "guide.save" => {
                self.save_guide()?;
                ok
            }
            "guide.recall" => {
                self.recall_guide()?;
                ok
            }
            "guide.opacity" => {
                let v = need_f(p, "value")?;
                let gid = match id(p, "id") {
                    Some(g) => g,
                    None => self.scene.active_guide.ok_or("no active guide")?,
                };
                let slot = self.scene.guides.iter_mut().find(|g| g.id == gid).ok_or("no such guide")?;
                Arc::make_mut(slot).set_opacity(v);
                self.changed();
                ok
            }
            "guide.primitive" => {
                let kind = match s(p, "kind") {
                    Some(k) => Primitive::parse(k).ok_or_else(|| format!("unknown primitive {k:?}"))?,
                    None => Primitive::Sphere,
                };
                let seg = p.get("segments").and_then(Value::as_u64).map_or(kind.default_segments(), |v| v.min(1000) as u32);
                if self.primitive.is_some() {
                    self.primitive_update(Some(kind), Some(seg))?;
                } else {
                    self.end_sessions(true);
                    self.tool = Tool::Primitive;
                    self.begin_primitive(kind, seg)?;
                }
                ok
            }
            "guide.segments" => {
                let n = p.get("value").and_then(Value::as_u64).ok_or("missing value")?;
                self.primitive_update(None, Some(n.min(1000) as u32))?;
                ok
            }
            "guide.loft" => {
                let list = ids(p, "ids")?;
                let order: Vec<u64> = p.get("ids").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
                let _ = list;
                self.end_sessions(true);
                self.tool = Tool::Loft;
                self.loft = Some(LoftState { curves: order, tension: f(p, "tension").unwrap_or(0.5).clamp(0.0, 1.0), guide: None, base: self.scene.clone() });
                self.loft_update()?;
                if b(p, "done") != Some(false) {
                    self.loft_done()?;
                }
                ok
            }
            "guide.tension" => {
                let v = need_f(p, "value")?;
                let st = self.loft.as_mut().ok_or("no loft in progress")?;
                st.tension = v.clamp(0.0, 1.0);
                self.loft_update()?;
                ok
            }
            "guide.done" => {
                if self.primitive.is_some() {
                    self.primitive_done()?;
                } else if self.loft.is_some() {
                    self.loft_done()?;
                } else {
                    return Err("nothing to finish".into());
                }
                ok
            }
            "guide.cancel" => {
                if self.primitive.is_some() {
                    self.primitive_cancel();
                } else if self.loft.is_some() {
                    self.loft_cancel();
                } else {
                    return Err("nothing to cancel".into());
                }
                ok
            }
            // ---- erase
            "erase.at" => {
                let (x, y) = (need_f(p, "x")?, need_f(p, "y")?);
                let r = f(p, "radius").unwrap_or(self.eraser_size).clamp(0.5, 2000.0);
                let view = self.view();
                self.checkpoint("Erase");
                let did = if b(p, "vacuum") == Some(true) { ops::vacuum(&mut self.scene, &view, [x, y], r) } else { ops::erase_points(&mut self.scene, &view, [x, y], r) };
                if did {
                    self.prune_selection();
                    self.changed();
                } else {
                    self.undo.pop();
                }
                Ok(json!(did))
            }
            "eraser.size" => {
                self.eraser_size = need_f(p, "value")?.clamp(1.0, 500.0);
                self.touch();
                ok
            }
            // ---- liquify
            "liquify.set" => {
                if let Some(k) = s(p, "kind") {
                    self.liquify.kind = LiquifyKind::parse(k).ok_or_else(|| format!("unknown liquify {k:?}"))?;
                }
                if let Some(v) = f(p, "size") {
                    self.liquify.size = v;
                }
                if let Some(v) = f(p, "range") {
                    self.liquify.range = v;
                }
                if let Some(v) = f(p, "strength") {
                    self.liquify.strength = v;
                }
                if let Some(v) = b(p, "inverse") {
                    self.liquify.inverse = v;
                }
                self.liquify.sanitize();
                self.touch();
                ok
            }
            "liquify.stroke" => {
                let pts = screen_points(p, "points")?;
                if self.selection.is_empty() {
                    return Err("select curves to liquify".into());
                }
                if self.liquify_before.is_none() {
                    self.liquify_before = Some(self.scene.clone());
                }
                self.checkpoint("Liquify");
                let view = self.view();
                let mut any = false;
                for w in pts.windows(2) {
                    any |= ops::liquify(&mut self.scene, &view, &self.selection, [w[1][0], w[1][1]], [w[1][0] - w[0][0], w[1][1] - w[0][1]], &self.liquify);
                }
                if any {
                    self.changed();
                } else {
                    self.undo.pop();
                }
                Ok(json!(any))
            }
            "liquify.undoAll" => {
                let before = self.liquify_before.clone().ok_or("liquify is not running")?;
                self.checkpoint("Liquify: undo all");
                self.scene = before;
                self.changed();
                ok
            }
            "liquify.compare" => {
                self.compare = b(p, "on").unwrap_or(!self.compare) && self.liquify_before.is_some();
                self.touch();
                Ok(json!(self.compare))
            }
            "liquify.apply" => {
                self.liquify_before = None;
                self.compare = false;
                if self.tool == Tool::Liquify {
                    self.tool = Tool::Select;
                }
                self.touch();
                ok
            }
            // ---- groups
            "group.new" => Ok(json!(self.new_group(s(p, "name"))?)),
            "group.activate" => {
                let gid = id(p, "id").ok_or("missing id")?;
                self.group_index(gid)?;
                self.scene.active_group = gid;
                self.touch();
                ok
            }
            "group.rename" => {
                let gid = id(p, "id").unwrap_or(self.scene.active_group);
                let name = s(p, "name").ok_or("missing name")?.chars().take(120).collect::<String>();
                let i = self.group_index(gid)?;
                self.checkpoint("Rename group");
                if let Some(g) = self.scene.groups.get_mut(i) {
                    g.name = name;
                }
                self.changed();
                ok
            }
            "group.visible" => {
                let gid = id(p, "id").unwrap_or(self.scene.active_group);
                let i = self.group_index(gid)?;
                let v = b(p, "visible").unwrap_or_else(|| !self.scene.groups.get(i).is_some_and(|g| g.visible));
                self.checkpoint(if v { "Show group" } else { "Hide group" });
                if let Some(g) = self.scene.groups.get_mut(i) {
                    g.visible = v;
                }
                self.changed();
                Ok(json!(v))
            }
            "group.hideActive" => self.run_inner("group.visible", &json!({"visible": false})),
            "group.showAll" => {
                self.checkpoint("Show all groups");
                for g in &mut self.scene.groups {
                    g.visible = true;
                }
                self.scene.isolated = None;
                self.changed();
                ok
            }
            "group.isolate" => {
                let gid = id(p, "id");
                if let Some(g) = gid {
                    self.group_index(g)?;
                }
                self.scene.isolated = if gid.is_some() && self.scene.isolated == gid { None } else { gid };
                self.changed();
                Ok(json!(self.scene.isolated))
            }
            "group.isolateActive" => {
                let g = self.scene.active_group;
                self.run_inner("group.isolate", &json!({"id": g}))
            }
            "group.delete" => {
                let set = ids(p, "ids")?;
                if set.len() >= self.scene.groups.len() {
                    return Err("a note keeps at least one group".into());
                }
                self.checkpoint("Delete group");
                self.scene.groups.retain(|g| !set.contains(&g.id));
                self.scene.strokes.retain(|s| !set.contains(&s.group));
                self.scene.repair();
                self.prune_selection();
                self.changed();
                ok
            }
            "group.duplicate" => {
                let set = ids(p, "ids")?;
                self.checkpoint("Duplicate group");
                let src: Vec<Group> = self.scene.groups.iter().filter(|g| set.contains(&g.id)).cloned().collect();
                for g in src {
                    let nid = self.scene.alloc_id();
                    self.scene.groups.push(Group { id: nid, name: format!("{} copy", g.name), visible: g.visible });
                    let members: HashSet<u64> = self.scene.strokes.iter().filter(|s| s.group == g.id).map(|s| s.id).collect();
                    let made = ops::duplicate(&mut self.scene, &members, |q| q, |n| n);
                    for st in self.scene.strokes.iter_mut().filter(|s| made.contains(&s.id)) {
                        Arc::make_mut(st).group = nid;
                    }
                }
                self.changed();
                ok
            }
            "group.merge" => {
                let order: Vec<u64> = p.get("ids").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
                let set: HashSet<u64> = order.iter().copied().filter(|g| self.scene.group(*g).is_some()).collect();
                if set.len() < 2 {
                    return Err("merge needs two or more groups".into());
                }
                self.checkpoint("Merge groups");
                let nid = self.scene.alloc_id();
                self.scene.groups.push(Group { id: nid, name: "Merged".into(), visible: true });
                for st in self.scene.strokes.iter_mut().filter(|s| set.contains(&s.group)) {
                    Arc::make_mut(st).group = nid;
                }
                self.scene.groups.retain(|g| !set.contains(&g.id));
                self.scene.active_group = nid;
                self.changed();
                Ok(json!(nid))
            }
            "group.move" => {
                let gid = id(p, "id").ok_or("missing id")?;
                let to = p.get("index").and_then(Value::as_u64).ok_or("missing index")? as usize;
                let i = self.group_index(gid)?;
                self.checkpoint("Reorder groups");
                let g = self.scene.groups.remove(i);
                let to = to.min(self.scene.groups.len());
                self.scene.groups.insert(to, g);
                self.changed();
                ok
            }
            "group.moveStrokes" | "group.fromSelection" => {
                if self.selection.is_empty() {
                    return Err("select curves first".into());
                }
                let gid = if cmd == "group.fromSelection" { self.new_group(s(p, "name"))? } else { id(p, "id").ok_or("missing id")? };
                self.group_index(gid)?;
                if cmd != "group.fromSelection" {
                    self.checkpoint("Move to group");
                }
                for st in self.scene.strokes.iter_mut().filter(|s| self.selection.contains(&s.id)) {
                    Arc::make_mut(st).group = gid;
                }
                self.changed();
                Ok(json!(gid))
            }
            // ---- resources
            "resource.state" => {
                let rid = id(p, "id").ok_or("missing id")?;
                let st = match s(p, "state").ok_or("missing state")? {
                    "active" => ResourceState::Active,
                    "visible" => ResourceState::Visible,
                    "hidden" => ResourceState::Hidden,
                    other => return Err(format!("unknown state {other:?}")),
                };
                self.checkpoint("Resource");
                if self.scene.guide(rid).is_some() {
                    self.scene.set_guide_state(rid, st);
                } else if let Some(im) = self.scene.images.iter_mut().find(|i| i.id == rid) {
                    Arc::make_mut(im).state = st;
                } else if let Some(m) = self.scene.models.iter_mut().find(|m| m.id == rid) {
                    Arc::make_mut(m).state = st;
                } else {
                    self.undo.pop();
                    return Err(format!("no resource {rid}"));
                }
                self.changed();
                ok
            }
            "resource.opacity" => {
                let rid = id(p, "id").ok_or("missing id")?;
                let v = need_f(p, "value")?.clamp(0.0, 1.0);
                if let Some(im) = self.scene.images.iter_mut().find(|i| i.id == rid) {
                    Arc::make_mut(im).opacity = v;
                } else if let Some(g) = self.scene.guides.iter_mut().find(|g| g.id == rid) {
                    Arc::make_mut(g).set_opacity(v);
                } else {
                    return Err(format!("no image or guide {rid}"));
                }
                self.changed();
                ok
            }
            "resource.rename" => {
                let rid = id(p, "id").ok_or("missing id")?;
                let name = s(p, "name").ok_or("missing name")?.chars().take(120).collect::<String>();
                if let Some(g) = self.scene.guides.iter_mut().find(|g| g.id == rid) {
                    Arc::make_mut(g).name = name;
                } else if let Some(im) = self.scene.images.iter_mut().find(|i| i.id == rid) {
                    Arc::make_mut(im).name = name;
                } else if let Some(m) = self.scene.models.iter_mut().find(|m| m.id == rid) {
                    Arc::make_mut(m).name = name;
                } else {
                    return Err(format!("no resource {rid}"));
                }
                self.changed();
                ok
            }
            "resource.delete" => {
                let set = ids(p, "ids")?;
                self.checkpoint("Delete resource");
                self.scene.guides.retain(|g| !set.contains(&g.id));
                if self.scene.active_guide.is_some_and(|g| set.contains(&g)) {
                    self.scene.active_guide = None;
                }
                self.scene.images.retain(|i| !set.contains(&i.id));
                self.scene.models.retain(|m| !set.contains(&m.id));
                self.prune_selection();
                self.changed();
                ok
            }
            "resource.addModel" => {
                let text = s(p, "obj").ok_or("missing obj text")?;
                let name = s(p, "name").unwrap_or("Model").to_string();
                Ok(json!(self.add_model(&name, text)?))
            }
            "resource.addImage" => {
                let w = p.get("width").and_then(Value::as_u64).ok_or("missing width")? as u32;
                let h = p.get("height").and_then(Value::as_u64).ok_or("missing height")? as u32;
                let hex = s(p, "rgbaHex").ok_or("missing rgbaHex (or use the shell's import)")?;
                if hex.len() > 64 << 20 {
                    return Err("image too big".into());
                }
                let bytes: Vec<u8> = (0..hex.len() / 2).filter_map(|i| hex.get(i * 2..i * 2 + 2).and_then(|b2| u8::from_str_radix(b2, 16).ok())).collect();
                Ok(json!(self.add_image(s(p, "name").unwrap_or("Image"), w, h, bytes)?))
            }
            // ---- environment
            "env.set" => {
                let e = &mut self.scene.environment;
                if let Some(v) = b(p, "grid") {
                    e.show_grid = v;
                }
                if let Some(v) = b(p, "axes") {
                    e.show_axes = v;
                }
                if let Some(v) = b(p, "fog") {
                    e.fog = v;
                }
                if let Some(v) = b(p, "render") {
                    e.render_mode = v;
                }
                if let Some(c) = s(p, "background") {
                    e.background = Rgba::from_hex(c).ok_or_else(|| format!("bad colour {c:?}"))?;
                }
                if let Some(l) = p.get("lighting") {
                    let li = &mut e.lighting;
                    if let Some(v) = f(l, "azimuth") {
                        li.azimuth = v;
                    }
                    if let Some(v) = f(l, "altitude") {
                        li.altitude = v;
                    }
                    if let Some(v) = f(l, "strength") {
                        li.strength = v;
                    }
                    if let Some(c) = s(l, "color") {
                        li.color = Rgba::from_hex(c).ok_or_else(|| format!("bad colour {c:?}"))?;
                    }
                    if let Some(v) = b(l, "groundShadow") {
                        li.ground_shadow = v;
                    }
                    if let Some(v) = b(l, "toon") {
                        li.toon = v;
                    }
                }
                if let Some(x) = p.get("effects") {
                    let ef = &mut e.effects;
                    for (k, slot) in [("glow", &mut ef.glow), ("dof", &mut ef.dof), ("grain", &mut ef.grain), ("pixelate", &mut ef.pixelate), ("bloom", &mut ef.bloom)] {
                        if let Some(v) = f(x, k) {
                            *slot = v;
                        }
                    }
                }
                self.scene.repair();
                self.changed();
                Ok(json!(self.scene.environment))
            }
            "env.clearBackgroundImage" => {
                self.set_background_image(None)?;
                ok
            }
            "env.toggleRender" => {
                self.scene.environment.render_mode = !self.scene.environment.render_mode;
                self.changed();
                Ok(json!(self.scene.environment.render_mode))
            }
            "env.lightFromView" => {
                let back = self.view().back;
                let li = &mut self.scene.environment.lighting;
                li.altitude = back.y.clamp(-1.0, 1.0).asin().to_degrees();
                li.azimuth = back.x.atan2(back.z).to_degrees().rem_euclid(360.0);
                self.changed();
                ok
            }
            // ---- boil
            "boil.set" => {
                let bo = &mut self.scene.boil;
                if let Some(v) = b(p, "enabled") {
                    bo.enabled = v;
                }
                if let Some(v) = f(p, "amount") {
                    bo.amount = v;
                }
                if let Some(v) = p.get("frames").and_then(Value::as_u64) {
                    bo.frames = v.min(1000) as u32;
                }
                if let Some(v) = f(p, "fps") {
                    bo.fps = v;
                }
                if let Some(v) = b(p, "world") {
                    bo.world_space = v;
                }
                if let Some(v) = f(p, "wavelength") {
                    bo.wavelength = v;
                }
                if let Some(v) = f(p, "thickness") {
                    bo.thickness = v;
                }
                bo.sanitize();
                self.changed();
                Ok(json!(self.scene.boil))
            }
            "boil.toggle" => {
                self.scene.boil.enabled = !self.scene.boil.enabled;
                self.changed();
                Ok(json!(self.scene.boil.enabled))
            }
            // ---- assists
            "mirror.set" => {
                if let Some(v) = b(p, "on") {
                    self.mirror_on = v;
                }
                for (k, key) in ["x", "y", "z"].iter().enumerate() {
                    if let Some(v) = b(p, key) {
                        self.mirror_axes[k] = v;
                    }
                }
                self.touch();
                ok
            }
            "mirror.toggle" => {
                self.mirror_on = !self.mirror_on;
                self.touch();
                Ok(json!(self.mirror_on))
            }
            "stable.set" => {
                self.stable = need_f(p, "value")?.clamp(0.0, 1.0);
                ok
            }
            "assist.set" => {
                if let Some(v) = b(p, "guideShape") {
                    self.guide_shape = v;
                }
                if let Some(v) = b(p, "drawInAir") {
                    self.draw_in_air = v;
                }
                if let Some(v) = b(p, "showOrbit") {
                    self.show_orbit = v;
                }
                if let Some(v) = b(p, "pinOrbit") {
                    self.pin_orbit = v;
                }
                self.touch();
                ok
            }
            // ---- sequence
            "shot.add" => {
                if self.scene.sequence.shots.len() >= crate::model::SHOTS_MAX {
                    return Err("too many shots".into());
                }
                self.checkpoint("Add shot");
                let sid = self.scene.alloc_id();
                let n = self.scene.sequence.shots.len() + 1;
                let at = match id(p, "after") {
                    Some(a) => self.scene.sequence.shots.iter().position(|x| x.id == a).map_or(self.scene.sequence.shots.len(), |i| i + 1),
                    None => self.scene.sequence.shots.len(),
                };
                self.scene.sequence.shots.insert(at, Shot { id: sid, name: format!("Shot {n}"), camera: self.camera });
                self.changed();
                Ok(json!(sid))
            }
            "shot.go" => {
                let sid = id(p, "id").ok_or("missing id")?;
                let cam = self.scene.sequence.shots.iter().find(|x| x.id == sid).map(|x| x.camera).ok_or("no such shot")?;
                let vp = self.camera.viewport;
                self.camera = cam;
                self.camera.viewport = vp;
                self.touch();
                ok
            }
            "shot.delete" => {
                let set = ids(p, "ids")?;
                self.checkpoint("Delete shot");
                self.scene.sequence.shots.retain(|x| !set.contains(&x.id));
                self.changed();
                ok
            }
            "shot.move" => {
                let sid = id(p, "id").ok_or("missing id")?;
                let to = p.get("index").and_then(Value::as_u64).ok_or("missing index")? as usize;
                let i = self.scene.sequence.shots.iter().position(|x| x.id == sid).ok_or("no such shot")?;
                self.checkpoint("Reorder shots");
                let sh = self.scene.sequence.shots.remove(i);
                let to = to.min(self.scene.sequence.shots.len());
                self.scene.sequence.shots.insert(to, sh);
                self.changed();
                ok
            }
            "sequence.set" => {
                if let Some(v) = f(p, "speed") {
                    self.scene.sequence.speed = v;
                }
                if let Some(m) = s(p, "mode") {
                    self.scene.sequence.mode = match m {
                        "once" => PlayMode::Once,
                        "loop" => PlayMode::Loop,
                        "swing" => PlayMode::Swing,
                        other => return Err(format!("unknown mode {other:?}")),
                    };
                }
                if let Some(v) = f(p, "secondsPerShot") {
                    self.scene.sequence.seconds_per_shot = v;
                }
                self.scene.repair();
                self.changed();
                ok
            }
            // ---- files
            "file.new" => {
                self.new_note();
                ok
            }
            "file.serialize" => {
                let bytes = crate::io::save(&self.scene, &self.camera)?;
                Ok(json!({"hex": bytes.iter().map(|x| format!("{x:02x}")).collect::<String>(), "bytes": bytes.len()}))
            }
            "file.deserialize" => {
                let hex = s(p, "hex").ok_or("missing hex")?;
                if hex.len() > (crate::io::MAX_FILE as usize).saturating_mul(2) {
                    return Err("file too big".into());
                }
                let bytes: Vec<u8> = (0..hex.len() / 2).filter_map(|i| hex.get(i * 2..i * 2 + 2).and_then(|b2| u8::from_str_radix(b2, 16).ok())).collect();
                let (scene, cam) = crate::io::load(&bytes)?;
                self.open(scene, cam);
                ok
            }
            "file.lighten" => {
                let tol = f(p, "tolerance").unwrap_or(0.25);
                let only = (!self.selection.is_empty() && b(p, "all") != Some(true)).then(|| self.selection.clone());
                let before = self.scene.clone();
                let n = ops::lighten(&mut self.scene, only.as_ref(), tol);
                if n == 0 {
                    return Ok(json!(0));
                }
                let after = std::mem::replace(&mut self.scene, before);
                self.checkpoint("Lighten");
                self.scene = after;
                self.status = format!("Lightened: {n} points fewer");
                self.changed();
                Ok(json!(n))
            }
            "file.importNote" => {
                let hex = s(p, "hex").ok_or("missing hex")?;
                let bytes: Vec<u8> = (0..hex.len() / 2).filter_map(|i| hex.get(i * 2..i * 2 + 2).and_then(|b2| u8::from_str_radix(b2, 16).ok())).collect();
                let (other, _) = crate::io::load(&bytes)?;
                Ok(json!(self.import_note(&other, s(p, "name").unwrap_or("Imported"))?))
            }
            "info.groupAt" => {
                let (x, y) = (need_f(p, "x")?, need_f(p, "y")?);
                let view = self.view();
                let name = ops::pick_stroke(&self.scene, &view, [x, y], 8.0, false)
                    .and_then(|sid| self.scene.stroke(sid))
                    .and_then(|st| self.scene.group(st.group))
                    .map(|g| g.name.clone());
                Ok(json!(name))
            }
            "export.obj" => {
                let (obj, mtl) = crate::io::export_obj(&self.scene, s(p, "mtl").unwrap_or("note.mtl"));
                Ok(json!({"obj": obj, "mtl": mtl}))
            }
            "keymap.rebind" => {
                let action = s(p, "action").ok_or("missing action")?;
                let chord = s(p, "chord").unwrap_or("");
                Ok(json!(self.keymap.rebind(action, chord)?))
            }
            "keymap.reset" => {
                self.keymap = Keymap::default();
                ok
            }
            "info" => Ok(self.info()),
            "commands" => Ok(json!(COMMANDS.iter().map(|(c, d)| json!({"id": c, "doc": d})).collect::<Vec<_>>())),
            _ => Err(format!("unknown command {cmd:?}")),
        }
    }

    /// A summary of the editor's state (for agents and tests).
    pub fn info(&self) -> Value {
        json!({
            "tool": self.tool.name(),
            "selectMode": self.select_mode.name(),
            "strokes": self.scene.strokes.len(),
            "groups": self.scene.groups.iter().map(|g| json!({"id": g.id, "name": g.name, "visible": g.visible, "curves": self.scene.strokes.iter().filter(|s| s.group == g.id).count()})).collect::<Vec<_>>(),
            "activeGroup": self.scene.active_group,
            "guides": self.scene.guides.iter().map(|g| json!({"id": g.id, "name": g.name, "state": format!("{:?}", self.scene.guide_state(g.id)), "saved": g.saved, "opacity": g.opacity})).collect::<Vec<_>>(),
            "activeGuide": self.scene.active_guide,
            "images": self.scene.images.len(),
            "models": self.scene.models.len(),
            "selection": self.selection.len(),
            "selectedResources": self.selected_resources.len(),
            "brush": self.brush,
            "camera": self.camera,
            "boil": self.scene.boil,
            "mirror": {"on": self.mirror_on, "axes": self.mirror_axes},
            "undo": self.undo.len(),
            "redo": self.redo.len(),
            "modal": self.modal.is_some(),
            "status": self.status,
        })
    }

    pub fn add_model(&mut self, name: &str, obj: &str) -> Result<u64, String> {
        if self.scene.models.len() >= crate::model::MODELS_MAX {
            return Err("too many models".into());
        }
        let (positions, triangles) = crate::io::parse_obj(obj)?;
        // Centre it on the grid (Feather: models import at the grid centre).
        let (mut lo, mut hi) = (v3(f32::INFINITY, f32::INFINITY, f32::INFINITY), v3(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY));
        for p in &positions {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let centre = (lo + hi) * 0.5;
        self.checkpoint("Import model");
        let mid = self.scene.alloc_id();
        self.scene.models.push(Arc::new(ModelResource {
            id: mid,
            name: name.chars().take(120).collect(),
            positions: positions.into_iter().map(|p| p - centre).collect(),
            triangles,
            xform: Xform::default(),
            color: Rgba::rgb(0xd8, 0xd4, 0xcc),
            state: ResourceState::Visible,
        }));
        self.changed();
        Ok(mid)
    }

    /// Bring another note's curves in as new groups (Feather: importing .feather files into a
    /// note makes them editable groups). Returns the new group ids.
    pub fn import_note(&mut self, other: &Scene, name: &str) -> Result<Vec<u64>, String> {
        if self.scene.groups.len() + other.groups.len() > crate::model::GROUPS_MAX {
            return Err("too many groups".into());
        }
        if self.scene.strokes.len() + other.strokes.len() > crate::model::STROKES_MAX {
            return Err("too many curves".into());
        }
        self.checkpoint("Import note");
        let mut made = Vec::new();
        for g in &other.groups {
            let gid = self.scene.alloc_id();
            self.scene.groups.push(Group { id: gid, name: format!("{name}: {}", g.name).chars().take(120).collect(), visible: g.visible });
            made.push(gid);
            for st in other.strokes.iter().filter(|s| s.group == g.id) {
                let id = self.scene.alloc_id();
                let mut c = (**st).clone();
                c.id = id;
                c.group = gid;
                self.scene.strokes.push(Arc::new(c));
            }
        }
        self.changed();
        Ok(made)
    }

    /// The background image (filling the view behind everything); `None` removes it.
    pub fn set_background_image(&mut self, image: Option<(String, u32, u32, Vec<u8>)>) -> Result<(), String> {
        let im = match image {
            Some((name, width, height, rgba)) => {
                if width == 0 || height == 0 || width > 16384 || height > 16384 || rgba.len() as u64 != width as u64 * height as u64 * 4 {
                    return Err("the image data does not match its size".into());
                }
                let id = self.scene.alloc_id();
                Some(Arc::new(ImageResource { id, name, width, height, rgba: Arc::new(rgba), xform: Xform::default(), opacity: 1.0, state: ResourceState::Visible }))
            }
            None => None,
        };
        self.checkpoint("Background image");
        self.scene.environment.background_image = im;
        self.changed();
        Ok(())
    }

    /// Add a reference image facing the view at the orbit point (Feather: two per note).
    pub fn add_image(&mut self, name: &str, width: u32, height: u32, rgba: Vec<u8>) -> Result<u64, String> {
        if self.scene.images.len() >= crate::model::IMAGES_MAX {
            return Err(format!("a note holds up to {} reference images", crate::model::IMAGES_MAX));
        }
        if width == 0 || height == 0 || width > 16384 || height > 16384 || rgba.len() as u64 != width as u64 * height as u64 * 4 {
            return Err("the image data does not match its size".into());
        }
        let view = self.view();
        let rot = Quat::from_to(Vec3::Z, view.back);
        // Make the image's up match the screen's up.
        let up_now = rot.rotate(Vec3::Y);
        let twist = up_now.cross(view.up).dot(view.back).atan2(up_now.dot(view.up));
        let rotation = Quat::from_axis_angle(view.back, twist) * rot;
        let height_world = view.half_height * 1.2;
        self.checkpoint("Import image");
        let iid = self.scene.alloc_id();
        self.scene.images.push(Arc::new(ImageResource {
            id: iid,
            name: name.chars().take(120).collect(),
            width,
            height,
            rgba: Arc::new(rgba),
            xform: Xform { translation: self.camera.target, rotation, scale: v3(height_world, height_world, height_world) },
            opacity: 0.8,
            state: ResourceState::Visible,
        }));
        self.changed();
        Ok(iid)
    }
}

/// The colour of an image resource at a world point on it.
fn image_colour(im: &ImageResource, p: Vec3) -> Option<Rgba> {
    let c = im.corners();
    let (ex, ey) = (c[1] - c[0], c[3] - c[0]);
    let rel = p - c[0];
    let u = rel.dot(ex) / ex.length_sq().max(1e-12);
    let v = rel.dot(ey) / ey.length_sq().max(1e-12);
    if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
        return None;
    }
    let x = ((u * im.width as f32) as u32).min(im.width.saturating_sub(1));
    let y = (((1.0 - v) * im.height as f32) as u32).min(im.height.saturating_sub(1));
    let i = (y as usize * im.width as usize + x as usize) * 4;
    let px = im.rgba.get(i..i + 4)?;
    Some(Rgba([px[0], px[1], px[2], 255]))
}

const COMMANDS: &[(&str, &str)] = &[
    ("camera.view", "{view: front|back|left|right|top|bottom|nearest} snap to a perfect view (orthographic)"),
    ("camera.orbit", "{dx, dy} orbit by a drag in pixels"),
    ("camera.pan", "{dx, dy} pan by a drag in pixels"),
    ("camera.zoom", "{factor} zoom (>1 in)"),
    ("camera.fov", "{mm} or {by: percent} lens 10–500 mm"),
    ("camera.toggleProjection", "perspective / orthographic"),
    ("camera.reset", "reset the view"),
    ("camera.frameAll", "frame every visible curve"),
    ("camera.frameSelected", "frame the selection"),
    ("camera.setOrbitPoint", "{x, y} pin the orbit point on what is under the screen point (empty space unpins, then resets)"),
    ("camera.set", "{yaw, pitch, distance, focal, orthographic, target: [x,y,z]}"),
    ("camera.viewport", "{width, height} the 3D view's size in pixels"),
    ("tool.set", "{tool: draw|shape|erase|vacuum|select|deselect|guide|bend|loft|primitive|liquify|injector|eyedropper|stamp, mode?: brush|box|circle|lasso, cycle?}"),
    ("tool.toggleMode", "Draw ↔ Select (Tab)"),
    ("brush.set", "{kind, color: #rrggbb, size: mm 1–300, opacity 0–1, pressure, material, glow, pattern: {kind,intensity,angle,contrast}|null, paint: {roughness,bristles,dryness,grain,taper,layers,boil,scatter,dabSize,jitter,colorJitter,echo:{color,offset:[x,y],width}|null}, applyToSelection}"),
    ("brush.nudge", "{size: ±1, opacity: ±1} step the size (10%) or opacity (10%)"),
    ("brush.cycle", "next brush kind"),
    ("brush.sample", "{x, y, whole?} Injector (whole brush) or Eyedropper (colour)"),
    ("preset.add", "{name?} save the brush as a preset"),
    ("preset.load", "{id}"),
    ("preset.delete", "{ids}"),
    ("stroke.add", "{points: [[x,y,z,pressure?]…]} add a curve in world space"),
    ("stroke.draw", "{points: [[x,y,pressure?]…]} draw with the current tool through screen points"),
    ("select.all", "select every visible curve"),
    ("select.none", "clear the selection"),
    ("select.invert", "invert the curve selection"),
    ("select.set", "{ids} select curves and resources by id"),
    ("select.at", "{x, y, extend?, subtract?} click-select"),
    ("select.box", "{x0, y0, x1, y1, extend?}"),
    ("select.group", "{id, extend?} select a group's curves"),
    ("select.linkedUnderMouse", "select the group of the curve under the mouse"),
    ("edit.undo", "undo"),
    ("edit.redo", "redo"),
    ("edit.delete", "delete the selection"),
    ("edit.duplicate", "{mode: inplace|move|view|mirror}"),
    ("edit.flip", "{axis: x|y|z} mirror the selection about the pivot"),
    ("edit.fill", "{spacing?, angle?, zigzag?, jitter?} fill the selected closed curves with strokes in the current brush"),
    ("transform.start", "{mode: grab|rotate|scale, x?, y?} start a Blender-style modal transform"),
    ("transform.axis", "{axis: x|y|z, plane?} constrain (again: local, again: free)"),
    ("transform.type", "{text} type an exact amount"),
    ("transform.mouse", "{x, y, snap?, precise?} move the mouse during a transform"),
    ("transform.confirm", "confirm the transform (or finish a primitive / loft)"),
    ("transform.cancel", "cancel the transform"),
    ("transform.apply", "{mode, axis?, plane?, amount} one exact transform (metres, degrees, factor)"),
    ("transform.pivot", "{pivot: median|bounds|orbit}"),
    ("transform.gizmo", "{kind: none|move|rotate|scale|all, local?}"),
    ("guide.draw", "{points: [[x,y]…], shape?} draw a 3D Guide from screen points"),
    ("guide.bend", "{points} bend the active guide along screen points"),
    ("guide.close", "close the active guide"),
    ("guide.save", "save the active guide to Resources"),
    ("guide.recall", "bring back the last closed guide"),
    ("guide.opacity", "{value 0–0.95, id?}"),
    ("guide.primitive", "{kind: cube|pyramid|sphere|tube|plane, segments?} start (or change) a primitive guide"),
    ("guide.segments", "{value} the primitive's segments"),
    ("guide.loft", "{ids: [curve ids in order], tension?, done?} loft a guide through curves"),
    ("guide.tension", "{value 0–1} loft tension"),
    ("guide.done", "finish the primitive or loft"),
    ("guide.cancel", "cancel the primitive or loft"),
    ("erase.at", "{x, y, radius?, vacuum?} erase under a screen point"),
    ("eraser.size", "{value} eraser / circle-select radius in pixels"),
    ("liquify.set", "{kind: push|pinch|comb, size, range, strength, inverse}"),
    ("liquify.stroke", "{points} liquify the selection along screen points"),
    ("liquify.undoAll", "back to before liquify"),
    ("liquify.compare", "{on?} show the curves as they were"),
    ("liquify.apply", "finish liquify"),
    ("group.new", "{name?} new group above the active one"),
    ("group.activate", "{id}"),
    ("group.rename", "{id?, name}"),
    ("group.visible", "{id?, visible?}"),
    ("group.hideActive", "hide the active group"),
    ("group.showAll", "show every group"),
    ("group.isolate", "{id|null} show only one group"),
    ("group.isolateActive", "show only the active group"),
    ("group.delete", "{ids}"),
    ("group.duplicate", "{ids}"),
    ("group.merge", "{ids} merge into a new group"),
    ("group.move", "{id, index} reorder"),
    ("group.moveStrokes", "{id} move the selected curves into a group"),
    ("group.fromSelection", "{name?} new group from the selected curves"),
    ("resource.state", "{id, state: active|visible|hidden}"),
    ("resource.opacity", "{id, value}"),
    ("resource.rename", "{id, name}"),
    ("resource.delete", "{ids}"),
    ("resource.addModel", "{obj, name?} import an OBJ as a 3D model resource"),
    ("resource.addImage", "{width, height, rgbaHex, name?} add a reference image"),
    ("env.set", "{grid, axes, fog, render, background, lighting: {azimuth, altitude, strength, color, groundShadow, toon}, effects: {glow, dof, grain, pixelate, bloom}}"),
    ("env.toggleRender", "render mode on / off"),
    ("env.clearBackgroundImage", "remove the background image"),
    ("env.lightFromView", "light from the view direction"),
    ("boil.set", "{enabled, amount, frames, fps, world, wavelength, thickness}"),
    ("boil.toggle", "boil on / off"),
    ("mirror.set", "{on, x, y, z}"),
    ("mirror.toggle", "mirror on / off"),
    ("stable.set", "{value 0–1} Stable Stroke"),
    ("assist.set", "{guideShape, drawInAir, showOrbit, pinOrbit}"),
    ("shot.add", "{after?} add a camera shot of the current view"),
    ("shot.go", "{id}"),
    ("shot.delete", "{ids}"),
    ("shot.move", "{id, index}"),
    ("sequence.set", "{speed, mode: once|loop|swing, secondsPerShot}"),
    ("file.new", "start a new note"),
    ("file.serialize", "the note as .wob3d bytes (hex)"),
    ("file.deserialize", "{hex} open .wob3d bytes"),
    ("export.obj", "{mtl?} the curves as OBJ tubes + MTL"),
    ("file.lighten", "{tolerance?, all?} drop redundant curve points (the selection, or everything)"),
    ("file.importNote", "{hex, name?} bring another .wob3d note in as new groups"),
    ("info.groupAt", "{x, y} the group of the curve under a screen point (Feather's Find Group)"),
    ("keymap.rebind", "{action, chord} rebind a shortcut"),
    ("keymap.reset", "Blender defaults"),
    ("info", "the editor's state"),
    ("commands", "this list"),
];

#[cfg(test)]
mod tests;
