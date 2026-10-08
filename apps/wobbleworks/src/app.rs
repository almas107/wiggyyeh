//! The application: state, actions and the frame loop. The canvas view lives in `canvas.rs`, the
//! panels and dialogs in `panels.rs`.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, Key, TextureHandle};

use crate::brush::{self, Progress};
use crate::fill;
use crate::geom::{self, Mirror, Resampler};
use crate::history::History;
use crate::model::{Brush, Doc, Layer, LayerId, MAX_LAYERS, MAX_POINTS, Playback, Pt, Stroke};
use crate::platform::{self, Inbox, Purpose};
use crate::project::{self, PngCache};
use crate::render::Renderer;
use crate::select::Floating;
use crate::settings::{Action, Settings, Tool};
use crate::store::{ProjectMeta, Store};
use crate::theme::{self, Look};

const SETTINGS_KEY: &str = "wobbleworks-settings";
/// Autosave this long after the last change.
const AUTOSAVE_MS: f64 = 1500.0;

/// Zoom and pan.
pub struct View {
    pub zoom: f32,
    /// Screen offset of the canvas's top-left from the view's top-left.
    pub offset: egui::Vec2,
    pub fit_pending: bool,
}

pub const ZMIN: f32 = 0.05;
pub const ZMAX: f32 = 32.0;

/// A stroke being drawn (one per symmetry copy).
pub struct Live {
    pub layer: LayerId,
    pub strokes: Vec<Stroke>,
    progs: Vec<Vec<Progress>>,
    res: Vec<Resampler>,
    smooth: Option<(f64, f64)>,
    last_raw: Pt,
}

/// What a pointer drag on the canvas is doing.
pub enum Gesture {
    None,
    Stroke,
    Shape {
        a: (f64, f64),
        b: (f64, f64),
    },
    Lasso(Vec<(f64, f64)>),
    DragFloat {
        grab: (f64, f64),
    },
    Pick,
    /// Panning since the pointer was at `from` with the view at `offset0`.
    Pan {
        from: egui::Pos2,
        offset0: egui::Vec2,
    },
}

pub struct ProjectState {
    pub id: String,
    pub name: String,
    pub dirty_since: Option<f64>,
    pub saved_at: Option<f64>,
    pub error: Option<String>,
}

pub enum Confirm {
    DeleteProject(String, String),
    DeleteLayer,
}

