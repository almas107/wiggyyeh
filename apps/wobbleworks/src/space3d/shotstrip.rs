//! Feather's Sequence bar: a strip along the bottom of the canvas with a thumbnail of every
//! camera shot (click to go there, drag to reorder, right-click to rename or delete, + to add
//! this view), and a Play tab with the transport and a timeline to scrub the camera between
//! shots. Thumbnails render from each shot's own camera and are redrawn only when the note
//! changes, one per frame, never while a stroke is being drawn.

use std::collections::HashMap;

use egui::{Color32, Rect, RichText, Sense, Ui, pos2, vec2};
use serde_json::json;
use wobbleworks_3d::editor::Editor;

use super::Space3d;
use crate::widgets::{self, Look};

/// Thumbnail size in pixels (16:10, like the canvas usually is).
pub const THUMB: (u32, u32) = (112, 70);

#[derive(Default)]
pub struct ShotStrip {
    /// The bar is showing (View › Sequence, or the Shots tab).
    pub open: bool,
    /// Play tab (transport and timeline) rather than Edit (the thumbnails).
    pub play_tab: bool,
    /// Thumbnails by shot id, with the key they were drawn for.
    thumbs: HashMap<u64, (String, egui::TextureHandle)>,
    /// The shot being renamed, and the text so far.
    renaming: Option<(u64, String)>,
    /// The shot being dragged to a new place.
    dragging: Option<u64>,
}

/// What a shot's thumbnail depends on: the note, how it is rendered, and the shot's camera.
fn key(ed: &Editor, i: usize) -> Option<String> {
    let shot = ed.scene.sequence.shots.get(i)?;
    Some(format!("{}|{:?}|{:?}", ed.scene_revision, ed.scene.environment, shot.camera))
}

impl ShotStrip {
    /// Redraw at most one stale thumbnail (not while drawing); forget deleted shots'.
    pub fn refresh(&mut self, ctx: &egui::Context, ed: &mut Editor) {
        let live: Vec<u64> = ed.scene.sequence.shots.iter().map(|s| s.id).collect();
        self.thumbs.retain(|id, _| live.contains(id));
        if !self.open || ed.is_busy() {
            return;
        }
        let stale = (0..live.len()).find_map(|i| {
            let k = key(ed, i)?;
            let id = *live.get(i)?;
            (self.thumbs.get(&id).map(|t| &t.0) != Some(&k)).then_some((id, k))
        });
        let Some((id, k)) = stale else { return };
        match ed.shot_thumbnail(id, THUMB.0, THUMB.1) {
            Ok(pic) => {
                let img = egui::ColorImage::from_rgba_unmultiplied([pic.width as usize, pic.height as usize], &pic.rgba);
                let tex = ctx.load_texture(format!("w3d-shot-{id}"), img, egui::TextureOptions::LINEAR);
                self.thumbs.insert(id, (k, tex));
            }
            Err(e) => ed.status = e,
        }
        // More may be waiting: come back next frame.
        ctx.request_repaint();
    }

    /// Whether a thumbnail is ready for this shot (tests).
    pub fn has_thumb(&self, id: u64) -> bool {
        self.thumbs.contains_key(&id)
    }
}

/// Draw the bar at the bottom of the canvas `rect`.
pub fn show(s: &mut Space3d, ui: &mut Ui, look: &Look, rect: Rect) {
    if !s.strip.open {
        return;
    }
    let ctx = ui.ctx().clone();
    s.strip.refresh(&ctx, &mut s.ed);
    let w = (rect.width() - 120.0).clamp(260.0, 900.0);
    let h = THUMB.1 as f32 + 34.0;
    let at = pos2(rect.center().x - w / 2.0, rect.max.y - h - 44.0);
    egui::Area::new(egui::Id::new("w3d-shot-strip")).order(egui::Order::Foreground).fixed_pos(at).show(&ctx, |ui| {
        ui.set_width(w);
        widgets::card(ui, look, "w3d-shot-strip-card", 8.0, |ui| {
            ui.set_width(w - 16.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    if ui.selectable_label(!s.strip.play_tab, "Edit").clicked() {
                        s.strip.play_tab = false;
                    }
                    if ui.selectable_label(s.strip.play_tab, "Play").clicked() {
                        s.strip.play_tab = true;
                    }
                    if ui.small_button("Hide").on_hover_text("Close the Sequence bar (Shots tab brings it back)").clicked() {
                        s.strip.open = false;
                    }
                });
                ui.separator();
                if s.strip.play_tab {
                    play_tab(s, ui, look);
                } else {
                    edit_tab(s, ui, look);
                }
            });
        });
    });
}

