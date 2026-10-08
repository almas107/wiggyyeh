//! The WobbleWorks window: a WigglyPaint-style screen around PhotoCraft's editor.
//!
//! Simple mode puts PhotoCraft in its canvas-only screen mode (View › Screen Mode › Full Screen
//! Mode) and surrounds the canvas with WobbleWorks' own hand-painted chrome: the top bar (file
//! buttons, undo, the editor switch), a tool strip, and a paint dock (colours, brush size and
//! opacity). "Advanced editor" switches PhotoCraft back to its standard screen mode with every
//! menu, tool and panel, recoloured to match. Both modes share one session, so nothing is lost
//! when switching. Everything the chrome does is a PhotoCraft command or tool.

use egui::{Color32, Rect, Ui, Vec2, pos2, vec2};
use photocraft_engine::Session;
use photocraft_engine::prefs::{CanvasBorder, CanvasColor, Theme as PrefTheme};
use photocraft_ui_egui::state::Tool;
use photocraft_ui_egui::{ExportSettings, PhotocraftApp, Services};
use serde_json::{Value, json};

use crate::icons::Icon;
use crate::pixfont;
use crate::rough::{self, Paint};
use crate::theme::{self, Theme, mix};
use crate::widgets::{self, Look, TEXT};

/// The canvas a new picture gets.
pub const NEW_SIZE: (u32, u32) = (1200, 800);

/// The paint dock's colours: (name, colour).
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

/// The tool strip: PhotoCraft tools with their icons. Every other tool is in the "More" drawer.
pub const TOOLS: [(Tool, Icon); 20] = [
    (Tool::Brush, Icon::Brush),
    (Tool::Pencil, Icon::Pencil),
    (Tool::Eraser, Icon::Eraser),
    (Tool::PaintBucket, Icon::Fill),
    (Tool::Gradient, Icon::Gradient),
    (Tool::Smudge, Icon::Smudge),
    (Tool::Blur, Icon::Blur),
    (Tool::CloneStamp, Icon::Stamp),
    (Tool::Lasso, Icon::Lasso),
    (Tool::RectMarquee, Icon::Marquee),
    (Tool::MagicWand, Icon::Wand),
    (Tool::Move, Icon::Move),
    (Tool::Type, Icon::Text),
    (Tool::Rectangle, Icon::Rect),
    (Tool::EllipseShape, Icon::Ellipse),
    (Tool::Line, Icon::Line),
    (Tool::Eyedropper, Icon::Pick),
    (Tool::Hand, Icon::Hand),
    (Tool::Zoom, Icon::Zoom),
    (Tool::Crop, Icon::Crop),
];

/// PhotoCraft's screen modes this shell switches between (see `photocraft_ui_egui::view_cmds`).
const SIMPLE_SCREEN: &str = "fullScreen";
const ADVANCED_SCREEN: &str = "standard";
/// Below this width the tool strip moves into the bottom dock (phones).
const NARROW: f32 = 720.0;

pub struct WobbleApp {
    /// PhotoCraft's editor, which owns the engine session.
    pub app: PhotocraftApp,
    /// Index into [`PALETTE`] of the colour last picked, `None` after a custom colour.
    pub colour: Option<usize>,
    pub theme: Theme,
    /// Boiling outlines (off holds the UI still).
    pub boiling: bool,
    /// The "More tools" drawer.
    pub show_tools: bool,
    /// The colour mixer popup.
    pub show_mixer: bool,
}

/// A tool's short name ("Brush Tool" → "Brush").
pub fn tool_name(tool: Tool) -> &'static str {
    let l = tool.label();
    l.strip_suffix(" Tool").unwrap_or(l)
}

impl WobbleApp {
    /// A WobbleWorks window over a fresh PhotoCraft session: simple mode, a blank picture and the
    /// Brush tool.
    pub fn new(services: Services) -> Self {
        let theme = Theme::default();
        let mut w = Self { app: PhotocraftApp::new(Session::new(), services), colour: Some(0), theme, boiling: true, show_tools: false, show_mixer: false };
        w.app.ui.theme = theme::base_kind(&theme);
        w.style_canvas();
        w.set_simple(true);
        if let Err(e) = w.new_picture() {
            w.report(&e);
        }
        if let Err(e) = w.pick_colour(0) {
            w.report(&e);
        }
        w
    }