pub struct App {
    pub doc: Doc,
    pub hist: History,
    pub r: Renderer,
    pub s: Settings,
    pub look: Look,
    applied: Option<(Look, f32, f32)>,
    pub view: View,
    pub frame: usize,
    play_pos: usize,
    pub paused: bool,
    last_tick: f64,
    pub live: Option<Live>,
    pub gesture: Gesture,
    pub floating: Option<Floating>,
    pub status: String,
    pub project: ProjectState,
    pub store: Option<Store>,
    png_cache: PngCache,
    pub inbox: Inbox,
    pub focus: bool,
    pub show_settings: bool,
    pub show_help: bool,
    pub show_new: bool,
    pub confirm: Option<Confirm>,
    pub projects: Vec<ProjectMeta>,
    pub thumbs: HashMap<String, (f64, TextureHandle)>,
    pub rebinding: Option<Action>,
    pub space_down: bool,
    pub settings_tab: u8,
    /// Hex text being typed into the colour field.
    pub hex_edit: Option<String>,
    pub rename: Option<(LayerId, String)>,
    pub pressure: f32,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut s: Settings = cc.storage.and_then(|st| st.get_string(SETTINGS_KEY)).and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
        s.sanitize();
        let mut app = App::with_settings(s, Store::open());
        app.boot();
        app
    }

    pub fn with_settings(s: Settings, store: Option<Store>) -> Self {
        let look = Look { t: s.theme, round: s.roundness, line: s.outline, shadow: s.shadow, boil: None, labels: s.show_labels };
        App {
            doc: Doc::new(s.new_w, s.new_h),
            hist: History::default(),
            r: Renderer::default(),
            look,
            applied: None,
            view: View { zoom: 1.0, offset: egui::Vec2::ZERO, fit_pending: true },
            frame: 0,
            play_pos: 0,
            paused: false,
            last_tick: 0.0,
            live: None,
            gesture: Gesture::None,
            floating: None,
            status: "Pick a brush and scribble!".into(),
            project: ProjectState { id: crate::store::new_id(), name: "Untitled".into(), dirty_since: None, saved_at: None, error: None },
            store,
            png_cache: PngCache::default(),
            inbox: Inbox::default(),
            focus: false,
            show_settings: false,
            show_help: false,
            show_new: false,
            confirm: None,
            projects: Vec::new(),
            thumbs: HashMap::new(),
            rebinding: None,
            space_down: false,
            settings_tab: 0,
            hex_edit: None,
            rename: None,
            pressure: 1.0,
            s,
        }
    }

    /// Recover lost autosaves and reopen the most recent project.
    fn boot(&mut self) {
        let Some(store) = &self.store else {
            self.status = "No storage here: use Export .wob to keep your work.".into();
            return;
        };
        let crashed = store.begin_session();
        let recovered = store.recover();
        self.projects = store.list();
        if let Some(last) = self.projects.first().map(|p| p.id.clone()) {
            self.open_project(&last);
            if crashed {
                self.status = "Last session ended unexpectedly: restored from autosave.".into();
            }
        } else {
            self.save_now();
        }
        if recovered > 0 {
            self.status = format!("{recovered} unlisted autosave(s) recovered.");
        }
    }

    // ------------------------------------------------------------------ status & bookkeeping

    pub fn say(&mut self, s: impl Into<String>) {
        self.status = s.into();
    }

    /// The document changed: schedule an autosave.
    pub fn touched(&mut self) {
        if self.project.dirty_since.is_none() {
            self.project.dirty_since = Some(platform::now_ms());
        }
    }

    /// Layer properties changed (visibility, opacity, order…): recomposite and autosave.
    pub fn restyled(&mut self) {
        self.r.invalidate();
        self.touched();
    }

    // ------------------------------------------------------------------ tools

    pub fn set_tool(&mut self, t: Tool) {
        if !matches!(t, Tool::Lasso | Tool::Move) {
            self.apply_floating();
        }
        self.s.tools.tool = t;
        self.say(t.hint());
    }

    pub fn set_brush(&mut self, b: Brush) {
        if b != Brush::Eraser {
            self.s.tools.last_brush = b;
        }
        self.s.tools.brush = b;
        if !matches!(self.s.tools.tool, Tool::Brush | Tool::Line | Tool::Rect | Tool::Ellipse) {
            self.set_tool(Tool::Brush);
        }
        self.say(format!("{}: {}", b.label(), b.hint()));
    }

    pub fn set_color(&mut self, c: Color32) {
        self.s.tools.color = Color32::from_rgb(c.r(), c.g(), c.b());
        self.hex_edit = None;
    }

    // ------------------------------------------------------------------ live strokes

    pub fn begin_stroke(&mut self, p: (f64, f64)) {
        self.apply_floating();
        let Some(l) = self.doc.layer() else { return };
        if !l.visible {
            self.say("That layer is hidden: turn it on first.");
            return;
        }
        let (id, lock) = (l.id, l.alpha_lock);
        self.r.ensure_caches(&self.doc);
        self.r.keep_copy(id);
        self.r.materialize(&self.doc, id);
        let t = &self.s.tools;
        let n = t.mirror.count();
        let base_seed = (brush::rnd((platform::now_ms() as u64 as u32) ^ (crate::model::next_id() as u32).wrapping_mul(2_246_822_519)) * 1e9) as u32;
        let step = (t.size * 0.5).clamp(3.0, 7.0);
        let strokes: Vec<Stroke> = (0..n)
            .map(|k| Stroke {
                brush: t.brush,
                tip: t.tip,
                color: t.color,
                size: t.size,
                seed: base_seed.wrapping_add(u32::try_from(k).unwrap_or(0).wrapping_mul(7919)),
                lock,
                pts: Vec::new(),
            })
            .collect();
        let frames = self.doc.frames;
        self.live = Some(Live {
            layer: id,
            strokes,
            progs: vec![vec![Progress::default(); frames]; n],
            res: vec![Resampler::new(step); n],
            smooth: None,
            last_raw: Pt::new(p.0, p.1),
        });
        self.gesture = Gesture::Stroke;
        self.feed_stroke(p, false);
    }

    /// Add a pointer sample to the live stroke(s).
    pub fn feed_stroke(&mut self, p: (f64, f64), exact: bool) {
        let (w, h) = (self.doc.w as f64, self.doc.h as f64);
        let pressure = if self.s.tools.pressure { self.pressure } else { 1.0 };
        let stab = f64::from(self.s.tools.stabilizer);
        let mirror = self.s.tools.mirror;
        let wiggle = self.doc.wiggle;
        let Some(live) = &mut self.live else { return };
        let p = (geom::clamp_coord(p.0), geom::clamp_coord(p.1));
        let q = match (live.smooth, stab > 0.0 && !exact) {
            (Some((sx, sy)), true) => {
                let k = 1.0 / (1.0 + stab * 0.7);
                (sx + (p.0 - sx) * k, sy + (p.1 - sy) * k)
            }
            _ => p,
        };
        live.smooth = Some(q);
        let pt = Pt { x: q.0, y: q.1, p: pressure };
        live.last_raw = Pt { x: p.0, y: p.1, p: pressure };
        let images = mirror.images(pt, w, h);
        let mut fresh = Vec::new();
        for (k, img) in images.into_iter().enumerate() {
            let (Some(res), Some(s)) = (live.res.get_mut(k), live.strokes.get_mut(k)) else { continue };
            if s.pts.len() >= MAX_POINTS {
                continue;
            }
            fresh.clear();
            res.push(img, &mut fresh);
            s.pts.extend_from_slice(&fresh);
        }
        let area = self.r.draw_live(live.layer, &live.strokes, &mut live.progs, wiggle, false);
        self.r.invalidate_rect(area);
    }

    pub fn end_stroke(&mut self) {
        // Land exactly where the pen lifted, even with smoothing on.
        if let Some(l) = &self.live {
            let last = l.last_raw;
            self.pressure = last.p;
            self.feed_stroke((last.x, last.y), true);
        }
        let Some(mut live) = self.live.take() else { return };
        for (res, s) in live.res.iter_mut().zip(live.strokes.iter_mut()) {
            let mut tail = Vec::new();
            res.finish(&mut tail);
            s.pts.extend(tail);
        }
        let area = self.r.draw_live(live.layer, &live.strokes, &mut live.progs, self.doc.wiggle, true);
        self.r.invalidate_rect(area);
        self.gesture = Gesture::None;
        self.commit_drawn(live.layer, live.strokes);
    }

    /// The strokes are already in the layer's render cache: record undo and add them to the
    /// layer without re-rendering it.
    fn commit_drawn(&mut self, layer: LayerId, strokes: Vec<Stroke>) {
        let strokes: Vec<Stroke> = strokes.into_iter().filter(|s| !s.pts.is_empty()).collect();
        if strokes.is_empty() {
            return;
        }
        self.hist.push(&self.doc);
        let Some(l) = self.doc.layers.iter_mut().find(|l| l.id == layer) else { return };
        Arc::make_mut(&mut l.strokes).extend(strokes);
        l.touch();
        let ver = l.ver;
        self.r.adopt_version(layer, ver);
        self.touched();
    }

    /// Abandon a live stroke (e.g. a second finger turned it into a pinch).
    pub fn cancel_stroke(&mut self) {
        if let Some(l) = self.live.take() {
            self.r.forget(l.layer);
            self.r.invalidate();
        }
        if matches!(self.gesture, Gesture::Stroke | Gesture::Shape { .. } | Gesture::Lasso(_)) {
            self.gesture = Gesture::None;
        }
    }

    /// Commit a shape outline (line/box/oval) with the current brush and symmetry.
    pub fn commit_shape(&mut self, mut pts: Vec<Pt>) {
        let Some(l) = self.doc.layer() else { return };
        if !l.visible {
            self.say("That layer is hidden: turn it on first.");
            return;
        }
        let (id, lock) = (l.id, l.alpha_lock);
        let t = self.s.tools.clone();
        let step = (t.size * 0.5).clamp(3.0, 7.0);
        if t.brush != Brush::Blob {
            pts = geom::resample(&pts, step);
        }
        let (w, h) = (self.doc.w as f64, self.doc.h as f64);
        let n = t.mirror.count();
        let seed = (brush::rnd(crate::model::next_id() as u32 ^ platform::now_ms() as u64 as u32) * 1e9) as u32;
        let strokes: Vec<Stroke> = (0..n)
            .map(|k| Stroke {
                brush: t.brush,
                tip: t.tip,
                color: t.color,
                size: t.size,
                seed: seed.wrapping_add(u32::try_from(k).unwrap_or(0).wrapping_mul(7919)),
                lock,
                pts: pts.iter().filter_map(|p| t.mirror.images(*p, w, h).get(k).copied()).collect(),
            })
            .collect();
        self.r.ensure_caches(&self.doc);
        self.r.keep_copy(id);
        self.r.materialize(&self.doc, id);
        let mut progs = vec![vec![Progress::default(); self.doc.frames]; n];
        let area = self.r.draw_live(id, &strokes, &mut progs, self.doc.wiggle, true);
        self.r.invalidate_rect(area);
        self.commit_drawn(id, strokes);
    }

    // ------------------------------------------------------------------ fill, pick

    pub fn fill_at(&mut self, p: (f64, f64)) {
        self.apply_floating();
        self.r.sync(&self.doc);
        if !self.doc.layer().is_some_and(|l| l.visible) {
            self.say("That layer is hidden: turn it on first.");
            return;
        }
        if let Some(id) = self.doc.layer().map(|l| l.id) {
            self.r.materialize(&self.doc, id);
            self.r.keep_copy(id);
        }
        self.hist.push(&self.doc);
        let area = fill::fill(&mut self.doc, &self.r, p, self.s.tools.color, self.s.tools.fill);
        self.r.refresh_rect(&self.doc, self.doc.current, area);
        if area.is_empty() {
            self.hist.discard_last();
            self.say("Nothing to fill there.");
        } else {
            self.s.remember_color(self.s.tools.color);
            self.touched();
            self.say("Filled: computed per frame so it boils with the outline.");
        }
    }

    pub fn pick_at(&mut self, p: (f64, f64)) {
        let Some(f) = self.r.frame(self.frame) else { return };
        let c = f.get(p.0.floor() as i32, p.1.floor() as i32);
        if c.a() < 8 {
            self.say("Nothing there to pick.");
            return;
        }
        let [r, g, b, _] = c.to_srgba_unmultiplied();
        self.set_color(Color32::from_rgb(r, g, b));
        self.say(format!("Picked {}", crate::pixels::to_hex(self.s.tools.color)));
    }

    // ------------------------------------------------------------------ selections

    pub fn finish_lasso(&mut self, poly: Vec<(f64, f64)>) {
        if poly.len() < 4 {
            return;
        }
        self.hist.push(&self.doc);
        self.keep_current_copy();
        match Floating::cut(&mut self.doc, poly) {
            Some(f) => {
                self.r.refresh_rect(&self.doc, self.doc.current, f.footprint(self.doc.wiggle));
                self.floating = Some(f);
                self.touched();
                self.say("Grabbed it! Drag to move, or use Transform. Enter applies.");
            }
            None => {
                self.hist.discard_last();
                self.say("Nothing on this layer inside that loop.");
            }
        }
    }

    /// Lift the whole current layer (Move tool with nothing selected).
    pub fn lift_layer(&mut self) -> bool {
        let (w, h) = (self.doc.w as f64, self.doc.h as f64);
        self.hist.push(&self.doc);
        self.keep_current_copy();
        match Floating::cut(&mut self.doc, vec![(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]) {
            Some(f) => {
                self.r.refresh_rect(&self.doc, self.doc.current, f.footprint(self.doc.wiggle));
                self.floating = Some(f);
                self.touched();
                true
            }
            None => {
                self.hist.discard_last();
                self.say("This layer is empty: nothing to move.");
                false
            }
        }
    }

    /// Keep the current layer's render cache so undoing the next change is instant.
    fn keep_current_copy(&mut self) {
        self.r.ensure_caches(&self.doc);
        if let Some(id) = self.doc.layer().map(|l| l.id) {
            self.r.keep_copy(id);
        }
    }

    pub fn apply_floating(&mut self) {
        if let Some(f) = self.floating.take() {
            let area = f.footprint(self.doc.wiggle);
            self.r.ensure_caches(&self.doc);
            if let Some(i) = f.bake(&mut self.doc) {
                self.doc.current = i;
                self.r.refresh_rect(&self.doc, i, area);
            }
            self.touched();
        }
    }

    pub fn delete_floating(&mut self) {
        if self.floating.take().is_some() {
            self.touched();
            self.say("Selection deleted.");
        }
    }

    /// Bake a copy and keep the original floating ("stamp").
    pub fn stamp_floating(&mut self) {
        let Some(f) = &self.floating else { return };
        let copy = f.duplicate();
        let area = copy.footprint(self.doc.wiggle);
        self.hist.push(&self.doc);
        self.r.ensure_caches(&self.doc);
        if let Some(i) = copy.bake(&mut self.doc) {
            self.r.refresh_rect(&self.doc, i, area);
            self.touched();
            self.say("Stamped a copy. Drag the selection to stamp again.");
        }
    }

    // ------------------------------------------------------------------ undo

    pub fn undo(&mut self) {
        self.cancel_stroke();
        self.floating = None;
        if self.hist.undo(&mut self.doc) {
            self.r.invalidate();
            self.touched();
            self.say("Undone.");
        } else {
            self.say("Nothing left to undo.");
        }
    }

    pub fn redo(&mut self) {
        self.cancel_stroke();
        self.floating = None;
        if self.hist.redo(&mut self.doc) {
            self.r.invalidate();
            self.touched();
            self.say("Redone.");
        } else {
            self.say("Nothing to redo.");
        }
    }

    // ------------------------------------------------------------------ layers

    pub fn new_layer(&mut self) {
        if self.doc.layers.len() >= MAX_LAYERS {
            self.say(format!("That's the most layers a drawing can have ({MAX_LAYERS})."));
            return;
        }
        self.apply_floating();
        self.hist.push(&self.doc);
        let l = Layer::new(self.doc.unique_layer_name(), self.doc.frames);
        let at = (self.doc.current + 1).min(self.doc.layers.len());
        self.doc.layers.insert(at, l);
        self.doc.current = at;
        self.restyled();
    }

    pub fn duplicate_layer(&mut self) {
        if self.doc.layers.len() >= MAX_LAYERS {
            self.say(format!("That's the most layers a drawing can have ({MAX_LAYERS})."));
            return;
        }
        self.apply_floating();
        let Some(src) = self.doc.layer() else { return };
        self.hist.push(&self.doc);
        let mut l = src.clone();
        l.id = crate::model::next_id();
        l.name = format!("{} copy", src.name);
        // Fresh seeds so the copy boils on its own.
        Arc::make_mut(&mut l.strokes).iter_mut().for_each(|s| s.seed = s.seed.wrapping_mul(2_654_435_761).wrapping_add(1));
        l.touch();
        let at = self.doc.current + 1;
        self.doc.layers.insert(at, l);
        self.doc.current = at;
        self.restyled();
    }

    pub fn clear_layer(&mut self) {
        self.floating = None;
        let frames = self.doc.frames;
        let Some(l) = self.doc.layer() else { return };
        if l.is_empty() {
            self.say("This layer is already empty.");
            return;
        }
        self.hist.push(&self.doc);
        if let Some(l) = self.doc.layer_mut() {
            l.strokes = Arc::new(Vec::new());
            l.raster = vec![None; frames];
            l.touch();
        }
        self.touched();
        self.say("Layer cleared.");
    }

    pub fn delete_layer(&mut self) {
        if self.doc.layers.len() <= 1 {
            self.say("A drawing needs at least one layer.");
            return;
        }
        self.floating = None;
        self.hist.push(&self.doc);
        self.doc.layers.remove(self.doc.current);
        self.doc.current = self.doc.current.min(self.doc.layers.len().saturating_sub(1));
        self.restyled();
    }

    /// Flatten the current layer onto the one below it (frame by frame, as rendered).
    pub fn merge_down(&mut self) {
        let i = self.doc.current;
        if i == 0 {
            self.say("There's no layer below to merge into.");
            return;
        }
        self.apply_floating();
        self.r.ensure_caches(&self.doc);
        for id in [self.doc.layers.get(i).map(|l| l.id), self.doc.layers.get(i - 1).map(|l| l.id)].into_iter().flatten() {
            self.r.materialize(&self.doc, id);
        }
        let (Some(top), Some(bottom)) = (self.doc.layers.get(i), self.doc.layers.get(i - 1)) else { return };
        let (Some(tc), Some(bc)) = (self.r.cache(top.id), self.r.cache(bottom.id)) else { return };
        let (op, mode, visible, clip) = ((top.opacity.clamp(0.0, 1.0) * 255.0).round() as u32, top.blend, top.visible, top.clip);
        let merged: Vec<Option<Arc<crate::pixels::Pixmap>>> = bc
            .frames
            .iter()
            .zip(&tc.frames)
            .map(|(b, t)| {
                let mut out = b.clone();
                if visible {
                    for (k, (d, s)) in out.px.iter_mut().zip(&t.px).enumerate() {
                        if clip && !bc.frames.iter().any(|f| f.px.get(k).is_some_and(|c| c.a() > 0)) {
                            continue;
                        }
                        *d = crate::pixels::blend(*d, *s, op, mode);
                    }
                }
                (!out.is_blank()).then(|| Arc::new(out))
            })
            .collect();
        self.hist.push(&self.doc);
        if let Some(b) = self.doc.layers.get_mut(i - 1) {
            b.raster = merged;
            b.strokes = Arc::new(Vec::new());
            b.touch();
        }
        self.doc.layers.remove(i);
        self.doc.current = i - 1;
        self.restyled();
        self.say("Merged down (the merged layer keeps boiling, frame by frame).");
    }

    pub fn move_layer(&mut self, from: usize, to: usize) {
        if from == to || from >= self.doc.layers.len() || to >= self.doc.layers.len() {
            return;
        }
        self.hist.push(&self.doc);
        let l = self.doc.layers.remove(from);
        self.doc.layers.insert(to, l);
        let c = self.doc.current;
        self.doc.current = if c == from {
            to
        } else if from < c && to >= c {
            c - 1
        } else if from > c && to <= c {
            c + 1
        } else {
            c
        };
        self.restyled();
    }

    pub fn select_layer(&mut self, i: usize) {
        if i < self.doc.layers.len() && i != self.doc.current {
            self.apply_floating();
            self.doc.current = i;
        }
    }

    // ------------------------------------------------------------------ canvas

    pub fn resize_canvas(&mut self, w: usize, h: usize) {
        self.apply_floating();
        if (w, h) == (self.doc.w, self.doc.h) {
            return;
        }
        self.hist.push(&self.doc);
        self.doc.resize(w, h);
        self.view.fit_pending = true;
        self.restyled();
        self.say(format!("Canvas is now {} × {}.", self.doc.w, self.doc.h));
    }

    pub fn set_frames(&mut self, n: usize) {
        if n == self.doc.frames {
            return;
        }
        self.apply_floating();
        self.hist.push(&self.doc);
        self.doc.set_frames(n);
        self.frame = self.frame.min(self.doc.frames - 1);
        self.play_pos = 0;
        self.restyled();
    }

    // ------------------------------------------------------------------ animation

    /// Advance the boil loop when its time comes. Returns seconds until the next tick.
    pub fn tick(&mut self, now: f64) -> Option<f64> {
        if self.paused {
            return None;
        }
        let period = f64::from(self.doc.speed_ms) / 1000.0;
        if now - self.last_tick >= period {
            self.last_tick = if now - self.last_tick > period * 3.0 { now } else { self.last_tick + period };
            self.step_frame();
        }
        Some((self.last_tick + period - now).max(0.001))
    }

    pub fn step_frame(&mut self) {
        let n = self.doc.frames.max(1);
        self.frame = match self.doc.playback {
            Playback::Loop => (self.frame + 1) % n,
            Playback::PingPong => {
                let order = project::play_order(n, Playback::PingPong);
                self.play_pos = (self.play_pos + 1) % order.len().max(1);
                order.get(self.play_pos).copied().unwrap_or(0)
            }
            Playback::Random => {
                let r = brush::rnd(crate::model::next_id() as u32);
                let k = 1 + (r * (n.saturating_sub(1)) as f64) as usize;
                (self.frame + k.min(n - 1).max(1)) % n
            }
        };
        let boil = u8::try_from(self.frame % 3).unwrap_or(0);
        self.look.boil = (self.s.wobbly_ui && !self.s.reduce_motion).then_some(boil);
    }

    // ------------------------------------------------------------------ files

    fn export_frames(&mut self) -> Vec<crate::pixels::Pixmap> {
        self.apply_floating();
        self.r.sync(&self.doc);
        let k = usize::from(self.s.export_scale.max(1));
        self.r.out.iter().map(|p| if k == 1 { p.clone() } else { p.scaled_nearest(p.w * k, p.h * k) }).collect()
    }

    fn file_stem(&self) -> String {
        let s: String = self.project.name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
        if s.is_empty() { "wobble".into() } else { s }
    }

    fn save_bytes(&mut self, name: &str, bytes: Result<Vec<u8>, String>) {
        match bytes.and_then(|b| platform::save_file(name, &b)) {
            Ok(Some(path)) => self.say(format!("Saved {path}")),
            Ok(None) => {}
            Err(e) => self.say(e),
        }
    }

    pub fn export_png(&mut self) {
        let frames = self.export_frames();
        let f = frames.get(self.frame).or(frames.first()).cloned();
        let name = format!("{}.png", self.file_stem());
        self.save_bytes(&name, f.ok_or_else(|| "nothing to export".to_string()).and_then(|p| project::png_bytes(&p)));
    }

    pub fn export_gif(&mut self) {
        let frames = self.export_frames();
        let order = project::play_order(frames.len(), self.doc.playback);
        let seq: Vec<_> = order.iter().filter_map(|i| frames.get(*i).cloned()).collect();
        let name = format!("{}.gif", self.file_stem());
        let speed = self.doc.speed_ms;
        self.save_bytes(&name, project::gif_bytes(&seq, speed));
    }

    pub fn export_sheet(&mut self) {
        let frames = self.export_frames();
        let name = format!("{}-sheet.png", self.file_stem());
        self.save_bytes(
            &name,
            project::sprite_sheet(&frames).ok_or_else(|| "the sprite sheet would be too wide".to_string()).and_then(|p| project::png_bytes(&p)),
        );
    }

    fn project_json(&mut self) -> Result<String, String> {
        let f = project::to_file(&self.doc, &self.project.name, Some(&self.project.id), platform::now_ms(), self.view.zoom, &mut self.png_cache);
        self.png_cache.retain(&self.doc);
        project::to_json(&f)
    }

    pub fn export_wob(&mut self) {
        self.apply_floating();
        let name = format!("{}.wob", self.file_stem());
        let json = self.project_json().map(String::into_bytes);
        self.save_bytes(&name, json);
    }

    pub fn import(&mut self) {
        platform::pick_file(&self.inbox.clone(), &self.ctx_or_default(), Purpose::Image);
    }

    pub fn open_wob(&mut self) {
        platform::pick_file(&self.inbox.clone(), &self.ctx_or_default(), Purpose::Project);
    }

    fn ctx_or_default(&self) -> egui::Context {
        CTX.with(|c| c.borrow().clone()).unwrap_or_default()
    }

    /// Handle files that finished loading (picker or drag and drop).
    pub fn receive_files(&mut self) {
        for item in self.inbox.take() {
            let f = match item {
                Ok(f) => f,
                Err(e) => {
                    self.say(e);
                    continue;
                }
            };
            let purpose = if f.purpose == Purpose::Any { platform::classify(&f.name, &f.bytes) } else { f.purpose };
            match purpose {
                Purpose::Project | Purpose::Any => self.load_wob_bytes(&f.name, &f.bytes),
                Purpose::Image => match project::decode_image(&f.bytes) {
                    Ok(img) => {
                        self.apply_floating();
                        if let Some(fl) = Floating::from_image(&self.doc, &img) {
                            self.hist.push(&self.doc);
                            self.floating = Some(fl);
                            self.s.tools.tool = Tool::Move;
                            let layer = self.doc.layer().map(|l| l.name.clone()).unwrap_or_default();
                            self.say(format!("Picture floating. Drag to place, scale or turn it, then Apply to bake it into \"{layer}\"."));
                        }
                    }
                    Err(e) => self.say(e),
                },
            }
        }
    }

    fn load_wob_bytes(&mut self, name: &str, bytes: &[u8]) {
        let text = match std::str::from_utf8(bytes) {
            Ok(t) => t,
            Err(_) => {
                self.say(format!("{name} isn't a WobbleWorks project."));
                return;
            }
        };
        match project::from_json(text) {
            Ok(l) => {
                self.save_if_dirty();
                self.install(l, None);
                // A file opened from disk becomes a new project; never clobber an existing one.
                self.project.id = crate::store::new_id();
                self.save_now();
                self.say(format!("Opened \"{}\" from file.", self.project.name));
            }
            Err(e) => self.say(format!("{name} would not open: {e}")),
        }
    }

    fn install(&mut self, l: project::Loaded, id: Option<String>) {
        self.cancel_stroke();
        self.floating = None;
        self.doc = l.doc;
        self.project.name = l.name;
        if let Some(id) = id {
            self.project.id = id;
        }
        self.project.dirty_since = None;
        self.project.error = None;
        self.hist.clear();
        self.r = Renderer::default();
        self.png_cache = PngCache::default();
        self.frame = 0;
        self.play_pos = 0;
        match l.zoom {
            Some(z) => {
                self.view.zoom = z;
                self.view.fit_pending = true;
            }
            None => self.view.fit_pending = true,
        }
        if let Some(w) = l.warnings.first() {
            self.say(format!("Opened with problems: {w}"));
        }
    }

    // ------------------------------------------------------------------ projects

    pub fn save_if_dirty(&mut self) {
        if self.project.dirty_since.is_some() {
            self.save_now();
        }
    }

    pub fn save_now(&mut self) {
        if self.store.is_none() {
            self.project.error = Some("no storage here: use Export .wob".into());
            return;
        }
        if self.live.is_some() {
            return;
        }
        let json = match self.project_json() {
            Ok(j) => j,
            Err(e) => {
                self.project.error = Some(e);
                return;
            }
        };
        let meta = ProjectMeta {
            id: self.project.id.clone(),
            name: self.project.name.clone(),
            modified: platform::now_ms(),
            w: self.doc.w,
            h: self.doc.h,
            good: String::new(),
            thumb: self.thumbnail(),
        };
        let Some(store) = &self.store else { return };
        match store.save(meta, &json) {
            Ok(()) => {
                self.project.dirty_since = None;
                self.project.saved_at = Some(platform::now_ms());
                self.project.error = None;
                self.projects = store.list();
            }
            Err(e) => {
                self.project.error = Some(e.clone());
                // Don't retry every frame.
                self.project.dirty_since = Some(platform::now_ms() + 20_000.0);
                self.say(format!("Autosave failed: {e}"));
            }
        }
    }

    fn thumbnail(&mut self) -> Option<String> {
        self.r.sync(&self.doc);
        let f = self.r.out.first()?;
        let tw = 96usize;
        let th = (f.h * tw / f.w.max(1)).clamp(1, 96);
        let small = crate::select::downscale(f, tw, th);
        project::png_bytes(&small).ok().map(|b| project::base64_encode(&b))
    }

    pub fn autosave(&mut self, now: f64) {
        if let Some(t) = self.project.dirty_since
            && now - t > AUTOSAVE_MS
            && self.live.is_none()
            && matches!(self.gesture, Gesture::None)
        {
            self.save_now();
        }
    }

    pub fn open_project(&mut self, id: &str) {
        let Some(store) = &self.store else { return };
        match store.load(id).and_then(|(t, backup)| project::from_json(&t).map(|l| (l, backup))) {
            Ok((l, backup)) => {
                self.install(l, Some(id.to_string()));
                self.say(if backup {
                    format!("Opened \"{}\" (from the backup slot).", self.project.name)
                } else {
                    format!("Opened \"{}\".", self.project.name)
                });
            }
            Err(e) => self.say(format!("Couldn't open that project: {e}")),
        }
    }

    pub fn switch_project(&mut self, id: &str) {
        if id == self.project.id {
            self.say("That one is already open.");
            return;
        }
        self.apply_floating();
        self.save_if_dirty();
        self.open_project(id);
    }

    pub fn delete_project(&mut self, id: &str) {
        let Some(store) = &self.store else { return };
        let res = store.delete(id);
        self.projects = store.list();
        if let Err(e) = res {
            self.say(format!("Couldn't delete: {e}"));
        }
        self.thumbs.remove(id);
        if id == self.project.id {
            self.new_project(self.s.new_w, self.s.new_h);
        }
    }

    pub fn new_project(&mut self, w: usize, h: usize) {
        self.apply_floating();
        self.save_if_dirty();
        let doc = Doc::new(w, h);
        self.install(project::Loaded { doc, name: "Untitled".into(), zoom: None, warnings: Vec::new() }, Some(crate::store::new_id()));
        self.save_now();
        self.say("New drawing started. Have fun!");
    }

    // ------------------------------------------------------------------ keys

    pub fn run(&mut self, a: Action) {
        match a {
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::Brush => {
                let b = if self.s.tools.brush == Brush::Eraser { self.s.tools.last_brush } else { self.s.tools.brush };
                self.set_brush(b);
                self.set_tool(Tool::Brush);
            }
            Action::Eraser => {
                if self.s.tools.brush == Brush::Eraser && self.s.tools.tool == Tool::Brush {
                    self.set_brush(self.s.tools.last_brush);
                } else {
                    self.set_brush(Brush::Eraser);
                    self.set_tool(Tool::Brush);
                }
            }
            Action::Line => self.set_tool(Tool::Line),
            Action::Rect => self.set_tool(Tool::Rect),
            Action::Ellipse => self.set_tool(Tool::Ellipse),
            Action::Fill => self.set_tool(Tool::Fill),
            Action::Lasso => self.set_tool(Tool::Lasso),
            Action::Move => self.set_tool(Tool::Move),
            Action::Pick => self.set_tool(Tool::Pick),
            Action::Hand => self.set_tool(Tool::Hand),
            Action::SizeDown => self.s.tools.size = (self.s.tools.size / 1.2).floor().max(1.0),
            Action::SizeUp => self.s.tools.size = (self.s.tools.size * 1.2).ceil().min(crate::model::MAX_SIZE),
            Action::PrevBrush | Action::NextBrush => {
                let all = Brush::ALL;
                let i = all.iter().position(|b| *b == self.s.tools.brush).unwrap_or(0);
                let n = all.len();
                let j = if a == Action::NextBrush { (i + 1) % n } else { (i + n - 1) % n };
                if let Some(b) = all.get(j) {
                    self.set_brush(*b);
                }
            }
            Action::Pause => self.paused = !self.paused,
            Action::NextFrame => {
                self.paused = true;
                self.step_frame();
            }
            Action::Focus => self.focus = !self.focus,
            Action::Mirror => {
                let all = Mirror::ALL;
                let i = all.iter().position(|m| *m == self.s.tools.mirror).unwrap_or(0);
                self.s.tools.mirror = all.get((i + 1) % all.len()).copied().unwrap_or_default();
                self.say(format!("Symmetry: {}", self.s.tools.mirror.label()));
            }
            Action::ZoomIn => self.zoom_center(self.view.zoom * 1.25),
            Action::ZoomOut => self.zoom_center(self.view.zoom / 1.25),
            Action::ZoomFit => self.view.fit_pending = true,
            Action::Zoom100 => self.zoom_center(1.0),
            Action::Apply => {
                if self.floating.is_some() {
                    self.apply_floating();
                    self.say("Baked into the layer.");
                }
            }
            Action::Delete => self.delete_floating(),
            Action::NewLayer => self.new_layer(),
            Action::Save => {
                self.apply_floating();
                self.save_now();
                if self.project.error.is_none() {
                    self.say("Saved.");
                }
            }
            Action::ExportPng => self.export_png(),
            Action::ExportGif => self.export_gif(),
            Action::Import => self.import(),
        }
    }

    /// Zoom keeping the view centre fixed; the canvas view supplies the real centre each frame.
    pub fn zoom_center(&mut self, z: f32) {
        let c = VIEW_CENTER.with(std::cell::Cell::get);
        self.zoom_at(z, c);
    }

    pub fn zoom_at(&mut self, z: f32, anchor: egui::Vec2) {
        let z = z.clamp(ZMIN, ZMAX);
        if !z.is_finite() || self.view.zoom <= 0.0 {
            return;
        }
        let k = z / self.view.zoom;
        self.view.offset = anchor - (anchor - self.view.offset) * k;
        self.view.zoom = z;
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let events = ctx.input(|i| i.events.clone());
        if let Some(a) = self.rebinding {
            for e in &events {
                if let egui::Event::Key { key, pressed: true, modifiers, .. } = e {
                    if *key == Key::Escape {
                        self.rebinding = None;
                        break;
                    }
                    let b = crate::settings::Binding { key: *key, cmd: modifiers.command, shift: modifiers.shift, alt: modifiers.alt };
                    self.s.keys.retain(|(x, k)| *x != a && *k != b);
                    self.s.keys.push((a, b));
                    self.rebinding = None;
                    self.say(format!("{} is now {}", a.label(), b.label()));
                    break;
                }
            }
            return;
        }
        let typing = ctx.egui_wants_keyboard_input();
        for e in &events {
            match e {
                egui::Event::Key { key: Key::Space, pressed, .. } if !typing => self.space_down = *pressed,
                egui::Event::Key { key: Key::Escape, pressed: true, .. } => {
                    if self.live.is_some() || matches!(self.gesture, Gesture::Lasso(_) | Gesture::Shape { .. }) {
                        self.cancel_stroke();
                    } else if let Some(f) = &mut self.floating {
                        f.xf.reset();
                    } else if self.focus {
                        self.focus = false;
                    }
                }
                egui::Event::Key { key, pressed: true, repeat, modifiers, .. } if !typing => {
                    if let Some(f) = &mut self.floating
                        && matches!(key, Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown)
                    {
                        let d = if modifiers.shift { 10.0 } else { 1.0 };
                        match key {
                            Key::ArrowLeft => f.xf.dx -= d,
                            Key::ArrowRight => f.xf.dx += d,
                            Key::ArrowUp => f.xf.dy -= d,
                            _ => f.xf.dy += d,
                        }
                        continue;
                    }
                    if let Some(a) = self.s.action_for(*key, *modifiers) {
                        let repeatable =
                            matches!(a, Action::SizeDown | Action::SizeUp | Action::ZoomIn | Action::ZoomOut | Action::Undo | Action::Redo | Action::NextFrame);
                        if !*repeat || repeatable {
                            self.run(a);
                        }
                    }
                }
                _ => {}
            }
        }
        if !ctx.input(|i| i.focused) {
            self.space_down = false;
        }
    }

    /// Re-apply the egui style when the look changed.
    fn apply_look(&mut self, ctx: &egui::Context) {
        let boil = self.look.boil.filter(|_| self.s.wobbly_ui && !self.s.reduce_motion);
        self.look = Look { t: self.s.theme, round: self.s.roundness, line: self.s.outline, shadow: self.s.shadow, boil, labels: self.s.show_labels };
        let key = (Look { boil: None, ..self.look }, self.s.text_size, self.s.ui_scale);
        if self.applied != Some(key) {
            theme::apply(ctx, &self.look, self.s.text_size);
            if (ctx.zoom_factor() - self.s.ui_scale).abs() > 0.001 {
                ctx.set_zoom_factor(self.s.ui_scale);
            }
            self.applied = Some(key);
        }
    }

    /// One frame of the app (everything except drawing the UI).
    pub fn frame_logic(&mut self, ctx: &egui::Context) {
        CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
        self.apply_look(ctx);
        platform::take_dropped(ctx, &self.inbox);
        self.receive_files();
        self.handle_keys(ctx);
        let now = ctx.input(|i| i.time);
        if let Some(wait) = self.tick(now) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
        self.autosave(platform::now_ms());
    }

    pub fn persist(&mut self, storage: &mut dyn eframe::Storage) {
        if let Ok(j) = serde_json::to_string(&self.s) {
            storage.set_string(SETTINGS_KEY, j);
        }
    }

    pub fn shutdown(&mut self) {
        self.cancel_stroke();
        self.apply_floating();
        self.save_if_dirty();
        if let Some(s) = &self.store {
            s.end_session();
        }
    }
}

