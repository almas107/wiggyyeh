//! WobbleWorks 3D: Feather-style 3D drawing inside WobbleWorks, driven like Blender.
//!
//! Everything here is presentation: the note, tools, transforms and commands live in the
//! `wobbleworks-3d` crate ([`Editor`]), and this module only lays out panels, maps input and runs
//! commands by id. Swapping this UI for another one means rewriting this module only.
//!
//! Layout (Blender-like, mirrored with "left-handed"): the header with menus on top, the toolbar
//! and Feather's brush panel on the left, the viewport with its navigation gizmo in the middle,
//! Feather's context bar under it, and the sidebar (Stage, Boil, Shots, Item, History, Keys,
//! Help) on the right.

pub mod export;
pub mod gpu;
mod panels;
pub mod previews;
pub mod view;

use std::sync::{Arc, Mutex};

use egui::{Color32, Event, Key, Pos2, Ui};
use serde_json::{Value, json};
use wobbleworks_3d::editor::{Editor, Event as EdEvent};
use wobbleworks_3d::model::Rgba;

use crate::widgets::Look;

/// Feather's orange (the guide's start edge, guide strokes in progress).
pub const ORANGE: Color32 = Color32::from_rgb(0xff, 0x8a, 0x1f);

/// Files picked asynchronously: (purpose, name, bytes).
pub type Inbox = Arc<Mutex<Vec<(String, String, Vec<u8>)>>>;
/// Ask the platform for a file: (inbox, purpose, extensions). The bytes arrive in the inbox.
pub type PickFn = Box<dyn FnMut(Inbox, &'static str, &'static [&'static str])>;
/// Save bytes under a suggested name (native: a dialog and a crash-safe write; web: a download).
/// `Ok(None)` when cancelled; `Ok(Some(name))` when written.
pub type SaveFn = Box<dyn FnMut(&str, &[u8]) -> Result<Option<String>, String>>;

/// Platform file access for the 3D mode.
#[derive(Default)]
pub struct Files {
    pub pick: Option<PickFn>,
    pub save: Option<SaveFn>,
}

pub const NOTE_EXTS: &[&str] = &["wob3d"];
pub const MODEL_EXTS: &[&str] = &["obj"];
pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff", "tga"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideTab {
    Stage,
    Boil,
    Shots,
    Item,
    History,
    Keys,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageTab {
    Groups,
    Resources,
    Environment,
}

/// Popups at the mouse.
#[derive(Debug, Clone, PartialEq)]
pub enum Menu {
    Add,
    Delete,
    Flip,
    MoveToGroup,
    ViewPie,
    Search(String),
    Size,
    Opacity,
    Rename(u64, String),
    Context,
}

/// What the 3D mode reports to the shell (sounds, Wob, effects).
#[derive(Debug, Clone, PartialEq)]
pub enum Feedback {
    Event(EdEvent),
    Saved,
    Exported,
}

pub struct Space3d {
    pub ed: Editor,
    pub view: view::Viewport,
    pub files: Files,
    pub inbox: Inbox,
    /// The note's file name (for Save).
    pub name: Option<String>,
    pub side: SideTab,
    pub stage: StageTab,
    pub show_tools: bool,
    pub show_brush: bool,
    pub show_side: bool,
    pub hide_ui: bool,
    pub left_handed: bool,
    /// Hold the boil while drawing, so the line being drawn stays put.
    pub hold_boil_while_drawing: bool,
    pub menu: Option<(Menu, Pos2)>,
    /// The action waiting for a new shortcut (Keys tab).
    pub rebinding: Option<String>,
    /// Playing the camera shots: (start time).
    pub playing: Option<f64>,
    /// Groups picked in the Groups tab (for merge, duplicate, delete).
    pub picked_groups: Vec<u64>,
    /// Export size multiplier (1–4).
    pub export_scale: u32,
    pub export_transparent: bool,
    pub feedback: Vec<Feedback>,
    /// Exact transform inputs (Item tab).
    pub item_mode: usize,
    pub item_axis: usize,
    pub item_amount: f32,
    /// The header's "2D" button: the shell switches back.
    pub leave: bool,
    /// Brush type sample tiles.
    pub previews: previews::Previews,
    /// Shots: a 3 × 3 framing grid over the view.
    pub show_thirds: bool,
    /// Shots: the camera's position, turn and lens over the view.
    pub show_camera_info: bool,
    /// Find Group: the group of the curve under the pointer (Select tool), and when it was looked up.
    pub hover_group: Option<String>,
    hover_checked: (f64, [f32; 2]),
    frame_index: u32,
}

impl Default for Space3d {
    fn default() -> Self {
        Space3d::new()
    }
}

/// egui's name for a key, as the keymap writes it.
pub fn key_name(k: Key) -> Option<String> {
    let s = match k {
        Key::Escape => "Escape",
        Key::Tab => "Tab",
        Key::Space => "Space",
        Key::Enter => "Enter",
        Key::Delete => "Delete",
        Key::Backspace => "Backspace",
        Key::Home => "Home",
        Key::End => "End",
        Key::PageUp => "PageUp",
        Key::PageDown => "PageDown",
        Key::ArrowUp => "Up",
        Key::ArrowDown => "Down",
        Key::ArrowLeft => "Left",
        Key::ArrowRight => "Right",
        Key::Minus => "-",
        Key::Equals => "=",
        Key::Plus => "+",
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::Backtick => "`",
        Key::Period => ".",
        Key::Comma => ",",
        Key::Slash => "/",
        Key::Num0 => "0",
        Key::Num1 => "1",
        Key::Num2 => "2",
        Key::Num3 => "3",
        Key::Num4 => "4",
        Key::Num5 => "5",
        Key::Num6 => "6",
        Key::Num7 => "7",
        Key::Num8 => "8",
        Key::Num9 => "9",
        Key::F1 => "F1",
        Key::F2 => "F2",
        Key::F3 => "F3",
        Key::F4 => "F4",
        Key::F5 => "F5",
        Key::F6 => "F6",
        Key::F7 => "F7",
        Key::F8 => "F8",
        Key::F9 => "F9",
        Key::F10 => "F10",
        Key::F11 => "F11",
        Key::F12 => "F12",
        other => {
            let n = other.name();
            if n.len() == 1 && n.chars().all(|c| c.is_ascii_alphabetic()) {
                return Some(n.to_ascii_uppercase());
            }
            return None;
        }
    };
    Some(s.to_string())
}

/// "Ctrl+Shift+Alt+Key" for a key press.
pub fn chord(key: Key, m: egui::Modifiers) -> Option<String> {
    let name = key_name(key)?;
    let mut out = String::new();
    if m.command || m.ctrl {
        out.push_str("Ctrl+");
    }
    if m.shift {
        out.push_str("Shift+");
    }
    if m.alt {
        out.push_str("Alt+");
    }
    out.push_str(&name);
    Some(out)
}

fn rgba(c: Color32) -> Rgba {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Rgba([r, g, b, a])
}

pub fn colour32(c: Rgba) -> Color32 {
    let [r, g, b, a] = c.0;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

impl Space3d {
    pub fn new() -> Space3d {
        let mut ed = Editor::new();
        // A first guide-free sketch works right away; the hint says how to make a guide.
        ed.status = "Draw (D) in the air, or draw a 3D Guide (Q) to draw on. Middle mouse orbits.".into();
        Space3d {
            ed,
            view: view::Viewport::default(),
            files: Files::default(),
            inbox: Arc::default(),
            name: None,
            side: SideTab::Stage,
            stage: StageTab::Groups,
            show_tools: true,
            show_brush: true,
            show_side: true,
            hide_ui: false,
            left_handed: false,
            hold_boil_while_drawing: true,
            menu: None,
            rebinding: None,
            playing: None,
            picked_groups: Vec::new(),
            export_scale: 2,
            export_transparent: false,
            feedback: Vec::new(),
            item_mode: 0,
            item_axis: 0,
            item_amount: 0.0,
            leave: false,
            previews: previews::Previews::default(),
            show_thirds: false,
            show_camera_info: false,
            hover_group: None,
            hover_checked: (0.0, [0.0, 0.0]),
            frame_index: 0,
        }
    }

    /// Draw the view on the GPU with a depth buffer (the app runs on wgpu).
    pub fn set_gpu(&mut self, rs: &eframe::egui_wgpu::RenderState) {
        self.view.gpu = Some(gpu::Gpu::new(rs));
    }

    /// Preferences kept between sessions: layout, the keymap and the drawing aids.
    pub fn settings(&self) -> Value {
        json!({
            "leftHanded": self.left_handed,
            "tools": self.show_tools,
            "brush": self.show_brush,
            "side": self.show_side,
            "emulateMmb": self.view.emulate_mmb,
            "holdBoil": self.hold_boil_while_drawing,
            "stable": self.ed.stable,
            "drawInAir": self.ed.draw_in_air,
            "guideShape": self.ed.guide_shape,
            "exportScale": self.export_scale,
            "keymap": self.ed.keymap,
        })
    }

    /// Restore [`Self::settings`]; anything unreadable is ignored.
    pub fn restore_settings(&mut self, v: &Value) {
        let flag = |k: &str| v.get(k).and_then(Value::as_bool);
        if let Some(b) = flag("leftHanded") {
            self.left_handed = b;
        }
        if let Some(b) = flag("tools") {
            self.show_tools = b;
        }
        if let Some(b) = flag("brush") {
            self.show_brush = b;
        }
        if let Some(b) = flag("side") {
            self.show_side = b;
        }
        if let Some(b) = flag("emulateMmb") {
            self.view.emulate_mmb = b;
        }
        if let Some(b) = flag("holdBoil") {
            self.hold_boil_while_drawing = b;
        }
        if let Some(x) = v.get("stable").and_then(Value::as_f64).filter(|x| x.is_finite()) {
            self.ed.stable = (x as f32).clamp(0.0, 1.0);
        }
        if let Some(b) = flag("drawInAir") {
            self.ed.draw_in_air = b;
        }
        if let Some(b) = flag("guideShape") {
            self.ed.guide_shape = b;
        }
        if let Some(x) = v.get("exportScale").and_then(Value::as_u64) {
            self.export_scale = (x as u32).clamp(1, 4);
        }
        if let Some(k) = v.get("keymap").and_then(|k| serde_json::from_value::<wobbleworks_3d::keys::Keymap>(k.clone()).ok()) {
            self.ed.keymap = k;
            self.ed.keymap.merge_defaults();
        }
    }

    /// Run a command, showing an error in the status line rather than failing.
    pub fn run(&mut self, cmd: &str, params: Value) -> Option<Value> {
        match self.ed.run(cmd, &params) {
            Ok(v) => Some(v),
            Err(e) => {
                self.ed.status = e;
                None
            }
        }
    }

    /// The tooltip for an action: its label and shortcut.
    pub fn tip(&self, text: &str, action: &str) -> String {
        match self.ed.keymap.chord(action) {
            Some(c) if !c.is_empty() => format!("{text}  ({c})"),
            _ => text.to_string(),
        }
    }

    /// The shell's colour strip picked a colour: it becomes the brush colour (and the selected
    /// curves' colour).
    pub fn set_colour(&mut self, c: Color32) {
        let hex = rgba(c).to_hex();
        self.run("brush.set", json!({"color": hex}));
    }

    pub fn brush_colour(&self) -> Color32 {
        colour32(self.ed.brush.color)
    }

    /// The boil frame showing now.
    pub fn frame_at(&self, t: f64) -> u32 {
        self.ed.scene.boil.frame_at(t)
    }

    /// Keyboard: modal transform keys first, then the keymap.
    fn keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        for ev in events {
            let Event::Key { key, pressed: true, repeat, modifiers, .. } = ev else { continue };
            if let Some(action) = self.rebinding.take() {
                if key != Key::Escape
                    && let Some(c) = chord(key, modifiers)
                {
                    self.run("keymap.rebind", json!({"action": action, "chord": c}));
                }
                continue;
            }
            if self.menu.is_some() {
                if key == Key::Escape {
                    self.menu = None;
                }
                continue;
            }
            if self.ed.in_modal() {
                let text = match key {
                    Key::X | Key::Y | Key::Z => {
                        let axis = match key {
                            Key::X => 0,
                            Key::Y => 1,
                            _ => 2,
                        };
                        self.run("transform.axis", json!({"axis": axis, "plane": modifiers.shift}));
                        None
                    }
                    Key::Enter | Key::Space => {
                        self.run("transform.confirm", Value::Null);
                        None
                    }
                    Key::Escape => {
                        self.run("transform.cancel", Value::Null);
                        None
                    }
                    Key::Backspace => Some("\u{8}".to_string()),
                    Key::Minus => Some("-".into()),
                    Key::Period | Key::Comma => Some(".".into()),
                    k => key_name(k).filter(|n| n.len() == 1 && n.chars().all(|c| c.is_ascii_digit())),
                };
                if let Some(t) = text {
                    self.run("transform.type", json!({"text": t}));
                }
                continue;
            }
            if repeat && !matches!(key, Key::OpenBracket | Key::CloseBracket | Key::Minus | Key::Equals) {
                continue;
            }
            // Enter finishes a primitive or loft (Feather's Done).
            if key == Key::Enter && modifiers.is_none() {
                self.run("guide.done", Value::Null);
                continue;
            }
            let Some(c) = chord(key, modifiers) else { continue };
            let Some(b) = self.ed.keymap.lookup(&c).cloned() else { continue };
            self.dispatch(ctx, &b.command, b.params.clone());
        }
    }

    /// Run a keymap or menu command: `ui.*` here, the rest in the editor.
    pub fn dispatch(&mut self, ctx: &egui::Context, command: &str, params: Value) {
        let at = ctx.pointer_latest_pos().unwrap_or(self.view.rect.center());
        match command {
            "ui.viewPie" => self.menu = Some((Menu::ViewPie, at)),
            "ui.addMenu" => self.menu = Some((Menu::Add, at)),
            "ui.deleteMenu" => {
                if self.ed.has_selection() {
                    self.menu = Some((Menu::Delete, at));
                }
            }
            "ui.flipMenu" => {
                if self.ed.has_selection() {
                    self.menu = Some((Menu::Flip, at));
                }
            }
            "ui.moveToGroup" => {
                if !self.ed.selection.is_empty() {
                    self.menu = Some((Menu::MoveToGroup, at));
                }
            }
            "ui.search" => self.menu = Some((Menu::Search(String::new()), at)),
            "ui.radialSize" => self.menu = Some((Menu::Size, at)),
            "ui.radialOpacity" => self.menu = Some((Menu::Opacity, at)),
            "ui.rename" => {
                let g = self.ed.scene.active_group;
                let name = self.ed.scene.group(g).map(|g| g.name.clone()).unwrap_or_default();
                self.menu = Some((Menu::Rename(g, name), at));
            }
            "ui.sidebar" => self.show_side = !self.show_side,
            "ui.toolbar" => {
                self.show_tools = !self.show_tools;
                self.show_brush = self.show_tools;
            }
            "ui.hideUi" => self.hide_ui = !self.hide_ui,
            "ui.save" => self.save(false),
            "ui.saveAs" => self.save(true),
            "ui.open" => self.pick("open", NOTE_EXTS),
            "ui.renderImage" => self.export_png(),
            "ui.renderAnimation" => self.export_gif(false),
            _ => {
                self.run(command, params);
            }
        }
    }

    pub fn pick(&mut self, purpose: &'static str, exts: &'static [&'static str]) {
        match self.files.pick.as_mut() {
            Some(p) => p(self.inbox.clone(), purpose, exts),
            None => self.ed.status = "Opening files is not available here".into(),
        }
    }

    fn save_bytes(&mut self, name: &str, bytes: &[u8]) -> Option<String> {
        let Some(save) = self.files.save.as_mut() else {
            self.ed.status = "Saving files is not available here".into();
            return None;
        };
        match save(name, bytes) {
            Ok(Some(n)) => Some(n),
            Ok(None) => None,
            Err(e) => {
                self.ed.status = format!("Couldn't save: {e}");
                None
            }
        }
    }

    pub fn save(&mut self, ask: bool) {
        let bytes = match wobbleworks_3d::io::save(&self.ed.scene, &self.ed.camera) {
            Ok(b) => b,
            Err(e) => {
                self.ed.status = e;
                return;
            }
        };
        let name = match (&self.name, ask) {
            (Some(n), false) => n.clone(),
            _ => "Note.wob3d".to_string(),
        };
        if let Some(n) = self.save_bytes(&name, &bytes) {
            self.ed.status = format!("Saved {}", crate::shell::file_name(&n));
            self.name = Some(n);
            self.ed.dirty = false;
            self.feedback.push(Feedback::Saved);
        }
    }

    pub fn export_png(&mut self) {
        let r = export::png(&mut self.ed, self.frame_index, self.export_scale, self.export_transparent);
        match r {
            Ok(bytes) => {
                if let Some(n) = self.save_bytes("WobbleWorks 3D.png", &bytes) {
                    self.ed.status = format!("Exported {}", crate::shell::file_name(&n));
                    self.feedback.push(Feedback::Exported);
                }
            }
            Err(e) => self.ed.status = e,
        }
    }

    pub fn export_gif(&mut self, turntable: bool) {
        let r = if turntable { export::turntable_gif(&mut self.ed, 1) } else { export::boil_gif(&mut self.ed, 1) };
        match r {
            Ok(bytes) => {
                let name = if turntable { "WobbleWorks 3D turntable.gif" } else { "WobbleWorks 3D.gif" };
                if let Some(n) = self.save_bytes(name, &bytes) {
                    self.ed.status = format!("Exported {}", crate::shell::file_name(&n));
                    self.feedback.push(Feedback::Exported);
                }
            }
            Err(e) => self.ed.status = e,
        }
    }

    pub fn export_obj(&mut self) {
        let (obj, _) = wobbleworks_3d::io::export_obj(&self.ed.scene, "note.mtl");
        if let Some(n) = self.save_bytes("WobbleWorks 3D.obj", obj.as_bytes()) {
            self.ed.status = format!("Exported {}", crate::shell::file_name(&n));
            self.feedback.push(Feedback::Exported);
        }
    }

    pub fn export_glb(&mut self) {
        match wobbleworks_3d::io::export_glb(&self.ed.scene) {
            Ok(bytes) => {
                if let Some(n) = self.save_bytes("WobbleWorks 3D.glb", &bytes) {
                    self.ed.status = format!("Exported {}", crate::shell::file_name(&n));
                    self.feedback.push(Feedback::Exported);
                }
            }
            Err(e) => self.ed.status = e,
        }
    }

    /// Files that arrived from pickers or drops.
    fn drain_inbox(&mut self) {
        let items = match self.inbox.lock() {
            Ok(mut g) => std::mem::take(&mut *g),
            Err(p) => std::mem::take(&mut *p.into_inner()),
        };
        for (purpose, name, bytes) in items {
            let r = self.receive(&purpose, &name, &bytes);
            if let Err(e) = r {
                self.ed.status = format!("Couldn't open {}: {e}", crate::shell::file_name(&name));
            }
        }
    }

    /// Open, import or place a file by what it is.
    pub fn receive(&mut self, purpose: &str, name: &str, bytes: &[u8]) -> Result<(), String> {
        let lower = name.to_ascii_lowercase();
        if purpose == "importNote" {
            let (scene, _) = wobbleworks_3d::io::load(bytes)?;
            let made = self.ed.import_note(&scene, crate::shell::file_name(name).trim_end_matches(".wob3d"))?;
            self.ed.status = format!("Imported {} groups", made.len());
            return Ok(());
        }
        if purpose == "open" || lower.ends_with(".wob3d") {
            let (scene, cam) = wobbleworks_3d::io::load(bytes)?;
            self.ed.open(scene, cam);
            self.name = Some(name.to_string());
            self.ed.status = format!("Opened {}", crate::shell::file_name(name));
            return Ok(());
        }
        if purpose == "model" || lower.ends_with(".obj") {
            let text = std::str::from_utf8(bytes).map_err(|_| "the OBJ is not text")?;
            self.ed.add_model(crate::shell::file_name(name).as_str(), text)?;
            self.ed.status = "Model added: set it to draw-on in Resources to draw on it".into();
            return Ok(());
        }
        let img = photocraft_codecs::decode(bytes).map_err(|e| e.to_string())?;
        let (w, h) = (img.width(), img.height());
        let rgba = img.to_rgba8();
        if purpose == "background" {
            self.ed.set_background_image(Some((crate::shell::file_name(name), w, h, rgba)))?;
            self.ed.status = "Background image set".into();
            return Ok(());
        }
        self.ed.add_image(crate::shell::file_name(name).as_str(), w, h, rgba)?;
        self.ed.status = "Image added: set it to draw-on in Resources to draw on it".into();
        Ok(())
    }

    /// Feather's Find Group: with the Select tool, the group of the curve under the pointer
    /// shows in the status line (looked up at most five times a second).
    fn find_group(&mut self, now: f64) {
        use wobbleworks_3d::editor::Tool;
        if !matches!(self.ed.tool, Tool::Select | Tool::Deselect | Tool::Injector | Tool::Eyedropper) || self.ed.is_busy() {
            self.hover_group = None;
            return;
        }
        let m = self.ed.mouse;
        if now - self.hover_checked.0 < 0.2 || m == self.hover_checked.1 {
            return;
        }
        self.hover_checked = (now, m);
        self.hover_group = self.ed.run("info.groupAt", &json!({"x": m[0], "y": m[1]})).ok().and_then(|v| v.as_str().map(str::to_string));
    }

    /// Camera shots playback.
    fn play_shots(&mut self, now: f64) {
        let Some(start) = self.playing else { return };
        let seq = &self.ed.scene.sequence;
        let n = seq.shots.len();
        if n < 2 {
            self.playing = None;
            return;
        }
        let per = (seq.seconds_per_shot / seq.speed.max(0.1)) as f64;
        let legs = (n - 1) as f64;
        let mut t = (now - start) / per;
        match seq.mode {
            wobbleworks_3d::model::PlayMode::Once => {
                if t >= legs {
                    t = legs;
                    self.playing = None;
                }
            }
            wobbleworks_3d::model::PlayMode::Loop => t %= legs,
            wobbleworks_3d::model::PlayMode::Swing => {
                let c = t % (2.0 * legs);
                t = if c > legs { 2.0 * legs - c } else { c };
            }
        }
        let i = (t.floor() as usize).min(n - 2);
        let f = (t - i as f64).clamp(0.0, 1.0) as f32;
        let e = f * f * (3.0 - 2.0 * f);
        let (Some(a), Some(b)) = (seq.shots.get(i).map(|s| s.camera), seq.shots.get(i + 1).map(|s| s.camera)) else { return };
        let lerp = |x: f32, y: f32| x + (y - x) * e;
        let mut dyaw = b.yaw - a.yaw;
        if dyaw > 180.0 {
            dyaw -= 360.0;
        } else if dyaw < -180.0 {
            dyaw += 360.0;
        }
        let vp = self.ed.camera.viewport;
        self.ed.camera = wobbleworks_3d::camera::Camera {
            target: a.target.lerp(b.target, e),
            yaw: a.yaw + dyaw * e,
            pitch: lerp(a.pitch, b.pitch),
            distance: lerp(a.distance, b.distance),
            focal_mm: lerp(a.focal_mm, b.focal_mm),
            orthographic: if e < 0.5 { a.orthographic } else { b.orthographic },
            snapped_from_perspective: false,
            viewport: vp,
        };
        self.run("camera.set", Value::Null);
    }

    /// Draw the 3D mode in `ui` (the area above the colour strip). `pressure` is the pen's.
    pub fn show(&mut self, ui: &mut Ui, look: &Look, pressure: f32) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.drain_inbox();
        self.keys(&ctx);
        self.play_shots(now);
        if self.playing.is_some() {
            ctx.request_repaint();
        }
        let holding = self.hold_boil_while_drawing && self.ed.is_busy();
        if !holding {
            self.frame_index = self.frame_at(now);
        }
        if self.ed.scene.boil.enabled {
            let fps = self.ed.scene.boil.fps.clamp(1.0, 24.0) as f64;
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(1.0 / fps));
        }
        let ui_shown = !self.hide_ui && self.playing.is_none();
        if ui_shown {
            panels::header(self, ui, look);
            if self.left_handed {
                if self.show_tools {
                    panels::toolbar(self, ui, look, true);
                }
                if self.show_brush {
                    panels::brush_panel(self, ui, look, true);
                }
                if self.show_side {
                    panels::sidebar(self, ui, look, false);
                }
            } else {
                if self.show_tools {
                    panels::toolbar(self, ui, look, false);
                }
                if self.show_brush {
                    panels::brush_panel(self, ui, look, false);
                }
                if self.show_side {
                    panels::sidebar(self, ui, look, true);
                }
            }
            panels::context_bar(self, ui, look);
        }
        let rect = ui.available_rect_before_wrap();
        let menu = self.view.show(ui, &mut self.ed, look, rect, self.frame_index, pressure);
        if let Some(at) = menu {
            self.menu = Some((Menu::Context, at));
        }
        self.find_group(now);
        panels::framing(self, ui, look, rect);
        if ui_shown {
            view::navigator(ui, &mut self.ed, look, rect);
            panels::status_line(self, ui, look, rect);
        } else {
            panels::show_ui_button(self, ui, look, rect);
        }
        panels::menus(self, &ctx, look);
        for e in std::mem::take(&mut self.ed.events) {
            self.feedback.push(Feedback::Event(e));
        }
        if self.ed.is_busy() {
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_and_key_names() {
        assert_eq!(chord(Key::G, egui::Modifiers::NONE).as_deref(), Some("G"));
        assert_eq!(chord(Key::Z, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT).as_deref(), Some("Ctrl+Shift+Z"));
        assert_eq!(chord(Key::Num1, egui::Modifiers::CTRL).as_deref(), Some("Ctrl+1"));
        assert_eq!(key_name(Key::F12).as_deref(), Some("F12"));
    }

    #[test]
    fn files_arrive_by_purpose_and_bad_files_are_errors() {
        let mut s = Space3d::new();
        assert!(s.receive("open", "x.wob3d", b"garbage").is_err());
        assert!(s.receive("model", "m.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").is_ok());
        assert_eq!(s.ed.scene.models.len(), 1);
        assert!(s.receive("image", "i.png", b"not a png").is_err());
        let bytes = wobbleworks_3d::io::save(&s.ed.scene, &s.ed.camera).expect("save");
        assert!(s.receive("open", "n.wob3d", &bytes).is_ok());
        assert_eq!(s.name.as_deref(), Some("n.wob3d"));
    }
}
