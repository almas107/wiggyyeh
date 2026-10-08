//! Hand-painted widgets: marker-outlined buttons with hard shadows that rise on hover and squash
//! when pressed, icon tiles, paint-blob swatches, wobbly sliders and painted cards. Labels use the
//! pixel font.

use egui::{Color32, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};

use crate::icons::{self, Icon};
use crate::pixfont;
use crate::rough::{self, Paint};
use crate::theme::{Theme, mix};

/// Everything a widget needs to paint itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub t: Theme,
    /// The boil frame (see [`rough::boil_frame`]).
    pub frame: u64,
}

impl Look {
    pub fn ink(&self, width: f32) -> Stroke {
        Stroke::new(width, self.t.ink)
    }
}

/// Text scale for labels and buttons (one font pixel = 2 points).
pub const TEXT: f32 = 2.0;
/// Hard shadow depth.
const DEPTH: f32 = 4.0;

/// How far a widget is lifted (hover) or pressed, eased (0 = resting).
fn motion(ui: &Ui, resp: &Response, enabled: bool) -> (f32, f32) {
    let hover = ui.ctx().animate_bool_with_time(resp.id.with("hover"), enabled && resp.hovered(), 0.12);
    let press = ui.ctx().animate_bool_with_time(resp.id.with("press"), enabled && resp.is_pointer_button_down_on(), 0.05);
    (hover, press)
}

/// The painted body of a button-like widget in `rect`; returns the rect its content goes in.
fn body(ui: &Ui, look: &Look, rect: Rect, resp: &Response, fill: Color32, enabled: bool, radius: f32) -> Rect {
    let (hover, press) = motion(ui, resp, enabled);
    // Rest with a shadow; lift a little on hover; squash into the shadow when pressed.
    let lift = vec2(-1.0, -1.5) * hover * (1.0 - press) + vec2(DEPTH, DEPTH) * 0.8 * press;
    let squash = 1.0 - 0.05 * press;
    let r = Rect::from_center_size(rect.center() + lift, vec2(rect.width() * (2.0 - squash), rect.height() * squash));
    let fill = if !enabled { mix(fill, look.t.paper, 0.55) } else { mix(fill, look.t.sun, 0.3 * hover * if fill == look.t.card { 1.0 } else { 0.4 }) };
    let ink = if enabled { look.t.ink } else { mix(look.t.ink, look.t.paper, 0.55) };
    let shadow = (enabled && press < 0.99).then(|| (vec2(DEPTH, DEPTH) * (1.0 - press) - lift.max(Vec2::ZERO), look.t.shadow));
    rough::boxed(ui.painter(), r, &Paint { fill, ink: Stroke::new(2.5, ink), shadow, radius, wobble: 1.4 }, resp.id.value(), look.frame);
    if resp.has_focus() {
        rough::boxed(
            ui.painter(),
            r.expand(4.0),
            &Paint { fill: Color32::TRANSPARENT, ink: Stroke::new(2.0, look.t.hot), shadow: None, radius: radius + 4.0, wobble: 1.0 },
            resp.id.value() ^ 7,
            look.frame,
        );
    }
    r
}

fn cursor(resp: Response, enabled: bool) -> Response {
    resp.on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::NotAllowed })
}

/// A chunky button: optional icon, pixel-font label. `on` shows it toggled.
pub fn button(ui: &mut Ui, look: &Look, icon: Option<Icon>, text: &str, on: bool, enabled: bool) -> Response {
    let ts = pixfont::size(text, TEXT);
    let icon_w = if icon.is_some() { 22.0 + if text.is_empty() { 0.0 } else { 6.0 } } else { 0.0 };
    let size = vec2((ts.x + icon_w + 24.0).max(40.0), 40.0) + Vec2::splat(DEPTH);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, on, text));
    if ui.is_rect_visible(rect) {
        let inner = Rect::from_min_size(rect.min, rect.size() - Vec2::splat(DEPTH));
        let fill = if on { look.t.sun } else { look.t.card };
        let r = body(ui, look, inner, &resp, fill, enabled, 11.0);
        let ink = if enabled { look.t.ink } else { mix(look.t.ink, look.t.paper, 0.5) };
        let content_w = icon_w + ts.x;
        let mut x = r.center().x - content_w / 2.0;
        if let Some(i) = icon {
            icons::paint(ui.painter(), Rect::from_center_size(pos2(x + 11.0, r.center().y), Vec2::splat(22.0)), i, ink, look.t.hot);
            x += icon_w;
        }
        if !text.is_empty() {
            pixfont::paint(ui.painter(), pos2(x.round(), (r.center().y - ts.y / 2.0).round()), text, TEXT, ink, |_| 0.0);
        }
    }
    cursor(resp, enabled)
}

/// A square icon tile (tools). `on` marks the current tool.
pub fn tile(ui: &mut Ui, look: &Look, icon: Icon, on: bool, side: f32, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(side) + Vec2::splat(DEPTH), if enabled { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        let inner = Rect::from_min_size(rect.min, Vec2::splat(side));
        let r = body(ui, look, inner, &resp, if on { look.t.sun } else { look.t.card }, enabled, side * 0.3);
        let ink = if enabled { look.t.ink } else { mix(look.t.ink, look.t.paper, 0.5) };
        icons::paint(ui.painter(), r.shrink(side * 0.2), icon, ink, look.t.hot);
    }
    cursor(resp, enabled)
}