thread_local! {
    static CTX: std::cell::RefCell<Option<egui::Context>> = const { std::cell::RefCell::new(None) };
    /// Centre of the canvas view, relative to its top-left (for keyboard zoom).
    pub static VIEW_CENTER: std::cell::Cell<egui::Vec2> = const { std::cell::Cell::new(egui::Vec2::ZERO) };
}

impl Renderer {
    /// Draw live strokes' new parts into a layer's cached frames. Returns the touched area.
    pub fn draw_live(&mut self, layer: LayerId, strokes: &[Stroke], progs: &mut [Vec<Progress>], wiggle: f64, finish: bool) -> crate::pixels::IRect {
        let (brushes, cache) = self.split(layer);
        let Some(cache) = cache else { return crate::pixels::IRect::EMPTY };
        let mut area = crate::pixels::IRect::EMPTY;
        for (s, prog) in strokes.iter().zip(progs.iter_mut()) {
            for (f, (px, p)) in cache.frames.iter_mut().zip(prog.iter_mut()).enumerate() {
                area = area.union(brushes.render_more(s, f, wiggle, px, (0, 0), p, finish));
            }
        }
        area
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut a = App::with_settings(Settings::default(), Some(Store::memory()));
        a.doc = Doc::new(64, 48);
        a
    }

