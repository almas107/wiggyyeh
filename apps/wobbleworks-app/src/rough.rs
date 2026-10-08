//! Hand-painted shapes: rounded boxes and blobs whose edges wander like a marker line, with a hard
//! drop shadow. Edges follow smooth value noise along the outline (seeded per widget, so each
//! one keeps its own wobble) and "boil" like WigglyPaint's lines: a few noise frames loop, so the
//! whole UI quietly shimmers. With boiling off, frame 0 holds still.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};

/// Boiling: frames in the loop and seconds per frame.
pub const BOIL_FRAMES: u64 = 3;
pub const BOIL_SECONDS: f64 = 0.13;

/// The boil frame at time `t` (0 when boiling is off).
pub fn boil_frame(t: f64, boiling: bool) -> u64 {
    if boiling && t.is_finite() && t >= 0.0 { (t / BOIL_SECONDS) as u64 % BOIL_FRAMES } else { 0 }
}

/// Hash to [-1, 1].
pub fn hash(seed: u64, i: u64) -> f32 {
    let mut x = seed ^ i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    x = x.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    x ^= x >> 33;
    (x >> 40) as f32 / (1u64 << 23) as f32 - 1.0
}

/// Smooth 1-D value noise in [-1, 1]: cells `cell` long along `s`.
fn smooth(seed: u64, s: f32, cell: f32) -> f32 {
    let u = (s / cell.max(1.0)).max(0.0);
    let i = u.floor();
    let f = u - i;
    let f = f * f * (3.0 - 2.0 * f);
    let i = i as u64;
    hash(seed, i) * (1.0 - f) + hash(seed, i + 1) * f
}

/// Points around a rounded rectangle, about `step` apart, clockwise from the top-left corner.
pub fn rounded_outline(rect: Rect, radius: f32, step: f32) -> Vec<Pos2> {
    let r = radius.clamp(0.0, rect.width().min(rect.height()) / 2.0);
    let corners = [
        (pos2(rect.max.x - r, rect.min.y + r), -90.0f32),
        (pos2(rect.max.x - r, rect.max.y - r), 0.0),
        (pos2(rect.min.x + r, rect.max.y - r), 90.0),
        (pos2(rect.min.x + r, rect.min.y + r), 180.0),
    ];
    let step = step.max(1.0);
    let mut pts = Vec::new();
    let mut prev_end = pos2(rect.min.x + r, rect.min.y);
    for (c, start) in corners {
        // The straight side up to this corner.
        let arc_start = c + Vec2::angled(start.to_radians()) * r;
        let n = ((arc_start - prev_end).length() / step).ceil().max(1.0) as usize;
        pts.extend((0..n).map(|k| prev_end + (arc_start - prev_end) * (k as f32 / n as f32)));
        // The quarter circle.
        let m = ((r * std::f32::consts::FRAC_PI_2) / step).ceil().max(1.0) as usize;
        pts.extend((0..m).map(|k| c + Vec2::angled((start + 90.0 * k as f32 / m as f32).to_radians()) * r));
        prev_end = c + Vec2::angled((start + 90.0).to_radians()) * r;
    }
    pts
}

/// Move `pts` (a closed outline around `center`) in and out along the radial direction by smooth
/// noise: a hand-drawn edge. `amp` is the wobble in points.
pub fn wobble(pts: &[Pos2], center: Pos2, seed: u64, frame: u64, amp: f32) -> Vec<Pos2> {
    let mut s = 0.0;
    let mut prev = pts.first().copied().unwrap_or(center);
    let boil = seed ^ (frame + 1).wrapping_mul(0xA24B_AED4_963E_E407);
    pts.iter()
        .map(|&p| {
            s += (p - prev).length();
            prev = p;
            let n = smooth(seed, s, 26.0) * 0.7 + smooth(boil, s, 11.0) * 0.45;
            let d = p - center;
            let dir = if d.length_sq() > 0.0 { d.normalized() } else { Vec2::ZERO };
            p + dir * n * amp
        })
        .collect()
}

/// A painted box: hard shadow, fill, marker outline.
pub struct Paint {
    pub fill: Color32,
    pub ink: Stroke,
    pub shadow: Option<(Vec2, Color32)>,
    pub radius: f32,
    pub wobble: f32,
}

/// Paint `style` in `rect`. `seed` gives each box its own wobble; `frame` is the boil frame.
pub fn boxed(painter: &Painter, rect: Rect, style: &Paint, seed: u64, frame: u64) {
    painter.extend(boxed_shapes(rect, style, seed, frame));
}