    /// PhotoCraft's canvas preferences for the paper look: the theme's paper around the image (our
    /// own sheet outline and shadow replace PhotoCraft's border), and the light layout.
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
        self.colour = Some(index);
        Ok(())
    }

    /// The foreground colour, for display.
    pub fn foreground(&self) -> Color32 {
        let [r, g, b, _] = self.app.session.tools.foreground;
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Color32::from_rgb(c(r), c(g), c(b))
    }

    /// Set the brush size (px) or opacity (0–1) through `tools.setBrush`; a drag is one gesture.
    pub fn set_brush(&mut self, key: &str, value: f32) -> Result<(), String> {
        self.run("tools.setBrush", json!({key: value, "coalesce": format!("wobble-{key}")})).map(|_| ())
    }

    /// Paint a brush stroke through `paint.stroke` (what agents and tests use; pointer strokes go
    /// through PhotoCraft's canvas and Brush tool).
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

    fn panel_frame(&self) -> egui::Frame {
        egui::Frame::NONE.fill(self.theme.paper).inner_margin(egui::Margin { left: 12, right: 12, top: 8, bottom: 8 })
    }

    /// The top bar: logo, file buttons, undo/redo and the editor switch.
    fn top_bar(&mut self, ui: &mut Ui, look: &Look, narrow: bool) {
        let frame = self.panel_frame();
        egui::Panel::top("wobble_top").show_separator_line(false).frame(frame).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            ui.horizontal_wrapped(|ui| {
                widgets::logo(ui, look, if narrow { 2.0 } else { 3.0 });
                ui.add_space(6.0);
                let t = |s: &'static str| if narrow { "" } else { s };
                if widgets::button(ui, look, Some(Icon::New), t("New"), false, true).on_hover_text("A fresh picture").clicked() {
                    let r = self.new_picture();
                    self.report_result(r);
                }
                if widgets::button(ui, look, Some(Icon::Folder), t("Open"), false, true).on_hover_text("Open a PSD or an image").clicked() {
                    self.app.open_dialog_file();
                }
                if widgets::button(ui, look, Some(Icon::Save), t("Save"), false, true).on_hover_text("Save as a Photoshop document (.psd)").clicked() {
                    let r = self.save_psd(None);
                    self.report_result(r);
                }
                if widgets::button(ui, look, Some(Icon::Export), t("Export"), false, true).on_hover_text("Export a PNG").clicked() {
                    let r = self.export_png(None);
                    self.report_result(r);
                }
                ui.add_space(6.0);
                for (id, icon, tip) in [("edit.undo", Icon::Undo, "Undo"), ("edit.redo", Icon::Redo, "Redo")] {
                    let enabled = self.app.session.is_enabled(id);
                    if widgets::button(ui, look, Some(icon), "", false, enabled).on_hover_text(tip).clicked() {
                        let _ = self.run(id, json!({}));
                    }
                }
                ui.add_space(6.0);
                let simple = self.is_simple();
                let label = if simple { t("Advanced") } else { t("Simple") };
                if widgets::button(ui, look, Some(Icon::Sliders), label, !simple, true)
                    .on_hover_text(if simple { "Every PhotoCraft menu, tool and panel" } else { "Back to the cosy screen" })
                    .clicked()
                {
                    self.set_simple(!simple);
                }
            });
        });
    }

    /// The tool strip (two columns of painted tiles) on wide screens.
    fn tool_strip(&mut self, ui: &mut Ui, look: &Look) {
        let frame = egui::Frame::NONE.fill(self.theme.paper).inner_margin(egui::Margin { left: 12, right: 6, top: 4, bottom: 8 });
        egui::Panel::left("wobble_tools").show_separator_line(false).resizable(false).exact_size(138.0).frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical().scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
                widgets::card(ui, look, "wobble-tools-card", 10.0, |ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    egui::Grid::new("wobble-tool-grid").spacing(vec2(6.0, 6.0)).show(ui, |ui| {
                        for (i, (tool, icon)) in TOOLS.iter().enumerate() {
                            self.tool_tile(ui, look, *tool, *icon);
                            if i % 2 == 1 {
                                ui.end_row();
                            }
                        }
                        if widgets::tile(ui, look, Icon::More, self.show_tools, 44.0, true).on_hover_text("All the tools").clicked() {
                            self.show_tools = !self.show_tools;
                        }
                    });
                });
            });
        });
    }

    fn tool_tile(&mut self, ui: &mut Ui, look: &Look, tool: Tool, icon: Icon) {
        if widgets::tile(ui, look, icon, self.app.ui.tool == tool, 44.0, true).on_hover_text(tool_name(tool)).clicked() {
            self.app.ui.tool = tool;
        }
    }

    /// The paint dock: current colour, palette, brush size and opacity, tool name, status. On
    /// narrow screens the tools ride along in a scrolling row.
    fn paint_dock(&mut self, ui: &mut Ui, look: &Look, narrow: bool) {
        let frame = self.panel_frame();
        egui::Panel::bottom("wobble_dock").show_separator_line(false).frame(frame).show(ui, |ui| {
            if narrow {
                egui::ScrollArea::horizontal().id_salt("wobble-tool-row").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(
                    ui,
                    |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            for (tool, icon) in TOOLS {
                                self.tool_tile(ui, look, tool, icon);
                            }
                            if widgets::tile(ui, look, Icon::More, self.show_tools, 44.0, true).clicked() {
                                self.show_tools = !self.show_tools;
                            }
                        });
                    },
                );
                ui.add_space(4.0);
            }
            let avail = ui.available_width();
            widgets::card(ui, look, "wobble-dock-card", 10.0, |ui| {
                ui.set_max_width((avail - 30.0).max(100.0));
                ui.spacing_mut().item_spacing = vec2(10.0, 8.0);
                ui.horizontal_wrapped(|ui| {
                    let fg = self.foreground();
                    if widgets::swatch(ui, look, fg, self.show_mixer, 20.0).on_hover_text("Mix a colour").clicked() {
                        self.show_mixer = !self.show_mixer;
                    }
                    ui.add_space(4.0);
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (i, (name, hex)) in PALETTE.iter().enumerate() {
                        let c = Color32::from_hex(hex).unwrap_or(Color32::BLACK);
                        if widgets::swatch(ui, look, c, self.colour == Some(i), 12.0).on_hover_text(*name).clicked() {
                            let r = self.pick_colour(i);
                            self.report_result(r);
                        }
                    }
                    ui.spacing_mut().item_spacing.x = 10.0;
                    ui.add_space(8.0);
                    let slider_w = if narrow { (avail - 60.0).clamp(120.0, 260.0) } else { 190.0 };
                    let mut size = self.app.session.tools.brush.size;
                    if widgets::slider(ui, look, "Size", &mut size, 1.0..=200.0, slider_w, |v| format!("{v:.0}")).changed_or_dragged() {
                        let r = self.set_brush("size", size.round().max(1.0));
                        self.report_result(r);
                    }
                    let mut opacity = self.app.session.tools.brush.opacity * 100.0;
                    if widgets::slider(ui, look, "Opacity", &mut opacity, 1.0..=100.0, slider_w, |v| format!("{v:.0}%")).changed_or_dragged() {
                        let r = self.set_brush("opacity", (opacity / 100.0).clamp(0.01, 1.0));
                        self.report_result(r);
                    }
                    self.status(ui, look);
                });
            });
        });
    }

    /// The tool name, picture switcher and status message.
    fn status(&mut self, ui: &mut Ui, look: &Look) {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            widgets::label(ui, look, tool_name(self.app.ui.tool), TEXT, look.t.hot);
            let n = self.app.session.documents().len();
            if n > 1
                && let Some(i) = self.app.session.active_index()
            {
                ui.horizontal(|ui| {
                    if widgets::label(ui, look, "<", TEXT, look.t.ink).interact(egui::Sense::click()).on_hover_text("Previous picture").clicked() {
                        self.app.session.set_active((i + n - 1) % n);
                    }
                    widgets::label(ui, look, &format!("{}/{}", i + 1, n), TEXT, look.t.dim);
                    if widgets::label(ui, look, ">", TEXT, look.t.ink).interact(egui::Sense::click()).on_hover_text("Next picture").clicked() {
                        self.app.session.set_active((i + 1) % n);
                    }
                });
            }
            if !self.app.ui.status.is_empty() {
                let w = ui.available_width().clamp(120.0, 360.0);
                let text = pixfont::fit(&self.app.ui.status, TEXT, w);
                let c = if self.app.ui.status_error { look.t.hot } else { look.t.dim };
                widgets::label(ui, look, &text, TEXT, c).on_hover_text(self.app.ui.status.clone());
            }
        });
    }

    /// The "More tools" drawer: every PhotoCraft tool by name.
    fn tool_drawer(&mut self, ctx: &egui::Context, look: &Look) {
        let screen = ctx.content_rect();
        let w = (screen.width() - 32.0).clamp(200.0, 620.0);
        egui::Area::new(egui::Id::new("wobble-tool-drawer")).order(egui::Order::Foreground).anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
            widgets::card(ui, look, "wobble-drawer-card", 14.0, |ui| {
                ui.set_max_width(w);
                ui.horizontal(|ui| {
                    widgets::label(ui, look, "All the tools", 3.0, look.t.ink);
                });
                ui.add_space(6.0);
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                let max_h = (screen.height() - 200.0).max(120.0);
                egui::ScrollArea::vertical().max_height(max_h).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.set_max_width(w);
                        for tool in Tool::ALL {
                            if widgets::button(ui, look, None, tool_name(tool), self.app.ui.tool == tool, true).clicked() {
                                self.app.ui.tool = tool;
                                self.show_tools = false;
                            }
                        }
                    });
                });
                ui.add_space(6.0);
                if widgets::button(ui, look, None, "Close", false, true).clicked() {
                    self.show_tools = false;
                }
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_tools = false;
        }
    }

    /// The colour mixer popup above the dock.
    fn mixer(&mut self, ctx: &egui::Context, look: &Look) {
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::new("wobble-mixer"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::LEFT_BOTTOM, vec2(16.0, -110.0))
            .constrain_to(screen)
            .show(ctx, |ui| {
                widgets::card(ui, look, "wobble-mixer-card", 12.0, |ui| {
                    widgets::label(ui, look, "Mix a colour", TEXT, look.t.ink);
                    ui.add_space(4.0);
                    let mut c = self.foreground();
                    ui.spacing_mut().slider_width = 220.0;
                    if egui::color_picker::color_picker_color32(ui, &mut c, egui::color_picker::Alpha::Opaque) {
                        let r = self.run("tools.setColors", json!({"foreground": theme::to_hex(c)}));
                        self.report_result(r);
                        self.colour = None;
                    }
                    ui.add_space(4.0);
                    if widgets::button(ui, look, None, "Done", false, true).clicked() {
                        self.show_mixer = false;
                    }
                });
            });
    }

    /// Dots on the paper around the picture, and the picture's own wobbly outline and shadow.
    fn sheet(&mut self, ui: &Ui, look: &Look) {
        let Some(idx) = self.app.session.active_index() else { return };
        let (Some(view), Some(st)) = (self.app.ui.views.get(idx), self.app.session.active()) else { return };
        let area = self.app.last_canvas_rect;
        let zoom = view.zoom;
        if !zoom.is_finite() || zoom <= 0.0 || !area.is_finite() {
            return;
        }
        let size = vec2(st.doc.size.width as f32, st.doc.size.height as f32) * zoom;
        let min = area.center() - vec2(view.center[0], view.center[1]) * zoom;
        let img = Rect::from_min_size(min, size);
        if !img.is_finite() {
            return;
        }
        let painter = ui.painter_at(area);
        // Dots everywhere but the picture (and its outline): little square pixels on a fixed grid.
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
        // Hard shadow on the paper, right and below the picture.
        let d = 8.0;
        painter.rect_filled(Rect::from_min_max(pos2(img.max.x, img.min.y + d), pos2(img.max.x + d, img.max.y + d)), 0.0, look.t.shadow);
        painter.rect_filled(Rect::from_min_max(pos2(img.min.x + d, img.max.y), pos2(img.max.x, img.max.y + d)), 0.0, look.t.shadow);
        let edge = Paint { fill: Color32::TRANSPARENT, ink: look.ink(3.0), shadow: None, radius: 3.0, wobble: 1.4 };
        rough::boxed(&painter, img.expand(2.5), &edge, 0x5ee7, look.frame);
    }
}

const CANCELLED: &str = "cancelled";

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

trait ChangedOrDragged {
    fn changed_or_dragged(&self) -> bool;
}

impl ChangedOrDragged for egui::Response {
    fn changed_or_dragged(&self) -> bool {
        self.dragged() || self.clicked()
    }
}

impl eframe::App for WobbleApp {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.logic(ctx, frame);
        // PhotoCraft (re)applies its own theme on its first frame and when preferences change.
        if !theme::is_applied(ctx, &self.theme) {
            theme::apply(ctx, &self.theme);
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.app.raw_input_hook(ctx, raw_input);
    }

    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let look = self.look(&ctx);
        if self.boiling {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(rough::BOIL_SECONDS));
        }
        let narrow = ui.available_width() < NARROW;
        self.top_bar(ui, &look, narrow);
        let simple = self.is_simple();
        if simple {
            self.paint_dock(ui, &look, narrow);
            if !narrow {
                self.tool_strip(ui, &look);
            }
        }
        self.app.ui(ui, frame);
        if simple {
            self.sheet(ui, &look);
            if self.show_tools {
                self.tool_drawer(&ctx, &look);
            }
            if self.show_mixer {
                self.mixer(&ctx, &look);
            }
        }
    }
}
