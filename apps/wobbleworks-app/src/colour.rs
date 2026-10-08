//! The colour picker: a strip of paint blobs under the editor (current colour, recent colours,
//! the palette) that grows, with a springy transition, into a square card with three tabs along
//! its bottom:
//!
//! - **Colour**: a colour wheel, HSV sliders or a hex field;
//! - **Palette**: recent colours and the user's palette (add, remove);
//! - **Reference**: an image to paint from; clicking it picks a colour.
//!
//! Everything here only reads and sets the foreground colour; the shell applies it through
//! PhotoCraft's `tools.setColors`.

use std::sync::{Arc, Mutex};

use egui::{Color32, Mesh, Pos2, Rect, Sense, Shape, Stroke, TextureHandle, Ui, Vec2, pos2, vec2};

use crate::rough::{self, Paint};
use crate::theme::{self, mix};
use crate::widgets::{self, Look, TEXT};

/// Card size when open (clamped to the window).
const OPEN_SIZE: Vec2 = Vec2::new(372.0, 420.0);
/// Seconds the strip takes to grow into the card.
const GROW_SECONDS: f32 = 0.3;
/// Longest side a reference image is kept at (its texture and the colours picked from it).
const REFERENCE_MAX: u32 = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Colour,
    Palette,
    Reference,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Wheel,
    Hsv,
    Hex,
}

/// A reference image: its pixels (for picking) and texture.
pub struct Reference {
    pub name: String,
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
    texture: Option<TextureHandle>,
}

impl Reference {
    /// Decode `bytes` (any format PhotoCraft's codecs read), shrunk to [`REFERENCE_MAX`].
    pub fn decode(name: &str, bytes: &[u8]) -> Result<Self, String> {
        let img = photocraft_codecs::decode(bytes).map_err(|e| format!("{name}: {e}"))?;
        let (w, h) = (img.width(), img.height());
        if w == 0 || h == 0 {
            return Err(format!("{name}: the image is empty"));
        }
        let rgba = img.to_rgba8();
        let (size, rgba) = shrink(w, h, &rgba, REFERENCE_MAX);
        Ok(Self { name: name.to_string(), size, rgba, texture: None })
    }

    /// The colour at `uv` (0–1 across the image).
    pub fn colour_at(&self, uv: Vec2) -> Option<Color32> {
        let [w, h] = self.size;
        if !(uv.x.is_finite() && uv.y.is_finite()) || w == 0 || h == 0 {
            return None;
        }
        let x = ((uv.x.clamp(0.0, 1.0) * w as f32) as usize).min(w - 1);
        let y = ((uv.y.clamp(0.0, 1.0) * h as f32) as usize).min(h - 1);
        let i = (y * w + x) * 4;
        let p = self.rgba.get(i..i + 3)?;
        Some(Color32::from_rgb(p[0], p[1], p[2]))
    }
}

/// Nearest-neighbour shrink of RGBA8 pixels so the longest side is at most `max`.
fn shrink(w: u32, h: u32, rgba: &[u8], max: u32) -> ([usize; 2], Vec<u8>) {
    let scale = (max as f32 / w.max(h) as f32).min(1.0);
    let (nw, nh) = (((w as f32 * scale) as usize).max(1), ((h as f32 * scale) as usize).max(1));
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        let sy = ((y as f32 / scale) as usize).min(h as usize - 1);
        for x in 0..nw {
            let sx = ((x as f32 / scale) as usize).min(w as usize - 1);
            let si = (sy * w as usize + sx) * 4;
            let di = (y * nw + x) * 4;
            if let (Some(src), Some(dst)) = (rgba.get(si..si + 4), out.get_mut(di..di + 4)) {
                dst.copy_from_slice(src);
            }
        }
    }
    ([nw, nh], out)
}

/// Files picked for the reference tab arrive here (the web reads them asynchronously).
pub type ReferenceInbox = Arc<Mutex<Option<(String, Vec<u8>)>>>;
/// Show a file picker and put the chosen file in the inbox.
pub type PickReference = Box<dyn FnMut(ReferenceInbox)>;

