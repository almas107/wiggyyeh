//! The WobbleWorks window: its own bar over PhotoCraft's editor.
//!
//! Simple mode puts PhotoCraft in its canvas-only screen mode (View › Screen Mode › Full Screen
//! Mode), so the window shows the bar and the picture. "Advanced editor" switches PhotoCraft back
//! to its standard screen mode, with every menu, tool and panel. Both modes share one session, so
//! nothing is lost when switching. Stage 1 keeps the bar plain; the WigglyPaint look comes later
//! (`docs/wobbleworks-spec.md`).

use photocraft_engine::Session;
use photocraft_ui_egui::state::Tool;
use photocraft_ui_egui::{PhotocraftApp, Services};
use serde_json::{Value, json};

/// The canvas a new picture gets.
pub const NEW_SIZE: (u32, u32) = (1200, 800);

/// The palette on the bar: (name, colour).
pub const PALETTE: [(&str, &str); 8] = [
    ("Ink", "#17161c"),
    ("Pink", "#ff2e88"),
    ("Orange", "#ff8a1f"),
    ("Yellow", "#ffd23f"),
    ("Green", "#2ec27e"),
    ("Blue", "#2f7bff"),
    ("Purple", "#8a4dff"),
    ("Paper", "#ffffff"),
];

/// PhotoCraft's screen modes this shell switches between (see `photocraft_ui_egui::view_cmds`).
const SIMPLE_SCREEN: &str = "fullScreen";
const ADVANCED_SCREEN: &str = "standard";

pub struct WobbleApp {
    /// PhotoCraft's editor, which owns the engine session.
    pub app: PhotocraftApp,
    /// Index into [`PALETTE`] of the colour last picked on the bar.
    pub colour: usize,
}

impl WobbleApp {
    /// A WobbleWorks window over a fresh PhotoCraft session: simple mode, a blank picture and the
    /// Brush tool.
    pub fn new(services: Services) -> Self {
        let mut w = Self { app: PhotocraftApp::new(Session::new(), services), colour: 0 };
        w.set_simple(true);
        if let Err(e) = w.new_picture() {
            w.report(&e);
        }
        if let Err(e) = w.pick_colour(0) {
            w.report(&e);
        }
        w
    }

    /// Is the simple (canvas-only) screen showing, rather than the advanced editor?
    pub fn is_simple(&self) -> bool {
        self.app.ui.view.screen_mode == SIMPLE_SCREEN
    }

    pub fn set_simple(&mut self, simple: bool) {
        self.app.ui.view.screen_mode = if simple { SIMPLE_SCREEN } else { ADVANCED_SCREEN }.into();
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

    /// Make `PALETTE[index]` the foreground colour.
    pub fn pick_colour(&mut self, index: usize) -> Result<(), String> {
        let (_, hex) = PALETTE.get(index).ok_or_else(|| format!("no palette colour {index}"))?;
        self.run("tools.setColors", json!({"foreground": hex}))?;
        self.colour = index;
        Ok(())
    }

    /// Paint a brush stroke through `paint.stroke` (what agents and tests use; pointer strokes go
    /// through PhotoCraft's canvas and Brush tool).
    pub fn stroke(&mut self, points: &[(f64, f64)], colour: &str, size: f64) -> Result<Value, String> {
        let points: Vec<Value> = points.iter().map(|&(x, y)| json!([x, y])).collect();
        self.run("paint.stroke", json!({"points": points, "color": colour, "size": size}))
    }

    /// Save the picture as a Photoshop document: to `path` (made `.psd` if it isn't a PSD or PSB
    /// name), else where the save dialog says. Returns the path written.
    pub fn save_psd(&mut self, path: Option<String>) -> Result<String, String> {
        let st = self.app.session.active().ok_or("there is no picture to save")?;
        let suggested = crate::io::psd_name(st.path.as_deref().unwrap_or(&st.doc.name));
        let path = match path {
            Some(p) => p,
            None => self.app.services.pick_save.as_mut().and_then(|f| f(&suggested)).ok_or(CANCELLED)?,
        };
        let lower = path.to_ascii_lowercase();
        let path = if lower.ends_with(".psd") || lower.ends_with(".psb") { path } else { crate::io::psd_name(&path) };
        self.app.save_as(Some(path)).map(|(p, _)| p)
    }

    /// Show `error` on the status line (a cancelled dialog is not an error).
    fn report(&mut self, error: &str) {
        if error != CANCELLED {
            self.app.ui.status = error.to_string();
            self.app.ui.status_error = true;
        }
    }

    /// WobbleWorks' own bar: file buttons, tools, palette, undo and the editor switch.
    fn bar(&mut self, ui: &mut egui::Ui) {
        let avail = ui.available_width();
        egui::Panel::top("wobble_bar").show(ui, |ui| {
            ui.set_max_width(avail);
            ui.horizontal_wrapped(|ui| {
                ui.strong("WobbleWorks");
                ui.separator();
                if ui.button("New").on_hover_text("A fresh picture").clicked()
                    && let Err(e) = self.new_picture()
                {
                    self.report(&e);
                }
                if ui.button("Open…").on_hover_text("Open a PSD or an image").clicked() {
                    self.app.open_dialog_file();
                }
                if ui.button("Save PSD…").on_hover_text("Save as a Photoshop document").clicked()
                    && let Err(e) = self.save_psd(None)
                {
                    self.report(&e);
                }
                ui.separator();
                for (tool, label) in [(Tool::Brush, "Brush"), (Tool::Eraser, "Eraser")] {
                    if ui.selectable_label(self.app.ui.tool == tool, label).clicked() {
                        self.app.ui.tool = tool;
                    }
                }
                ui.separator();
                for (i, (name, hex)) in PALETTE.iter().enumerate() {
                    let fill = egui::Color32::from_hex(hex).unwrap_or(egui::Color32::BLACK);
                    let size = egui::vec2(22.0, 22.0);
                    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
                    let stroke_w = if i == self.colour { 3.0 } else { 1.0 };
                    ui.painter().rect(rect.shrink(1.0), 6.0, fill, egui::Stroke::new(stroke_w, ui.visuals().strong_text_color()), egui::StrokeKind::Inside);
                    if resp.on_hover_text(*name).clicked()
                        && let Err(e) = self.pick_colour(i)
                    {
                        self.report(&e);
                    }
                }
                ui.separator();
                for (id, label) in [("edit.undo", "Undo"), ("edit.redo", "Redo")] {
                    let enabled = self.app.session.is_enabled(id);
                    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                        let _ = self.run(id, json!({}));
                    }
                }
                ui.separator();
                let simple = self.is_simple();
                let label = if simple { "Advanced editor" } else { "Simple mode" };
                if ui.button(label).on_hover_text("Every PhotoCraft menu, tool and panel").clicked() {
                    self.set_simple(!simple);
                }
                if simple && !self.app.ui.status.is_empty() {
                    ui.separator();
                    let colour = if self.app.ui.status_error { ui.visuals().error_fg_color } else { ui.visuals().weak_text_color() };
                    ui.add(egui::Label::new(egui::RichText::new(&self.app.ui.status).color(colour)).truncate());
                }
            });
        });
    }
}

const CANCELLED: &str = "cancelled";

impl eframe::App for WobbleApp {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.logic(ctx, frame);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.app.raw_input_hook(ctx, raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.bar(ui);
        self.app.ui(ui, frame);
    }
}
