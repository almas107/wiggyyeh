//! The 3D viewport: draws the editor's frame as egui meshes and turns mouse, pen and touch input
//! into navigation (Blender's: middle drag orbits, Shift pans, Ctrl zooms, the wheel zooms,
//! Alt+middle click snaps to the nearest view) and tool events.

use std::collections::HashMap;

use egui::{Color32, Event, Mesh, PointerButton, Pos2, Rect, Shape, Stroke, TextureHandle, Ui, pos2, vec2};
use wobbleworks_3d::editor::{Editor, Mods, Overlay};
use wobbleworks_3d::render::{Frame, Tex};
use wobbleworks_3d::transform::Part;

use crate::widgets::Look;

/// Axis colours (X red, Y green, Z blue, view white), as in Blender.
pub const AXIS: [Color32; 4] = [
    Color32::from_rgb(0xe8, 0x44, 0x4a),
    Color32::from_rgb(0x6c, 0xc0, 0x3c),
    Color32::from_rgb(0x44, 0x7c, 0xf0),
    Color32::from_rgb(0xf4, 0xf4, 0xf4),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavMode {
    Orbit,
    Pan,
    Zoom,
    Lens,
}

/// GPU textures for the atlas and image resources, uploaded when they change.
#[derive(Default)]
pub struct Textures {
    atlas: Option<(u64, u64, TextureHandle)>,
    images: HashMap<u64, TextureHandle>,
}

impl Textures {
    fn sync(&mut self, ctx: &egui::Context, ed: &Editor) {
        let a = &ed.atlas;
        let stale = self.atlas.as_ref().is_none_or(|(rev, gen_, _)| *rev != a.revision || *gen_ != a.generation);
        if stale && a.height() > 0 {
            let img = egui::ColorImage::from_rgba_unmultiplied([a.width(), a.height()], &a.pixels);
            match &mut self.atlas {
                Some((rev, gen_, h)) if h.size() == [a.width(), a.height()] => {
                    h.set(img, egui::TextureOptions::LINEAR);
                    *rev = a.revision;
                    *gen_ = a.generation;
                }
                _ => self.atlas = Some((a.revision, a.generation, ctx.load_texture("wobble3d-atlas", img, egui::TextureOptions::LINEAR))),
            }
        }
        let live: Vec<u64> = ed.scene.images.iter().map(|i| i.id).collect();
        self.images.retain(|id, _| live.contains(id));
        for im in &ed.scene.images {
            if self.images.contains_key(&im.id) {
                continue;
            }
            let (w, h) = (im.width as usize, im.height as usize);
            if im.rgba.len() == w * h * 4 {
                let img = egui::ColorImage::from_rgba_unmultiplied([w, h], &im.rgba);
                self.images.insert(im.id, ctx.load_texture(format!("wobble3d-image-{}", im.id), img, egui::TextureOptions::LINEAR));
            }
        }
    }

    fn id(&self, tex: Tex) -> Option<egui::TextureId> {
        match tex {
            Tex::Atlas => self.atlas.as_ref().map(|(_, _, h)| h.id()),
            Tex::Image(i) => self.images.get(&i).map(TextureHandle::id),
        }
    }
}

/// The frame as egui shapes at `origin`.
pub fn shapes(frame: &Frame, textures: &Textures, origin: Pos2) -> Vec<Shape> {
    let mut out = Vec::with_capacity(frame.batches.len());
    for b in &frame.batches {
        let Some(tid) = textures.id(b.tex) else { continue };
        let mut m = Mesh::with_texture(tid);
        m.vertices.reserve(b.vertices.len());
        for v in &b.vertices {
            let [r, g, bl, a] = v.color;
            m.vertices.push(egui::epaint::Vertex {
                pos: pos2(origin.x + v.pos[0], origin.y + v.pos[1]),
                uv: pos2(v.uv[0], v.uv[1]),
                color: Color32::from_rgba_premultiplied(r, g, bl, a),
            });
        }
        m.indices.clone_from(&b.indices);
        out.push(Shape::mesh(m));
    }
    out
}

/// (editor revision, viewport size): while it holds, each boil frame's picture is reused.
type CacheKey = (u64, [u32; 2]);
/// Each boil frame's shapes and triangle count.
type Pictures = HashMap<u32, (Vec<Shape>, usize)>;

/// What the viewport keeps between frames.
pub struct Viewport {
    pub textures: Textures,
    /// The cached pictures, one per boil frame, for one editor revision and size (a still view
    /// that boils costs nothing after the first loop). Meshes are shared (`Arc`), so drawing
    /// one again is cheap.
    cache: Option<(CacheKey, Pictures)>,
    nav: Option<(NavMode, Pos2)>,
    /// The primary button went down in the viewport (and goes to the editor until it lifts).
    drawing: bool,
    /// Alt+middle click: snap when released without dragging.
    alt_click: Option<Pos2>,
    /// Emulate a three-button mouse: Alt+left drag orbits (Blender preference).
    pub emulate_mmb: bool,
    pub rect: Rect,
    /// Where the cached pictures were laid out.
    cache_origin: Pos2,
    /// Triangles drawn last frame (for the stats line).
    pub triangles: usize,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport { textures: Textures::default(), cache: None, nav: None, drawing: false, alt_click: None, emulate_mmb: false, rect: Rect::NOTHING, cache_origin: Pos2::ZERO, triangles: 0 }
    }
}

