//! The 3D mode's panels: the header (menus), the toolbar, Feather's brush panel and context bar,
//! the sidebar (Stage with Groups / Resources / Environment, Boil, Shots, Item, History, Keys,
//! Help) and the popup menus. Every control runs editor commands; tooltips name the shortcut.

use egui::{Color32, Pos2, Rect, RichText, Ui, pos2, vec2};
use serde_json::{Value, json};
use wobbleworks_3d::editor::{Session, Tool};
use wobbleworks_3d::guide::Primitive;
use wobbleworks_3d::model::{BrushKind, Material, PatternKind, ResourceState, SIZE_MAX_MM, SIZE_MIN_MM};

use super::{IMAGE_EXTS, MODEL_EXTS, Menu, NOTE_EXTS, SideTab, Space3d, StageTab, colour32};
use crate::svgicon::{self, IconInk};
use crate::widgets::{self, Look};

fn ink(look: &Look) -> IconInk {
    IconInk { ink: look.t.ink, wash: crate::theme::mix(look.t.cool, look.t.card, 0.7), card: look.t.card }
}

/// A square tool tile with a hand-drawn icon.
fn tile(ui: &mut Ui, look: &Look, icon: &str, on: bool, tip: &str) -> egui::Response {
    let ink = ink(look);
    let frame = look.frame;
    let name = icon.to_string();
    widgets::sample_tile(ui, look, on, 38.0, move |p, r| {
        if !svgicon::paint(p, r.expand(4.0), &name, ink.ink, ink, frame) {
            p.circle_filled(r.center(), 4.0, ink.ink);
        }
    })
    .on_hover_text(tip)
}

/// The header: menus, the note's name and a way back to the 2D editor.
pub fn header(s: &mut Space3d, ui: &mut Ui, look: &Look) {
    let frame = egui::Frame::NONE.fill(look.t.paper).inner_margin(egui::Margin { left: 8, right: 8, top: 4, bottom: 4 });
    let (mut leave, mut toggle_render) = (false, false);
    let render = s.ed.scene.environment.render_mode;
    let render_tip = s.tip("Render mode: materials, light and effects", "ui.render");
    egui::Panel::top("w3d_header").show_separator_line(false).frame(frame).show(ui, |ui| {
        egui::Sides::new().show(
            ui,
            |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.menu_button("File", |ui| file_menu(s, ui));
                    ui.menu_button("Edit", |ui| edit_menu(s, ui));
                    ui.menu_button("View", |ui| view_menu(s, ui));
                    ui.menu_button("Add", |ui| add_menu(s, ui));
                    ui.menu_button("Select", |ui| {
                        for (label, cmd, action) in [
                            ("All", "select.all", "select.all"),
                            ("None", "select.none", "select.none"),
                            ("Invert", "select.invert", "select.invert"),
                            ("Group under the mouse", "select.linkedUnderMouse", "select.linked"),
                        ] {
                            if ui.button(s.tip(label, action)).clicked() {
                                s.run(cmd, Value::Null);
                                ui.close();
                            }
                        }
                    });
                    ui.menu_button("Help", |ui| {
                        if ui.button("Shortcuts and tips").clicked() {
                            s.side = SideTab::Help;
                            s.show_side = true;
                            ui.close();
                        }
                        if ui.button("Search commands  (F3)").clicked() {
                            s.menu = Some((Menu::Search(String::new()), ui.ctx().pointer_latest_pos().unwrap_or_default()));
                            ui.close();
                        }
                    });
                });
                ui.separator();
                let name = s.name.as_deref().map_or_else(|| "Untitled".to_string(), crate::shell::file_name);
                let dirty = if s.ed.dirty { " •" } else { "" };
                ui.label(RichText::new(format!("{name}{dirty}")).color(look.t.dim));
            },
            |ui| {
                if widgets::button(ui, look, "2D", false, true).on_hover_text("Back to the 2D picture (the 3D note stays as it is)").clicked() {
                    leave = true;
                }
                if widgets::button(ui, look, "Render", render, true).on_hover_text(&render_tip).clicked() {
                    toggle_render = true;
                }
            },
        );
    });
    if leave {
        s.leave = true;
    }
    if toggle_render {
        s.run("env.toggleRender", Value::Null);
    }
}

fn file_menu(s: &mut Space3d, ui: &mut Ui) {
    if ui.button("New note").clicked() {
        s.run("file.new", Value::Null);
        s.name = None;
        ui.close();
    }
    if ui.button(s.tip("Open…", "file.open")).clicked() {
        s.pick("open", NOTE_EXTS);
        ui.close();
    }
    if ui.button(s.tip("Save", "file.save")).clicked() {
        s.save(false);
        ui.close();
    }
    if ui.button(s.tip("Save as…", "file.saveAs")).clicked() {
        s.save(true);
        ui.close();
    }
    ui.separator();
    if ui.button("Import reference image…").clicked() {
        s.pick("image", IMAGE_EXTS);
        ui.close();
    }
    if ui.button("Import 3D model (OBJ)…").clicked() {
        s.pick("model", MODEL_EXTS);
        ui.close();
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Image size");
        for k in 1..=4u32 {
            if ui.selectable_label(s.export_scale == k, format!("{k}×")).clicked() {
                s.export_scale = k;
            }
        }
    });
    ui.checkbox(&mut s.export_transparent, "Transparent background");
    if ui.button(s.tip("Export PNG", "file.render")).clicked() {
        s.export_png();
        ui.close();
    }
    if ui.button(s.tip("Export boil GIF", "file.renderAnim")).clicked() {
        s.export_gif(false);
        ui.close();
    }
    if ui.button("Export 360° turntable GIF").clicked() {
        s.export_gif(true);
        ui.close();
    }
    if ui.button("Export OBJ (tubes, vertex colours)").clicked() {
        s.export_obj();
        ui.close();
    }
    if ui.button("Export glTF (.glb)").clicked() {
        s.export_glb();
        ui.close();
    }
}

fn edit_menu(s: &mut Space3d, ui: &mut Ui) {
    let undo = s.ed.undo_label().map_or_else(|| "Undo".to_string(), |l| format!("Undo {l}"));
    if ui.add_enabled(s.ed.can_undo(), egui::Button::new(s.tip(&undo, "edit.undo"))).clicked() {
        s.run("edit.undo", Value::Null);
        ui.close();
    }
    let redo = s.ed.redo_label().map_or_else(|| "Redo".to_string(), |l| format!("Redo {l}"));
    if ui.add_enabled(s.ed.can_redo(), egui::Button::new(s.tip(&redo, "edit.redo"))).clicked() {
        s.run("edit.redo", Value::Null);
        ui.close();
    }
    ui.separator();
    let sel = s.ed.has_selection();
    for (label, cmd, params, action) in [
        ("Duplicate and move", "edit.duplicate", json!({"mode": "move"}), "edit.duplicate"),
        ("Duplicate in place", "edit.duplicate", json!({"mode": "inplace"}), ""),
        ("Duplicate symmetrically by view", "edit.duplicate", json!({"mode": "view"}), "edit.duplicateView"),
        ("Duplicate by mirror", "edit.duplicate", json!({"mode": "mirror"}), "edit.duplicateMirror"),
        ("Delete", "edit.delete", Value::Null, "edit.deleteNow"),
    ] {
        if ui.add_enabled(sel, egui::Button::new(s.tip(label, action))).clicked() {
            s.run(cmd, params);
            ui.close();
        }
    }
    ui.menu_button("Mirror selection", |ui| {
        for a in ["x", "y", "z"] {
            if ui.button(a.to_uppercase()).clicked() {
                s.run("edit.flip", json!({"axis": a}));
                ui.close();
            }
        }
    });
    ui.separator();
    if ui.button("Keyboard shortcuts…").clicked() {
        s.side = SideTab::Keys;
        s.show_side = true;
        ui.close();
    }
}