pub struct ColourPicker {
    pub open: bool,
    pub tab: Tab,
    pub mode: Mode,
    /// Hue, saturation, value (0–1), kept so hue survives greys.
    pub hsv: [f32; 3],
    pub hex: String,
    pub reference: Option<Reference>,
    pub inbox: ReferenceInbox,
    pub pick_reference: Option<PickReference>,
    /// Where the strip was last frame (the card grows from it).
    strip: Rect,
    /// The colour the HSV state was last synced to.
    synced: Option<Color32>,
}

impl Default for ColourPicker {
    fn default() -> Self {
        Self {
            open: false,
            tab: Tab::Colour,
            mode: Mode::Wheel,
            hsv: [0.0, 0.0, 0.0],
            hex: String::new(),
            reference: None,
            inbox: Arc::default(),
            pick_reference: None,
            strip: Rect::NOTHING,
            synced: None,
        }
    }
}

/// What the user did this frame.
#[derive(Default)]
pub struct Picked {
    /// A new foreground colour.
    pub colour: Option<Color32>,
    /// Add the foreground colour to the palette.
    pub add: bool,
    /// Remove this palette entry.
    pub remove: Option<usize>,
    /// A message for the status line.
    pub error: Option<String>,
}

pub fn rgb_to_hsv(c: Color32) -> [f32; 3] {
    let (r, g, b) = (f32::from(c.r()) / 255.0, f32::from(c.g()) / 255.0, f32::from(c.b()) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= f32::EPSILON {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    let s = if max <= f32::EPSILON { 0.0 } else { d / max };
    [h, s, max]
}

pub fn hsv_to_rgb([h, s, v]: [f32; 3]) -> Color32 {
    let fin = |x: f32| if x.is_finite() { x } else { 0.0 };
    let (h, s, v) = (fin(h).rem_euclid(1.0) * 6.0, fin(s).clamp(0.0, 1.0), fin(v).clamp(0.0, 1.0));
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let q = |t: f32| ((t + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(q(r), q(g), q(b))
}

impl ColourPicker {
    /// Keep the HSV and hex state in step with the foreground colour when it changes elsewhere.
    fn sync(&mut self, fg: Color32) {
        if self.synced != Some(fg) {
            let hsv = rgb_to_hsv(fg);
            // Greys and black have no hue (or saturation): keep the old ones.
            self.hsv = [if hsv[1] > 0.0 && hsv[2] > 0.0 { hsv[0] } else { self.hsv[0] }, if hsv[2] > 0.0 { hsv[1] } else { self.hsv[1] }, hsv[2]];
            self.hex = theme::to_hex(fg);
            self.synced = Some(fg);
        }
    }

    /// Take a file that arrived for the reference tab.
    fn drain(&mut self, ctx: &egui::Context, out: &mut Picked) {
        let arrived = self.inbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        if let Some((name, bytes)) = arrived {
            match Reference::decode(&name, &bytes) {
                Ok(r) => {
                    self.reference = Some(r);
                    self.tab = Tab::Reference;
                    ctx.request_repaint();
                }
                Err(e) => out.error = Some(e),
            }
        }
    }

    /// The strip (inside a bottom panel) and, when open, the card. `recent` and `palette` are
    /// the shell's colour lists.
    pub fn show(&mut self, ui: &mut Ui, look: &Look, fg: Color32, recent: &[Color32], palette: &[Color32]) -> Picked {
        let mut out = Picked::default();
        let ctx = ui.ctx().clone();
        self.sync(fg);
        self.drain(&ctx, &mut out);
        self.strip_ui(ui, look, fg, recent, palette, &mut out);
        self.card(&ctx, look, fg, recent, palette, &mut out);
        if let Some(c) = out.colour {
            self.sync(c);
        }
        out
    }

    fn strip_ui(&mut self, ui: &mut Ui, look: &Look, fg: Color32, recent: &[Color32], palette: &[Color32], out: &mut Picked) {
        let avail = ui.available_width();
        // Clicking the strip anywhere but a blob opens the card (registered first, so the blobs
        // on top of it keep their own clicks).
        let bg = ui.interact(self.strip, ui.id().with("strip-bg"), Sense::click());
        if bg.clicked() {
            self.open = !self.open;
        }
        let card_rect = widgets::card(ui, look, "wobble-colour-strip", 8.0, |ui| {
            ui.set_max_width((avail - 24.0).max(100.0));
            ui.spacing_mut().item_spacing = vec2(2.0, 4.0);
            ui.horizontal_wrapped(|ui| {
                if widgets::swatch(ui, look, fg, self.open, 17.0).on_hover_text("Colours").clicked() {
                    self.open = !self.open;
                }
                ui.add_space(6.0);
                for &c in recent.iter().take(8) {
                    if widgets::swatch(ui, look, c, c == fg, 11.0).clicked() {
                        out.colour = Some(c);
                    }
                }
                if !recent.is_empty() {
                    // A little dot between recent colours and the palette.
                    let (r, _) = ui.allocate_exact_size(vec2(12.0, 28.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 2.0, look.t.dim);
                }
                for (i, &c) in palette.iter().enumerate() {
                    let r = widgets::swatch(ui, look, c, c == fg, 11.0);
                    if r.clicked() {
                        out.colour = Some(c);
                    }
                    if r.secondary_clicked() {
                        out.remove = Some(i);
                    }
                }
            });
            ui.min_rect()
        });
        self.strip = card_rect.expand(8.0);
        if bg.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }

    /// The card: grows out of the strip's corner with a little overshoot, content fading in.
    fn card(&mut self, ctx: &egui::Context, look: &Look, fg: Color32, recent: &[Color32], palette: &[Color32], out: &mut Picked) {
        let t = ctx.animate_bool_with_time(egui::Id::new("wobble-colour-grow"), self.open, GROW_SECONDS);
        if t <= 0.0 || !self.strip.is_finite() {
            return;
        }
        let e = if self.open { ease_out_back(t) } else { t * t };
        let screen = ctx.content_rect();
        let size = vec2(OPEN_SIZE.x.min(screen.width() - 16.0), OPEN_SIZE.y.min(screen.height() - 16.0)).max(vec2(160.0, 200.0));
        let bottom_left = pos2(self.strip.min.x + 8.0, self.strip.min.y - 6.0);
        let open = Rect::from_min_size(bottom_left - vec2(0.0, size.y), size);
        let from = Rect::from_min_size(pos2(self.strip.min.x + 8.0, self.strip.min.y + 8.0), vec2(44.0, 44.0));
        let rect = Rect::from_min_max(from.min + (open.min - from.min) * e, from.max + (open.max - from.max) * e);
        egui::Area::new(egui::Id::new("wobble-colour-card")).order(egui::Order::Foreground).fixed_pos(rect.min).constrain(false).show(ctx, |ui| {
            let style = Paint { fill: look.t.card, ink: look.ink(2.5), shadow: Some((vec2(5.0, 5.0), look.t.shadow)), radius: 18.0, wobble: 1.6 };
            rough::boxed(ui.painter(), rect, &style, 0xc010, look.frame);
            let (_, bg) = ui.allocate_exact_size(rect.size(), Sense::click_and_drag());
            let _ = bg;
            if t < 0.75 {
                return;
            }
            ui.set_opacity(((t - 0.75) / 0.25).clamp(0.0, 1.0));
            let inner = rect.shrink(14.0);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
            self.contents(&mut child, look, fg, recent, palette, inner, out);
        });
        // Clicking away (or Esc) closes the card.
        let away = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p) && !self.strip.contains(p)));
        if self.open && (away || ctx.input(|i| i.key_pressed(egui::Key::Escape))) && t >= 1.0 {
            self.open = false;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn contents(&mut self, ui: &mut Ui, look: &Look, fg: Color32, recent: &[Color32], palette: &[Color32], inner: Rect, out: &mut Picked) {
        let tabs_h = 44.0;
        let body = Rect::from_min_max(inner.min, pos2(inner.max.x, inner.max.y - tabs_h - 6.0));
        let mut b = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
        b.set_clip_rect(body.expand(4.0));
        match self.tab {
            Tab::Colour => self.colour_tab(&mut b, look, fg, out),
            Tab::Palette => palette_tab(&mut b, look, fg, recent, palette, out),
            Tab::Reference => self.reference_tab(&mut b, look, out),
        }
        // The three tabs along the bottom.
        let bar = Rect::from_min_max(pos2(inner.min.x, inner.max.y - tabs_h), inner.max);
        let mut t = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::left_to_right(egui::Align::Center)));
        t.spacing_mut().item_spacing.x = 6.0;
        for (tab, name) in [(Tab::Colour, "Colour"), (Tab::Palette, "Palette"), (Tab::Reference, "Reference")] {
            if widgets::button(&mut t, look, name, self.tab == tab, true).clicked() {
                self.tab = tab;
            }
        }
    }

    fn colour_tab(&mut self, ui: &mut Ui, look: &Look, fg: Color32, out: &mut Picked) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (mode, name) in [(Mode::Wheel, "Wheel"), (Mode::Hsv, "HSV"), (Mode::Hex, "Hex")] {
                if widgets::button(ui, look, name, self.mode == mode, true).clicked() {
                    self.mode = mode;
                }
            }
        });
        ui.add_space(6.0);
        let before = self.hsv;
        match self.mode {
            Mode::Wheel => {
                let side = ui.available_width().min(ui.available_height() - 110.0).clamp(80.0, 220.0);
                ui.horizontal(|ui| {
                    ui.add_space(((ui.available_width() - side) / 2.0).max(0.0));
                    wheel(ui, look, &mut self.hsv, side);
                });
                let w = ui.available_width();
                let mut v = self.hsv[2] * 100.0;
                if widgets::slider(ui, look, "Bright", &mut v, 0.0..=100.0, w, |v| format!("{v:.0}")).dragged_or_clicked() {
                    self.hsv[2] = v / 100.0;
                }
            }
            Mode::Hsv => {
                let w = ui.available_width();
                let mut h = self.hsv[0] * 360.0;
                let mut s = self.hsv[1] * 100.0;
                let mut v = self.hsv[2] * 100.0;
                if widgets::slider(ui, look, "Hue", &mut h, 0.0..=360.0, w, |v| format!("{v:.0}")).dragged_or_clicked() {
                    self.hsv[0] = h / 360.0;
                }
                if widgets::slider(ui, look, "Sat", &mut s, 0.0..=100.0, w, |v| format!("{v:.0}")).dragged_or_clicked() {
                    self.hsv[1] = s / 100.0;
                }
                if widgets::slider(ui, look, "Value", &mut v, 0.0..=100.0, w, |v| format!("{v:.0}")).dragged_or_clicked() {
                    self.hsv[2] = v / 100.0;
                }
            }
            Mode::Hex => {
                widgets::label(ui, look, "Hex colour", TEXT, look.t.dim);
                let r = ui.add(egui::TextEdit::singleline(&mut self.hex).desired_width(ui.available_width().min(200.0)).char_limit(9));
                if r.changed()
                    && let Ok(c) = Color32::from_hex(self.hex.trim())
                {
                    out.colour = Some(c);
                }
            }
        }
        if self.hsv != before {
            out.colour = Some(hsv_to_rgb(self.hsv));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let c = out.colour.unwrap_or(fg);
            widgets::swatch(ui, look, c, false, 16.0);
            ui.add_space(6.0);
            if widgets::button(ui, look, "+ Palette", false, true).on_hover_text("Add this colour to your palette").clicked() {
                out.add = true;
            }
        });
    }

    fn reference_tab(&mut self, ui: &mut Ui, look: &Look, out: &mut Picked) {
        let can_pick = self.pick_reference.is_some();
        ui.horizontal(|ui| {
            let label = if self.reference.is_some() { "Change…" } else { "Load image…" };
            if widgets::button(ui, look, label, false, can_pick).on_hover_text("An image to paint from").clicked()
                && let Some(pick) = self.pick_reference.as_mut()
            {
                pick(self.inbox.clone());
            }
            if self.reference.is_some() && widgets::button(ui, look, "Clear", false, true).clicked() {
                self.reference = None;
            }
        });
        ui.add_space(6.0);
        let Some(reference) = self.reference.as_mut() else {
            widgets::label(ui, look, "Load a picture to paint from.", TEXT, look.t.dim);
            widgets::label(ui, look, "Click it to pick a colour.", TEXT, look.t.dim);
            return;
        };
        let avail = ui.available_size();
        let [w, h] = reference.size;
        let scale = (avail.x / w as f32).min(avail.y / h as f32).min(4.0);
        if !scale.is_finite() || scale <= 0.0 {
            return;
        }
        let tex = reference
            .texture
            .get_or_insert_with(|| {
                ui.ctx().load_texture(
                    format!("wobble-reference-{}", reference.name),
                    egui::ColorImage::from_rgba_unmultiplied([w, h], &reference.rgba),
                    egui::TextureOptions::LINEAR,
                )
            })
            .id();
        let (rect, resp) = ui.allocate_exact_size(vec2(w as f32, h as f32) * scale, Sense::click_and_drag());
        ui.painter().image(tex, rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
        ui.painter().rect_stroke(rect, 2.0, Stroke::new(2.0, look.t.ink), egui::StrokeKind::Outside);
        if let Some(p) = resp.hover_pos() {
            let uv = (p - rect.min) / rect.size();
            if let Some(c) = reference.colour_at(uv) {
                rough::blob(
                    ui.painter(),
                    p + vec2(16.0, -16.0),
                    10.0,
                    &Paint { fill: c, ink: look.ink(2.0), shadow: None, radius: 0.0, wobble: 0.0 },
                    0x9e,
                    look.frame,
                );
                if resp.clicked() || resp.dragged() {
                    out.colour = Some(c);
                }
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
    }
}

fn palette_tab(ui: &mut Ui, look: &Look, fg: Color32, recent: &[Color32], palette: &[Color32], out: &mut Picked) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        widgets::label(ui, look, "Recent", TEXT, look.t.dim);
        ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
        ui.horizontal_wrapped(|ui| {
            if recent.is_empty() {
                widgets::label(ui, look, "Paint something!", TEXT, mix(look.t.dim, look.t.card, 0.4));
            }
            for &c in recent {
                if widgets::swatch(ui, look, c, c == fg, 12.0).on_hover_text(theme::to_hex(c)).clicked() {
                    out.colour = Some(c);
                }
            }
        });
        ui.add_space(8.0);
        widgets::label(ui, look, "Palette", TEXT, look.t.dim);
        ui.horizontal_wrapped(|ui| {
            for (i, &c) in palette.iter().enumerate() {
                let r = widgets::swatch(ui, look, c, c == fg, 12.0).on_hover_text(format!("{}  (right-click to remove)", theme::to_hex(c)));
                if r.clicked() {
                    out.colour = Some(c);
                }
                if r.secondary_clicked() {
                    out.remove = Some(i);
                }
            }
            if widgets::button(ui, look, "+", false, !palette.contains(&fg)).on_hover_text("Add the current colour").clicked() {
                out.add = true;
            }
        });
    });
}