/// The shapes [`boxed`] paints (for painting behind content added earlier).
pub fn boxed_shapes(rect: Rect, style: &Paint, seed: u64, frame: u64) -> Vec<Shape> {
    if !rect.is_finite() || rect.width() < 1.0 || rect.height() < 1.0 {
        return Vec::new();
    }
    let base = rounded_outline(rect, style.radius, 5.0);
    let edge = wobble(&base, rect.center(), seed, frame, style.wobble);
    let mut out = Vec::with_capacity(4);
    if let Some((off, colour)) = style.shadow {
        let sh: Vec<Pos2> = edge.iter().map(|p| *p + off).collect();
        out.push(Shape::convex_polygon(sh, colour, Stroke::NONE));
    }
    out.push(Shape::convex_polygon(edge.clone(), style.fill, Stroke::NONE));
    // A second, thinner pass on another wobble: the doubled line of a quick marker sketch.
    let sketch = wobble(&base, rect.center(), seed ^ 0x5bd1_e995, frame, style.wobble * 0.8);
    out.push(Shape::closed_line(sketch, Stroke::new(style.ink.width * 0.45, style.ink.color.gamma_multiply(0.55))));
    out.push(Shape::closed_line(edge, style.ink));
    out
}

/// A painted blob (a lumpy circle): swatches, knobs. `style.radius` and `style.wobble` are unused
/// (the wobble follows the blob's size).
pub fn blob(painter: &Painter, center: Pos2, radius: f32, style: &Paint, seed: u64, frame: u64) {
    let Paint { fill, ink, shadow, .. } = *style;
    if !center.is_finite() || !radius.is_finite() || radius < 0.5 {
        return;
    }
    let n = ((radius * 1.6) as usize).clamp(12, 64);
    let base: Vec<Pos2> = (0..n).map(|i| center + Vec2::angled(i as f32 / n as f32 * std::f32::consts::TAU) * radius).collect();
    let edge = wobble(&base, center, seed, frame, (radius * 0.09).clamp(0.6, 2.2));
    if let Some((off, colour)) = shadow {
        painter.add(Shape::convex_polygon(edge.iter().map(|p| *p + off).collect(), colour, Stroke::NONE));
    }
    painter.add(Shape::convex_polygon(edge.clone(), fill, Stroke::NONE));
    painter.add(Shape::closed_line(edge, ink));
}

/// A wobbly marker line through `pts`.
pub fn line(painter: &Painter, pts: &[Pos2], ink: Stroke, seed: u64, frame: u64, amp: f32) {
    let (Some(&first), true) = (pts.first(), pts.len() >= 2) else { return };
    let mut s = 0.0;
    let mut prev = first;
    let boil = seed ^ (frame + 1).wrapping_mul(0xA24B_AED4_963E_E407);
    let out: Vec<Pos2> = pts
        .iter()
        .map(|&p| {
            s += (p - prev).length();
            prev = p;
            p + vec2(smooth(seed, s, 20.0), smooth(boil, s + 500.0, 9.0)) * amp
        })
        .collect();
    painter.add(Shape::line(out, ink));
}

/// `a` to `b` in `n` even steps (for wobbly straight lines).
pub fn segment(a: Pos2, b: Pos2, n: usize) -> Vec<Pos2> {
    let n = n.max(1);
    (0..=n).map(|k| a + (b - a) * (k as f32 / n as f32)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_bounded_and_deterministic() {
        for i in 0..2000 {
            let h = hash(42, i);
            assert!((-1.0..=1.0).contains(&h));
            assert_eq!(h, hash(42, i));
        }
        assert_ne!(hash(1, 5), hash(2, 5));
        assert!((-1.0..=1.0).contains(&smooth(7, 123.4, 26.0)));
    }

    #[test]
    fn outlines_hug_their_box_and_survive_degenerate_input() {
        let r = Rect::from_min_size(pos2(10.0, 20.0), vec2(120.0, 40.0));
        let pts = rounded_outline(r, 10.0, 5.0);
        assert!(pts.len() > 20);
        assert!(pts.iter().all(|p| r.expand(0.01).contains(*p)));
        let w = wobble(&pts, r.center(), 9, 1, 2.0);
        assert!(w.iter().all(|p| r.expand(2.5).contains(*p)));
        // Zero-size, huge radius and empty input don't panic.
        assert!(!rounded_outline(Rect::from_min_size(Pos2::ZERO, Vec2::ZERO), 50.0, 0.0).is_empty());
        assert!(wobble(&[], Pos2::ZERO, 1, 0, 1.0).is_empty());
    }

    #[test]
    fn boil_frames_loop_and_hold_still_when_off() {
        assert_eq!(boil_frame(0.0, true), 0);
        assert_eq!(boil_frame(BOIL_SECONDS * 1.5, true), 1);
        assert_eq!(boil_frame(BOIL_SECONDS * 3.2, true), 0);
        assert_eq!(boil_frame(5.0, false), 0);
        assert_eq!(boil_frame(f64::NAN, true), 0);
        assert_eq!(boil_frame(-1.0, true), 0);
    }
}