fn mods(m: egui::Modifiers) -> Mods {
    Mods { shift: m.shift, ctrl: m.command || m.ctrl, alt: m.alt }
}

impl Viewport {
    /// Show the viewport in `rect`; returns whether the right button asked for a context menu.
    pub fn show(&mut self, ui: &mut Ui, ed: &mut Editor, look: &Look, rect: Rect, frame_index: u32, pressure: f32) -> Option<Pos2> {
        self.rect = rect;
        let ctx = ui.ctx().clone();
        ed.set_viewport(rect.width(), rect.height());
        let id = ui.id().with("wobble3d-viewport");
        let resp = ui.interact(rect, id, egui::Sense::click_and_drag());
        let layer = ui.layer_id();
        let over_us = |p: Pos2| rect.contains(p) && ctx.layer_id_at(p).is_none_or(|l| l == layer);
        let t = ctx.input(|i| i.time);
        let local = |p: Pos2| (p.x - rect.min.x, p.y - rect.min.y);
        let mut context_menu = None;

        let events = ctx.input(|i| i.events.clone());
        for ev in &events {
            match ev {
                Event::PointerButton { pos, button, pressed, modifiers } => {
                    let m = mods(*modifiers);
                    let (x, y) = local(*pos);
                    match (button, pressed) {
                        (PointerButton::Middle, true) if over_us(*pos) => {
                            let mode = if modifiers.shift {
                                NavMode::Pan
                            } else if modifiers.command || modifiers.ctrl {
                                NavMode::Zoom
                            } else {
                                NavMode::Orbit
                            };
                            if modifiers.alt {
                                self.alt_click = Some(*pos);
                            }
                            self.nav = Some((mode, *pos));
                        }
                        (PointerButton::Middle, false) => {
                            if let Some(start) = self.alt_click.take()
                                && start.distance(*pos) < 4.0
                            {
                                let _ = ed.run("camera.view", &serde_json::json!({"view": "nearest"}));
                            }
                            self.nav = None;
                        }
                        (PointerButton::Primary, true) if over_us(*pos) => {
                            if self.emulate_mmb && modifiers.alt && !ed.in_modal() {
                                self.nav = Some((NavMode::Orbit, *pos));
                            } else {
                                self.drawing = true;
                                ed.pointer_down(x, y, pressure, t, m);
                            }
                        }
                        (PointerButton::Primary, false) => {
                            if self.nav.is_some() && self.emulate_mmb {
                                self.nav = None;
                            }
                            if self.drawing {
                                self.drawing = false;
                                ed.pointer_up(x, y, t, m);
                            }
                        }
                        (PointerButton::Secondary, true) if over_us(*pos) && !ed.cancel() => {
                            context_menu = Some(*pos);
                        }
                        _ => {}
                    }
                }
                Event::PointerMoved(pos) => {
                    if let Some((mode, last)) = self.nav {
                        let d = *pos - last;
                        match mode {
                            NavMode::Orbit => ed.camera.orbit(d.x, d.y),
                            NavMode::Pan => ed.camera.pan(d.x, d.y),
                            NavMode::Zoom => ed.camera.zoom((-d.y * 0.01).exp()),
                            NavMode::Lens => {
                                let mm = ed.camera.focal_mm * (-d.y * 0.01).exp();
                                ed.camera.set_focal(mm);
                            }
                        }
                        if d.length() > 3.0 {
                            self.alt_click = None;
                        }
                        self.nav = Some((mode, *pos));
                        let _ = ed.run("camera.set", &serde_json::Value::Null);
                    } else if self.drawing || ed.in_modal() || rect.contains(*pos) {
                        let (x, y) = local(*pos);
                        let m = mods(ctx.input(|i| i.modifiers));
                        ed.pointer_move(x, y, pressure, t, m);
                    }
                }
                Event::MouseWheel { delta, modifiers, .. } if resp.hovered() => {
                    let dy = delta.y + delta.x;
                    if modifiers.command || modifiers.ctrl {
                        let _ = ed.run("camera.fov", &serde_json::json!({"by": -dy * 2.0}));
                    } else if modifiers.shift {
                        let _ = ed.run("camera.pan", &serde_json::json!({"dx": 0.0, "dy": dy * 20.0}));
                    } else {
                        let _ = ed.run("camera.zoom", &serde_json::json!({"factor": (dy * 0.25).exp()}));
                    }
                }
                Event::PointerGone => {
                    self.nav = None;
                }
                _ => {}
            }
        }
        // Two-finger touch: pinch zooms, drag pans (Feather's gestures on touch screens).
        if let Some(mt) = ctx.multi_touch()
            && resp.hovered()
        {
            if (mt.zoom_delta - 1.0).abs() > 1e-4 {
                ed.camera.zoom(mt.zoom_delta);
            }
            let d = mt.translation_delta;
            if d.length() > 0.0 {
                ed.camera.pan(d.x, d.y);
            }
            let _ = ed.run("camera.set", &serde_json::Value::Null);
        }
        // Pinch on a trackpad.
        let zoom = ctx.input(|i| i.zoom_delta());
        if resp.hovered() && (zoom - 1.0).abs() > 1e-4 && ctx.multi_touch().is_none() {
            ed.camera.zoom(zoom);
            let _ = ed.run("camera.set", &serde_json::Value::Null);
        }
        if self.drawing {
            ed.tick(t);
        }
        if self.drawing || ed.in_modal() || self.nav.is_some() {
            ctx.request_repaint();
        }

        // The picture.
        let painter = ui.painter_at(rect);
        let bg = ed.scene.environment.background.0;
        painter.rect_filled(rect, 0.0, Color32::from_rgb(bg[0], bg[1], bg[2]));
        let key = (ed.revision, [rect.width() as u32, rect.height() as u32]);
        let origin_moved = self.cache_origin != rect.min;
        if self.cache.as_ref().is_none_or(|(k, _)| *k != key) || origin_moved {
            self.cache = Some((key, HashMap::new()));
            self.cache_origin = rect.min;
        }
        let cached = self.cache.as_ref().and_then(|(_, m)| m.get(&frame_index)).is_some();
        if !cached {
            let f = ed.render(frame_index, true);
            self.textures.sync(&ctx, ed);
            let s = shapes(&f, &self.textures, rect.min);
            if let Some((_, m)) = &mut self.cache {
                m.insert(frame_index, (s, f.triangles));
            }
        }
        if let Some((s, tris)) = self.cache.as_ref().and_then(|(_, m)| m.get(&frame_index)) {
            self.triangles = *tris;
            painter.extend(s.iter().cloned());
        }
        let overlay = ed.overlay();
        self.overlay(&painter, &overlay, look, rect, ed);
        if resp.hovered() {
            let icon = match ed.tool {
                _ if ed.in_modal() => egui::CursorIcon::Move,
                wobbleworks_3d::editor::Tool::Select | wobbleworks_3d::editor::Tool::Deselect => egui::CursorIcon::Default,
                _ => egui::CursorIcon::Crosshair,
            };
            ctx.set_cursor_icon(icon);
        }
        context_menu
    }