fn view_menu(s: &mut Space3d, ui: &mut Ui) {
    for (label, view, action) in [
        ("Front", "front", "view.front"),
        ("Back", "back", "view.back"),
        ("Right", "right", "view.right"),
        ("Left", "left", "view.left"),
        ("Top", "top", "view.top"),
        ("Bottom", "bottom", "view.bottom"),
        ("Nearest", "nearest", "view.nearest"),
    ] {
        if ui.button(s.tip(label, action)).clicked() {
            s.run("camera.view", json!({"view": view}));
            ui.close();
        }
    }
    ui.separator();
    for (label, cmd, action) in [
        ("Perspective / orthographic", "camera.toggleProjection", "view.projection"),
        ("Frame selected", "camera.frameSelected", "view.frameSelected"),
        ("Frame all", "camera.frameAll", "view.frameAll"),
        ("Reset view", "camera.reset", "view.reset"),
    ] {
        if ui.button(s.tip(label, action)).clicked() {
            s.run(cmd, Value::Null);
            ui.close();
        }
    }
    let mut mm = s.ed.camera.focal_mm;
    if ui.add(egui::Slider::new(&mut mm, 10.0..=500.0).logarithmic(true).text("Lens mm")).changed() {
        s.run("camera.fov", json!({"mm": mm}));
    }
    ui.separator();
    let mut grid = s.ed.scene.environment.show_grid;
    if ui.checkbox(&mut grid, "Grid").changed() {
        s.run("env.set", json!({"grid": grid}));
    }
    let mut axes = s.ed.scene.environment.show_axes;
    if ui.checkbox(&mut axes, "Global axes").changed() {
        s.run("env.set", json!({"axes": axes}));
    }
    let mut orbit = s.ed.show_orbit;
    if ui.checkbox(&mut orbit, "Show orbit point").changed() {
        s.run("assist.set", json!({"showOrbit": orbit}));
    }
    ui.checkbox(&mut s.view.emulate_mmb, "Emulate 3-button mouse (Alt+drag orbits)");
    ui.checkbox(&mut s.left_handed, "Left-handed layout");
    ui.checkbox(&mut s.show_tools, "Toolbar");
    ui.checkbox(&mut s.show_brush, "Brush panel");
    ui.checkbox(&mut s.show_side, "Sidebar");
    if ui.button(s.tip("Hide the UI", "ui.hide")).clicked() {
        s.hide_ui = true;
        ui.close();
    }
}

fn add_menu(s: &mut Space3d, ui: &mut Ui) {
    ui.label(RichText::new("3D Guide").strong());
    for (label, kind) in [("Cube", "cube"), ("Pyramid", "pyramid"), ("Sphere", "sphere"), ("Tube", "tube"), ("Plane", "plane")] {
        if ui.button(label).clicked() {
            s.run("guide.primitive", json!({"kind": kind}));
            ui.close();
        }
    }
    if ui.button(s.tip("Draw a guide", "tool.guide")).clicked() {
        s.run("tool.set", json!({"tool": "guide"}));
        ui.close();
    }
    if ui.button("Loft through curves").clicked() {
        s.run("tool.set", json!({"tool": "loft"}));
        ui.close();
    }
    ui.separator();
    if ui.button("Reference image…").clicked() {
        s.pick("image", IMAGE_EXTS);
        ui.close();
    }
    if ui.button("3D model (OBJ)…").clicked() {
        s.pick("model", MODEL_EXTS);
        ui.close();
    }
    if ui.button("Camera shot of this view").clicked() {
        s.run("shot.add", Value::Null);
        ui.close();
    }
}

const TOOLS: &[(Tool, &str, &str, &str)] = &[
    (Tool::Draw, "pencil", "Draw on the guide (D; D again: Draw Shape)", "tool.draw"),
    (Tool::DrawShape, "ruler", "Draw Shape: lines, curves and circles; hold to adjust (D, D)", "tool.draw"),
    (Tool::Erase, "eraser", "Erase the points under the eraser (E; E again: Vacuum)", "tool.erase"),
    (Tool::Vacuum, "eraser-magic", "Vacuum: remove whole curves (E, E)", "tool.erase"),
    (Tool::Select, "mouse-pointer-2", "Select: click or drag over curves (W cycles drag, box, circle, lasso)", "tool.select"),
    (Tool::Deselect, "square-dashed-mouse-pointer", "Deselect: drag over curves to drop them", ""),
    (Tool::Guide, "spline", "Draw a 3D Guide: the stroke is extruded along the view", "tool.guide"),
    (Tool::Bend, "pen-tool", "Bend the guide along a stroke drawn from another view", "guide.bend"),
    (Tool::Loft, "layers", "Loft: pick curves in order to make a guide through them", ""),
    (Tool::Primitive, "pentagon", "Primitive guides: cube, pyramid, sphere, tube, plane (Shift+A)", "add.menu"),
    (Tool::Liquify, "droplet", "Liquify the selected curves: push, pinch, comb", "tool.liquify"),
    (Tool::Injector, "wand-sparkles", "Injector: take a whole brush from a curve", "tool.injector"),
    (Tool::Eyedropper, "pipette", "Eyedropper: take a colour from a curve or image", "tool.eyedropper"),
];

/// The toolbar (Blender's T panel with Feather's tools).
pub fn toolbar(s: &mut Space3d, ui: &mut Ui, look: &Look, right: bool) {
    let frame = egui::Frame::NONE.fill(look.t.paper).inner_margin(egui::Margin { left: 6, right: 4, top: 4, bottom: 4 });
    let panel = if right { egui::Panel::right("w3d_tools") } else { egui::Panel::left("w3d_tools") };
    panel.show_separator_line(false).resizable(false).exact_size(58.0).frame(frame).show(ui, |ui| {
        egui::ScrollArea::vertical().scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            for (tool, icon, text, action) in TOOLS {
                let icon = if *tool == Tool::Select {
                    match s.ed.select_mode {
                        wobbleworks_3d::editor::SelectMode::Box => "square-dashed",
                        wobbleworks_3d::editor::SelectMode::Circle => "circle-dashed",
                        wobbleworks_3d::editor::SelectMode::Lasso => "lasso",
                        wobbleworks_3d::editor::SelectMode::Brush => icon,
                    }
                } else {
                    icon
                };
                let tip = if action.is_empty() { (*text).to_string() } else { s.tip(text, action) };
                if tile(ui, look, icon, s.ed.tool == *tool, &tip).clicked() {
                    if *tool == Tool::Select && s.ed.tool == Tool::Select {
                        s.run("tool.set", json!({"tool": "select", "cycle": true}));
                    } else {
                        s.run("tool.set", json!({"tool": tool.name()}));
                    }
                }
            }
            ui.add_space(6.0);
            let mirror = s.ed.mirror_on;
            if tile(ui, look, "arrow-left-right", mirror, &s.tip("Mirror while drawing", "mirror.toggle")).clicked() {
                s.run("mirror.toggle", Value::Null);
            }
            if mirror {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    for (k, name) in ["X", "Y", "Z"].iter().enumerate() {
                        let on = s.ed.mirror_axes.get(k).copied().unwrap_or(false);
                        let c = super::view::AXIS.get(k).copied().unwrap_or(Color32::WHITE);
                        let text = RichText::new(*name).color(if on { Color32::WHITE } else { c }).strong();
                        let b = egui::Button::new(text).fill(if on { c } else { Color32::TRANSPARENT }).min_size(vec2(14.0, 18.0));
                        if ui.add(b).on_hover_text(format!("Mirror across {name}")).clicked() {
                            s.run("mirror.set", json!({name.to_lowercase(): !on}));
                        }
                    }
                });
            }
            ui.add_space(6.0);
            if tile(ui, look, "undo-2", false, &s.tip("Undo", "edit.undo")).clicked() {
                s.run("edit.undo", Value::Null);
            }
            if tile(ui, look, "redo-2", false, &s.tip("Redo", "edit.redo")).clicked() {
                s.run("edit.redo", Value::Null);
            }
        });
    });
}