    fn scribble(a: &mut App, pts: &[(f64, f64)]) {
        a.begin_stroke(pts[0]);
        for p in &pts[1..] {
            a.feed_stroke(*p, false);
        }
        a.end_stroke();
    }

    #[test]
    fn a_stroke_lands_in_the_layer_and_undoes() {
        let mut a = app();
        scribble(&mut a, &[(5.0, 5.0), (30.0, 20.0), (50.0, 40.0)]);
        assert_eq!(a.doc.layers[0].strokes.len(), 1);
        a.r.sync(&a.doc);
        // The live-drawn cache must match a fresh render exactly.
        let live = a.r.out.clone();
        let fresh = crate::render::export_frames(&a.doc);
        assert!(live == fresh, "live drawing and committed render differ");
        a.undo();
        assert!(a.doc.layers[0].strokes.is_empty());
        a.redo();
        assert_eq!(a.doc.layers[0].strokes.len(), 1);
    }

    #[test]
    fn symmetry_makes_copies_and_hidden_layers_refuse_paint() {
        let mut a = app();
        a.s.tools.mirror = Mirror::Quad;
        scribble(&mut a, &[(5.0, 5.0), (10.0, 9.0)]);
        assert_eq!(a.doc.layers[0].strokes.len(), 4);
        a.doc.layers[0].visible = false;
        scribble(&mut a, &[(5.0, 5.0), (10.0, 9.0)]);
        assert_eq!(a.doc.layers[0].strokes.len(), 4);
    }

