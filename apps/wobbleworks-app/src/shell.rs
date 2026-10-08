//! The WobbleWorks window: PhotoCraft's full editor, redrawn by hand.
//!
//! The editor is PhotoCraft's own (menus, options bar, tools, panels, canvas), so every feature is
//! there. WobbleWorks restyles it: its colours (theme tokens), the pixel font (built into a
//! TrueType font at startup), wobbly boiling outlines on every box and line (`handdrawn`), its
//! icons redrawn as soft marker lines (`svgicon`), and dots on the paper around the picture.
//! Below the editor sits the colour picker (`colour`): the current colour, recently used colours
//! and the user's own palette (kept between sessions), opening into a card with a colour wheel,
//! the palette and a reference image.

use std::sync::{Arc, Mutex};

use egui::{Color32, Pos2, Rect, Ui, pos2, vec2};
use photocraft_engine::prefs::{CanvasBorder, CanvasColor, Theme as PrefTheme};
use photocraft_engine::{Session, wiggle_cmds};
use photocraft_ui_egui::state::Tool;
use photocraft_ui_egui::{ExportSettings, PhotocraftApp, Services};
use serde_json::{Value, json};

use crate::audio::{Audio, Sound};
use crate::colour::ColourPicker;
use crate::juice::Juice;
use crate::mascot::{self, Mascot};
use crate::rough::{self, Paint};
use crate::svgicon::{self, IconInk};
use crate::theme::{self, Theme, mix};
use crate::widgets::{self, Look};
use crate::{handdrawn, ttf};

/// The canvas a new picture gets.
pub const NEW_SIZE: (u32, u32) = (1200, 800);

/// The starting palette: (name, colour).
pub const PALETTE: [(&str, &str); 16] = [
    ("Ink", "#17161c"),
    ("Bubblegum", "#ff2e88"),
    ("Tangerine", "#ff8a1f"),
    ("Sunshine", "#ffd23f"),
    ("Lime", "#9be34a"),
    ("Mint", "#2ec27e"),
    ("Pool", "#2ee6c5"),
    ("Sky", "#4fb3ff"),
    ("Blueberry", "#2f5bff"),
    ("Grape", "#8a4dff"),
    ("Orchid", "#e05aff"),
    ("Peach", "#ffb38a"),
    ("Cocoa", "#7a4a2e"),
    ("Pebble", "#8b8798"),
    ("Cloud", "#d9d6e4"),
    ("Paper", "#ffffff"),
];

/// Recent colours kept.
pub const RECENT_MAX: usize = 12;
/// Palette colours kept.
pub const PALETTE_MAX: usize = 48;
/// The pixel font's size against the fonts it replaces (one font pixel ≈ 1.25 points at
/// PhotoCraft's 12.5 pt body text; larger would overflow its panels, as pixel letters run wide).
pub const FONT_SCALE: f32 = 1.0;

const STORE_KEY: &str = "wobbleworks.colours";
const SETTINGS_KEY: &str = "wobbleworks.settings";

pub struct WobbleApp {
    /// PhotoCraft's editor, which owns the engine session.
    pub app: PhotocraftApp,
    pub theme: Theme,
    /// Boiling outlines (off holds the UI still).
    pub boiling: bool,
    /// Redraw PhotoCraft's boxes and lines by hand (off shows them as PhotoCraft draws them).
    pub hand_drawn: bool,
    /// Use the pixel font for PhotoCraft's text.
    pub pixel_font: bool,
    /// Draw PhotoCraft's icons as hand-drawn lines.
    pub custom_icons: bool,
    /// Colours recently painted with, newest first.
    pub recent: Vec<Color32>,
    /// The user's palette.
    pub palette: Vec<Color32>,
    /// The colour picker under the editor.
    pub picker: ColourPicker,
    /// Boil playback: wiggle layers flip through their frames.
    pub boil_play: bool,
    /// How far wiggle strokes wander (pixels).
    pub wiggle_amount: f32,
    /// Boil frames per second (playback and GIF export).
    pub boil_fps: f32,
    /// The boil frame last shown.
    shown_frame: Option<u64>,
    /// Journal entries already looked at for strokes to spread onto every boil frame.
    journal_seen: usize,
    /// Synthesized sound effects.
    pub audio: Audio,
    /// Pops, shake and particles.
    pub juice: Juice,
    /// Wob, the mascot.
    pub mascot: Mascot,
    /// Reduce motion: no boiling UI, pops, shake, particles or bouncing mascot.
    pub reduce_motion: bool,
    /// The settings card.
    pub show_settings: bool,
    /// Journal entries already looked at for sounds and effects.
    fx_seen: usize,
    last_tool: Option<Tool>,
    last_status: String,
    picker_was_open: bool,
    /// Something has been drawn this session (the first stroke is celebrated).
    drawn: bool,
    /// The boil frame and colours the icon painter reads.
    icons: Arc<Mutex<(u64, IconInk)>>,
    /// (document, history length) last seen, to notice painting.
    seen: Option<(u64, u64)>,
    frames: u64,
}