/// A slider that changes the brush while dragging and the selected curves when let go (one undo
/// step instead of one per frame).
#[allow(clippy::too_many_arguments)]
fn brush_slider(s: &mut Space3d, ui: &mut Ui, label: &str, value: f32, range: std::ops::RangeInclusive<f32>, log: bool, key: &str, paint: bool) {
    let mut v = value;
    let r = ui.add(egui::Slider::new(&mut v, range).logarithmic(log).text(label));
    let wrap = |v: f32| if paint { json!({"paint": {key: v}}) } else { json!({key: v}) };
    if r.changed() {
        let mut p = wrap(v);
        if let Some(o) = p.as_object_mut() {
            o.insert("applyToSelection".into(), json!(!r.dragged()));
        }
        s.run("brush.set", p);
    }
    if r.drag_stopped() && !s.ed.selection.is_empty() {
        s.run("brush.set", wrap(v));
    }
}

/// Feather's brush panel: type, colour, size, opacity, pressure, material, pattern, the
/// painterly options, presets and drawing aids.
pub fn brush_panel(s: &mut Space3d, ui: &mut Ui, look: &Look, right: bool) {
    let frame = egui::Frame::NONE.fill(look.t.paper).inner_margin(egui::Margin { left: 4, right: 8, top: 4, bottom: 4 });
    let panel = if right { egui::Panel::right("w3d_brush") } else { egui::Panel::left("w3d_brush") };
    panel.show_separator_line(false).resizable(true).default_size(250.0).frame(frame).show(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            widgets::card(ui, look, "w3d-brush-card", 8.0, |ui| {
                ui.set_max_width(ui.available_width());
                let b = s.ed.brush;
                let editing = if s.ed.selection.is_empty() { "Brush".to_string() } else { format!("Brush → {} selected", s.ed.selection.len()) };
                widgets::label(ui, look, &editing, widgets::TEXT, look.t.ink);
                ui.horizontal_wrapped(|ui| {
                    for k in BrushKind::ALL {
                        let tip = match k {
                            BrushKind::Pen => "A round, shaded wire",
                            BrushKind::Marker => "A flat band that faces you",
                            BrushKind::Flat => "Tape lying on the guide",
                            BrushKind::Square => "A square tube",
                            BrushKind::Nib => "Calligraphy: thin and thick with direction",
                            BrushKind::Oil => "Painterly: oil with bristle streaks",
                            BrushKind::Gouache => "Painterly: thick overlapping gouache",
                            BrushKind::DryBrush => "Painterly: a dry brush that breaks up",
                            BrushKind::Chalk => "Painterly: chalk grain",
                            BrushKind::Ink => "Painterly: a pointed ink brush",
                        };
                        if ui.selectable_label(b.kind == k, k.label()).on_hover_text(s.tip(tip, "brush.next")).clicked() {
                            s.run("brush.set", json!({"kind": k.name()}));
                        }
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    let mut c = colour32(b.color);
                    if ui.color_edit_button_srgba(&mut c).on_hover_text("Brush colour (the colour strip below sets it too)").changed() {
                        s.set_colour(c);
                    }
                    let mut hex = b.color.to_hex();
                    let r = ui.add(egui::TextEdit::singleline(&mut hex).desired_width(80.0));
                    if r.lost_focus() && let Some(c) = wobbleworks_3d::model::Rgba::from_hex(&hex) {
                        s.run("brush.set", json!({"color": c.to_hex()}));
                    }
                    if ui.selectable_label(s.ed.tool == Tool::Injector, "Injector").on_hover_text(s.tip("Take a whole brush from a curve", "tool.injector")).clicked() {
                        s.run("tool.set", json!({"tool": "injector"}));
                    }
                });
                brush_slider(s, ui, "Size mm", b.size_mm, SIZE_MIN_MM..=SIZE_MAX_MM, true, "size", false);
                brush_slider(s, ui, "Opacity", b.opacity, 0.0..=1.0, false, "opacity", false);
                let mut pressure = b.pressure;
                if ui.checkbox(&mut pressure, "Pen pressure changes the size").changed() {
                    s.run("brush.set", json!({"pressure": pressure}));
                }
                ui.separator();
                ui.label("Material");
                ui.horizontal_wrapped(|ui| {
                    for m in Material::ALL {
                        let tip = match m {
                            Material::Shadeless => "Flat colour, no light",
                            Material::Shaded => "Lit, casts ground shadows, toon shading",
                            Material::Glow => "Glows (no light, no pattern)",
                            Material::Cutout => "Shows the background through",
                        };
                        if ui.selectable_label(b.material == m, m.name()).on_hover_text(format!("{tip} (shown in Render mode)")).clicked() {
                            s.run("brush.set", json!({"material": m.name()}));
                        }
                    }
                });
                if b.material == Material::Glow {
                    brush_slider(s, ui, "Glow", b.glow, 0.0..=1.0, false, "glow", false);
                }
                ui.label("Pattern");
                ui.horizontal_wrapped(|ui| {
                    if ui.selectable_label(b.pattern.is_none(), "none").clicked() {
                        s.run("brush.set", json!({"pattern": null}));
                    }
                    for k in PatternKind::ALL {
                        let on = b.pattern.is_some_and(|p| p.kind == k);
                        if ui.selectable_label(on, k.name()).clicked() {
                            s.run("brush.set", json!({"pattern": {"kind": k.name()}}));
                        }
                    }
                });
                if let Some(p) = b.pattern {
                    for (label, v, key, range) in [("Intensity", p.intensity, "intensity", 0.0..=1.0), ("Angle", p.angle, "angle", 0.0..=180.0), ("Contrast", p.contrast, "contrast", 0.0..=1.0)] {
                        let mut x = v;
                        if ui.add(egui::Slider::new(&mut x, range).text(label)).changed() {
                            s.run("brush.set", json!({"pattern": {key: x}}));
                        }
                    }
                }
            });
            ui.add_space(6.0);
            widgets::card(ui, look, "w3d-paint-card", 8.0, |ui| {
                ui.set_max_width(ui.available_width());
                let b = s.ed.brush;
                widgets::label(ui, look, "Paint & boil", widgets::TEXT, look.t.ink);
                let pa = b.paint;
                for (label, v, key) in [
                    ("Rough edges", pa.roughness, "roughness"),
                    ("Bristles", pa.bristles, "bristles"),
                    ("Dry brush", pa.dryness, "dryness"),
                    ("Grain", pa.grain, "grain"),
                    ("Taper", pa.taper, "taper"),
                ] {
                    brush_slider(s, ui, label, v, 0.0..=1.0, false, key, true);
                }
                brush_slider(s, ui, "Boil ×", pa.boil, 0.0..=4.0, false, "boil", true);
                let mut layers = pa.layers as f32;
                if ui.add(egui::Slider::new(&mut layers, 1.0..=4.0).step_by(1.0).text("Layers")).on_hover_text("Overlapping passes, like a painted shape built up in strokes").changed() {
                    s.run("brush.set", json!({"paint": {"layers": layers as u64}}));
                }
                let mut echo = pa.echo.is_some();
                if ui.checkbox(&mut echo, "Echo (painted shadow or outline)").changed() {
                    s.run("brush.set", json!({"paint": {"echo": if echo { json!({}) } else { Value::Null }}}));
                }
                if let Some(e) = pa.echo {
                    ui.horizontal(|ui| {
                        let mut c = colour32(e.color);
                        if ui.color_edit_button_srgba(&mut c).changed() {
                            let [r, g, bl, _] = c.to_array();
                            s.run("brush.set", json!({"paint": {"echo": {"color": format!("#{r:02x}{g:02x}{bl:02x}")}}}));
                        }
                        let (mut x, mut y, mut w) = (e.offset[0], e.offset[1], e.width);
                        let a = ui.add(egui::DragValue::new(&mut x).range(-200.0..=200.0).prefix("x ")).changed();
                        let bb = ui.add(egui::DragValue::new(&mut y).range(-200.0..=200.0).prefix("y ")).changed();
                        let c2 = ui.add(egui::DragValue::new(&mut w).range(0.1..=4.0).speed(0.01).prefix("w ")).changed();
                        if a || bb || c2 {
                            s.run("brush.set", json!({"paint": {"echo": {"offset": [x, y], "width": w}}}));
                        }
                    });
                }
                if ui.button("Metaphor look").on_hover_text("Gouache with ragged edges, layered passes and a black echo, boiling").clicked() {
                    s.run("brush.set", json!({"kind": "gouache", "paint": {"roughness": 0.75, "bristles": 0.35, "dryness": 0.1, "layers": 3, "taper": 0.25, "boil": 1.5, "echo": {"color": "#111111", "offset": [9, 7], "width": 1.12}}}));
                }
            });
            ui.add_space(6.0);
            widgets::card(ui, look, "w3d-assist-card", 8.0, |ui| {
                ui.set_max_width(ui.available_width());
                widgets::label(ui, look, "Assist", widgets::TEXT, look.t.ink);
                let mut st = s.ed.stable;
                if ui.add(egui::Slider::new(&mut st, 0.0..=1.0).text("Stable stroke")).changed() {
                    s.run("stable.set", json!({"value": st}));
                }
                let mut air = s.ed.draw_in_air;
                if ui.checkbox(&mut air, "Draw in the air without a guide").changed() {
                    s.run("assist.set", json!({"drawInAir": air}));
                }
                let mut gs = s.ed.guide_shape;
                if ui.checkbox(&mut gs, "Hold to straighten guide strokes").changed() {
                    s.run("assist.set", json!({"guideShape": gs}));
                }
            });
            ui.add_space(6.0);
            widgets::card(ui, look, "w3d-preset-card", 8.0, |ui| {
                ui.set_max_width(ui.available_width());
                ui.horizontal(|ui| {
                    widgets::label(ui, look, "Presets", widgets::TEXT, look.t.ink);
                    if ui.button("+").on_hover_text("Save this brush as a preset").clicked() {
                        s.run("preset.add", Value::Null);
                    }
                });
                let presets: Vec<(u64, String)> = s.ed.scene.presets.iter().map(|p| (p.id, p.name.clone())).collect();
                if presets.is_empty() {
                    ui.label(RichText::new("No presets yet").color(look.t.dim));
                }
                for (id, name) in presets {
                    let r = ui.button(name).on_hover_text("Click to use; right-click to delete");
                    if r.clicked() {
                        s.run("preset.load", json!({"id": id}));
                    }
                    if r.secondary_clicked() {
                        s.run("preset.delete", json!({"ids": [id]}));
                    }
                }
            });
        });
    });
}