    fn overlay(&self, painter: &egui::Painter, o: &Overlay, look: &Look, rect: Rect, ed: &Editor) {
        let at = |p: [f32; 2]| pos2(rect.min.x + p[0], rect.min.y + p[1]);
        let ink = look.t.ink;
        if o.polyline.len() > 1 {
            let pts: Vec<Pos2> = o.polyline.iter().map(|p| at(*p)).collect();
            painter.add(Shape::line(pts.clone(), Stroke::new(4.0, Color32::from_white_alpha(200))));
            painter.add(Shape::line(pts, Stroke::new(2.0, crate::space3d::ORANGE)));
        }
        if let Some([x0, y0, x1, y1]) = o.rect {
            let r = Rect::from_two_pos(at([x0, y0]), at([x1, y1]));
            painter.rect_filled(r, 2.0, Color32::from_rgba_unmultiplied(80, 140, 255, 30));
            painter.rect_stroke(r, 2.0, Stroke::new(1.5, Color32::from_rgb(80, 140, 255)), egui::StrokeKind::Inside);
        }
        if let Some((r, inner)) = o.circle {
            let c = at(ed.mouse);
            painter.circle_stroke(c, r, Stroke::new(1.5, Color32::from_black_alpha(160)));
            painter.circle_stroke(c, r + 1.0, Stroke::new(1.0, Color32::from_white_alpha(160)));
            if inner > 0.5 {
                painter.circle_stroke(c, inner, Stroke::new(1.0, Color32::from_black_alpha(90)));
            }
        }
        if let Some((a, b, axis)) = o.axis_line {
            painter.line_segment([at(a), at(b)], Stroke::new(1.5, AXIS.get(axis).copied().unwrap_or(ink)));
        }
        let hover = wobbleworks_3d::transform::pick(&o.gizmo, ed.mouse);
        for part in &o.gizmo {
            draw_part(painter, part, rect.min, hover == Some(part.handle));
        }
        if o.adjusting {
            let c = at(ed.mouse);
            painter.circle_filled(c, 5.0, crate::space3d::ORANGE);
        }
        if let Some(h) = &o.header {
            let pos = rect.left_top() + vec2(12.0, 10.0);
            let galley = painter.layout_no_wrap(h.clone(), egui::FontId::proportional(14.0), Color32::WHITE);
            let bg = Rect::from_min_size(pos - vec2(6.0, 4.0), galley.size() + vec2(12.0, 8.0));
            painter.rect_filled(bg, 6.0, Color32::from_black_alpha(170));
            painter.galley(pos, galley, Color32::WHITE);
        }
    }
}

