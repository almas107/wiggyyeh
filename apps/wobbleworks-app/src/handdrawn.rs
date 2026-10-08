//! The hand-drawn pass: after PhotoCraft has painted a frame, every box, line and dot it drew
//! (panels, buttons, fields, separators, checkboxes) is swapped for a wobbly marker version that
//! boils with the rest of the UI. Text, images, meshes and the canvas are left exactly as drawn:
//! the picture and its overlays (selections, handles, the brush cursor) must stay precise.
//!
//! Each shape's wobble is seeded from where it is, so a still UI only changes on boil frames.

use egui::epaint::{CircleShape, RectShape};
use egui::{Color32, Context, LayerId, Order, Pos2, Rect, Shape, Stroke};

use crate::rough;

/// Smallest box (points) worth roughening; tiny ones (text cursors, ticks) stay crisp.
const MIN_SIDE: f32 = 5.0;
/// Outline weight for roughened boxes: a marker, not a hairline.
const MIN_WIDTH: f32 = 1.5;

/// Roughen every layer's shapes for this frame. Shapes in `keep` on background layers (the canvas
/// area) are left alone.
pub fn apply(ctx: &Context, keep: Rect, frame: u64) {
    let layers: Vec<LayerId> = ctx.memory(|m| m.layer_ids().collect());
    ctx.graphics_mut(|g| {
        for layer in layers {
            let skip_canvas = layer.order == Order::Background;
            let Some(list) = g.get_mut(layer) else { continue };
            for i in 0..list.next_idx().0 {
                list.mutate_shape(egui::layers::ShapeIdx(i), |cs| {
                    if skip_canvas && keep.contains_rect(cs.shape.visual_bounding_rect()) {
                        return;
                    }
                    if roughen(&mut cs.shape, frame) {
                        // Wobbly edges stray a little past the shape; don't clip them flat.
                        cs.clip_rect = cs.clip_rect.expand(2.0);
                    }
                });
            }
        }
    });
}

fn seed_at(p: Pos2, salt: u64) -> u64 {
    let x = (p.x.round() as i64) as u64;
    let y = (p.y.round() as i64) as u64;
    x.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ y.wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ salt
}

/// Swap `shape` for its hand-drawn version; `true` when it changed.
fn roughen(shape: &mut Shape, frame: u64) -> bool {
    match shape {
        Shape::Vec(v) => v.iter_mut().fold(false, |any, s| roughen(s, frame) | any),
        Shape::Rect(r) => match rect(r, frame) {
            Some(s) => {
                *shape = s;
                true
            }
            None => false,
        },
        Shape::LineSegment { points, stroke } => {
            let (a, b) = (points[0], points[1]);
            let len = (b - a).length();
            if len < 8.0 || !len.is_finite() || stroke.width <= 0.0 {
                return false;
            }
            let n = ((len / 6.0) as usize).clamp(2, 400);
            let pts = rough::jitter_line(&rough::segment(a, b, n), seed_at(a, 1), frame, 0.9);
            *shape = Shape::line(pts, Stroke::new(stroke.width.max(1.2), stroke.color));
            true
        }
        Shape::Circle(c) => match circle(c, frame) {
            Some(s) => {
                *shape = s;
                true
            }
            None => false,
        },
        _ => false,
    }
}