/// Feather's bottom context menu: guide tools, the session in progress, selection actions.
pub fn context_bar(s: &mut Space3d, ui: &mut Ui, look: &Look) {
    let frame = egui::Frame::NONE.fill(look.t.paper).inner_margin(egui::Margin { left: 8, right: 8, top: 4, bottom: 4 });
    egui::Panel::bottom("w3d_context").show_separator_line(false).frame(frame).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            match s.ed.session() {
                Session::Primitive { kind, segments } => {
                    ui.label(RichText::new("Primitive").strong());
                    for k in Primitive::ALL {
                        if ui.selectable_label(kind == k, k.name()).clicked() {
                            s.run("guide.primitive", json!({"kind": k.name(), "segments": k.default_segments()}));
                        }
                    }
                    let mut n = segments as f32;
                    if ui.add(egui::Slider::new(&mut n, 3.0..=64.0).step_by(1.0).text("Segments")).changed() {
                        s.run("guide.segments", json!({"value": n as u64}));
                    }
                    ui.label(RichText::new("G / R / S to place it").color(look.t.dim));
                    if widgets::button(ui, look, "Done", true, true).on_hover_text("Use it as the guide (Enter)").clicked() {
                        s.run("guide.done", Value::Null);
                    }
                    if widgets::button(ui, look, "Cancel", false, true).clicked() {
                        s.run("guide.cancel", Value::Null);
                    }
                    return;
                }
                Session::Loft { curves, tension } => {
                    ui.label(RichText::new(format!("Loft: {curves} curves picked (click curves in order)")).strong());
                    let mut t = tension;
                    if ui.add(egui::Slider::new(&mut t, 0.0..=1.0).text("Tension")).on_hover_text("Up: smoother; down: sharper bends").changed() {
                        s.run("guide.tension", json!({"value": t}));
                    }
                    if widgets::button(ui, look, "Done", true, curves >= 2).clicked() {
                        s.run("guide.done", Value::Null);
                    }
                    if widgets::button(ui, look, "Cancel", false, true).clicked() {
                        s.run("guide.cancel", Value::Null);
                    }
                    return;
                }
                Session::Liquify => {
                    ui.label(RichText::new("Liquify").strong());
                    for k in ["push", "pinch", "comb"] {
                        if ui.selectable_label(s.ed.liquify.kind.name() == k, k).clicked() {
                            s.run("liquify.set", json!({"kind": k}));
                        }
                    }
                    let l = s.ed.liquify;
                    for (label, v, key, range) in [("Size", l.size, "size", 4.0..=600.0), ("Range", l.range, "range", 0.0..=1.0), ("Strength", l.strength, "strength", 0.0..=1.0)] {
                        let mut x = v;
                        if ui.add(egui::Slider::new(&mut x, range).text(label)).changed() {
                            s.run("liquify.set", json!({key: x}));
                        }
                    }
                    if ui.button("Undo all").clicked() {
                        s.run("liquify.undoAll", Value::Null);
                    }
                    let r = ui.button("Compare (hold)");
                    let held = r.is_pointer_button_down_on();
                    if held != s.ed.compare {
                        s.run("liquify.compare", json!({"on": held}));
                    }
                    if widgets::button(ui, look, "Apply", true, true).clicked() {
                        s.run("liquify.apply", Value::Null);
                    }
                    return;
                }
                Session::None => {}
            }
            // Guides.
            for (label, tool, action) in [("Draw guide", "guide", "tool.guide"), ("Bend", "bend", "guide.bend"), ("Loft", "loft", "")] {
                let on = s.ed.tool.name() == tool;
                let tip = s.tip(label, action);
                if ui.selectable_label(on, label).on_hover_text(tip).clicked() {
                    s.run("tool.set", json!({"tool": tool}));
                }
            }
            ui.menu_button("Primitives", |ui| {
                for k in Primitive::ALL {
                    if ui.button(k.name()).clicked() {
                        s.run("guide.primitive", json!({"kind": k.name()}));
                        ui.close();
                    }
                }
            });
            if let Some(g) = s.ed.scene.active_guide().cloned() {
                ui.separator();
                let mut op = g.opacity;
                if ui.add(egui::Slider::new(&mut op, 0.0..=wobbleworks_3d::guide::OPACITY_MAX).show_value(false).text("Guide")).on_hover_text("Guide opacity (it never covers your drawing completely)").changed() {
                    s.run("guide.opacity", json!({"value": op}));
                }
                if ui.button("Close").on_hover_text(s.tip("Close the guide", "guide.close")).clicked() {
                    s.run("guide.close", Value::Null);
                }
                if ui.button("Save").on_hover_text(s.tip("Save the guide to Resources", "guide.save")).clicked() {
                    s.run("guide.save", Value::Null);
                }
            } else if ui.button("Recall guide").on_hover_text(s.tip("Bring back the last closed guide", "guide.recall")).clicked() {
                s.run("guide.recall", Value::Null);
            }
            if matches!(s.ed.tool, Tool::Erase | Tool::Vacuum) || (s.ed.tool == Tool::Select && s.ed.select_mode == wobbleworks_3d::editor::SelectMode::Circle) {
                ui.separator();
                let mut r = s.ed.eraser_size;
                if ui.add(egui::Slider::new(&mut r, 2.0..=200.0).text("Radius")).changed() {
                    s.run("eraser.size", json!({"value": r}));
                }
            }
            if s.ed.has_selection() {
                ui.separator();
                ui.label(RichText::new(format!("{} selected", s.ed.selection.len() + s.ed.selected_resources.len())).strong());
                let at = ui.ctx().pointer_latest_pos().unwrap_or_default();
                let local = [at.x - s.view.rect.min.x, at.y - s.view.rect.min.y];
                for (label, mode, action) in [("Move", "grab", "transform.grab"), ("Rotate", "rotate", "transform.rotate"), ("Scale", "scale", "transform.scale")] {
                    if ui.button(label).on_hover_text(s.tip(label, action)).clicked() {
                        let c = s.view.rect.center();
                        let _ = local;
                        s.run("transform.start", json!({"mode": mode, "x": c.x - s.view.rect.min.x + 60.0, "y": c.y - s.view.rect.min.y}));
                    }
                }
                if !s.ed.selection.is_empty() {
                    if ui.button("Liquify").on_hover_text(s.tip("Liquify", "tool.liquify")).clicked() {
                        s.run("tool.set", json!({"tool": "liquify"}));
                    }
                    if ui.button("Duplicate").on_hover_text(s.tip("Duplicate", "edit.duplicate")).clicked() {
                        s.run("edit.duplicate", json!({"mode": "inplace"}));
                    }
                    if ui.button("Duplicate by view").on_hover_text(s.tip("Duplicate by view", "edit.duplicateView")).clicked() {
                        s.run("edit.duplicate", json!({"mode": "view"}));
                    }
                    if ui.add_enabled(s.ed.mirror_on, egui::Button::new("Duplicate by mirror")).on_hover_text(s.tip("One copy per mirror axis (turn Mirror on first)", "edit.duplicateMirror")).clicked() {
                        s.run("edit.duplicate", json!({"mode": "mirror"}));
                    }
                }
                if ui.button("Delete").on_hover_text(s.tip("Delete", "edit.deleteNow")).clicked() {
                    s.run("edit.delete", Value::Null);
                }
            }
        });
    });
}