    #[test]
    fn shapes_lasso_and_apply() {
        let mut a = app();
        a.commit_shape(geom::rect_pts((10.0, 10.0), (30.0, 30.0)));
        assert_eq!(a.doc.layers[0].strokes.len(), 1);
        a.finish_lasso(vec![(0.0, 0.0), (40.0, 0.0), (40.0, 40.0), (0.0, 40.0)]);
        assert!(a.floating.is_some());
        assert!(a.doc.layers[0].strokes.is_empty());
        if let Some(f) = &mut a.floating {
            f.xf.dx = 5.0;
        }
        a.run(Action::Apply);
        assert!(a.floating.is_none());
        assert_eq!(a.doc.layers[0].strokes[0].pts[0].x.round(), 15.0);
        // The partially refreshed caches must equal a full re-render.
        a.r.sync(&a.doc);
        assert!(a.r.out == crate::render::export_frames(&a.doc), "cut/apply refresh differs from a full render");
        a.fill_at((20.0, 20.0));
        a.r.sync(&a.doc);
        assert!(a.r.out == crate::render::export_frames(&a.doc), "fill refresh differs from a full render");
        a.undo();
        a.r.sync(&a.doc);
        assert!(a.r.out == crate::render::export_frames(&a.doc), "undo from the spare cache differs");
        // Lasso around nothing: no selection and no stray undo step.
        a.finish_lasso(vec![(50.0, 40.0), (60.0, 40.0), (60.0, 47.0), (50.0, 47.0)]);
        assert!(a.floating.is_none());
    }