fn rect(r: &RectShape, frame: u64) -> Option<Shape> {
    let b = r.rect;
    // Textured rects are images (thumbnails, swatches drawn as textures): leave them be. Blurred
    // ones are soft shadows.
    if r.brush.is_some() || r.blur_width > 0.0 || !b.is_finite() || b.width() < MIN_SIDE || b.height() < MIN_SIDE {
        return None;
    }
    let has_fill = r.fill.a() > 0;
    let has_stroke = r.stroke.width > 0.0 && r.stroke.color.a() > 0;
    if !has_fill && !has_stroke {
        return None;
    }
    let radius = f32::from(r.corner_radius.nw.max(r.corner_radius.ne).max(r.corner_radius.sw).max(r.corner_radius.se));
    // Big panels wobble a bit more than small fields, but never enough to look broken.
    let amp = (b.width().min(b.height()) * 0.06).clamp(0.5, 1.6);
    let base = rough::rounded_outline(b, radius, 4.0);
    let seed = seed_at(b.min, (b.width() as u64) << 16 ^ b.height() as u64);
    let edge = rough::wobble(&base, b.center(), seed, frame, amp);
    let mut out = Vec::with_capacity(2);
    if has_fill {
        out.push(Shape::convex_polygon(edge.clone(), r.fill, Stroke::NONE));
    }
    if has_stroke {
        let w = r.stroke.width.max(MIN_WIDTH);
        out.push(Shape::closed_line(edge, Stroke::new(w, r.stroke.color)));
    }
    Some(Shape::Vec(out))
}

fn circle(c: &CircleShape, frame: u64) -> Option<Shape> {
    if c.radius < 3.5 || !c.radius.is_finite() || !c.center.is_finite() {
        return None;
    }
    let n = ((c.radius * 1.4) as usize).clamp(14, 64);
    let base: Vec<Pos2> = (0..n).map(|i| c.center + egui::Vec2::angled(i as f32 / n as f32 * std::f32::consts::TAU) * c.radius).collect();
    let edge = rough::wobble(&base, c.center, seed_at(c.center, 7), frame, (c.radius * 0.07).clamp(0.4, 1.4));
    let mut out = Vec::with_capacity(2);
    if c.fill.a() > 0 {
        out.push(Shape::convex_polygon(edge.clone(), c.fill, Stroke::NONE));
    }
    if c.stroke.width > 0.0 && c.stroke.color != Color32::TRANSPARENT {
        out.push(Shape::closed_line(edge, Stroke::new(c.stroke.width.max(1.2), c.stroke.color)));
    }
    Some(Shape::Vec(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    #[test]
    fn boxes_lines_and_dots_get_wobbly_and_images_text_stay() {
        let r = Rect::from_min_size(pos2(10.0, 10.0), egui::vec2(80.0, 30.0));
        let mut s = Shape::rect_filled(r, 4.0, Color32::RED);
        assert!(roughen(&mut s, 0));
        assert!(matches!(s, Shape::Vec(ref v) if v.len() == 1));
        let mut line = Shape::line_segment([pos2(0.0, 0.0), pos2(100.0, 0.0)], Stroke::new(1.0, Color32::BLACK));
        assert!(roughen(&mut line, 1));
        let mut dot = Shape::circle_filled(pos2(5.0, 5.0), 8.0, Color32::BLUE);
        assert!(roughen(&mut dot, 2));
        // Tiny things, invisible boxes, textured rects and NaNs are left alone.
        let mut tiny = Shape::rect_filled(Rect::from_min_size(Pos2::ZERO, egui::vec2(2.0, 20.0)), 0.0, Color32::RED);
        assert!(!roughen(&mut tiny, 0));
        let mut clear = Shape::rect_filled(r, 0.0, Color32::TRANSPARENT);
        assert!(!roughen(&mut clear, 0));
        let mut img = Shape::image(egui::TextureId::Managed(1), r, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
        assert!(!roughen(&mut img, 0));
        let mut nan = Shape::rect_filled(Rect::from_min_size(pos2(f32::NAN, 0.0), egui::vec2(20.0, 20.0)), 0.0, Color32::RED);
        assert!(!roughen(&mut nan, 0));
        let mut nan_line = Shape::line_segment([pos2(f32::NAN, 0.0), pos2(100.0, 0.0)], Stroke::new(1.0, Color32::BLACK));
        assert!(!roughen(&mut nan_line, 0));
    }

    #[test]
    fn a_still_ui_keeps_its_wobble_between_frames() {
        let r = Rect::from_min_size(pos2(10.0, 10.0), egui::vec2(80.0, 30.0));
        let a = rect(&RectShape::filled(r, 4.0, Color32::RED), 0);
        let b = rect(&RectShape::filled(r, 4.0, Color32::RED), 0);
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
    }
}
