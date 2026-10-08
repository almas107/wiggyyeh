//! Top bar, tool panel, side cards, status bar and the dialogs.

use egui::{Color32, Frame, Margin, RichText, ScrollArea, Sense, Stroke, StrokeKind, Ui, vec2};

use crate::app::{App, Confirm};
use crate::geom::Mirror;
use crate::icons::Icon;
use crate::model::{Brush, MAX_FRAMES, MAX_SIZE, MIN_FRAMES, Playback, Tip};
use crate::pixels::{Blend, MAX_SIDE, MIN_SIDE, parse_hex, to_hex};
use crate::settings::{Action, Backdrop, PALETTES, Tool, palette};
use crate::theme::{self, PRESETS, mix};
use crate::widgets::{self, button, button_ex, card, hint, icon_button, icon_button_sized, row, swatch};

pub const SIZE_PRESETS: &[(usize, usize, &str)] =
    &[(512, 342, "Classic"), (640, 480, "Default"), (800, 600, "Big"), (480, 480, "Square"), (1080, 1080, "Post"), (1080, 1920, "Story"), (320, 240, "Tiny")];

impl App {
    pub fn top_bar(&mut self, ui: &mut Ui) {
        let look = self.look;
        ui.horizontal(|ui| {
            widgets::wobbly_title(ui, &look, "WobbleWorks", 22.0).on_hover_text("WobbleWorks: everything wiggles");
            ui.add_space(6.0);
            if icon_button_sized(ui, &look, Icon::Undo, false, "Undo (Ctrl+Z)", Some("Undo"), look.t.hot, 38.0, self.hist.can_undo()).clicked() {
                self.undo();
            }
            if icon_button_sized(ui, &look, Icon::Redo, false, "Redo (Ctrl+Shift+Z)", Some("Redo"), look.t.hot, 38.0, self.hist.can_redo()).clicked() {
                self.redo();
            }
            let (icon, tip) = if self.paused { (Icon::Play, "Play the boil (P)") } else { (Icon::Pause, "Pause the boil (P)") };
            if icon_button(ui, &look, icon, self.paused, tip, Some(if self.paused { "Play" } else { "Pause" }), look.t.hot).clicked() {
                self.paused = !self.paused;
            }
            ui.add_space(4.0);
            if icon_button(ui, &look, Icon::Image, false, "Import a picture (Ctrl+I) — or drop one on the window", Some("Import"), look.t.cool).clicked() {
                self.import();
            }
            let save = icon_button(ui, &look, Icon::Save, false, "Save PNG, GIF or a sprite sheet", Some("Export"), look.t.cool);
            egui::Popup::menu(&save).show(|ui| {
                ui.set_min_width(190.0);
                if button(ui, &look, "Save PNG (this frame)", false).clicked() {
                    self.export_png();
                    ui.close();
                }
                if button(ui, &look, "Save animated GIF", false).clicked() {
                    self.export_gif();
                    ui.close();
                }
                if button(ui, &look, "Save sprite sheet PNG", false).clicked() {
                    self.export_sheet();
                    ui.close();
                }
                ui.separator();
                row(ui, &look, "Scale", |ui| {
                    for k in [1u8, 2, 3, 4] {
                        if button(ui, &look, &format!("{k}×"), self.s.export_scale == k).clicked() {
                            self.s.export_scale = k;
                        }
                    }
                });
                hint(ui, &look, "Scaling keeps pixels crisp (nearest neighbour).");
            });

            // Right-hand group, laid out from the right edge.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, &look, Icon::Gear, self.show_settings, "Settings: make it yours", None, look.t.hot).clicked() {
                    self.show_settings = !self.show_settings;
                }
                if icon_button(ui, &look, Icon::Help, self.show_help, "Shortcuts and tips", None, look.t.hot).clicked() {
                    self.show_help = !self.show_help;
                }
                if icon_button(ui, &look, Icon::Focus, self.focus, "Hide panels (Tab)", None, look.t.hot).clicked() {
                    self.focus = !self.focus;
                }
                ui.add_space(6.0);
                if icon_button(ui, &look, Icon::Fit, false, "Fit to window (Ctrl+9)", None, look.t.hot).clicked() {
                    self.run(Action::ZoomFit);
                }
                if icon_button(ui, &look, Icon::ZoomIn, false, "Zoom in (Ctrl +)", None, look.t.hot).clicked() {
                    self.run(Action::ZoomIn);
                }
                let pct = format!("{:.0}%", self.view.zoom * 100.0);
                if button(ui, &look, &pct, false).on_hover_text("Actual pixels (Ctrl+0)").clicked() {
                    self.run(Action::Zoom100);
                }
                if icon_button(ui, &look, Icon::ZoomOut, false, "Zoom out (Ctrl −)", None, look.t.hot).clicked() {
                    self.run(Action::ZoomOut);
                }
            });
        });
    }

    pub fn tools_panel(&mut self, ui: &mut Ui) {
        let look = self.look;
        let tool = self.s.tools.tool;
        let brush = self.s.tools.brush;
        card(ui, &look, "tools", "Tools", true, |ui| {
            let tools = [
                (Icon::Brush(if brush == Brush::Eraser { self.s.tools.last_brush } else { brush }), Tool::Brush, "Brush (B)"),
                (Icon::Brush(Brush::Eraser), Tool::Brush, "Eraser (E)"),
                (Icon::Line, Tool::Line, "Line (L)"),
                (Icon::Rect, Tool::Rect, "Box (R)"),
                (Icon::Ellipse, Tool::Ellipse, "Oval (O)"),
                (Icon::Fill, Tool::Fill, "Fill (G)"),
                (Icon::Lasso, Tool::Lasso, "Lasso (S)"),
                (Icon::Move, Tool::Move, "Move (V)"),
                (Icon::Pick, Tool::Pick, "Pick colour (I)"),
                (Icon::Hand, Tool::Hand, "Hand (H, or hold Space)"),
            ];
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(5.0, 5.0);
                for (i, (icon, t, tip)) in tools.into_iter().enumerate() {
                    let eraser = i == 1;
                    let on = if eraser {
                        tool == Tool::Brush && brush == Brush::Eraser
                    } else if i == 0 {
                        tool == Tool::Brush && brush != Brush::Eraser
                    } else {
                        tool == t
                    };
                    let caption = if eraser { "Eraser" } else { t.label() };
                    if icon_button(ui, &look, icon, on, tip, Some(caption), self.s.tools.color).clicked() {
                        if eraser {
                            self.run(Action::Eraser);
                        } else if i == 0 {
                            self.run(Action::Brush);
                        } else {
                            self.set_tool(t);
                        }
                    }
                }
            });
        });
        card(ui, &look, "brushes", "Brushes", true, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(5.0, 5.0);
                for b in Brush::ALL {
                    if b == Brush::Eraser {
                        continue;
                    }
                    let tip = format!("{}: {}", b.label(), b.hint());
                    if icon_button(ui, &look, Icon::Brush(b), brush == b, &tip, Some(b.label()), self.s.tools.color).clicked() {
                        self.set_brush(b);
                    }
                }
            });
            hint(ui, &look, "Shapes (Line, Box, Oval) draw with the chosen brush. Blob fill + Box = a solid wobbly box.");
        });
        card(ui, &look, "tip", "Tip & symmetry", true, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(5.0, 5.0);
                for t in Tip::ALL {
                    if icon_button_sized(ui, &look, Icon::Tip(t), self.s.tools.tip == t, t.label(), None, self.s.tools.color, 30.0, true).clicked() {
                        self.s.tools.tip = t;
                    }
                }
            });
            egui::ComboBox::from_id_salt("mirror")
                .selected_text(format!("Symmetry: {}", self.s.tools.mirror.label()))
                .width(ui.available_width() - 8.0)
                .show_ui(ui, |ui| {
                    for m in Mirror::ALL {
                        ui.selectable_value(&mut self.s.tools.mirror, m, m.label());
                    }
                });
            row(ui, &look, "Steady", |ui| {
                ui.add(egui::Slider::new(&mut self.s.tools.stabilizer, 0..=10).show_value(true))
                    .on_hover_text("Smooths shaky hands: higher is smoother but lags a little.");
            });
            ui.checkbox(&mut self.s.tools.pressure, "Pen pressure changes size");
        });
    }

    pub fn side_panel(&mut self, ui: &mut Ui) {
        let look = self.look;
        self.paint_card(ui);
        if self.s.tools.tool == Tool::Fill {
            card(ui, &look, "fill", "Fill", true, |ui| {
                row(ui, &look, "Match", |ui| {
                    ui.add(egui::Slider::new(&mut self.s.tools.fill.tolerance, 0..=255)).on_hover_text("How different a colour can be and still get filled.")
                });
                row(ui, &look, "Grow", |ui| {
                    ui.add(egui::Slider::new(&mut self.s.tools.fill.grow, 0..=crate::fill::MAX_GROW).suffix(" px"))
                        .on_hover_text("Tuck the fill under the outline so no gaps show.")
                });
                ui.checkbox(&mut self.s.tools.fill.sample_all, "Look at all layers");
            });
        }
        if self.floating.is_some() {
            self.transform_card(ui);
        }
        self.layers_card(ui);
        self.canvas_card(ui);
        self.projects_card(ui);
    }

    fn paint_card(&mut self, ui: &mut Ui) {
        let look = self.look;
        card(ui, &look, "paint", "Paint", true, |ui| {
            ui.horizontal(|ui| {
                let mut c = self.s.tools.color;
                if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::Opaque).changed() {
                    self.set_color(c);
                }
                let mut text = self.hex_edit.clone().unwrap_or_else(|| to_hex(self.s.tools.color));
                let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(72.0).font(egui::TextStyle::Monospace));
                if r.changed() {
                    if let Some(c) = parse_hex(&text) {
                        self.s.tools.color = c;
                    }
                    self.hex_edit = Some(text);
                }
                if r.lost_focus() {
                    self.hex_edit = None;
                    let c = self.s.tools.color;
                    self.s.remember_color(c);
                }
                let mut size = self.s.tools.size;
                ui.add(egui::DragValue::new(&mut size).range(1.0..=MAX_SIZE).speed(0.3).suffix(" px")).on_hover_text("Brush size ( [ and ] )");
                self.s.tools.size = size.round().clamp(1.0, MAX_SIZE);
            });
            row(ui, &look, "Size", |ui| ui.add(egui::Slider::new(&mut self.s.tools.size, 1.0..=MAX_SIZE).logarithmic(true).show_value(false)));
            let mut wig = (self.doc.wiggle * 100.0).round() as i32;
            if row(ui, &look, "Wiggle", |ui| ui.add(egui::Slider::new(&mut wig, 0..=400).suffix("%"))).changed() {
                self.doc.wiggle = f64::from(wig) / 100.0;
                self.touched();
            }
            let mut spd = self.doc.speed_ms;
            if row(ui, &look, "Speed", |ui| ui.add(egui::Slider::new(&mut spd, 40..=600).suffix(" ms").logarithmic(true)))
                .on_hover_text("Time each frame stays on screen.")
                .changed()
            {
                self.doc.speed_ms = spd;
                self.touched();
            }
            row(ui, &look, "Frames", |ui| {
                let mut n = self.doc.frames;
                if ui.add(egui::Slider::new(&mut n, MIN_FRAMES..=MAX_FRAMES)).on_hover_text("How many drawings the boil cycles through.").changed() {
                    self.set_frames(n);
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("PLAY").size(10.5).color(look.t.dim));
                for p in Playback::ALL {
                    if button(ui, &look, p.label(), self.doc.playback == p).clicked() && self.doc.playback != p {
                        self.doc.playback = p;
                        self.touched();
                    }
                }
            });
            ui.add_space(2.0);
            // Palette: click to use, right-click to replace with the current colour.
            let cur = self.s.tools.color;
            let mut pick = None;
            let mut replace = None;
            let mut remove = None;
            let side = ((ui.available_width() - 7.0 * 4.0) / 8.0).clamp(14.0, 30.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                for (i, c) in self.s.palette.iter().enumerate() {
                    let r = swatch(ui, &look, *c, *c == cur, side)
                        .on_hover_text(format!("{}\nRight-click: replace with current colour\nMiddle-click: remove", to_hex(*c)));
                    if r.clicked() {
                        pick = Some(*c);
                    }
                    if r.secondary_clicked() {
                        replace = Some(i);
                    }
                    if r.middle_clicked() {
                        remove = Some(i);
                    }
                }
                if self.s.palette.len() < crate::settings::MAX_PALETTE
                    && icon_button_sized(ui, &look, Icon::Plus, false, "Add the current colour", None, look.t.hot, side - 3.0, true).clicked()
                {
                    self.s.palette.push(cur);
                }
            });
            if let Some(c) = pick {
                self.set_color(c);
            }
            if let Some(i) = replace
                && let Some(slot) = self.s.palette.get_mut(i)
            {
                *slot = cur;
            }
            if let Some(i) = remove
                && self.s.palette.len() > 1
                && i < self.s.palette.len()
            {
                self.s.palette.remove(i);
            }
            if !self.s.recent.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
                    ui.label(RichText::new("RECENT").size(10.0).color(look.t.dim));
                    let recent = self.s.recent.clone();
                    for c in recent {
                        if swatch(ui, &look, c, c == cur, (side * 0.7).max(12.0)).clicked() {
                            self.set_color(c);
                        }
                    }
                });
            }
        });
    }

    fn transform_card(&mut self, ui: &mut Ui) {
        let look = self.look;
        card(ui, &look, "xform", "Selection", true, |ui| {
            let Some(f) = &mut self.floating else { return };
            let mut scale = (f.xf.scale * 100.0).round() as i32;
            if row(ui, &look, "Scale", |ui| ui.add(egui::Slider::new(&mut scale, 5..=800).suffix("%").logarithmic(true))).changed() {
                f.xf.scale = (f64::from(scale) / 100.0).clamp(crate::select::MIN_SCALE, crate::select::MAX_SCALE);
            }
            let mut rot = f.xf.rot.round() as i32;
            if row(ui, &look, "Turn", |ui| ui.add(egui::Slider::new(&mut rot, -180..=180).suffix("°"))).changed() {
                f.xf.rot = f64::from(rot);
            }
            let mut act = None;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(5.0, 5.0);
                if icon_button_sized(ui, &look, Icon::FlipH, f.xf.fh, "Flip left ↔ right", None, look.t.hot, 30.0, true).clicked() {
                    f.xf.fh = !f.xf.fh;
                }
                if icon_button_sized(ui, &look, Icon::FlipV, f.xf.fv, "Flip top ↕ bottom", None, look.t.hot, 30.0, true).clicked() {
                    f.xf.fv = !f.xf.fv;
                }
                if button(ui, &look, "Reset", false).on_hover_text("Undo the move/scale/turn (Esc)").clicked() {
                    f.xf.reset();
                }
            });
            ui.horizontal_wrapped(|ui| {
                if button(ui, &look, "Apply", true).on_hover_text("Bake into the layer (Enter)").clicked() {
                    act = Some(0);
                }
                if button(ui, &look, "Stamp", false).on_hover_text("Bake a copy and keep the selection floating").clicked() {
                    act = Some(1);
                }
                if button(ui, &look, "Recolor", false).on_hover_text("Paint the selection with the current colour").clicked() {
                    act = Some(2);
                }
                if button(ui, &look, "Delete", false).on_hover_text("Throw it away (Delete)").clicked() {
                    act = Some(3);
                }
            });
            hint(ui, &look, "Drag it on the canvas; arrow keys nudge (Shift = 10 px).");
            match act {
                Some(0) => self.run(Action::Apply),
                Some(1) => self.stamp_floating(),
                Some(2) => {
                    let c = self.s.tools.color;
                    if let Some(f) = &mut self.floating {
                        f.recolor(c);
                    }
                    self.say("Recolored.");
                }
                Some(3) => self.delete_floating(),
                _ => {}
            }
        });
    }

    fn layers_card(&mut self, ui: &mut Ui) {
        let look = self.look;
        card(ui, &look, "layers", "Layers", true, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(5.0, 5.0);
                let side = 30.0;
                if icon_button_sized(ui, &look, Icon::Plus, false, "New layer (Ctrl+Shift+N)", None, look.t.hot, side, true).clicked() {
                    self.new_layer();
                }
                if icon_button_sized(ui, &look, Icon::Copy, false, "Duplicate layer (it boils on its own)", None, look.t.hot, side, true).clicked() {
                    self.duplicate_layer();
                }
                if icon_button_sized(ui, &look, Icon::Merge, false, "Merge down", None, look.t.hot, side, self.doc.current > 0).clicked() {
                    self.merge_down();
                }
                if icon_button_sized(ui, &look, Icon::Clear, false, "Clear layer", None, look.t.hot, side, true).clicked() {
                    self.clear_layer();
                }
                if icon_button_sized(ui, &look, Icon::Trash, false, "Delete layer", None, look.t.hot, side, self.doc.layers.len() > 1).clicked() {
                    if self.s.confirm_delete && !self.doc.layer().is_some_and(crate::model::Layer::is_empty) {
                        self.confirm = Some(Confirm::DeleteLayer);
                    } else {
                        self.delete_layer();
                    }
                }
            });
            let n = self.doc.layers.len();
            let mut moved: Option<(usize, usize)> = None;
            let mut select = None;
            let mut restyle = false;
            for i in (0..n).rev() {
                let is_cur = i == self.doc.current;
                let fill = if is_cur { mix(look.t.cool, look.t.card, 0.25) } else { mix(look.t.paper, look.t.card, 0.4) };
                let frame = Frame::new().fill(fill).stroke(Stroke::new(look.line, look.t.ink)).corner_radius(look.radius()).inner_margin(Margin::same(6));
                let resp = frame
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let Some(l) = self.doc.layers.get_mut(i) else { return };
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                            // Drag handle.
                            let (hr, hresp) = ui.allocate_exact_size(vec2(12.0, 22.0), Sense::drag());
                            for k in 0..3 {
                                for c in 0..2 {
                                    ui.painter().circle_filled(hr.min + vec2(3.0 + c as f32 * 5.0, 6.0 + k as f32 * 5.0), 1.4, look.t.dim);
                                }
                            }
                            hresp.dnd_set_drag_payload(i);
                            let eye = if l.visible { Icon::Eye } else { Icon::EyeOff };
                            if icon_button_sized(ui, &look, eye, false, if l.visible { "Hide" } else { "Show" }, None, look.t.hot, 24.0, true).clicked() {
                                l.visible = !l.visible;
                                restyle = true;
                            }
                            let renaming = self.rename.as_ref().is_some_and(|(id, _)| *id == l.id);
                            if renaming {
                                if let Some((_, text)) = &mut self.rename {
                                    let r = ui.add(egui::TextEdit::singleline(text).desired_width(ui.available_width() - 64.0));
                                    r.request_focus();
                                    if r.lost_focus() {
                                        let t = text.trim().chars().take(80).collect::<String>();
                                        if !t.is_empty() {
                                            l.name = t;
                                        }
                                        self.rename = None;
                                        restyle = true;
                                    }
                                }
                            } else {
                                let name = ui
                                    .add(egui::Label::new(RichText::new(&l.name).strong()).truncate().sense(Sense::click()))
                                    .on_hover_text("Click to select, double-click to rename");
                                if name.clicked() {
                                    select = Some(i);
                                }
                                if name.double_clicked() {
                                    self.rename = Some((l.id, l.name.clone()));
                                }
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if i > 0
                                    && icon_button_sized(
                                        ui,
                                        &look,
                                        Icon::Clip,
                                        l.clip,
                                        "Clip to the layer below: paint only shows where it has paint",
                                        None,
                                        look.t.hot,
                                        24.0,
                                        true,
                                    )
                                    .clicked()
                                {
                                    l.clip = !l.clip;
                                    restyle = true;
                                }
                                if icon_button_sized(
                                    ui,
                                    &look,
                                    Icon::Lock,
                                    l.alpha_lock,
                                    "Alpha lock: paint only lands on pixels already there",
                                    None,
                                    look.t.hot,
                                    24.0,
                                    true,
                                )
                                .clicked()
                                {
                                    l.alpha_lock = !l.alpha_lock;
                                }
                            });
                        });
                        if is_cur {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().slider_width = (ui.available_width() - 160.0).max(40.0);
                                let mut op = (l.opacity * 100.0).round() as i32;
                                if ui.add(egui::Slider::new(&mut op, 0..=100).suffix("%").text("")).changed() {
                                    l.opacity = op as f32 / 100.0;
                                    restyle = true;
                                }
                                egui::ComboBox::from_id_salt(("blend", l.id)).selected_text(l.blend.label()).width(84.0).show_ui(ui, |ui| {
                                    for b in Blend::ALL {
                                        if ui.selectable_value(&mut l.blend, b, b.label()).changed() {
                                            restyle = true;
                                        }
                                    }
                                });
                            });
                            ui.horizontal(|ui| {
                                if i + 1 < n && icon_button_sized(ui, &look, Icon::Up, false, "Move up", None, look.t.hot, 22.0, true).clicked() {
                                    moved = Some((i, i + 1));
                                }
                                if i > 0 && icon_button_sized(ui, &look, Icon::Down, false, "Move down", None, look.t.hot, 22.0, true).clicked() {
                                    moved = Some((i, i - 1));
                                }
                                let n = l.strokes.len();
                                let info = format!("{n} stroke{}", if n == 1 { "" } else { "s" });
                                ui.label(RichText::new(info).small().color(look.t.dim));
                            });
                        }
                    })
                    .response;
                let resp = resp.interact(Sense::click());
                if resp.clicked() {
                    select = Some(i);
                }
                if let Some(from) = resp.dnd_release_payload::<usize>() {
                    moved = Some((*from, i));
                }
                if resp.dnd_hover_payload::<usize>().is_some() {
                    ui.painter().rect_stroke(resp.rect.expand(2.0), look.radius(), Stroke::new(2.5, look.t.hot), StrokeKind::Outside);
                }
            }
            if let Some(i) = select {
                self.select_layer(i);
            }
            if let Some((a, b)) = moved {
                self.move_layer(a, b);
            }
            if restyle {
                self.restyled();
            }
        });
    }

    fn canvas_card(&mut self, ui: &mut Ui) {
        let look = self.look;
        card(ui, &look, "canvas", "Canvas", false, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (w, h, name) in SIZE_PRESETS {
                    let on = (self.doc.w, self.doc.h) == (*w, *h);
                    if button(ui, &look, name, on).on_hover_text(format!("{w} × {h}")).clicked() {
                        self.resize_canvas(*w, *h);
                    }
                }
            });
            let id = egui::Id::new("canvas-size-edit");
            let mut wh = ui.data_mut(|d| *d.get_temp_mut_or(id, (self.doc.w, self.doc.h)));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut wh.0).range(MIN_SIDE..=MAX_SIDE));
                ui.label("×");
                ui.add(egui::DragValue::new(&mut wh.1).range(MIN_SIDE..=MAX_SIDE));
                if button(ui, &look, "Set", false).clicked() {
                    self.resize_canvas(wh.0, wh.1);
                }
            });
            ui.data_mut(|d| d.insert_temp(id, wh));
            ui.horizontal(|ui| {
                let mut bg = self.doc.bg;
                if egui::color_picker::color_edit_button_srgba(ui, &mut bg, egui::color_picker::Alpha::Opaque).changed() {
                    self.doc.bg = bg;
                    self.restyled();
                }
                ui.label("Background");
                let mut t = self.doc.transparent;
                if ui.checkbox(&mut t, "See-through").changed() {
                    self.doc.transparent = t;
                    self.restyled();
                }
            });
            hint(ui, &look, "Resizing keeps your drawing at the top-left. It's undoable.");
        });
    }

    fn projects_card(&mut self, ui: &mut Ui) {
        let look = self.look;
        card(ui, &look, "projects", "Projects", true, |ui| {
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut self.project.name).desired_width(ui.available_width()).hint_text("Name your drawing"));
                if r.changed() {
                    self.project.name = self.project.name.chars().take(80).collect();
                    self.touched();
                }
            });
            ui.horizontal_wrapped(|ui| {
                if button(ui, &look, "+ New", false).clicked() {
                    self.show_new = true;
                }
                if button(ui, &look, "Save now", false).on_hover_text("Ctrl+S (it autosaves anyway)").clicked() {
                    self.run(Action::Save);
                }
                if button(ui, &look, "Export .wob", false).on_hover_text("Keep a copy somewhere safe, or share it").clicked() {
                    self.export_wob();
                }
                if button(ui, &look, "Open .wob", false).on_hover_text("Opens files from WobbleWorks, old and new").clicked() {
                    self.open_wob();
                }
            });
            let state = match (&self.project.error, self.project.dirty_since, self.project.saved_at) {
                (Some(e), _, _) => format!("Save problem: {e}"),
                (None, Some(_), _) => "Unsaved changes… (autosaving)".into(),
                (None, None, Some(_)) => "All changes saved.".into(),
                _ => "Not saved yet.".into(),
            };
            hint(ui, &look, &state);
            let list = self.projects.clone();
            let ctx = ui.ctx().clone();
            for p in &list {
                let cur = p.id == self.project.id;
                let fill = if cur { mix(look.t.cool, look.t.card, 0.25) } else { mix(look.t.paper, look.t.card, 0.4) };
                Frame::new().fill(fill).stroke(Stroke::new(look.line, look.t.ink)).corner_radius(look.radius()).inner_margin(Margin::same(5)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let tex = self.thumb_texture(&ctx, p);
                        let (r, _) = ui.allocate_exact_size(vec2(46.0, 34.0), Sense::hover());
                        ui.painter().rect_filled(r, 4.0, Color32::WHITE);
                        if let Some(t) = tex {
                            let sz = t.size_vec2();
                            let k = (r.width() / sz.x).min(r.height() / sz.y);
                            ui.painter().image(
                                t.id(),
                                egui::Rect::from_center_size(r.center(), sz * k),
                                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                        ui.painter().rect_stroke(r, 4.0, Stroke::new(1.0, look.t.ink), StrokeKind::Inside);
                        ui.vertical(|ui| {
                            ui.add(egui::Label::new(RichText::new(&p.name).strong()).truncate());
                            ui.label(RichText::new(format!("{}×{} · {}", p.w, p.h, ago(p.modified))).small().color(look.t.dim));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icon_button_sized(ui, &look, Icon::Trash, false, "Delete this project", None, look.t.hot, 24.0, true).clicked() {
                                self.confirm = Some(Confirm::DeleteProject(p.id.clone(), p.name.clone()));
                            }
                            if !cur && icon_button_sized(ui, &look, Icon::Folder, false, "Open", None, look.t.hot, 24.0, true).clicked() {
                                self.switch_project(&p.id);
                            }
                        });
                    });
                });
            }
            hint(ui, &look, &format!("Saved in {}.", crate::store::Store::location()));
        });
    }

    fn thumb_texture(&mut self, ctx: &egui::Context, p: &crate::store::ProjectMeta) -> Option<egui::TextureHandle> {
        if let Some((m, t)) = self.thumbs.get(&p.id)
            && *m == p.modified
        {
            return Some(t.clone());
        }
        let bytes = crate::project::base64_decode(p.thumb.as_deref()?)?;
        let img = crate::project::decode_image(&bytes).ok()?;
        let tex = ctx.load_texture(format!("thumb-{}", p.id), egui::ColorImage::new([img.w, img.h], img.px), egui::TextureOptions::LINEAR);
        self.thumbs.insert(p.id.clone(), (p.modified, tex.clone()));
        Some(tex)
    }

    pub fn status_bar(&mut self, ui: &mut Ui) {
        let look = self.look;
        ui.horizontal(|ui| {
            // Frame dots: click to scrub.
            for f in 0..self.doc.frames {
                let (r, resp) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
                let on = f == self.frame;
                ui.painter().circle(
                    r.center(),
                    if on { 5.5 } else { 4.0 },
                    if on { look.t.hot } else { look.t.card },
                    Stroke::new(look.line.min(2.0), look.t.ink),
                );
                if resp.on_hover_text(format!("Frame {} (click to hold it)", f + 1)).clicked() {
                    self.paused = true;
                    self.frame = f;
                }
            }
            ui.add_space(8.0);
            ui.label(RichText::new(&self.status).color(look.t.ink));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{} × {} · {} frames · {} layers", self.doc.w, self.doc.h, self.doc.frames, self.doc.layers.len()))
                        .small()
                        .color(look.t.dim),
                );
                if self.s.tools.mirror != Mirror::Off {
                    ui.label(RichText::new(format!("Symmetry: {}", self.s.tools.mirror.label())).small().color(look.t.hot));
                }
            });
        });
    }

    pub fn dialogs(&mut self, ctx: &egui::Context) {
        let look = self.look;
        if self.show_settings {
            let mut open = true;
            egui::Window::new("Settings").open(&mut open).default_width(420.0).resizable(true).collapsible(false).show(ctx, |ui| self.settings_ui(ui));
            self.show_settings = open;
        }
        if self.show_help {
            let mut open = true;
            egui::Window::new("Shortcuts & tips").open(&mut open).default_width(380.0).collapsible(false).show(ctx, |ui| {
                ScrollArea::vertical().max_height(460.0).show(ui, |ui| {
                    egui::Grid::new("keys").num_columns(2).striped(false).show(ui, |ui| {
                        for a in Action::ALL {
                            ui.label(a.label());
                            let keys: Vec<String> = self.s.bindings_for(a).iter().map(crate::settings::Binding::label).collect();
                            ui.label(RichText::new(keys.join("  /  ")).monospace().color(look.t.hot));
                            ui.end_row();
                        }
                        for (what, how) in [
                            ("Pan", "Space-drag, middle-drag, scroll, two fingers"),
                            ("Zoom", "Ctrl+scroll, pinch"),
                            ("Nudge selection", "Arrow keys (Shift ×10)"),
                            ("Cancel / reset", "Esc"),
                        ] {
                            ui.label(what);
                            ui.label(RichText::new(how).color(look.t.dim));
                            ui.end_row();
                        }
                    });
                    ui.separator();
                    hint(ui, &look, "Everything wiggles: each stroke is drawn a few times with tiny differences, then played in a loop. Fills are worked out per frame so they boil with their outlines.");
                    hint(ui, &look, "Change any shortcut in Settings → Keys.");
                });
            });
            self.show_help = open;
        }
        if self.show_new {
            let mut open = true;
            let mut create = None;
            egui::Window::new("New drawing").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (w, h, name) in SIZE_PRESETS {
                        if button(ui, &look, &format!("{name}  {w}×{h}"), (self.s.new_w, self.s.new_h) == (*w, *h)).clicked() {
                            self.s.new_w = *w;
                            self.s.new_h = *h;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut self.s.new_w).range(MIN_SIDE..=MAX_SIDE));
                    ui.label("×");
                    ui.add(egui::DragValue::new(&mut self.s.new_h).range(MIN_SIDE..=MAX_SIDE));
                });
                if button(ui, &look, "Start drawing!", true).clicked() {
                    create = Some((self.s.new_w, self.s.new_h));
                }
            });
            if let Some((w, h)) = create {
                self.new_project(w, h);
                open = false;
            }
            self.show_new = open;
        }
        if let Some(c) = &self.confirm {
            let (title, msg) = match c {
                Confirm::DeleteProject(_, name) => ("Delete project?", format!("Delete \"{name}\"? This can't be undone.")),
                Confirm::DeleteLayer => ("Delete layer?", "Delete this layer? (Undo brings it back.)".to_string()),
            };
            let mut answer = None;
            egui::Modal::new(egui::Id::new("confirm")).show(ctx, |ui| {
                ui.heading(title);
                ui.label(msg);
                ui.horizontal(|ui| {
                    if button(ui, &look, "Yes", true).clicked() {
                        answer = Some(true);
                    }
                    if button(ui, &look, "No", false).clicked() {
                        answer = Some(false);
                    }
                });
            });
            if let Some(yes) = answer {
                if let Some(c) = self.confirm.take()
                    && yes
                {
                    match c {
                        Confirm::DeleteProject(id, _) => self.delete_project(&id),
                        Confirm::DeleteLayer => self.delete_layer(),
                    }
                }
                self.confirm = None;
            }
        }
    }

    fn settings_ui(&mut self, ui: &mut Ui) {
        let look = self.look;
        ui.horizontal(|ui| {
            for (i, name) in ["Look", "Layout", "Canvas", "Palette", "Keys"].iter().enumerate() {
                if button(ui, &look, name, self.settings_tab == i as u8).clicked() {
                    self.settings_tab = i as u8;
                }
            }
        });
        ui.separator();
        ScrollArea::vertical().max_height(520.0).show(ui, |ui| match self.settings_tab {
            0 => {
                ui.label(RichText::new("Theme").strong());
                ui.horizontal_wrapped(|ui| {
                    for (name, t) in PRESETS {
                        let r = theme_chip(ui, &look, name, t, self.s.preset == *name);
                        if r.clicked() {
                            self.s.preset = (*name).into();
                            self.s.theme = *t;
                        }
                    }
                });
                ui.add_space(4.0);
                ui.label(RichText::new("Colours (make your own)").strong());
                let t = &mut self.s.theme;
                let mut changed = false;
                egui::Grid::new("tokens").num_columns(4).show(ui, |ui| {
                    let fields: [(&str, &mut Color32); 8] = [
                        ("Ink", &mut t.ink),
                        ("Paper", &mut t.paper),
                        ("Cards", &mut t.card),
                        ("Accent", &mut t.hot),
                        ("Selected", &mut t.cool),
                        ("Toggled", &mut t.sun),
                        ("Quiet text", &mut t.dim),
                        ("Shadow", &mut t.shadow),
                    ];
                    for (k, (name, c)) in fields.into_iter().enumerate() {
                        changed |= egui::color_picker::color_edit_button_srgba(ui, c, egui::color_picker::Alpha::Opaque).changed();
                        ui.label(name);
                        if k % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
                changed |= ui.checkbox(&mut t.dark, "Dark widgets").changed();
                if changed {
                    self.s.preset = "Custom".into();
                }
                ui.add_space(4.0);
                ui.label(RichText::new("Shape").strong());
                row(ui, &look, "Round", |ui| ui.add(egui::Slider::new(&mut self.s.roundness, 0.0..=24.0)));
                row(ui, &look, "Outline", |ui| ui.add(egui::Slider::new(&mut self.s.outline, 0.5..=5.0)));
                row(ui, &look, "Shadow", |ui| ui.add(egui::Slider::new(&mut self.s.shadow, 0.0..=10.0)));
                row(ui, &look, "Text", |ui| ui.add(egui::Slider::new(&mut self.s.text_size, 9.0..=22.0)));
                row(ui, &look, "UI size", |ui| ui.add(egui::Slider::new(&mut self.s.ui_scale, 0.6..=2.5).suffix("×")));
                ui.checkbox(&mut self.s.wobbly_ui, "Boiling UI (outlines wiggle with the drawing)");
                ui.checkbox(&mut self.s.reduce_motion, "Reduce motion (UI holds still; the drawing still plays)");
            }
            1 => {
                ui.checkbox(&mut self.s.show_labels, "Show tool names under icons");
                ui.checkbox(&mut self.s.swap_sides, "Tools on the right, panels on the left");
                row(ui, &look, "Panels", |ui| ui.add(egui::Slider::new(&mut self.s.panel_width, 200.0..=480.0).suffix(" px")));
                ui.checkbox(&mut self.s.confirm_delete, "Ask before deleting layers");
                hint(ui, &look, "Click any card's title to fold it away. Tab hides all panels.");
            }
            2 => {
                ui.label(RichText::new("Backdrop").strong());
                ui.horizontal_wrapped(|ui| {
                    for b in Backdrop::ALL {
                        if button(ui, &look, b.label(), self.s.backdrop == b).clicked() {
                            self.s.backdrop = b;
                        }
                    }
                });
                ui.horizontal(|ui| {
                    egui::color_picker::color_edit_button_srgba(ui, &mut self.s.checker_a, egui::color_picker::Alpha::Opaque);
                    egui::color_picker::color_edit_button_srgba(ui, &mut self.s.checker_b, egui::color_picker::Alpha::Opaque);
                    ui.label("See-through checkerboard");
                });
                ui.checkbox(&mut self.s.pixel_grid, "Pixel grid when zoomed in (800%+)");
                ui.checkbox(&mut self.s.brush_cursor, "Show brush size around the cursor");
            }
            3 => {
                ui.label(RichText::new("Load a palette").strong());
                ui.horizontal_wrapped(|ui| {
                    for (name, _) in PALETTES {
                        if button(ui, &look, name, false).clicked() {
                            self.s.palette = palette(name);
                        }
                    }
                });
                hint(
                    ui,
                    &look,
                    "In the Paint card: click a swatch to use it, right-click to replace it with the current colour, middle-click to remove it, + to add.",
                );
                if button(ui, &look, "Clear recent colours", false).clicked() {
                    self.s.recent.clear();
                }
            }
            _ => {
                hint(ui, &look, "Click a shortcut, then press the new key combination (Esc cancels).");
                egui::Grid::new("rebind").num_columns(2).show(ui, |ui| {
                    for a in Action::ALL {
                        ui.label(a.label());
                        let keys: Vec<String> = self.s.bindings_for(a).iter().map(crate::settings::Binding::label).collect();
                        let text = if self.rebinding == Some(a) {
                            "press a key…".to_string()
                        } else if keys.is_empty() {
                            "—".into()
                        } else {
                            keys.join(" / ")
                        };
                        if button_ex(ui, &look, &text, self.rebinding == Some(a), true).clicked() {
                            self.rebinding = Some(a);
                        }
                        ui.end_row();
                    }
                });
                if button(ui, &look, "Restore default shortcuts", false).clicked() {
                    self.s.keys = crate::settings::default_keys();
                }
            }
        });
        ui.separator();
        if button(ui, &look, "Reset everything to defaults", false).clicked() {
            let tools = self.s.tools.clone();
            self.s = crate::settings::Settings { tools, ..Default::default() };
        }
        self.s.sanitize();
    }
}

fn theme_chip(ui: &mut Ui, look: &crate::theme::Look, name: &str, t: &theme::Theme, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(118.0, 46.0), Sense::click());
    let p = ui.painter();
    let r = rect.shrink(2.0);
    p.rect_filled(r.translate(vec2(3.0, 3.0)), look.radius(), t.shadow);
    p.rect(r, look.radius(), t.paper, Stroke::new(if on { 3.0 } else { look.line }, if on { look.t.hot } else { t.ink }), StrokeKind::Inside);
    for (k, c) in [t.card, t.hot, t.cool, t.sun].iter().enumerate() {
        p.circle(r.left_bottom() + vec2(12.0 + k as f32 * 15.0, -11.0), 6.0, *c, Stroke::new(1.5, t.ink));
    }
    p.text(r.left_top() + vec2(8.0, 6.0), egui::Align2::LEFT_TOP, name, egui::FontId::proportional(12.0), t.ink);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// "5 min ago"-style time.
fn ago(ms: f64) -> String {
    let s = ((crate::platform::now_ms() - ms) / 1000.0).max(0.0);
    match s {
        s if s < 60.0 => "just now".into(),
        s if s < 3600.0 => format!("{} min ago", (s / 60.0) as u32),
        s if s < 86_400.0 => format!("{} h ago", (s / 3600.0) as u32),
        s => format!("{} days ago", (s / 86_400.0) as u32),
    }
}