/// The sidebar (Blender's N panel and Feather's Stage panel).
pub fn sidebar(s: &mut Space3d, ui: &mut Ui, look: &Look, right: bool) {
    let frame = egui::Frame::NONE.fill(look.t.paper).inner_margin(egui::Margin { left: 6, right: 8, top: 4, bottom: 4 });
    let panel = if right { egui::Panel::right("w3d_side") } else { egui::Panel::left("w3d_side") };
    panel.show_separator_line(false).resizable(true).default_size(300.0).frame(frame).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            for (tab, label) in [
                (SideTab::Stage, "Stage"),
                (SideTab::Boil, "Boil"),
                (SideTab::Shots, "Shots"),
                (SideTab::Item, "Item"),
                (SideTab::History, "History"),
                (SideTab::Keys, "Keys"),
                (SideTab::Help, "Help"),
            ] {
                if ui.selectable_label(s.side == tab, label).clicked() {
                    s.side = tab;
                }
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| match s.side {
            SideTab::Stage => stage(s, ui, look),
            SideTab::Boil => boil(s, ui),
            SideTab::Shots => shots(s, ui, look),
            SideTab::Item => item(s, ui),
            SideTab::History => history(s, ui, look),
            SideTab::Keys => keys(s, ui, look),
            SideTab::Help => help(ui, look),
        });
    });
}

fn stage(s: &mut Space3d, ui: &mut Ui, look: &Look) {
    ui.horizontal(|ui| {
        for (tab, label) in [(StageTab::Groups, "Groups"), (StageTab::Resources, "Resources"), (StageTab::Environment, "Environment")] {
            if ui.selectable_label(s.stage == tab, label).clicked() {
                s.stage = tab;
            }
        }
    });
    ui.separator();
    match s.stage {
        StageTab::Groups => groups(s, ui, look),
        StageTab::Resources => resources(s, ui, look),
        StageTab::Environment => environment(s, ui),
    }
}

fn groups(s: &mut Space3d, ui: &mut Ui, look: &Look) {
    ui.horizontal_wrapped(|ui| {
        if ui.button("+ Group").on_hover_text("A new group above the active one").clicked() {
            s.run("group.new", Value::Null);
        }
        let picked = s.picked_groups.clone();
        if ui.add_enabled(!picked.is_empty(), egui::Button::new("Duplicate")).clicked() {
            s.run("group.duplicate", json!({"ids": picked}));
        }
        if ui.add_enabled(picked.len() >= 2, egui::Button::new("Merge")).clicked() {
            s.run("group.merge", json!({"ids": picked}));
            s.picked_groups.clear();
        }
        if ui.add_enabled(!picked.is_empty(), egui::Button::new("Delete")).clicked() {
            s.run("group.delete", json!({"ids": picked}));
            s.picked_groups.clear();
        }
    });
    ui.label(RichText::new("Click: draw into it. Ctrl+click: pick for merge/delete. Double-click: rename.").small().color(look.t.dim));
    let groups: Vec<_> = s.ed.scene.groups.iter().rev().cloned().collect();
    let n = groups.len();
    for (row, g) in groups.into_iter().enumerate() {
        let active = s.ed.scene.active_group == g.id;
        let picked = s.picked_groups.contains(&g.id);
        let count = s.ed.scene.strokes.iter().filter(|st| st.group == g.id).count();
        let sel = s.ed.scene.strokes.iter().filter(|st| st.group == g.id && s.ed.selection.contains(&st.id)).count();
        ui.horizontal(|ui| {
            let shown = s.ed.scene.group_shown(g.id);
            let eye = if shown { "eye" } else { "eye-off" };
            let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::click());
            photocraft_ui_egui::icons::paint(ui, r, eye, 16.0, look.t.ink);
            let isolated = s.ed.scene.isolated == Some(g.id);
            let resp = resp.on_hover_text("Show / hide (right-click: show only this group)");
            if resp.clicked() {
                s.run("group.visible", json!({"id": g.id}));
            }
            if resp.secondary_clicked() {
                s.run("group.isolate", json!({"id": g.id}));
            }
            let mut text = RichText::new(format!("{}{}  ({count})", g.name, if isolated { "  [solo]" } else { "" }));
            if sel > 0 {
                text = text.color(if sel == count { Color32::from_rgb(0x2f, 0xa8, 0x55) } else { Color32::from_rgb(0x7f, 0xc8, 0x8f) });
            }
            if active {
                text = text.strong();
            }
            let r = ui.selectable_label(active || picked, text);
            if r.double_clicked() {
                s.menu = Some((Menu::Rename(g.id, g.name.clone()), r.rect.left_bottom()));
            } else if r.clicked() {
                if ui.input(|i| i.modifiers.command || i.modifiers.ctrl) {
                    if picked {
                        s.picked_groups.retain(|x| *x != g.id);
                    } else {
                        s.picked_groups.push(g.id);
                    }
                } else {
                    s.run("group.activate", json!({"id": g.id}));
                    s.picked_groups = vec![g.id];
                }
            }
            r.context_menu(|ui| {
                if ui.button("Select its curves").clicked() {
                    s.run("select.group", json!({"id": g.id}));
                    ui.close();
                }
                if ui.add_enabled(!s.ed.selection.is_empty(), egui::Button::new("Move the selected curves here")).clicked() {
                    s.run("group.moveStrokes", json!({"id": g.id}));
                    ui.close();
                }
                if ui.button("Rename").clicked() {
                    s.menu = Some((Menu::Rename(g.id, g.name.clone()), ui.ctx().pointer_latest_pos().unwrap_or_default()));
                    ui.close();
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Rows are shown top first; the list is bottom first.
                let index = n - 1 - row;
                if ui.small_button("▼").on_hover_text("Move down").clicked() && index > 0 {
                    s.run("group.move", json!({"id": g.id, "index": index - 1}));
                }
                if ui.small_button("▲").on_hover_text("Move up").clicked() {
                    s.run("group.move", json!({"id": g.id, "index": index + 1}));
                }
            });
        });
    }
}

fn state_icon(st: ResourceState) -> (&'static str, &'static str) {
    match st {
        ResourceState::Active => ("square-dot", "Draw-on: visible and drawn on (click: visible)"),
        ResourceState::Visible => ("square", "Visible (click: hidden)"),
        ResourceState::Hidden => ("square-dashed", "Hidden (click: draw-on)"),
    }
}

fn next_state(st: ResourceState) -> &'static str {
    match st {
        ResourceState::Active => "visible",
        ResourceState::Visible => "hidden",
        ResourceState::Hidden => "active",
    }
}