/// A paint-blob colour swatch. `on` marks the current colour.
pub fn swatch(ui: &mut Ui, look: &Look, colour: Color32, on: bool, radius: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(radius * 2.0 + 6.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let (hover, press) = motion(ui, &resp, true);
        let grow = 1.0 + 0.12 * hover - 0.1 * press + if on { 0.08 } else { 0.0 };
        let c = rect.center() - vec2(1.5, 1.5);
        let seed = resp.id.value();
        rough::blob(ui.painter(), c, radius * grow, &blob_paint(colour, look.ink(2.0), Some((vec2(3.0, 3.0), look.t.shadow))), seed, look.frame);
        if on {
            rough::blob(ui.painter(), c, radius * grow + 5.0, &blob_paint(Color32::TRANSPARENT, Stroke::new(2.5, look.t.hot), None), seed ^ 3, look.frame);
            // A white glint, like a drop of wet paint.
            ui.painter().circle_filled(c + vec2(-radius * 0.35, -radius * 0.35), radius * 0.18, Color32::from_white_alpha(200));
        }
    }
    cursor(resp, true)
}

/// A wobbly slider with a pixel-font label and value. Returns the response; `value` changes while
/// dragging or clicking along the track.
pub fn slider(
    ui: &mut Ui,
    look: &Look,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    width: f32,
    show: impl Fn(f32) -> String,
) -> Response {
    let (lo, hi) = (*range.start(), *range.end());
    let width = width.max(80.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 40.0), Sense::click_and_drag());
    let track = Rect::from_min_max(pos2(rect.min.x + 10.0, rect.min.y + 26.0), pos2(rect.max.x - 10.0, rect.min.y + 26.0));
    if (resp.dragged() || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
        && track.width() > 0.0
        && hi > lo
    {
        let f = ((p.x - track.min.x) / track.width()).clamp(0.0, 1.0);
        *value = lo + f * (hi - lo);
    }
    if !value.is_finite() {
        *value = lo;
    }
    *value = value.clamp(lo, hi);
    if ui.is_rect_visible(rect) {
        let f = if hi > lo { (*value - lo) / (hi - lo) } else { 0.0 };
        let p = ui.painter();
        pixfont::paint(p, pos2(rect.min.x + 2.0, rect.min.y + 2.0), label, TEXT, look.t.ink, |_| 0.0);
        let v = show(*value);
        let vs = pixfont::size(&v, TEXT);
        pixfont::paint(p, pos2(rect.max.x - vs.x - 2.0, rect.min.y + 2.0), &v, TEXT, look.t.dim, |_| 0.0);
        let seed = resp.id.value();
        let a = track.left_center();
        let b = track.right_center();
        let knob = a + (b - a) * f;
        rough::line(p, &rough::segment(a, b, 24), Stroke::new(5.0, mix(look.t.ink, look.t.card, 0.75)), seed, look.frame, 0.8);
        rough::line(p, &rough::segment(a, knob, 24), Stroke::new(5.0, look.t.hot), seed, look.frame, 0.8);
        let (hover, press) = motion(ui, &resp, true);
        rough::blob(p, knob, 8.0 + 2.0 * hover - 1.5 * press, &blob_paint(look.t.card, look.ink(2.5), Some((vec2(2.0, 2.0), look.t.shadow))), seed ^ 11, look.frame);
    }
    resp.on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
}

fn blob_paint(fill: Color32, ink: Stroke, shadow: Option<(Vec2, Color32)>) -> Paint {
    Paint { fill, ink, shadow, radius: 0.0, wobble: 0.0 }
}

/// Pixel-font text.
pub fn label(ui: &mut Ui, look: &Look, text: &str, scale: f32, colour: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(pixfont::size(text, scale) + vec2(0.0, 2.0 * scale), Sense::hover());
    if ui.is_rect_visible(rect) {
        pixfont::paint(ui.painter(), rect.min, text, scale, colour, |_| 0.0);
    }
    let _ = look;
    resp
}

/// The WobbleWorks logo: boiling pixel letters in alternating colours with a hard shadow.
pub fn logo(ui: &mut Ui, look: &Look, scale: f32) -> Response {
    let text = "WobbleWorks";
    // As tall as a button, so the letters sit on the bar's centre line with room to jiggle.
    let size = pixfont::size(text, scale) + vec2(scale, 0.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(size.x, size.y.max(44.0)), Sense::hover());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let jiggle = |i: usize| (rough::hash(look.frame + 1, i as u64) * 0.6).round();
        let pos = pos2(rect.min.x, (rect.center().y - size.y / 2.0 - 2.0).round());
        pixfont::paint(p, pos + vec2(scale, scale), text, scale, look.t.shadow, jiggle);
        let colours = [look.t.hot, look.t.ink];
        // One letter at a time, so each gets its own colour.
        let mut x = pos.x;
        for (i, ch) in text.chars().enumerate() {
            let s = ch.to_string();
            let y = pos.y + jiggle(i) * scale;
            let c = colours.get(usize::from(i >= 6)).copied().unwrap_or(look.t.ink);
            pixfont::paint(p, pos2(x, y), &s, scale, c, |_| 0.0);
            x += pixfont::size(&s, scale).x + scale;
        }
    }
    resp
}

/// A painted card around `add`. The card's paint goes behind the content.
pub fn card<R>(ui: &mut Ui, look: &Look, id: &str, margin: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let behind = ui.painter().add(Shape::Noop);
    let out = egui::Frame::NONE.inner_margin(egui::Margin::same(margin.clamp(0.0, 100.0) as i8)).show(ui, add);
    let rect = out.response.rect;
    let style = Paint { fill: look.t.card, ink: look.ink(2.5), shadow: Some((vec2(DEPTH + 1.0, DEPTH + 1.0), look.t.shadow)), radius: 16.0, wobble: 1.8 };
    ui.painter().set(behind, Shape::Vec(rough::boxed_shapes(rect, &style, egui::Id::new(id).value(), look.frame)));
    out.inner
}