/// Below this window width the side panels start closed, so the picture gets the room.
pub const NARROW: f32 = 700.0;

/// A tool's short name ("Brush Tool" → "Brush").
pub fn tool_name(tool: Tool) -> &'static str {
    let l = tool.label();
    l.strip_suffix(" Tool").unwrap_or(l)
}

fn hex(c: &str) -> Option<Color32> {
    Color32::from_hex(c).ok()
}

impl WobbleApp {
    /// A WobbleWorks window over a fresh PhotoCraft session with a blank picture and the Brush.
    pub fn new(services: Services) -> Self {
        let theme = Theme::default();
        let ink = IconInk { ink: theme.ink, wash: mix(theme.cool, theme.card, 0.7), card: theme.card };
        let mut w = Self {
            app: PhotocraftApp::new(Session::new(), services),
            theme,
            boiling: true,
            hand_drawn: true,
            pixel_font: true,
            custom_icons: true,
            recent: Vec::new(),
            palette: PALETTE.iter().filter_map(|(_, h)| hex(h)).collect(),
            picker: ColourPicker::default(),
            boil_play: true,
            wiggle_amount: wiggle_cmds::DEFAULT_AMOUNT as f32,
            boil_fps: (1.0 / rough::BOIL_SECONDS) as f32,
            shown_frame: None,
            journal_seen: 0,
            audio: Audio::default(),
            juice: Juice::default(),
            mascot: Mascot::default(),
            reduce_motion: false,
            show_settings: false,
            fx_seen: 0,
            last_tool: None,
            last_status: String::new(),
            picker_was_open: false,
            drawn: false,
            icons: Arc::new(Mutex::new((0, ink))),
            seen: None,
            frames: 0,
        };
        w.app.ui.theme = theme::base_kind(&theme);
        w.style_canvas();
        if let Err(e) = w.new_picture() {
            w.report(&e);
        }
        if let Err(e) = w.pick_colour(0) {
            w.report(&e);
        }
        w
    }

    /// PhotoCraft's canvas preferences for the paper look: the theme's paper around the picture
    /// (WobbleWorks' own outline and shadow replace PhotoCraft's border) and the light layout.
    fn style_canvas(&mut self) {
        let paper = theme::to_hex(self.theme.paper);
        let dark = self.theme.dark;
        self.app.session.edit_prefs(|p| {
            p.interface.canvas_color = CanvasColor::Custom;
            p.interface.canvas_custom_color = paper;
            p.interface.canvas_border = CanvasBorder::None;
            p.interface.theme = if dark { PrefTheme::Studio } else { PrefTheme::StudioLight };
        });
    }