fn draw_part(painter: &egui::Painter, part: &Part, origin: Pos2, hovered: bool) {
    let c = AXIS.get(part.axis).copied().unwrap_or(Color32::WHITE);
    let c = if hovered { lighten(c) } else { c };
    let pts: Vec<Pos2> = part.line.iter().map(|p| pos2(origin.x + p[0], origin.y + p[1])).collect();
    if part.filled && pts.len() >= 3 {
        painter.add(Shape::convex_polygon(pts, c, Stroke::new(1.0, Color32::from_black_alpha(120))));
    } else if pts.len() >= 2 {
        let w = if hovered { 4.0 } else { 2.5 };
        painter.add(Shape::line(pts.clone(), Stroke::new(w + 1.5, Color32::from_black_alpha(90))));
        painter.add(Shape::line(pts, Stroke::new(w, c)));
    }
}

fn lighten(c: Color32) -> Color32 {
    let f = |v: u8| v.saturating_add(((255 - v as u16) / 2) as u8);
    Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

/// Blender's navigation gizmo: an axis ball (click an axis to look along it, drag to orbit),
/// with zoom, pan, projection and frame buttons under it. Returns whether it used the pointer.
pub fn navigator(ui: &mut Ui, ed: &mut Editor, look: &Look, rect: Rect) {
    let r = 46.0;
    let centre = pos2(rect.max.x - r - 14.0, rect.min.y + r + 14.0);
    let ball = Rect::from_center_size(centre, vec2(r * 2.0, r * 2.0));
    let id = ui.id().with("wobble3d-nav-ball");
    let resp = ui.interact(ball, id, egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let hovered = resp.hovered() || resp.dragged();
    painter.circle_filled(centre, r, Color32::from_black_alpha(if hovered { 60 } else { 28 }));
    let view = ed.view();
    // Each axis end: (screen pos, depth towards the viewer, axis, positive?).
    let mut ends: Vec<(Pos2, f32, usize, bool)> = Vec::new();
    for (k, a) in [wobbleworks_3d::math::Vec3::X, wobbleworks_3d::math::Vec3::Y, wobbleworks_3d::math::Vec3::Z].iter().enumerate() {
        for positive in [true, false] {
            let d = if positive { *a } else { -*a };
            let p = centre + vec2(d.dot(view.right), -d.dot(view.up)) * (r - 12.0);
            ends.push((p, d.dot(view.back), k, positive));
        }
    }
    ends.sort_by(|a, b| a.1.total_cmp(&b.1));
    let mouse = ui.ctx().pointer_hover_pos();
    let mut clicked_end = None;
    for (p, _, k, positive) in &ends {
        let c = AXIS.get(*k).copied().unwrap_or(Color32::WHITE);
        if *positive {
            painter.line_segment([centre, *p], Stroke::new(2.5, c));
        }
        let rad = if *positive { 9.0 } else { 7.0 };
        let hot = mouse.is_some_and(|m| m.distance(*p) <= rad + 2.0);
        let fill = if *positive { c } else { Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 110) };
        painter.circle_filled(*p, rad + if hot { 2.0 } else { 0.0 }, fill);
        if hot {
            painter.circle_stroke(*p, rad + 2.0, Stroke::new(1.5, Color32::WHITE));
        }
        if *positive {
            let name = ["X", "Y", "Z"].get(*k).copied().unwrap_or("");
            painter.text(*p, egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(11.0), look.t.ink);
        }
        if hot && resp.clicked() {
            clicked_end = Some((*k, *positive));
        }
    }
    if let Some((k, positive)) = clicked_end {
        let v = match (k, positive) {
            (0, true) => "right",
            (0, false) => "left",
            (1, true) => "top",
            (1, false) => "bottom",
            (2, true) => "front",
            _ => "back",
        };
        let _ = ed.run("camera.view", &serde_json::json!({"view": v}));
    } else if resp.dragged() {
        let d = resp.drag_delta();
        let _ = ed.run("camera.orbit", &serde_json::json!({"dx": d.x, "dy": d.y}));
    }
    let resp = resp.on_hover_text("Click an axis to look along it; drag to orbit (middle mouse drag in the view does the same)");
    let _ = resp;
    // Buttons: zoom (drag), pan (drag), perspective / orthographic, frame all.
    let mut y = centre.y + r + 10.0;
    let buttons: [(&str, &str, &str); 4] = [
        ("zoom-in", "zoom", "Drag to zoom (Ctrl+middle drag, or the wheel)"),
        ("hand", "pan", "Drag to pan (Shift+middle drag)"),
        ("grid-2x2", "persp", "Perspective / orthographic (5)"),
        ("maximize-2", "frame", "Frame everything (Home)"),
    ];
    for (icon, what, tip) in buttons {
        let b = Rect::from_center_size(pos2(centre.x, y + 16.0), vec2(30.0, 30.0));
        let resp = ui.interact(b, ui.id().with(("wobble3d-nav", what)), egui::Sense::click_and_drag());
        let on = what == "persp" && ed.camera.orthographic;
        painter.circle_filled(b.center(), 15.0, if resp.hovered() || on { Color32::from_black_alpha(80) } else { Color32::from_black_alpha(30) });
        photocraft_ui_egui::icons::paint(ui, b, icon, 18.0, look.t.ink);
        let d = resp.drag_delta();
        match what {
            "zoom" if resp.dragged() => ed.camera.zoom((-d.y * 0.01).exp()),
            "pan" if resp.dragged() => ed.camera.pan(d.x, d.y),
            "persp" if resp.clicked() => {
                let _ = ed.run("camera.toggleProjection", &serde_json::Value::Null);
            }
            "frame" if resp.clicked() => {
                let _ = ed.run("camera.frameAll", &serde_json::Value::Null);
            }
            _ => {}
        }
        if resp.dragged() {
            let _ = ed.run("camera.set", &serde_json::Value::Null);
        }
        resp.on_hover_text(tip);
        y += 36.0;
    }
}