    #[test]
    fn layer_management() {
        let mut a = app();
        a.new_layer();
        a.duplicate_layer();
        assert_eq!(a.doc.layers.len(), 3);
        assert_eq!(a.doc.current, 2);
        a.move_layer(2, 0);
        assert_eq!(a.doc.current, 0);
        a.select_layer(1);
        scribble(&mut a, &[(5.0, 5.0), (20.0, 20.0)]);
        a.select_layer(2);
        a.merge_down();
        assert_eq!(a.doc.layers.len(), 2);
        a.delete_layer();
        a.delete_layer();
        assert_eq!(a.doc.layers.len(), 1);
        a.clear_layer();
    }

    #[test]
    fn fill_pick_and_frames() {
        let mut a = app();
        a.set_color(Color32::RED);
        a.fill_at((3.0, 3.0));
        a.r.sync(&a.doc);
        a.set_color(Color32::BLUE);
        a.pick_at((3.0, 3.0));
        assert_eq!(a.s.tools.color, Color32::RED);
        a.set_frames(6);
        assert_eq!(a.doc.frames, 6);
        a.doc.playback = Playback::PingPong;
        for _ in 0..20 {
            a.step_frame();
            assert!(a.frame < 6);
        }
        a.doc.playback = Playback::Random;
        for _ in 0..20 {
            let before = a.frame;
            a.step_frame();
            assert_ne!(before, a.frame);
        }
    }