/// A colour wheel: hue around, saturation outward, at the current value. Click or drag to pick.
fn wheel(ui: &mut Ui, look: &Look, hsv: &mut [f32; 3], side: f32) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(side), Sense::click_and_drag());
    let c = rect.center();
    let r = side / 2.0 - 4.0;
    if (resp.dragged() || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let d = p - c;
        let a = d.y.atan2(d.x);
        hsv[0] = (a / std::f32::consts::TAU).rem_euclid(1.0);
        hsv[1] = (d.length() / r).clamp(0.0, 1.0);
    }
    if !ui.is_rect_visible(rect) {
        return;
    }
    let n = 72;
    let rings = 6;
    let mut mesh = Mesh::default();
    for ring in 0..=rings {
        let s = ring as f32 / rings as f32;
        for i in 0..n {
            let h = i as f32 / n as f32;
            let p = c + Vec2::angled(h * std::f32::consts::TAU) * r * s;
            mesh.colored_vertex(p, hsv_to_rgb([h, s, hsv[2]]));
        }
    }
    for ring in 0..rings {
        for i in 0..n {
            let a = ring * n + i;
            let b = ring * n + (i + 1) % n;
            let (a2, b2) = (a + n, b + n);
            mesh.add_triangle(a as u32, b as u32, b2 as u32);
            mesh.add_triangle(a as u32, b2 as u32, a2 as u32);
        }
    }
    ui.painter().add(Shape::mesh(mesh));
    // A hand-drawn rim and a blob where the colour is.
    let rim: Vec<Pos2> = (0..96).map(|i| c + Vec2::angled(i as f32 / 96.0 * std::f32::consts::TAU) * (r + 1.0)).collect();
    ui.painter().add(Shape::closed_line(rough::wobble(&rim, c, 0x77ee1, look.frame, 1.2), look.ink(2.5)));
    let at = c + Vec2::angled(hsv[0] * std::f32::consts::TAU) * r * hsv[1];
    rough::blob(ui.painter(), at, 7.0, &Paint { fill: hsv_to_rgb(*hsv), ink: look.ink(2.5), shadow: None, radius: 0.0, wobble: 0.0 }, 0x51, look.frame);
}

