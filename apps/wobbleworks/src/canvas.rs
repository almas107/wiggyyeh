//! The canvas view: backdrop, the boiling picture, overlays, and pointer input for every tool.

use egui::{Color32, Event, Mesh, PointerButton, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2};

use crate::app::{App, Gesture, VIEW_CENTER, ZMAX, ZMIN};
use crate::geom;
use crate::model::{Brush, Pt};
use crate::settings::{Backdrop, Tool};
use crate::theme::mix;

/// Tiled textures for the backdrop pattern and the transparency checkerboard.
#[derive(Default)]
pub struct Tiles {
    backdrop: Option<((Backdrop, Color32, Color32), TextureHandle)>,
    checker: Option<((Color32, Color32), TextureHandle)>,
}

const TILE: usize = 28;

fn backdrop_image(b: Backdrop, paper: Color32, ink: Color32) -> egui::ColorImage {
    let dot = mix(paper, ink, 0.4);
    let soft = mix(paper, ink, 0.1);
    let mut px = vec![paper; TILE * TILE];
    for y in 0..TILE {
        for x in 0..TILE {
            let c = match b {
                Backdrop::Dots => {
                    let (dx, dy) = (x as f32 - 13.5, y as f32 - 13.5);
                    (dx * dx + dy * dy <= 5.0).then_some(dot)
                }
                Backdrop::Grid => (x == 0 || y == 0).then_some(soft).or_else(|| (x == 14 || y == 14).then_some(mix(paper, ink, 0.05))),
                Backdrop::Stripes => ((x + y) % TILE < 7).then_some(soft),
                Backdrop::Plain => None,
            };
            if let (Some(c), Some(p)) = (c, px.get_mut(y * TILE + x)) {
                *p = c;
            }
        }
    }
    egui::ColorImage::new([TILE, TILE], px)
}

const REPEAT: TextureOptions = TextureOptions {
    magnification: egui::TextureFilter::Linear,
    minification: egui::TextureFilter::Linear,
    wrap_mode: egui::TextureWrapMode::Repeat,
    mipmap_mode: None,
};
const REPEAT_NEAREST: TextureOptions = TextureOptions {
    magnification: egui::TextureFilter::Nearest,
    minification: egui::TextureFilter::Nearest,
    wrap_mode: egui::TextureWrapMode::Repeat,
    mipmap_mode: None,
};

/// One quad with a repeating texture; `scale` is screen points per texel.
fn tiled(painter: &egui::Painter, rect: Rect, tex: &TextureHandle, tile_pts: f32, origin: Pos2) {
    let mut m = Mesh::with_texture(tex.id());
    let uv = |p: Pos2| pos2((p.x - origin.x) / tile_pts, (p.y - origin.y) / tile_pts);
    m.add_rect_with_uv(rect, Rect::from_min_max(uv(rect.min), uv(rect.max)), Color32::WHITE);
    painter.add(m);
}