fn edit_tab(s: &mut Space3d, ui: &mut Ui, look: &Look) {
    let shots: Vec<(u64, String)> = s.ed.scene.sequence.shots.iter().map(|x| (x.id, x.name.clone())).collect();
    let add_w = 36.0;
    let avail = ui.available_width() - add_w - 8.0;
    egui::ScrollArea::horizontal().id_salt("w3d-shot-scroll").max_width(avail).show(ui, |ui| {
        ui.horizontal(|ui| {
            if shots.is_empty() {
                ui.add_sized(vec2(avail - 8.0, THUMB.1 as f32), egui::Label::new(RichText::new("Add new shot: + saves this view").color(look.t.dim)));
            }
            let mut drop_at: Option<usize> = None;
            for (i, (id, name)) in shots.iter().enumerate() {
                ui.vertical(|ui| {
                    let size = vec2(THUMB.0 as f32, THUMB.1 as f32);
                    let (r, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
                    match s.strip.thumbs.get(id) {
                        Some((_, tex)) => {
                            ui.painter().image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        }
                        None => {
                            ui.painter().rect_filled(r, 6.0, look.t.dim.gamma_multiply(0.25));
                        }
                    }
                    let edge = if s.strip.dragging == Some(*id) { look.t.hot } else { look.t.ink.gamma_multiply(0.5) };
                    ui.painter().rect_stroke(r, 6.0, egui::Stroke::new(1.5, edge), egui::StrokeKind::Inside);
                    if resp.clicked() {
                        s.run("shot.go", json!({"id": id}));
                    }
                    if resp.drag_started() {
                        s.strip.dragging = Some(*id);
                    }
                    if s.strip.dragging.is_some() && s.strip.dragging != Some(*id) && ui.rect_contains_pointer(r) && ui.input(|i| i.pointer.any_released()) {
                        drop_at = Some(i);
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Rename").clicked() {
                            s.strip.renaming = Some((*id, name.clone()));
                            ui.close();
                        }
                        if ui.button("Update to this view").clicked() {
                            let at = i;
                            s.run("shot.delete", json!({"ids": [id]}));
                            if let Some(new) = s.run("shot.add", json!({})).and_then(|v| v.as_u64()) {
                                s.run("shot.move", json!({"id": new, "index": at}));
                                s.run("shot.rename", json!({"id": new, "name": name}));
                            }
                            ui.close();
                        }
                        if ui.button("Delete").clicked() {
                            s.run("shot.delete", json!({"ids": [id]}));
                            ui.close();
                        }
                    });
                    // The name on a band across the thumbnail's foot; renaming edits it there.
                    let band = Rect::from_min_max(pos2(r.min.x + 1.5, r.max.y - 17.0), pos2(r.max.x - 1.5, r.max.y - 1.5));
                    match &mut s.strip.renaming {
                        Some((rid, text)) if rid == id => {
                            let edit = ui.put(band, egui::TextEdit::singleline(text).font(egui::TextStyle::Small));
                            if edit.lost_focus() {
                                let t = text.clone();
                                s.strip.renaming = None;
                                s.run("shot.rename", json!({"id": id, "name": t}));
                            } else {
                                edit.request_focus();
                            }
                        }
                        _ => {
                            ui.painter().rect_filled(band, 4.0, look.t.card.gamma_multiply(0.85));
                            let text = ui.painter().layout(name.clone(), egui::FontId::proportional(11.0), look.t.ink, band.width() - 8.0);
                            ui.painter().with_clip_rect(band).galley(pos2(band.min.x + 4.0, band.center().y - text.size().y / 2.0), text, look.t.ink);
                        }
                    }
                });
            }
            if let (Some(i), Some(id)) = (drop_at, s.strip.dragging) {
                s.run("shot.move", json!({"id": id, "index": i}));
            }
            if ui.input(|i| i.pointer.any_released()) {
                s.strip.dragging = None;
            }
        });
    });
    if ui.add_sized(vec2(add_w, THUMB.1 as f32), egui::Button::new(RichText::new("+").size(22.0))).on_hover_text("Save this view as a shot").clicked() {
        s.run("shot.add", json!({}));
    }
}