    /// Run a PhotoCraft command by id (errors also go to the status line).
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.app.run(id, params)
    }

    /// File › New with WobbleWorks' canvas size, on white, with a wiggle layer to draw on (so
    /// everything drawn boils, as in WigglyPaint), then the Brush tool.
    pub fn new_picture(&mut self) -> Result<(), String> {
        self.run("file.new", json!({"width": NEW_SIZE.0, "height": NEW_SIZE.1, "background": "white", "name": "Wobble"}))?;
        self.run("wiggle.new", json!({}))?;
        self.app.ui.tool = Tool::Brush;
        self.journal_seen = self.app.session.journal.len();
        Ok(())
    }

    /// Add a wiggle layer above the active layer.
    pub fn new_wiggle_layer(&mut self) -> Result<(), String> {
        self.run("wiggle.new", json!({})).map(|_| ())?;
        self.journal_seen = self.app.session.journal.len();
        Ok(())
    }

    /// Is the active layer part of a wiggle layer?
    pub fn on_wiggle_layer(&self) -> bool {
        self.app.session.active().is_some_and(|st| st.active_layer.is_some_and(|id| wiggle_cmds::wiggle_of(&st.doc, id).is_some()))
    }

    /// Strokes and fills just made with PhotoCraft's tools on one boil frame are taken back and
    /// made again on every frame of the wiggle layer (one undo step), so everything drawn boils.
    pub fn spread_to_frames(&mut self) {
        let journal = &self.app.session.journal;
        if journal.len() <= self.journal_seen {
            self.journal_seen = self.journal_seen.min(journal.len());
            return;
        }
        let last = journal.last().cloned();
        self.journal_seen = journal.len();
        let Some((id, params)) = last else { return };
        if !wiggle_cmds::APPLY.contains(&id.as_str()) || !self.on_wiggle_layer() || !self.app.session.is_enabled("edit.undo") {
            return;
        }
        self.app.session.undo();
        let r = self.run("wiggle.apply", json!({"command": id, "params": params, "amount": self.wiggle_amount}));
        self.report_result(r);
        self.journal_seen = self.app.session.journal.len();
    }

    /// Show the boil frame for time `t` (seconds) on every wiggle layer.
    pub fn play_boil(&mut self, t: f64) {
        if !self.boil_play || !t.is_finite() || t < 0.0 {
            return;
        }
        let frame = (t * f64::from(self.boil_fps.clamp(1.0, 50.0))) as u64;
        if self.shown_frame == Some(frame) {
            return;
        }
        self.shown_frame = Some(frame);
        if self.app.session.active().is_some_and(|st| wiggle_cmds::frame_count(&st.doc) > 0) {
            let _ = self.app.session.execute("wiggle.showFrame", json!({"frame": frame}));
            self.app.sync_views();
        }
    }

    /// Export the boil frames as an animated GIF. Returns the path written.
    pub fn export_gif(&mut self, path: Option<String>) -> Result<String, String> {
        let path = self.target(path, "gif", &[])?;
        let doc = self.app.session.active().ok_or("there is no picture to export")?.doc.clone();
        let bytes = crate::anim::gif(&doc, self.boil_fps)?;
        let write = self.app.services.write.as_mut().ok_or("no writer configured")?;
        write(&path, &bytes)?;
        self.app.ui.status = format!("Exported {}", file_name(&path));
        self.app.ui.status_error = false;
        Ok(path)
    }

    /// Export every boil frame as `name_1.png`, `name_2.png`, …. Returns the paths written.
    pub fn export_png_sequence(&mut self, path: Option<String>) -> Result<Vec<String>, String> {
        let path = self.target(path, "png", &[])?;
        let doc = self.app.session.active().ok_or("there is no picture to export")?.doc.clone();
        let frames = crate::anim::png_frames(&doc)?;
        let stem = path.strip_suffix(".png").or_else(|| path.strip_suffix(".PNG")).unwrap_or(&path).to_string();
        let write = self.app.services.write.as_mut().ok_or("no writer configured")?;
        let mut written = Vec::new();
        for (i, bytes) in frames.iter().enumerate() {
            let p = format!("{stem}_{}.png", i + 1);
            write(&p, bytes)?;
            written.push(p);
        }
        self.app.ui.status = format!("Exported {} frames", written.len());
        self.app.ui.status_error = false;
        Ok(written)
    }

    /// Make palette colour `index` the foreground colour.
    pub fn pick_colour(&mut self, index: usize) -> Result<(), String> {
        let c = *self.palette.get(index).ok_or_else(|| format!("no palette colour {index}"))?;
        self.set_foreground(c)
    }

    pub fn set_foreground(&mut self, c: Color32) -> Result<(), String> {
        self.run("tools.setColors", json!({"foreground": theme::to_hex(c)})).map(|_| ())
    }

    /// The foreground colour.
    pub fn foreground(&self) -> Color32 {
        let [r, g, b, _] = self.app.session.tools.foreground;
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Color32::from_rgb(c(r), c(g), c(b))
    }

    /// Add the foreground colour to the palette (once). Returns whether it was added.
    pub fn add_to_palette(&mut self) -> bool {
        let c = self.foreground();
        if self.palette.contains(&c) || self.palette.len() >= PALETTE_MAX {
            return false;
        }
        self.palette.push(c);
        true
    }

    /// Remember the foreground colour as recently used when the picture changed since last time
    /// (something was painted, filled or drawn).
    pub fn track_recent(&mut self) {
        // History length, not the revision: boil playback changes the revision but paints nothing.
        let now = self.app.session.active().map(|st| (st.doc.id.0, st.history.past_len() as u64));
        if now != self.seen {
            let painted = matches!((self.seen, now), (Some((a, r0)), Some((b, r1))) if a == b && r1 > r0);
            self.seen = now;
            if painted {
                let c = self.foreground();
                self.recent.retain(|&x| x != c);
                self.recent.insert(0, c);
                self.recent.truncate(RECENT_MAX);
            }
        }
    }

    /// Paint a brush stroke through `paint.stroke` (agents and tests; pointer strokes go through
    /// PhotoCraft's canvas and Brush tool), on every boil frame when on a wiggle layer.
    pub fn stroke(&mut self, points: &[(f64, f64)], colour: &str, size: f64) -> Result<Value, String> {
        let points: Vec<Value> = points.iter().map(|&(x, y)| json!([x, y])).collect();
        let params = json!({"points": points, "color": colour, "size": size});
        if self.on_wiggle_layer() {
            let r = self.run("wiggle.apply", json!({"command": "paint.stroke", "params": params, "amount": self.wiggle_amount}));
            self.journal_seen = self.app.session.journal.len();
            r
        } else {
            self.run("paint.stroke", params)
        }
    }

    /// The path a save writes: `path`, else where the save dialog says, with extension `ext`.
    fn target(&mut self, path: Option<String>, ext: &str, also: &[&str]) -> Result<String, String> {
        let st = self.app.session.active().ok_or("there is no picture to save")?;
        let suggested = crate::io::with_extension(st.path.as_deref().unwrap_or(&st.doc.name), ext);
        let path = match path {
            Some(p) => p,
            None => self.app.services.pick_save.as_mut().and_then(|f| f(&suggested)).ok_or(CANCELLED)?,
        };
        let lower = path.to_ascii_lowercase();
        Ok(if std::iter::once(&ext).chain(also).any(|e| lower.ends_with(&format!(".{e}"))) { path } else { crate::io::with_extension(&path, ext) })
    }

    /// Save the picture as a Photoshop document: to `path` (made `.psd` if it isn't a PSD or PSB
    /// name), else where the save dialog says. Returns the path written.
    pub fn save_psd(&mut self, path: Option<String>) -> Result<String, String> {
        let path = self.target(path, "psd", &["psb"])?;
        self.app.save_as(Some(path)).map(|(p, _)| p)
    }

    /// Export a flattened PNG copy (the picture keeps its own file). Returns the path written.
    pub fn export_png(&mut self, path: Option<String>) -> Result<String, String> {
        let path = self.target(path, "png", &[])?;
        let doc = self.app.session.active().ok_or("there is no picture to export")?.doc.clone();
        let export = self.app.services.export.as_ref().ok_or("no exporter configured")?;
        let (bytes, _) = export(&doc, &path, &ExportSettings::default())?;
        let write = self.app.services.write.as_mut().ok_or("no writer configured")?;
        write(&path, &bytes)?;
        self.app.ui.status = format!("Exported {}", file_name(&path));
        self.app.ui.status_error = false;
        Ok(path)
    }

    /// The colours as saved between sessions.
    pub fn colours_json(&self) -> String {
        let list = |v: &[Color32]| v.iter().map(|&c| theme::to_hex(c)).collect::<Vec<_>>();
        json!({"palette": list(&self.palette), "recent": list(&self.recent)}).to_string()
    }

    /// Restore colours saved by [`Self::colours_json`]; anything unreadable is ignored.
    pub fn restore_colours(&mut self, text: &str) {
        let Ok(v) = serde_json::from_str::<Value>(text) else { return };
        let read = |k: &str, max: usize| -> Option<Vec<Color32>> {
            let list: Vec<Color32> = v.get(k)?.as_array()?.iter().filter_map(|c| hex(c.as_str()?)).take(max).collect();
            Some(list)
        };
        if let Some(p) = read("palette", PALETTE_MAX)
            && !p.is_empty()
        {
            self.palette = p;
        }
        if let Some(r) = read("recent", RECENT_MAX) {
            self.recent = r;
        }
    }

    /// Restore saved colours from eframe's storage.
    pub fn restore(&mut self, storage: Option<&dyn eframe::Storage>) {
        if let Some(text) = storage.and_then(|s| s.get_string(STORE_KEY)) {
            self.restore_colours(&text);
        }
        if let Some(text) = storage.and_then(|s| s.get_string(SETTINGS_KEY)) {
            self.restore_settings(&text);
        }
    }

    /// Show `error` on the status line (a cancelled dialog is not an error).
    fn report(&mut self, error: &str) {
        if error != CANCELLED {
            self.app.ui.status = error.to_string();
            self.app.ui.status_error = true;
        }
    }

    fn report_result<T>(&mut self, r: Result<T, String>) {
        if let Err(e) = r {
            self.report(&e);
        }
    }

    fn look(&self, ctx: &egui::Context) -> Look {
        Look { t: self.theme, frame: rough::boil_frame(ctx.input(|i| i.time), self.boiling) }
    }

    /// Keep the pixel font first in PhotoCraft's font stacks (PhotoCraft rebuilds them when it
    /// sets up and when the UI font size changes).
    fn ensure_font(&self, ctx: &egui::Context) {
        // PhotoCraft installs its fonts on its first frames; add ours on top once they're in.
        if self.frames < 2 || !self.pixel_font {
            return;
        }
        let present = ctx.fonts(|f| f.definitions().font_data.contains_key(ttf::FONT_NAME));
        if !present {
            let mut defs = ctx.fonts(|f| f.definitions().clone());
            ttf::install(&mut defs, FONT_SCALE);
            ctx.set_fonts(defs);
        }
    }

    /// The colour picker: a strip of blobs under the editor that opens into a card.
    fn colour_strip(&mut self, ui: &mut Ui, look: &Look) {
        let frame = egui::Frame::NONE.fill(self.theme.paper).inner_margin(egui::Margin { left: 10, right: 10, top: 6, bottom: 8 });
        let fg = self.foreground();
        let (recent, palette) = (self.recent.clone(), self.palette.clone());
        let narrow = ui.available_width() < NARROW;
        let picked = egui::Panel::bottom("wobble_colours")
            .show_separator_line(false)
            .frame(frame)
            .show(ui, |ui| {
                if narrow {
                    let p = self.picker.show(ui, look, fg, &recent, &palette);
                    ui.add_space(4.0);
                    self.wiggle_dock(ui, look);
                    p
                } else {
                    ui.horizontal_top(|ui| {
                        let dock_w = 470.0;
                        let strip_w = (ui.available_width() - dock_w - 16.0).max(200.0);
                        let p = ui.allocate_ui(vec2(strip_w, 60.0), |ui| self.picker.show(ui, look, fg, &recent, &palette)).inner;
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| self.wiggle_dock(ui, look));
                        p
                    })
                    .inner
                }
            })
            .inner;
        if let Some(c) = picked.colour {
            let r = self.set_foreground(c);
            self.report_result(r);
        }
        if picked.add {
            self.add_to_palette();
        }
        if let Some(i) = picked.remove
            && i < self.palette.len()
        {
            self.palette.remove(i);
        }
        if let Some(e) = picked.error {
            self.report(&e);
        }
    }

    /// Boil controls: play/pause, how much lines wander, a new wiggle layer, animation export.
    fn wiggle_dock(&mut self, ui: &mut Ui, look: &Look) {
        let avail = ui.available_width();
        widgets::card(ui, look, "wobble-wiggle-dock", 6.0, |ui| {
            ui.set_max_width((avail - 20.0).max(120.0));
            // Left to right even when the dock is pinned to the right; top-aligned so the row is
            // only as tall as its widgets.
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let label = if self.boil_play { "Boil: on" } else { "Boil: off" };
                if widgets::button(ui, look, label, self.boil_play, true).on_hover_text("Play the boil frames").clicked() {
                    self.boil_play = !self.boil_play;
                    self.shown_frame = None;
                }
                let mut amount = self.wiggle_amount;
                if widgets::slider(ui, look, "Wiggle", &mut amount, 0.0..=10.0, 120.0, |v| format!("{v:.0}")).dragged() {
                    self.wiggle_amount = amount.round();
                }
                if widgets::button(ui, look, "+ Layer", false, true).on_hover_text("A new wiggle layer: everything drawn on it boils").clicked() {
                    let r = self.new_wiggle_layer();
                    self.report_result(r);
                }
                if widgets::button(ui, look, "GIF", false, true).on_hover_text("Export the boil as an animated GIF").clicked() {
                    let r = self.export_gif(None);
                    self.report_result(r);
                }
                if widgets::button(ui, look, "PNGs", false, true).on_hover_text("Export each boil frame as a PNG").clicked() {
                    let r = self.export_png_sequence(None);
                    self.report_result(r);
                }
                if widgets::button(ui, look, "Settings", self.show_settings, true).on_hover_text("Sound, motion and Wob").clicked() {
                    self.show_settings = !self.show_settings;
                }
            });
        });
    }

    /// Reduce motion everywhere (the UI holds still; no pops, shake, particles or hops).
    pub fn set_reduce_motion(&mut self, on: bool) {
        self.reduce_motion = on;
        self.boiling = !on;
        self.juice.reduce_motion = on;
        self.mascot.reduce_motion = on;
    }

    /// The settings as saved between sessions.
    pub fn settings_json(&self) -> String {
        json!({
            "volume": self.audio.volume,
            "muted": self.audio.muted,
            "reduceMotion": self.reduce_motion,
            "mascot": self.mascot.enabled,
            "handDrawn": self.hand_drawn,
            "wiggle": self.wiggle_amount,
        })
        .to_string()
    }

    /// Restore settings saved by [`Self::settings_json`]; anything unreadable is ignored.
    pub fn restore_settings(&mut self, text: &str) {
        let Ok(v) = serde_json::from_str::<Value>(text) else { return };
        let num = |k: &str| v.get(k).and_then(Value::as_f64).filter(|x| x.is_finite());
        let flag = |k: &str| v.get(k).and_then(Value::as_bool);
        if let Some(x) = num("volume") {
            self.audio.volume = (x as f32).clamp(0.0, 1.0);
        }
        if let Some(b) = flag("muted") {
            self.audio.muted = b;
        }
        if let Some(b) = flag("reduceMotion") {
            self.set_reduce_motion(b);
        }
        if let Some(b) = flag("mascot") {
            self.mascot.enabled = b;
        }
        if let Some(b) = flag("handDrawn") {
            self.hand_drawn = b;
        }
        if let Some(x) = num("wiggle") {
            self.wiggle_amount = (x as f32).clamp(0.0, 10.0).round();
        }
    }

    /// The settings card above the dock.
    fn settings(&mut self, ctx: &egui::Context, look: &Look) {
        egui::Area::new(egui::Id::new("wobble-settings"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, vec2(-14.0, -84.0))
            .constrain_to(ctx.content_rect())
            .show(ctx, |ui| {
                widgets::card(ui, look, "wobble-settings-card", 14.0, |ui| {
                    ui.set_max_width(280.0);
                    widgets::label(ui, look, "Settings", 3.0, look.t.ink);
                    ui.add_space(4.0);
                    let mut v = self.audio.volume * 100.0;
                    if widgets::slider(ui, look, "Volume", &mut v, 0.0..=100.0, 250.0, |v| format!("{v:.0}")).dragged() {
                        self.audio.volume = v / 100.0;
                    }
                    let toggle = |ui: &mut Ui, label: &str, on: bool| widgets::button(ui, look, label, on, true).clicked();
                    ui.horizontal_wrapped(|ui| {
                        if toggle(ui, if self.audio.muted { "Muted" } else { "Sound on" }, !self.audio.muted) {
                            self.audio.muted = !self.audio.muted;
                            self.audio.play(Sound::Click);
                        }
                        if toggle(ui, "Reduce motion", self.reduce_motion) {
                            let on = !self.reduce_motion;
                            self.set_reduce_motion(on);
                        }
                        if toggle(ui, "Wob", self.mascot.enabled) {
                            self.mascot.enabled = !self.mascot.enabled;
                        }
                        if toggle(ui, "Hand-drawn UI", self.hand_drawn) {
                            self.hand_drawn = !self.hand_drawn;
                        }
                    });
                    ui.add_space(4.0);
                    if widgets::button(ui, look, "Done", false, true).clicked() {
                        self.show_settings = false;
                    }
                });
            });
    }

    /// Sounds, particles, shake and Wob's reactions to what just happened.
    fn effects(&mut self, ctx: &egui::Context, now: f64) {
        let pointer = ctx.pointer_latest_pos();
        let canvas = self.app.last_canvas_rect;
        let at = crate::juice::centre_or(pointer, canvas);
        let fg = self.foreground();
        let confetti = [self.theme.hot, self.theme.sun, self.theme.cool, fg, Color32::WHITE];
        if ctx.input(|i| !i.events.is_empty()) {
            self.mascot.input(now);
        }
        // Commands that just ran.
        let journal = &self.app.session.journal;
        let start = self.fx_seen.min(journal.len());
        let new: Vec<(String, Value)> = journal.get(start..).map(<[_]>::to_vec).unwrap_or_default();
        self.fx_seen = journal.len();
        for (id, p) in new {
            let inner = if id == "wiggle.apply" { p.get("command").and_then(Value::as_str).unwrap_or("").to_string() } else { id.clone() };
            match inner.as_str() {
                "edit.undo" => {
                    self.audio.play(Sound::Undo);
                    self.mascot.react(now, mascot::Event::Undo);
                    self.juice.splat(now, at, &[self.theme.dim, self.theme.cool], 10);
                }
                "edit.redo" => self.audio.play(Sound::Redo),
                "paint.bucket" | "edit.fill" | "paint.gradient" => {
                    self.audio.play(Sound::Pop);
                    self.juice.splat(now, at, &[fg, crate::theme::mix(fg, Color32::WHITE, 0.4)], 22);
                }
                "paint.stroke" | "paint.pencil" | "paint.mixerBrush" if id == "wiggle.apply" || !self.on_wiggle_layer() => {
                    if !self.drawn {
                        self.drawn = true;
                        self.audio.play(Sound::Chime);
                        self.juice.confetti(now, at, &confetti, 60);
                        self.mascot.react(now, mascot::Event::FirstStroke);
                    }
                }
                x if x.starts_with("stamp.") => {
                    self.audio.play(Sound::Pop);
                    self.juice.splat(now, at, &[fg], 12);
                }
                "layer.delete" | "edit.clear" | "layer.mergeDown" | "layer.mergeVisible" | "layer.flatten" | "image.flatten" | "layer.flattenImage" => {
                    self.audio.play(Sound::Thud);
                    self.juice.shake(now, 7.0);
                    self.mascot.react(now, mascot::Event::BigAction);
                }
                _ => {}
            }
        }
        // Saves and exports (ours and PhotoCraft's File menu both report on the status line).
        if self.app.ui.status != self.last_status {
            self.last_status = self.app.ui.status.clone();
            // Confetti bursts up out of the picture.
            let top = if canvas.is_finite() && canvas.width() > 0.0 {
                pos2(canvas.center().x, canvas.min.y + canvas.height() * 0.4)
            } else {
                ctx.content_rect().center()
            };
            if self.last_status.starts_with("Saved") {
                self.audio.play(Sound::Chime);
                self.juice.confetti(now, top, &confetti, 80);
                self.mascot.react(now, mascot::Event::Saved);
            } else if self.last_status.starts_with("Exported") {
                self.audio.play(Sound::Chime);
                self.juice.confetti(now, top, &confetti, 80);
                self.mascot.react(now, mascot::Event::Exported);
            }
        }
        // A new tool pops.
        let tool = self.app.ui.tool;
        if self.last_tool.is_some_and(|t| t != tool) {
            self.audio.play(Sound::Pop);
            // The button just clicked pops (keyboard shortcuts sparkle at the pointer instead).
            if let Some(p) = pointer.filter(|p| !canvas.contains(*p)) {
                self.juice.pop_at(now, Rect::from_center_size(p, vec2(36.0, 36.0)));
            }
            self.juice.splat(now, at, &[self.theme.sun, self.theme.cool], 8);
        }
        self.last_tool = Some(tool);
        if self.picker.open != self.picker_was_open {
            self.picker_was_open = self.picker.open;
            self.audio.play(Sound::Pop);
        }
        // Clicks off the canvas click; drawing on it scratches with the pointer's speed.
        let (clicked, down, origin, speed) =
            ctx.input(|i| (i.pointer.primary_clicked(), i.pointer.primary_down(), i.pointer.press_origin(), i.pointer.velocity().length()));
        let on_canvas = |p: Option<Pos2>| p.is_some_and(|p| canvas.contains(p));
        if clicked && !on_canvas(pointer) {
            self.audio.play(Sound::Click);
        }
        if down && on_canvas(origin) {
            self.audio.scratch(now, speed);
        }
    }

    /// Dots on the paper around the picture, and the picture's own wobbly outline and shadow.
    fn sheet(&self, ui: &Ui, look: &Look) {
        let Some(idx) = self.app.session.active_index() else { return };
        let (Some(view), Some(st)) = (self.app.ui.views.get(idx), self.app.session.active()) else { return };
        let area = self.app.last_canvas_rect;
        let zoom = view.zoom;
        // No room for a canvas (the panels fill a phone screen): nothing to decorate.
        if !zoom.is_finite() || zoom <= 0.0 || !area.is_finite() || area.width() < 120.0 || area.height() < 120.0 {
            return;
        }
        let size = vec2(st.doc.size.width as f32, st.doc.size.height as f32) * zoom;
        let img = Rect::from_min_size(area.center() - vec2(view.center[0], view.center[1]) * zoom, size);
        if !img.is_finite() {
            return;
        }
        let painter = ui.painter_at(area);
        // Dots everywhere but the picture: little square pixels on a fixed grid.
        let hole = img.expand(8.0);
        let dot = mix(look.t.ink, look.t.paper, 0.6);
        let step = 18.0;
        let mut y = (area.min.y / step).ceil() * step;
        while y < area.max.y {
            let mut x = (area.min.x / step).ceil() * step;
            while x < area.max.x {
                let p = pos2(x, y);
                if !hole.contains(p) {
                    painter.rect_filled(Rect::from_min_size(p, vec2(2.0, 2.0)), 0.0, dot);
                }
                x += step;
            }
            y += step;
        }
        // Hard shadow on the paper, right and below the picture, then a marker outline.
        let d = 7.0;
        painter.rect_filled(Rect::from_min_max(pos2(img.max.x, img.min.y + d), pos2(img.max.x + d, img.max.y + d)), 0.0, look.t.shadow);
        painter.rect_filled(Rect::from_min_max(pos2(img.min.x + d, img.max.y), pos2(img.max.x, img.max.y + d)), 0.0, look.t.shadow);
        let edge = Paint { fill: Color32::TRANSPARENT, ink: look.ink(2.5), shadow: None, radius: 3.0, wobble: 1.3 };
        rough::boxed(&painter, img.expand(2.0), &edge, 0x5ee7, look.frame);
    }
}