impl App {
    pub fn canvas_ui(&mut self, ui: &mut Ui, tiles: &mut Tiles) {
        let view = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(view, Sense::click_and_drag());
        let painter = ui.painter_at(view);
        let ctx = ui.ctx().clone();
        VIEW_CENTER.with(|c| c.set(view.size() / 2.0));

        // Fit on first show, after resizes and on request.
        let (dw, dh) = (self.doc.w as f32, self.doc.h as f32);
        if self.view.fit_pending && view.width() > 40.0 && view.height() > 40.0 {
            let pad = 40.0;
            let z = ((view.width() - pad) / dw).min((view.height() - pad) / dh).clamp(ZMIN, ZMAX);
            // Snap to whole multiples when magnifying so pixels stay even.
            let z = if z >= 1.0 { z.floor() } else { z };
            self.view.zoom = z;
            self.view.offset = (view.size() - vec2(dw, dh) * z) / 2.0;
            self.view.fit_pending = false;
        }

        // Backdrop.
        let t = self.look.t;
        let bkey = (self.s.backdrop, t.paper, t.ink);
        if tiles.backdrop.as_ref().is_none_or(|(k, _)| *k != bkey) {
            tiles.backdrop = Some((bkey, ctx.load_texture("wob-backdrop", backdrop_image(self.s.backdrop, t.paper, t.ink), REPEAT)));
        }
        if let Some((_, tex)) = &tiles.backdrop {
            tiled(&painter, view, tex, TILE as f32 / 2.0, view.min);
        }

        let zoom = self.view.zoom;
        let origin = view.min + self.view.offset;
        let canvas = Rect::from_min_size(origin, vec2(dw, dh) * zoom);
        let to_doc = move |p: Pos2| (f64::from((p.x - origin.x) / zoom), f64::from((p.y - origin.y) / zoom));
        let to_screen = move |(x, y): (f64, f64)| pos2(origin.x + x as f32 * zoom, origin.y + y as f32 * zoom);

        // Paper card with a hard shadow.
        let depth = (self.look.shadow * 1.5).max(2.0);
        painter.rect_filled(canvas.translate(vec2(depth, depth)).expand(self.look.line), 4.0, t.shadow);
        if self.doc.transparent {
            let ck = (self.s.checker_a, self.s.checker_b);
            if tiles.checker.as_ref().is_none_or(|(k, _)| *k != ck) {
                let img = egui::ColorImage::new([2, 2], vec![ck.0, ck.1, ck.1, ck.0]);
                tiles.checker = Some((ck, ctx.load_texture("wob-checker", img, REPEAT_NEAREST)));
            }
            if let Some((_, tex)) = &tiles.checker {
                tiled(&painter, canvas, tex, 24.0, canvas.min);
            }
        }
        self.r.sync(&self.doc);
        self.r.upload(&ctx);
        if let Some(tex) = self.r.texture(self.frame) {
            painter.image(tex, canvas, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        painter.rect_stroke(canvas.expand(self.look.line), 4.0, Stroke::new(self.look.line + 1.0, t.ink), StrokeKind::Outside);

        // Pixel grid when zoomed right in.
        if self.s.pixel_grid && zoom >= 8.0 {
            let vis = canvas.intersect(view);
            let grid = Stroke::new(1.0, Color32::from_black_alpha(28));
            let (x0, x1) = (((vis.min.x - origin.x) / zoom).floor() as i32, ((vis.max.x - origin.x) / zoom).ceil() as i32);
            let (y0, y1) = (((vis.min.y - origin.y) / zoom).floor() as i32, ((vis.max.y - origin.y) / zoom).ceil() as i32);
            for x in x0..=x1 {
                let sx = origin.x + x as f32 * zoom;
                painter.line_segment([pos2(sx, vis.min.y), pos2(sx, vis.max.y)], grid);
            }
            for y in y0..=y1 {
                let sy = origin.y + y as f32 * zoom;
                painter.line_segment([pos2(vis.min.x, sy), pos2(vis.max.x, sy)], grid);
            }
        }

        // Floating selection.
        let float_opacity = self.floating.as_ref().and_then(|f| self.doc.layers.iter().find(|l| l.id == f.layer)).map_or(1.0, |l| l.opacity);
        if let Some(f) = &mut self.floating {
            let painter_canvas = ui.painter_at(view);
            f.paint(&ctx, &painter_canvas, &mut self.r.brushes, &self.doc, self.frame, float_opacity, &to_screen);
            let pts: Vec<Pos2> = f.outline().into_iter().map(to_screen).collect();
            ants(&painter, &pts, self.frame, t.hot);
        }

        // Symmetry guides.
        let mirror = self.s.tools.mirror;
        if mirror != geom::Mirror::Off && matches!(self.s.tools.tool, Tool::Brush | Tool::Line | Tool::Rect | Tool::Ellipse) {
            let g = Stroke::new(1.0, t.hot.gamma_multiply(0.5));
            let c = canvas.center();
            if matches!(mirror, geom::Mirror::Horizontal | geom::Mirror::Quad) {
                painter.add(Shape::dashed_line(&[pos2(c.x, canvas.top()), pos2(c.x, canvas.bottom())], g, 6.0, 5.0));
            }
            if matches!(mirror, geom::Mirror::Vertical | geom::Mirror::Quad) {
                painter.add(Shape::dashed_line(&[pos2(canvas.left(), c.y), pos2(canvas.right(), c.y)], g, 6.0, 5.0));
            }
            if mirror == geom::Mirror::Radial6 {
                painter.circle_filled(c, 3.0, g.color);
            }
        }

        self.canvas_input(&ctx, &resp, view, &to_doc);

        // In-progress overlays.
        match &self.gesture {
            Gesture::Lasso(pts) if pts.len() > 1 => {
                let v: Vec<Pos2> = pts.iter().copied().map(to_screen).collect();
                ants(&painter, &v, self.frame, t.hot);
            }
            Gesture::Shape { a, b } => {
                let pts = self.shape_points(*a, *b);
                let v: Vec<Pos2> = pts.iter().map(|p| to_screen((p.x, p.y))).collect();
                let w = (self.s.tools.size as f32 * zoom).clamp(1.0, 400.0);
                let col = if self.s.tools.brush == Brush::Eraser { t.dim } else { self.s.tools.color };
                if self.s.tools.brush == Brush::Blob && v.len() > 2 {
                    painter.add(Shape::Path(egui::epaint::PathShape {
                        points: v,
                        closed: true,
                        fill: col.gamma_multiply(0.5),
                        stroke: Stroke::new(1.5, t.ink).into(),
                    }));
                } else {
                    painter.add(Shape::line(v, Stroke::new(w, col.gamma_multiply(0.75))));
                }
            }
            Gesture::Stroke if self.s.tools.brush == Brush::Blob => {
                if let Some(l) = &self.live {
                    for s in &l.strokes {
                        let v: Vec<Pos2> = s.pts.iter().map(|p| to_screen((p.x, p.y))).collect();
                        if v.len() > 2 {
                            painter.add(Shape::closed_line(v, Stroke::new(2.0, s.color)));
                        }
                    }
                }
            }
            _ => {}
        }

        // Cursor.
        if let Some(hover) = resp.hover_pos() {
            let tool = self.s.tools.tool;
            let panning = self.space_down || tool == Tool::Hand || matches!(self.gesture, Gesture::Pan { .. });
            let icon = if panning {
                if matches!(self.gesture, Gesture::Pan { .. }) { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab }
            } else if tool == Tool::Move
                || matches!(self.gesture, Gesture::DragFloat { .. })
                || (tool == Tool::Lasso && self.floating.as_ref().is_some_and(|f| f.contains(to_doc(hover))))
            {
                egui::CursorIcon::Move
            } else {
                egui::CursorIcon::Crosshair
            };
            ctx.set_cursor_icon(icon);
            if self.s.brush_cursor && !panning && matches!(tool, Tool::Brush | Tool::Line | Tool::Rect | Tool::Ellipse) && self.s.tools.brush != Brush::Blob {
                let r = (self.s.tools.size as f32 * zoom / 2.0).max(2.0);
                painter.circle_stroke(hover, r + 1.0, Stroke::new(1.0, Color32::WHITE));
                painter.circle_stroke(hover, r, Stroke::new(1.0, Color32::BLACK));
            }
        }
    }

    pub fn shape_points(&self, a: (f64, f64), b: (f64, f64)) -> Vec<Pt> {
        match self.s.tools.tool {
            Tool::Line => geom::line_pts(a, b),
            Tool::Rect => geom::rect_pts(a, b),
            _ => geom::ellipse_pts(a, b),
        }
    }

    fn canvas_input(&mut self, ctx: &egui::Context, resp: &egui::Response, view: Rect, to_doc: &dyn Fn(Pos2) -> (f64, f64)) {
        let (events, pointer, multi, zoom_delta, scroll, mods) =
            ctx.input(|i| (i.events.clone(), i.pointer.clone(), i.multi_touch(), i.zoom_delta(), i.smooth_scroll_delta(), i.modifiers));
        for e in &events {
            if let Event::Touch { force: Some(f), .. } = e {
                self.pressure = (0.35 + f).clamp(0.2, 1.6);
            }
        }
        if let Some(p) = crate::platform::pen_pressure() {
            self.pressure = (0.35 + p).clamp(0.2, 1.6);
        }

        // Two fingers: pinch to zoom, drag to pan; abandon any stroke.
        if let Some(m) = multi.filter(|m| m.num_touches >= 2) {
            if resp.hovered() || resp.is_pointer_button_down_on() {
                self.cancel_stroke();
                let anchor = m.center_pos - view.min;
                self.zoom_at(self.view.zoom * m.zoom_delta, anchor);
                self.view.offset += m.translation_delta;
            }
            return;
        }

        if resp.hovered() {
            if (zoom_delta - 1.0).abs() > 1e-4
                && let Some(p) = pointer.hover_pos()
            {
                self.zoom_at(self.view.zoom * zoom_delta, p - view.min);
            } else if scroll != Vec2::ZERO && !mods.command {
                self.view.offset += scroll;
            }
        }

        // Keep at least a corner of the canvas on screen.
        let size = vec2(self.doc.w as f32, self.doc.h as f32) * self.view.zoom;
        let margin = 48.0;
        self.view.offset.x = self.view.offset.x.clamp(margin - size.x, view.width() - margin);
        self.view.offset.y = self.view.offset.y.clamp(margin - size.y, view.height() - margin);

        // Pan: middle button, Space, or the hand tool.
        let pan_start = resp.hovered()
            && (pointer.button_pressed(PointerButton::Middle) || (pointer.primary_pressed() && (self.space_down || self.s.tools.tool == Tool::Hand)));
        if pan_start && let Some(p) = pointer.interact_pos() {
            self.gesture = Gesture::Pan { from: p, offset0: self.view.offset };
        }
        if let Gesture::Pan { from, offset0 } = self.gesture {
            if let Some(p) = pointer.latest_pos() {
                self.view.offset = offset0 + (p - from);
            }
            if !pointer.any_down() {
                self.gesture = Gesture::None;
            }
            return;
        }

        let Some(pos) = pointer.interact_pos() else { return };
        if pointer.primary_pressed() && resp.hovered() {
            self.press(to_doc(pos), mods.shift);
        }
        if pointer.primary_down() {
            let moves: Vec<Pos2> = events.iter().filter_map(|e| if let Event::PointerMoved(p) = e { Some(*p) } else { None }).collect();
            for p in moves {
                self.drag(to_doc(p), mods.shift);
            }
        }
        if pointer.primary_released() {
            self.release(to_doc(pos), mods.shift);
        }
    }

    fn press(&mut self, p: (f64, f64), _shift: bool) {
        match self.s.tools.tool {
            Tool::Brush => self.begin_stroke(p),
            Tool::Line | Tool::Rect | Tool::Ellipse => {
                self.apply_floating();
                self.gesture = Gesture::Shape { a: p, b: p };
            }
            Tool::Fill => self.fill_at(p),
            Tool::Pick => {
                self.gesture = Gesture::Pick;
                self.pick_at(p);
            }
            Tool::Lasso => {
                if let Some(f) = &self.floating
                    && f.contains(p)
                {
                    self.gesture = Gesture::DragFloat { grab: (p.0 - f.xf.dx, p.1 - f.xf.dy) };
                    return;
                }
                self.apply_floating();
                self.gesture = Gesture::Lasso(vec![p]);
            }
            Tool::Move => {
                if self.floating.is_none() && !self.lift_layer() {
                    return;
                }
                if let Some(f) = &self.floating {
                    self.gesture = Gesture::DragFloat { grab: (p.0 - f.xf.dx, p.1 - f.xf.dy) };
                }
            }
            Tool::Hand => {}
        }
    }

    fn drag(&mut self, p: (f64, f64), shift: bool) {
        let zoom = f64::from(self.view.zoom.max(0.01));
        match &mut self.gesture {
            Gesture::Stroke => self.feed_stroke(p, false),
            Gesture::Shape { a, b } => {
                *b = if shift { geom::constrain(*a, p, self.s.tools.tool == Tool::Line) } else { p };
            }
            Gesture::Lasso(pts) => {
                if pts.last().is_none_or(|l| (l.0 - p.0).hypot(l.1 - p.1) * zoom > 3.0) && pts.len() < 20_000 {
                    pts.push(p);
                }
            }
            Gesture::DragFloat { grab } => {
                if let Some(f) = &mut self.floating {
                    f.xf.dx = p.0 - grab.0;
                    f.xf.dy = p.1 - grab.1;
                }
            }
            Gesture::Pick => self.pick_at(p),
            Gesture::None | Gesture::Pan { .. } => {}
        }
    }

    fn release(&mut self, p: (f64, f64), shift: bool) {
        self.drag(p, shift);
        match std::mem::replace(&mut self.gesture, Gesture::None) {
            Gesture::Stroke => {
                self.gesture = Gesture::Stroke;
                self.end_stroke();
            }
            Gesture::Shape { a, b } => {
                if (a.0 - b.0).hypot(a.1 - b.1) >= 1.0 {
                    let pts = self.shape_points(a, b);
                    self.commit_shape(pts);
                }
            }
            Gesture::Lasso(pts) => self.finish_lasso(pts),
            Gesture::Pick => {
                let c = self.s.tools.color;
                self.s.remember_color(c);
            }
            _ => {}
        }
    }
}

/// Marching ants along a closed outline.
fn ants(painter: &egui::Painter, pts: &[Pos2], frame: usize, hot: Color32) {
    if pts.len() < 2 {
        return;
    }
    let mut closed = pts.to_vec();
    if let Some(first) = pts.first() {
        closed.push(*first);
    }
    painter.add(Shape::line(closed.clone(), Stroke::new(3.0, Color32::WHITE)));
    let offset = (frame % 4) as f32 * 2.5;
    painter.add(Shape::dashed_line_with_offset(&closed, Stroke::new(1.8, hot), &[6.0], &[4.0], offset));
}