/// Ease out with a little overshoot (the card pops open).
pub fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c1 = 1.4;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

trait DraggedOrClicked {
    fn dragged_or_clicked(&self) -> bool;
}

impl DraggedOrClicked for egui::Response {
    fn dragged_or_clicked(&self) -> bool {
        self.dragged() || self.clicked()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trips() {
        for c in [
            Color32::from_rgb(255, 46, 136),
            Color32::from_rgb(10, 200, 30),
            Color32::from_gray(128),
            Color32::BLACK,
            Color32::WHITE,
            Color32::from_rgb(40, 80, 250),
        ] {
            let back = hsv_to_rgb(rgb_to_hsv(c));
            let d = |a: u8, b: u8| (i16::from(a) - i16::from(b)).abs();
            assert!(d(back.r(), c.r()) <= 1 && d(back.g(), c.g()) <= 1 && d(back.b(), c.b()) <= 1, "{c:?} → {back:?}");
        }
        assert_eq!(hsv_to_rgb([f32::NAN, 2.0, -1.0]), Color32::BLACK);
        assert_eq!(hsv_to_rgb([1.0, 1.0, 1.0]), Color32::from_rgb(255, 0, 0), "hue wraps");
    }

    #[test]
    fn greys_keep_their_hue() {
        let mut p = ColourPicker::default();
        p.sync(Color32::from_rgb(255, 0, 0));
        p.sync(Color32::BLACK);
        assert_eq!(p.hsv[0], 0.0);
        p.sync(Color32::from_rgb(0, 0, 255));
        let h = p.hsv[0];
        p.sync(Color32::from_gray(90));
        assert_eq!(p.hsv[0], h);
        assert_eq!(p.hex, "#5a5a5a");
    }

    #[test]
    fn references_decode_shrink_and_pick() {
        // A 2 × 1 PNG: red then blue.
        let img = photocraft_codecs::Image::from_u8(2, 1, photocraft_codecs::ChannelLayout::Rgba, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
        let png = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        let r = Reference::decode("ref.png", &png).unwrap();
        assert_eq!(r.size, [2, 1]);
        assert_eq!(r.colour_at(vec2(0.1, 0.5)), Some(Color32::RED));
        assert_eq!(r.colour_at(vec2(0.9, 0.5)), Some(Color32::BLUE));
        assert_eq!(r.colour_at(vec2(5.0, -3.0)), Some(Color32::BLUE), "clamped");
        assert_eq!(r.colour_at(vec2(f32::NAN, 0.0)), None);
        assert!(Reference::decode("bad.png", b"nope").is_err());
        let (size, px) = shrink(4000, 2000, &vec![7u8; 4000 * 2000 * 4], 1000);
        assert_eq!(size, [1000, 500]);
        assert_eq!(px.len(), 1000 * 500 * 4);
    }

    #[test]
    fn the_card_pops_with_a_little_overshoot() {
        assert!(ease_out_back(0.0).abs() < 1e-6);
        assert!((ease_out_back(1.0) - 1.0).abs() < 1e-6);
        assert!((1..10).map(|i| ease_out_back(i as f32 / 10.0)).any(|v| v > 1.0), "overshoots");
    }
}