fn play_tab(s: &mut Space3d, ui: &mut Ui, look: &Look) {
    let n = s.ed.scene.sequence.shots.len();
    if n < 2 {
        ui.label(RichText::new("Add two shots or more to play the camera between them.").color(look.t.dim));
        return;
    }
    let legs = (n - 1) as f64;
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            let now = ui.input(|i| i.time);
            let at = s.timeline(now).unwrap_or(0.0);
            if ui.button("⏮").on_hover_text("Previous shot").clicked() {
                s.playing = None;
                s.run("sequence.at", json!({"t": (at.ceil() - 1.0).max(0.0)}));
            }
            let playing = s.playing.is_some();
            if ui.button(if playing { "⏸" } else { "▶" }).on_hover_text(if playing { "Pause" } else { "Play" }).clicked() {
                s.playing = if playing { None } else { Some(now - at * s.seconds_per_leg()) };
            }
            if ui.button("⏭").on_hover_text("Next shot").clicked() {
                s.playing = None;
                s.run("sequence.at", json!({"t": (at.floor() + 1.0).min(legs)}));
            }
            let seq = s.ed.scene.sequence.clone();
            for sp in [0.5f32, 1.0, 2.0] {
                if ui.selectable_label((seq.speed - sp).abs() < 1e-3, format!("{sp}×")).clicked() {
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
        // The timeline: drag to fly the camera between the shots.
        let now = ui.input(|i| i.time);
        let mut t = s.timeline(now).unwrap_or(s.scrub);
        let resp = ui.add(egui::Slider::new(&mut t, 0.0..=legs).show_value(false).text("timeline"));
        if resp.changed() {
            s.playing = None;
            s.scrub = t;
            s.run("sequence.at", json!({"t": t}));
        }
        let secs = s.seconds_per_leg() * legs;
        ui.label(
            RichText::new(format!("{:.1} s of {:.1} s  ·  shot {} of {n}", t * s.seconds_per_leg(), secs, (t.round() as usize + 1).min(n)))
                .small()
                .color(look.t.dim),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnails_follow_the_note_not_the_view() {
        let ctx = egui::Context::default();
        let mut ed = Editor::new();
        ed.set_viewport(400.0, 300.0);
        ed.run("stroke.add", &json!({"points": [[0, 0, 0], [1, 1, 0]]})).expect("curve");
        let a = ed.run("shot.add", &json!({})).expect("a").as_u64().expect("id");
        ed.run("camera.set", &json!({"yaw": 60})).expect("turn");
        let b = ed.run("shot.add", &json!({})).expect("b").as_u64().expect("id");
        let mut strip = ShotStrip::default();
        strip.refresh(&ctx, &mut ed);
        assert!(!strip.has_thumb(a), "closed: nothing is drawn");
        strip.open = true;
        strip.refresh(&ctx, &mut ed);
        assert!(strip.has_thumb(a) && !strip.has_thumb(b), "one per frame");
        strip.refresh(&ctx, &mut ed);
        assert!(strip.has_thumb(b));
        // Orbiting the view leaves the thumbnails as they are; editing the note redraws them.
        let before = strip.thumbs.get(&a).map(|t| t.0.clone());
        ed.run("camera.set", &json!({"yaw": 10})).expect("orbit");
        strip.refresh(&ctx, &mut ed);
        assert_eq!(strip.thumbs.get(&a).map(|t| t.0.clone()), before, "the view moving is not a change");
        ed.run("stroke.add", &json!({"points": [[0, 0, 1], [1, 0, 1]]})).expect("another");
        strip.refresh(&ctx, &mut ed);
        assert_ne!(strip.thumbs.get(&a).map(|t| t.0.clone()), before, "the note changed");
        // A deleted shot's thumbnail goes.
        ed.run("shot.delete", &json!({"ids": [b]})).expect("delete");
        strip.refresh(&ctx, &mut ed);
        assert!(!strip.has_thumb(b));
    }
}