fn resources(s: &mut Space3d, ui: &mut Ui, look: &Look) {
    ui.horizontal_wrapped(|ui| {
        if ui.button("Import image…").on_hover_text("A reference image (two per note)").clicked() {
            s.pick("image", IMAGE_EXTS);
        }
        if ui.button("Import OBJ…").on_hover_text("A 3D model to draw on").clicked() {
            s.pick("model", MODEL_EXTS);
        }
        if ui.button("Open note…").clicked() {
            s.pick("open", NOTE_EXTS);
        }
    });
    let mut rows: Vec<(u64, String, ResourceState, &'static str, Option<f32>)> = Vec::new();
    for g in &s.ed.scene.guides {
        rows.push((g.id, g.name.clone(), s.ed.scene.guide_state(g.id), "Surface", Some(g.opacity)));
    }
    for i in &s.ed.scene.images {
        rows.push((i.id, i.name.clone(), i.state, "Image", Some(i.opacity)));
    }
    for m in &s.ed.scene.models {
        rows.push((m.id, m.name.clone(), m.state, "Model", None));
    }
    if rows.is_empty() {
        ui.label(RichText::new("Saved guides, images and models show here.").color(look.t.dim));
    }
    for (id, name, st, kind, opacity) in rows {
        ui.horizontal(|ui| {
            let (icon, tip) = state_icon(st);
            let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::click());
            photocraft_ui_egui::icons::paint(ui, r, icon, 16.0, look.t.ink);
            if resp.on_hover_text(tip).clicked() {
                s.run("resource.state", json!({"id": id, "state": next_state(st)}));
            }
            let selected = s.ed.selected_resources.contains(&id);
            let r = ui.selectable_label(selected, format!("{name}  ·  {kind}"));
            if r.clicked() {
                s.run("select.set", json!({"ids": [id]}));
            }
            if r.double_clicked() {
                s.menu = Some((Menu::Rename(id, name.clone()), r.rect.left_bottom()));
            }
            if let Some(mut o) = opacity
                && ui.add(egui::Slider::new(&mut o, 0.0..=1.0).show_value(false)).on_hover_text("Opacity").changed()
            {
                s.run("resource.opacity", json!({"id": id, "value": o}));
            }
            if ui.small_button("✕").on_hover_text("Delete").clicked() {
                s.run("resource.delete", json!({"ids": [id]}));
            }
        });
    }
}

fn environment(s: &mut Space3d, ui: &mut Ui) {
    let env = s.ed.scene.environment.clone();
    let mut changed = serde_json::Map::new();
    let mut render = env.render_mode;
    if ui.checkbox(&mut render, "Render mode (materials, light, effects)").changed() {
        changed.insert("render".into(), json!(render));
    }
    ui.horizontal(|ui| {
        let mut grid = env.show_grid;
        if ui.checkbox(&mut grid, "Grid (1 square = 1 m)").changed() {
            changed.insert("grid".into(), json!(grid));
        }
        let mut axes = env.show_axes;
        if ui.checkbox(&mut axes, "Axes").changed() {
            changed.insert("axes".into(), json!(axes));
        }
    });
    ui.horizontal(|ui| {
        ui.label("Background");
        let mut c = colour32(env.background);
        if ui.color_edit_button_srgba(&mut c).changed() {
            let [r, g, b, _] = c.to_array();
            changed.insert("background".into(), json!(format!("#{r:02x}{g:02x}{b:02x}")));
        }
        let mut fog = env.fog;
        if ui.checkbox(&mut fog, "Fog").changed() {
            changed.insert("fog".into(), json!(fog));
        }
    });
    ui.separator();
    ui.label(RichText::new("Lighting").strong());
    let l = env.lighting;
    let mut light = serde_json::Map::new();
    for (label, v, key, range) in [("Azimuth", l.azimuth, "azimuth", 0.0..=360.0), ("Altitude", l.altitude, "altitude", -90.0..=90.0), ("Strength", l.strength, "strength", 0.0..=2.0)] {
        let mut x = v;
        if ui.add(egui::Slider::new(&mut x, range).text(label)).changed() {
            light.insert(key.into(), json!(x));
        }
    }
    ui.horizontal(|ui| {
        ui.label("Light colour");
        let mut c = colour32(l.color);
        if ui.color_edit_button_srgba(&mut c).changed() {
            let [r, g, b, _] = c.to_array();
            light.insert("color".into(), json!(format!("#{r:02x}{g:02x}{b:02x}")));
        }
        if ui.button("From the view").on_hover_text("Light from where you're looking").clicked() {
            s.run("env.lightFromView", Value::Null);
        }
    });
    let mut gs = l.ground_shadow;
    if ui.checkbox(&mut gs, "Ground shadow (Shaded curves)").changed() {
        light.insert("groundShadow".into(), json!(gs));
    }
    let mut toon = l.toon;
    if ui.checkbox(&mut toon, "Toon shading").changed() {
        light.insert("toon".into(), json!(toon));
    }
    if !light.is_empty() {
        changed.insert("lighting".into(), Value::Object(light));
    }
    ui.separator();
    ui.label(RichText::new("Effects").strong());
    let e = env.effects;
    let mut fx = serde_json::Map::new();
    for (label, v, key, range, tip) in [
        ("Glow area", e.glow, "glow", 0.0..=1.0, "Halo size around Glow curves"),
        ("Bloom", e.bloom, "bloom", 0.0..=1.0, "Bright colours bleed light (in exports)"),
        ("Grain", e.grain, "grain", 0.0..=1.0, "Film grain (in exports)"),
        ("Pixelate", e.pixelate, "pixelate", 0.0..=32.0, "Pixel size (in exports)"),
        ("Depth of field", e.dof, "dof", 0.0..=22.0, "F-stop; focus on the orbit point (not drawn yet)"),
    ] {
        let mut x = v;
        if ui.add(egui::Slider::new(&mut x, range).text(label)).on_hover_text(tip).changed() {
            fx.insert(key.into(), json!(x));
        }
    }
    if !fx.is_empty() {
        changed.insert("effects".into(), Value::Object(fx));
    }
    if !changed.is_empty() {
        s.run("env.set", Value::Object(changed));
    }
}

fn boil(s: &mut Space3d, ui: &mut Ui) {
    let b = s.ed.scene.boil;
    let mut p = serde_json::Map::new();
    let mut on = b.enabled;
    if ui.checkbox(&mut on, "Lines boil").changed() {
        p.insert("enabled".into(), json!(on));
    }
    for (label, v, key, range, tip) in [
        ("Wiggle", b.amount, "amount", 0.0..=40.0, "How far lines wander (pixels)"),
        ("Speed fps", b.fps, "fps", 1.0..=24.0, "Boil frames per second"),
        ("Waviness", b.wavelength, "wavelength", 4.0..=400.0, "Wavelength along a line (pixels)"),
        ("Thickness wobble", b.thickness, "thickness", 0.0..=1.0, "How much line width boils"),
    ] {
        let mut x = v;
        if ui.add(egui::Slider::new(&mut x, range).text(label)).on_hover_text(tip).changed() {
            p.insert(key.into(), json!(x));
        }
    }
    let mut frames = b.frames as f32;
    if ui.add(egui::Slider::new(&mut frames, 2.0..=12.0).step_by(1.0).text("Frames")).changed() {
        p.insert("frames".into(), json!(frames as u64));
    }
    let mut world = b.world_space;
    if ui.checkbox(&mut world, "Boil in 3D (nearer lines wiggle more)").on_hover_text("Off: a cartoon boil on screen, the same at any zoom").changed() {
        p.insert("world".into(), json!(world));
    }
    ui.checkbox(&mut s.hold_boil_while_drawing, "Hold still while drawing");
    if !p.is_empty() {
        s.run("boil.set", Value::Object(p));
    }
    ui.label(RichText::new("Each brush has its own Boil × in the brush panel. Painterly brushes also change their paint texture every frame.").small());
}

