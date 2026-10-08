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

use egui::{Color32, Rect, Ui, pos2, vec2};
use photocraft_engine::Session;
use photocraft_engine::prefs::{CanvasBorder, CanvasColor, Theme as PrefTheme};
use photocraft_ui_egui::state::Tool;
use photocraft_ui_egui::{ExportSettings, PhotocraftApp, Services};
use serde_json::{Value, json};

use crate::colour::ColourPicker;
use crate::rough::{self, Paint};
use crate::svgicon::{self, IconInk};
use crate::theme::{self, Theme, mix};
use crate::widgets::Look;
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
    /// The boil frame and colours the icon painter reads.
    icons: Arc<Mutex<(u64, IconInk)>>,
    /// (document, revision) last seen, to notice painting.
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

    /// File › New with WobbleWorks' canvas size, on white, then the Brush tool.
    pub fn new_picture(&mut self) -> Result<(), String> {
        self.run("file.new", json!({"width": NEW_SIZE.0, "height": NEW_SIZE.1, "background": "white", "name": "Wobble"}))?;
        self.app.ui.tool = Tool::Brush;
        Ok(())
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
        let now = self.app.session.active().map(|st| (st.doc.id.0, st.revision));
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
    /// PhotoCraft's canvas and Brush tool).
    pub fn stroke(&mut self, points: &[(f64, f64)], colour: &str, size: f64) -> Result<Value, String> {
        let points: Vec<Value> = points.iter().map(|&(x, y)| json!([x, y])).collect();
        self.run("paint.stroke", json!({"points": points, "color": colour, "size": size}))
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
        let picked = egui::Panel::bottom("wobble_colours")
            .show_separator_line(false)
            .frame(frame)
            .show(ui, |ui| self.picker.show(ui, look, fg, &recent, &palette))
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
        self.app.ui(ui, frame);
        self.track_recent();
        self.sheet(ui, &look);
        if self.hand_drawn {
            handdrawn::apply(&ctx, self.app.last_canvas_rect, look.frame);
        }
    }
}
