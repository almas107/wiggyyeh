//! Chunky cartoon widgets: outlined buttons with hard drop shadows that squash when pressed,
//! icon tiles, swatches and collapsible cards. When "boiling UI" is on, outlines jitter by half
//! a pixel in step with the canvas, like the drawing they sit around.

use egui::{Align2, Color32, FontId, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, vec2};

use crate::icons::{self, Icon};
use crate::theme::{Look, mix};

/// Paint the chunky body of a widget: shadow, fill, outline. Returns the body rect.
fn body(ui: &Ui, look: &Look, rect: Rect, resp: &Response, fill: Color32, enabled: bool) -> Rect {
    let pressed = resp.is_pointer_button_down_on() && enabled;
    let depth = look.shadow.clamp(0.0, 12.0);
    let j = look.jitter(resp.id);
    let r = rect.translate(j);
    let r = if pressed { r.translate(vec2(depth, depth) * 0.8) } else { r };
    let p = ui.painter();
    if !pressed && depth > 0.0 {
        p.rect_filled(rect.translate(vec2(depth, depth) * 0.6 + j), look.radius(), if enabled { look.t.shadow } else { mix(look.t.shadow, look.t.card, 0.6) });
    }
    let fill = if !enabled {
        mix(fill, look.t.paper, 0.5)
    } else if resp.hovered() && !pressed {
        mix(fill, look.t.sun, 0.22)
    } else {
        fill
    };
    p.rect(r, look.radius(), fill, Stroke::new(look.line, if enabled { look.t.ink } else { mix(look.t.ink, look.t.card, 0.5) }), StrokeKind::Inside);
    if resp.has_focus() {
        p.rect_stroke(r.expand(3.0), look.radius(), Stroke::new(2.5, look.t.hot), StrokeKind::Outside);
    }
    r
}

/// A text button. `on` shows it toggled.
pub fn button(ui: &mut Ui, look: &Look, text: &str, on: bool) -> Response {
    button_ex(ui, look, text, on, true)
}

pub fn button_ex(ui: &mut Ui, look: &Look, text: &str, on: bool, enabled: bool) -> Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, look.t.ink);
    let pad = ui.spacing().button_padding;
    let size = vec2(galley.size().x + pad.x * 2.0, (galley.size().y + pad.y * 2.0).max(ui.spacing().interact_size.y)) + Vec2::splat(look.shadow * 0.6);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        let inner = Rect::from_min_size(rect.min, rect.size() - Vec2::splat(look.shadow * 0.6));
        let r = body(ui, look, inner, &resp, if on { look.t.sun } else { look.t.card }, enabled);
        let color = if enabled { look.t.ink } else { mix(look.t.ink, look.t.card, 0.5) };
        ui.painter().galley(r.center() - galley.size() / 2.0, galley, color);
    }
    resp.on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::NotAllowed })
}

/// A square icon tile, optionally with a caption (Settings → Look → "Show tool names").
pub fn icon_button(ui: &mut Ui, look: &Look, icon: Icon, on: bool, tip: &str, caption: Option<&str>, accent: Color32) -> Response {
    icon_button_sized(ui, look, icon, on, tip, caption, accent, 38.0, true)
}

#[allow(clippy::too_many_arguments)]
pub fn icon_button_sized(
    ui: &mut Ui,
    look: &Look,
    icon: Icon,
    on: bool,
    tip: &str,
    caption: Option<&str>,
    accent: Color32,
    side: f32,
    enabled: bool,
) -> Response {
    let cap = caption.filter(|_| look.labels);
    let extra = if cap.is_some() { 14.0 } else { 0.0 };
    let size = vec2(side, side + extra) + Vec2::splat(look.shadow * 0.6);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        let inner = Rect::from_min_size(rect.min, rect.size() - Vec2::splat(look.shadow * 0.6));
        let r = body(ui, look, inner, &resp, if on { look.t.sun } else { look.t.card }, enabled);
        let ink = if enabled { look.t.ink } else { mix(look.t.ink, look.t.card, 0.55) };
        let icon_rect = Rect::from_min_size(r.min, vec2(r.width(), r.width())).shrink(r.width() * 0.16);
        icons::paint(ui.painter(), icon_rect, icon, ink, if enabled { accent } else { ink });
        if let Some(c) = cap {
            ui.painter().text(egui::pos2(r.center().x, r.max.y - 8.0), Align2::CENTER_CENTER, c, FontId::proportional(9.5), ink);
        }
    }
    let resp = if tip.is_empty() { resp } else { resp.on_hover_text(tip) };
    resp.on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::NotAllowed })
}