fn shots(s: &mut Space3d, ui: &mut Ui, look: &Look) {
    ui.horizontal_wrapped(|ui| {
        if ui.button("+ Shot").on_hover_text("A camera shot of this view").clicked() {
            s.run("shot.add", Value::Null);
        }
        let playing = s.playing.is_some();
        if ui.add_enabled(s.ed.scene.sequence.shots.len() >= 2, egui::Button::new(if playing { "Stop" } else { "Play" })).clicked() {
            s.playing = if playing { None } else { Some(ui.input(|i| i.time)) };
        }
    });
    let seq = s.ed.scene.sequence.clone();
    ui.horizontal(|ui| {
        for sp in [0.5f32, 1.0, 2.0] {
            if ui.selectable_label((seq.speed - sp).abs() < 1e-3, format!("{sp}x")).clicked() {
                s.run("sequence.set", json!({"speed": sp}));
            }
        }
        for m in ["once", "loop", "swing"] {
            let on = format!("{:?}", seq.mode).eq_ignore_ascii_case(m);
            if ui.selectable_label(on, m).clicked() {
                s.run("sequence.set", json!({"mode": m}));
            }
        }
    });
    let mut secs = seq.seconds_per_shot;
    if ui.add(egui::Slider::new(&mut secs, 0.2..=10.0).text("Seconds per shot")).changed() {
        s.run("sequence.set", json!({"secondsPerShot": secs}));
    }
    if seq.shots.is_empty() {
        ui.label(RichText::new("Add shots, then Play to fly between them.").color(look.t.dim));
    }
    let n = seq.shots.len();
    for (i, shot) in seq.shots.iter().enumerate() {
        ui.horizontal(|ui| {
            if ui.button(&shot.name).on_hover_text("Go to this shot").clicked() {
                s.run("shot.go", json!({"id": shot.id}));
            }
            if ui.small_button("▲").clicked() && i > 0 {
                s.run("shot.move", json!({"id": shot.id, "index": i - 1}));
            }
            if ui.small_button("▼").clicked() && i + 1 < n {
                s.run("shot.move", json!({"id": shot.id, "index": i + 1}));
            }
            if ui.small_button("✕").clicked() {
                s.run("shot.delete", json!({"ids": [shot.id]}));
            }
        });
    }
}

fn item(s: &mut Space3d, ui: &mut Ui) {
    ui.label(RichText::new("Transform").strong());
    ui.horizontal(|ui| {
        ui.label("Pivot");
        for (p, label) in [("median", "Median"), ("bounds", "Bounds"), ("orbit", "Orbit point")] {
            let on = format!("{:?}", s.ed.pivot).to_lowercase().starts_with(&p[..3]);
            if ui.selectable_label(on, label).clicked() {
                s.run("transform.pivot", json!({"pivot": p}));
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label("Gizmo");
        for k in ["none", "move", "rotate", "scale", "all"] {
            let on = match s.ed.gizmo {
                None => k == "none",
                Some(g) => format!("{g:?}").eq_ignore_ascii_case(k),
            };
            if ui.selectable_label(on, k).clicked() {
                s.run("transform.gizmo", json!({"kind": k}));
            }
        }
    });
    let mut local = s.ed.gizmo_local;
    if ui.checkbox(&mut local, "Local axes").changed() {
        s.run("transform.gizmo", json!({"kind": s.ed.gizmo.map_or("none".to_string(), |g| format!("{g:?}").to_lowercase()), "local": local}));
    }
    ui.separator();
    ui.label("Exact (like typing a number after G / R / S)");
    ui.horizontal(|ui| {
        for (i, m) in ["Move m", "Rotate °", "Scale ×"].iter().enumerate() {
            if ui.selectable_label(s.item_mode == i, *m).clicked() {
                s.item_mode = i;
            }
        }
    });
    ui.horizontal(|ui| {
        for (i, a) in ["X", "Y", "Z", "View"].iter().enumerate() {
            if ui.selectable_label(s.item_axis == i, *a).clicked() {
                s.item_axis = i;
            }
        }
        ui.add(egui::DragValue::new(&mut s.item_amount).speed(0.05));
        if ui.add_enabled(s.ed.has_selection(), egui::Button::new("Apply")).clicked() {
            let mode = ["grab", "rotate", "scale"].get(s.item_mode).copied().unwrap_or("grab");
            let mut p = json!({"mode": mode, "amount": s.item_amount});
            if s.item_axis < 3
                && let Some(o) = p.as_object_mut()
            {
                o.insert("axis".into(), json!(s.item_axis));
            }
            s.run("transform.apply", p);
        }
    });
    if let Some(p) = s.ed.pivot_point() {
        ui.label(format!("Centre: {:.3}, {:.3}, {:.3}", p.x, p.y, p.z));
    }
    ui.separator();
    let c = s.ed.camera;
    ui.label(RichText::new("Camera").strong());
    ui.label(format!("Orbit point {:.2}, {:.2}, {:.2}", c.target.x, c.target.y, c.target.z));
    ui.label(format!("Yaw {:.1}°  Pitch {:.1}°  Distance {:.2} m  Lens {:.0} mm  {}", c.yaw, c.pitch, c.distance, c.focal_mm, if c.orthographic { "ortho" } else { "persp" }));
}

fn history(s: &mut Space3d, ui: &mut Ui, look: &Look) {
    let (done, undone) = s.ed.history();
    if done.is_empty() && undone.is_empty() {
        ui.label(RichText::new("Nothing to undo yet.").color(look.t.dim));
    }
    for (i, label) in done.iter().enumerate() {
        if ui.selectable_label(i + 1 == done.len(), label).on_hover_text("Go back to just after this step").clicked() {
            s.ed.undo_to(i + 1);
        }
    }
    for label in &undone {
        if ui.selectable_label(false, RichText::new(label).color(look.t.dim)).on_hover_text("Redo up to here").clicked() {
            let target = label.clone();
            while s.ed.redo_label().is_some() {
                let last = s.ed.redo_label().map(str::to_string);
                s.ed.redo();
                if last.as_deref() == Some(target.as_str()) {
                    break;
                }
            }
        }
    }
}

fn keys(s: &mut Space3d, ui: &mut Ui, look: &Look) {
    ui.label(RichText::new("Blender shortcuts. Click a shortcut, then press the new keys (Esc keeps it).").small().color(look.t.dim));
    if ui.button("Reset to Blender defaults").clicked() {
        s.run("keymap.reset", Value::Null);
    }
    let bindings = s.ed.keymap.bindings.clone();
    egui::Grid::new("w3d-keys").num_columns(2).striped(true).show(ui, |ui| {
        for b in bindings {
            ui.label(&b.label);
            let waiting = s.rebinding.as_deref() == Some(b.action.as_str());
            let text = if waiting { "press keys…".to_string() } else if b.chord.is_empty() { "—".to_string() } else { b.chord.clone() };
            if ui.selectable_label(waiting, text).clicked() {
                s.rebinding = Some(b.action.clone());
            }
            ui.end_row();
        }
    });
}

fn help(ui: &mut Ui, look: &Look) {
    let lines = [
        ("Drawing in 3D", ""),
        ("", "Every curve lands on a 3D Guide. Pick Draw 3D Guide (Q) and draw: the stroke is pulled straight back along your view into a surface, like bent paper."),
        ("", "Turn the view (middle drag) and draw on the guide (D). Draw another guide from another side to build the shape."),
        ("", "Bend (Ctrl+B): draw from another view and the guide sweeps along that stroke (a circle bent by a bigger circle makes a doughnut)."),
        ("", "Loft joins curves in order into a guide; Primitives (Shift+A) give cubes, spheres, tubes, pyramids and planes."),
        ("", "Esc closes the guide; Save keeps it in Resources. Curves behind an opaque guide are protected from erasing and selecting."),
        ("Moving things (Blender)", ""),
        ("", "Select (W) and click or drag; A selects all, Alt+A none. G moves, R rotates, S scales: then X / Y / Z for an axis (twice: local), Shift+X for a plane, type a number, Ctrl snaps, Shift is precise. Click or Enter confirms; right-click or Esc cancels."),
        ("", "Shift+D duplicates and moves; X or Delete deletes; Ctrl+M mirrors; M moves curves to a group."),
        ("Views", ""),
        ("", "Middle drag orbits, Shift+middle pans, the wheel zooms. 1 / 3 / 7 front, right, top (Ctrl: the other side), 5 perspective, . frames the selection, Home frames everything, Alt+middle click snaps to the nearest view. The axis ball top right does the same with clicks."),
        ("The WobbleWorks part", ""),
        ("", "Lines boil (Space toggles). Painterly brushes (oil, gouache, dry brush, chalk, ink) have ragged edges, bristles, layers and an echo; their paint shimmers as they boil. Try \"Metaphor look\" in the brush panel."),
    ];
    for (head, body) in lines {
        if !head.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new(head).strong().color(look.t.ink));
        } else {
            ui.label(body);
        }
    }
}