    #[test]
    fn projects_save_reopen_and_delete() {
        let mut a = app();
        scribble(&mut a, &[(5.0, 5.0), (20.0, 20.0)]);
        a.project.name = "Cat".into();
        a.save_now();
        assert!(a.project.error.is_none());
        let first = a.project.id.clone();
        a.new_project(32, 32);
        assert_eq!(a.projects.len(), 2);
        a.switch_project(&first);
        assert_eq!(a.project.name, "Cat");
        assert_eq!(a.doc.layers[0].strokes.len(), 1);
        // Deleting the open project starts a fresh one in its place.
        a.delete_project(&first);
        assert_eq!(a.projects.len(), 2);
        assert!(a.projects.iter().all(|p| p.id != first));
        assert_ne!(a.project.id, first);
    }

    #[test]
    fn every_action_runs_without_panicking() {
        let mut a = app();
        scribble(&mut a, &[(5.0, 5.0), (20.0, 20.0)]);
        for act in Action::ALL {
            // Skip the ones that open native dialogs.
            if matches!(act, Action::ExportPng | Action::ExportGif | Action::Import) {
                continue;
            }
            a.run(act);
        }
        a.receive_files();
        a.inbox.push(Ok(platform::Incoming { name: "x.png".into(), bytes: b"garbage".to_vec(), purpose: Purpose::Any }));
        a.inbox.push(Ok(platform::Incoming { name: "x.wob".into(), bytes: vec![0xff, 0xfe], purpose: Purpose::Any }));
        a.receive_files();
        let png = project::png_bytes(&crate::pixels::Pixmap::filled(200, 100, Color32::RED)).unwrap();
        a.inbox.push(Ok(platform::Incoming { name: "red.png".into(), bytes: png, purpose: Purpose::Any }));
        a.receive_files();
        assert!(a.floating.is_some());
    }
}