/// A colour swatch; `on` rings it.
pub fn swatch(ui: &mut Ui, look: &Look, c: Color32, on: bool, side: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(side), Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let r = rect.shrink(1.0).translate(look.jitter(resp.id));
        let rr = egui::CornerRadius::same((look.round * 0.6).clamp(0.0, 12.0) as u8);
        if c.a() < 255 {
            checker(p, r, 4.0, Color32::WHITE, Color32::from_gray(200));
        }
        p.rect(r, rr, c, Stroke::new(look.line.min(2.0), look.t.ink), StrokeKind::Inside);
        if on {
            p.rect_stroke(r.expand(2.5), rr, Stroke::new(2.5, look.t.hot), StrokeKind::Outside);
        }
        if resp.hovered() {
            p.rect_stroke(r.expand(1.0), rr, Stroke::new(1.5, look.t.ink), StrokeKind::Outside);
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A small checkerboard (for transparent swatches).
pub fn checker(p: &egui::Painter, r: Rect, cell: f32, a: Color32, b: Color32) {
    p.rect_filled(r, 0.0, a);
    let (nx, ny) = ((r.width() / cell).ceil() as i32, (r.height() / cell).ceil() as i32);
    for y in 0..ny {
        for x in 0..nx {
            if (x + y) % 2 == 1 {
                let c = Rect::from_min_size(r.min + vec2(x as f32 * cell, y as f32 * cell), Vec2::splat(cell)).intersect(r);
                p.rect_filled(c, 0.0, b);
            }
        }
    }
}

/// A cartoon card with a clickable header that folds it away. The open state is remembered.
pub fn card<R>(ui: &mut Ui, look: &Look, id: &str, title: &str, default_open: bool, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let id = Id::new(("wob-card", id));
    let mut open = ui.data_mut(|d| *d.get_persisted_mut_or(id, default_open));
    let frame = egui::Frame::new()
        .fill(look.t.card)
        .stroke(look.outline())
        .corner_radius(look.radius())
        .shadow(look.hard_shadow())
        .inner_margin(egui::Margin::same(10));
    let out = frame
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let head = ui.horizontal(|ui| {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::click());
                let p = ui.painter();
                let tri = if open {
                    vec![r.left_center() + vec2(0.0, -4.0), r.left_center() + vec2(9.0, -4.0), r.left_center() + vec2(4.5, 4.0)]
                } else {
                    vec![r.left_center() + vec2(1.0, -5.0), r.left_center() + vec2(9.0, 0.0), r.left_center() + vec2(1.0, 5.0)]
                };
                p.add(egui::Shape::convex_polygon(tri, look.t.hot, Stroke::NONE));
                for dx in [0.0, 0.6] {
                    p.text(r.left_center() + vec2(16.0 + dx, 0.0), Align2::LEFT_CENTER, title.to_uppercase(), FontId::proportional(11.5), look.t.dim);
                }
                resp
            });
            if head.inner.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                open = !open;
            }
            if open {
                // Dashed rule under the title, like the original.
                let y = ui.cursor().top() - 2.0;
                let (l, r) = (ui.min_rect().left(), ui.max_rect().right());
                ui.painter().add(egui::Shape::dashed_line(
                    &[egui::pos2(l, y), egui::pos2(r, y)],
                    Stroke::new(1.5, mix(look.t.ink, look.t.card, 0.6)),
                    5.0,
                    4.0,
                ));
                ui.add_space(4.0);
                Some(add(ui))
            } else {
                None
            }
        })
        .inner;
    ui.data_mut(|d| d.insert_persisted(id, open));
    ui.add_space(look.shadow.max(2.0) + 4.0);
    out
}

/// The title with each letter boiling on its own.
pub fn wobbly_title(ui: &mut Ui, look: &Look, text: &str, size: f32) -> Response {
    let font = FontId::proportional(size);
    let glyphs: Vec<_> = text.chars().map(|c| ui.painter().layout_no_wrap(c.to_string(), font.clone(), look.t.ink)).collect();
    let w: f32 = glyphs.iter().map(|g| g.size().x + 0.5).sum();
    let (rect, resp) = ui.allocate_exact_size(vec2(w + 8.0, size * 1.3), Sense::click());
    let colors = [look.t.hot, look.t.ink, look.t.ink, look.t.ink];
    let mut x = rect.min.x + 2.0;
    for (i, g) in glyphs.into_iter().enumerate() {
        let id = Id::new(("title", i));
        let j = look.jitter(id) * 2.0;
        let lift = if i % 2 == 0 { -1.0 } else { 1.0 };
        let pos = egui::pos2(x, rect.center().y - g.size().y / 2.0 + lift) + j;
        let c = colors.get(i % colors.len()).copied().unwrap_or(look.t.ink);
        // Shadowed letters, cartoon style.
        ui.painter().galley(pos + vec2(look.shadow * 0.5, look.shadow * 0.5), g.clone(), look.t.shadow);
        x += g.size().x + 0.5;
        // Faux bold: the default font is light, so draw each letter twice.
        let ink = if i == 0 { c } else { look.t.ink };
        ui.painter().galley(pos + vec2(0.8, 0.0), g.clone(), ink);
        ui.painter().galley(pos, g, ink);
    }
    resp
}

/// A labelled row: small caps label on the left, controls after it.
pub fn row<R>(ui: &mut Ui, look: &Look, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        if !label.is_empty() {
            ui.add_sized([58.0, 18.0], egui::Label::new(egui::RichText::new(label.to_uppercase()).size(10.5).color(look.t.dim)));
        }
        // Sliders fill what's left, leaving room for their value box.
        ui.spacing_mut().slider_width = (ui.available_width() - 72.0).max(48.0);
        add(ui)
    })
    .inner
}

/// A small dim hint paragraph.
pub fn hint(ui: &mut Ui, look: &Look, text: &str) {
    ui.label(egui::RichText::new(text).small().color(look.t.dim));
}