/// The status line in the viewport's bottom-left corner.
pub fn status_line(s: &mut Space3d, ui: &mut Ui, look: &Look, rect: Rect) {
    if s.ed.in_modal() {
        return;
    }
    let text = if s.ed.status.is_empty() { format!("{}  ·  {} curves", s.ed.tool.label(), s.ed.scene.strokes.len()) } else { s.ed.status.clone() };
    let painter = ui.painter_at(rect);
    let pos = pos2(rect.min.x + 10.0, rect.max.y - 26.0);
    let galley = painter.layout_no_wrap(text, egui::FontId::proportional(13.0), look.t.ink);
    let bg = Rect::from_min_size(pos - vec2(6.0, 3.0), galley.size() + vec2(12.0, 6.0));
    painter.rect_filled(bg, 6.0, crate::theme::mix(look.t.card, Color32::TRANSPARENT, 0.15));
    painter.galley(pos, galley, look.t.ink);
}

pub fn show_ui_button(s: &mut Space3d, ui: &mut Ui, look: &Look, rect: Rect) {
    let r = Rect::from_min_size(pos2(rect.min.x + 10.0, rect.max.y - 46.0), vec2(36.0, 36.0));
    let resp = ui.interact(r, ui.id().with("w3d-show-ui"), egui::Sense::click());
    ui.painter().circle_filled(r.center(), 17.0, crate::theme::mix(look.t.card, Color32::TRANSPARENT, 0.2));
    photocraft_ui_egui::icons::paint(ui, r, "eye", 18.0, look.t.ink);
    if resp.on_hover_text("Show the UI (Ctrl+Space)").clicked() {
        s.hide_ui = false;
        s.playing = None;
    }
}

/// Popup menus at the mouse.
pub fn menus(s: &mut Space3d, ctx: &egui::Context, look: &Look) {
    let Some((menu, at)) = s.menu.clone() else { return };
    let release_closes = matches!(menu, Menu::Add | Menu::Context);
    let mut close = false;
    let area = egui::Area::new(egui::Id::new("w3d-popup")).order(egui::Order::Foreground).fixed_pos(at).constrain_to(ctx.content_rect());
    let inner = area.show(ctx, |ui| {
        widgets::card(ui, look, "w3d-popup-card", 10.0, |ui| {
            ui.set_min_width(180.0);
            match menu {
                Menu::Add => add_menu(s, ui),
                Menu::Delete => {
                    let n = s.ed.selection.len() + s.ed.selected_resources.len();
                    if ui.button(format!("Delete {n}")).clicked() {
                        s.run("edit.delete", Value::Null);
                        close = true;
                    }
                }
                Menu::Flip => {
                    ui.label("Mirror along");
                    ui.horizontal(|ui| {
                        for a in ["x", "y", "z"] {
                            if ui.button(a.to_uppercase()).clicked() {
                                s.run("edit.flip", json!({"axis": a}));
                                close = true;
                            }
                        }
                    });
                }
                Menu::MoveToGroup => {
                    ui.label("Move to group");
                    let groups: Vec<(u64, String)> = s.ed.scene.groups.iter().map(|g| (g.id, g.name.clone())).collect();
                    for (id, name) in groups {
                        if ui.button(name).clicked() {
                            s.run("group.moveStrokes", json!({"id": id}));
                            close = true;
                        }
                    }
                    if ui.button("+ New group").clicked() {
                        s.run("group.fromSelection", Value::Null);
                        close = true;
                    }
                }
                Menu::ViewPie => {
                    egui::Grid::new("w3d-pie").show(ui, |ui| {
                        for row in [["", "top", ""], ["left", "front", "right"], ["back", "bottom", "nearest"]] {
                            for v in row {
                                if v.is_empty() {
                                    ui.label("");
                                } else if ui.button(v).clicked() {
                                    s.run("camera.view", json!({"view": v}));
                                    close = true;
                                }
                            }
                            ui.end_row();
                        }
                    });
                }
                Menu::Search(mut q) => {
                    let r = ui.add(egui::TextEdit::singleline(&mut q).hint_text("Search commands"));
                    r.request_focus();
                    let ql = q.to_lowercase();
                    let hits: Vec<_> = s.ed.keymap.bindings.iter().filter(|b| ql.is_empty() || b.label.to_lowercase().contains(&ql)).take(14).cloned().collect();
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    for (i, b) in hits.iter().enumerate() {
                        let label = if b.chord.is_empty() { b.label.clone() } else { format!("{}  ({})", b.label, b.chord) };
                        if ui.button(label).clicked() || (enter && i == 0) {
                            s.menu = None;
                            s.dispatch(ctx, &b.command, b.params.clone());
                            return;
                        }
                    }
                    if let Some((Menu::Search(old), _)) = &mut s.menu {
                        *old = q;
                    }
                }
                Menu::Size => {
                    let mut v = s.ed.brush.size_mm;
                    if ui.add(egui::Slider::new(&mut v, SIZE_MIN_MM..=SIZE_MAX_MM).logarithmic(true).text("Size mm")).changed() {
                        s.run("brush.set", json!({"size": v}));
                    }
                    if ui.button("OK").clicked() {
                        close = true;
                    }
                }
                Menu::Opacity => {
                    let mut v = s.ed.brush.opacity;
                    if ui.add(egui::Slider::new(&mut v, 0.0..=1.0).text("Opacity")).changed() {
                        s.run("brush.set", json!({"opacity": v}));
                    }
                    if ui.button("OK").clicked() {
                        close = true;
                    }
                }
                Menu::Rename(id, mut name) => {
                    let r = ui.add(egui::TextEdit::singleline(&mut name));
                    r.request_focus();
                    let done = ui.input(|i| i.key_pressed(egui::Key::Enter)) || ui.button("Rename").clicked();
                    if done {
                        if s.ed.scene.group(id).is_some() {
                            s.run("group.rename", json!({"id": id, "name": name}));
                        } else {
                            s.run("resource.rename", json!({"id": id, "name": name}));
                        }
                        close = true;
                    } else if let Some((Menu::Rename(_, old), _)) = &mut s.menu {
                        *old = name;
                    }
                }
                Menu::Context => {
                    let sel = s.ed.has_selection();
                    for (label, cmd, params, enabled) in [
                        ("Undo", "edit.undo", Value::Null, s.ed.can_undo()),
                        ("Move (G)", "transform.start", json!({"mode": "grab"}), sel),
                        ("Rotate (R)", "transform.start", json!({"mode": "rotate"}), sel),
                        ("Scale (S)", "transform.start", json!({"mode": "scale"}), sel),
                        ("Duplicate", "edit.duplicate", json!({"mode": "inplace"}), !s.ed.selection.is_empty()),
                        ("Delete", "edit.delete", Value::Null, sel),
                        ("Select all", "select.all", Value::Null, true),
                        ("Frame all", "camera.frameAll", Value::Null, true),
                    ] {
                        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                            s.run(cmd, params);
                            close = true;
                        }
                    }
                    ui.separator();
                    add_menu(s, ui);
                }
            }
        })
    });
    // Close on a click outside, or after an item was used.
    let outside = ctx.input(|i| i.pointer.any_pressed()) && ctx.pointer_interact_pos().is_some_and(|p| !inner.response.rect.contains(p));
    let used_add = release_closes && ctx.input(|i| i.pointer.any_released()) && ctx.pointer_interact_pos().is_some_and(|p| inner.response.rect.contains(p));
    if close || outside || used_add {
        s.menu = None;
    }
    let _ = Pos2::ZERO;
}