#[cfg(test)]
mod perf {
    use super::*;

    /// Timings for the hot paths on a 1920×1080 canvas with 10 layers:
    /// `cargo test --release -p wobbleworks perf -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing report; run on demand in release"]
    fn hot_paths() {
        let mut a = App::with_settings(Settings::default(), Some(Store::memory()));
        a.doc = Doc::new(1920, 1080);
        for _ in 0..9 {
            a.new_layer();
        }
        let t = std::time::Instant::now();
        for k in 0..300 {
            let y = 20.0 + f64::from(k % 100) * 10.0;
            a.s.tools.brush = Brush::ALL[k as usize % 12];
            a.s.tools.size = 4.0 + f64::from(k % 20);
            a.begin_stroke((10.0, y));
            for i in 1..60 {
                a.feed_stroke((10.0 + f64::from(i) * 30.0, y + f64::from(i % 7) * 4.0), false);
                a.r.sync(&a.doc);
            }
            a.end_stroke();
        }
        let per_point = t.elapsed().as_secs_f64() * 1000.0 / (300.0 * 60.0);
        println!("live drawing: {per_point:.3} ms per pointer sample (stamp + recomposite dirty rect, 3 frames, 10 layers)");
        a.r.sync(&a.doc);
        let t = std::time::Instant::now();
        a.doc.wiggle = 1.5;
        a.r.sync(&a.doc);
        println!("re-boil after wiggle change (300 strokes, 3 frames): {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
        let t = std::time::Instant::now();
        a.r.invalidate();
        a.r.sync(&a.doc);
        println!("full recomposite (1920x1080, 3 frames, 10 layers): {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
        let t = std::time::Instant::now();
        a.fill_at((1900.0, 1070.0));
        a.r.sync(&a.doc);
        println!("fill (3 frames) + rebuild: {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
        let t = std::time::Instant::now();
        for _ in 0..10 {
            a.undo();
            a.r.sync(&a.doc);
        }
        println!("undo + resync: {:.1} ms each", t.elapsed().as_secs_f64() * 100.0);
        let t = std::time::Instant::now();
        a.save_now();
        println!("autosave (first, encodes rasters): {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
        a.touched();
        let t = std::time::Instant::now();
        a.save_now();
        println!("autosave (cached PNGs): {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
        let t = std::time::Instant::now();
        let j = a.project_json().unwrap();
        println!("  json: {:.1} ms ({} KB)", t.elapsed().as_secs_f64() * 1000.0, j.len() / 1024);
        let t = std::time::Instant::now();
        let _ = a.thumbnail();
        println!("  thumb: {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
        let t = std::time::Instant::now();
        let _ = project::pack(&j);
        println!("  pack: {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
        let t = std::time::Instant::now();
        let _ = project::from_json(&j).unwrap();
        println!("  load: {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
    }
}