const CANCELLED: &str = "cancelled";

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

impl eframe::App for WobbleApp {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.logic(ctx, frame);
        // PhotoCraft (re)applies its own theme on its first frame and when preferences change.
        if !theme::is_applied(ctx, &self.theme) {
            theme::apply(ctx, &self.theme);
            let painter = self.custom_icons.then(|| svgicon::painter(self.icons.clone()));
            photocraft_ui_egui::icons::set_painter(ctx, painter);
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.app.raw_input_hook(ctx, raw_input);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(STORE_KEY, self.colours_json());
        storage.set_string(SETTINGS_KEY, self.settings_json());
    }

    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.frames += 1;
        if self.frames == 1 && ctx.content_rect().width() < NARROW {
            // The rail on the right still opens each panel.
            let p = &mut self.app.ui.panels;
            (p.layers, p.color, p.properties, p.navigator, p.history) = (false, false, false, false, false);
        }
        self.ensure_font(&ctx);
        let look = self.look(&ctx);
        if let Ok(mut s) = self.icons.lock() {
            s.0 = look.frame;
        }
        if self.boiling {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(rough::BOIL_SECONDS));
        }
        // Paper under everything, so wobbly panel edges show paper rather than a gap.
        ctx.layer_painter(egui::LayerId::background()).rect_filled(ctx.content_rect(), 0.0, self.theme.paper);
        self.colour_strip(ui, &look);
        // Hold the boil still while the pointer is down, so a stroke being drawn stays on screen.
        if !ctx.input(|i| i.pointer.any_down()) {
            self.play_boil(ctx.input(|i| i.time));
        }
        self.app.ui(ui, frame);
        self.spread_to_frames();
        self.track_recent();
        self.sheet(ui, &look);
        let now = ctx.input(|i| i.time);
        self.effects(&ctx, now);
        if self.show_settings {
            self.settings(&ctx, &look);
        }
        let canvas = self.app.last_canvas_rect;
        if canvas.width() > 200.0 && canvas.height() > 160.0 && self.mascot.show(&ctx, &look, now, canvas.right_bottom() - vec2(56.0, 14.0)) {
            self.audio.play(Sound::Boing);
            self.mascot.react(now, mascot::Event::Poked);
            self.juice.confetti(now, canvas.right_bottom() - vec2(56.0, 50.0), &[self.theme.hot, self.theme.sun, self.theme.cool], 24);
        }
        if self.hand_drawn {
            let pointer = ctx.pointer_hover_pos().map(|pos| handdrawn::Pointer {
                pos,
                pressed: ctx.input(|i| i.pointer.primary_down()),
                jiggle: if self.reduce_motion { look.frame } else { (now * 24.0) as u64 },
            });
            handdrawn::apply(&ctx, self.app.last_canvas_rect, look.frame, pointer);
        }
        let own =
            [egui::Id::new("wobble-colour-card"), egui::Id::new("wobble-mascot"), egui::Id::new("wobble-mascot-paint"), egui::Id::new("wobble-particles")];
        self.juice.apply(&ctx, now, &own);
        if self.mascot.enabled && !self.reduce_motion {
            // Wob breathes.
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}
