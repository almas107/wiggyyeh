//! The brush strip, WigglyPaint style: a few wiggle brushes, each a PhotoCraft brush setup
//! (through `tools.setBrush`) plus how far its lines wander. The tiles show a boiling sample of
//! each brush's mark.

use egui::{Painter, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};
use serde_json::{Value, json};

use crate::rough;
use crate::widgets::Look;

/// A wiggle brush.
pub struct WigglyBrush {
    pub name: &'static str,
    pub tip: &'static str,
    /// Pixels the lines wander.
    pub wiggle: f32,
    /// `tools.setBrush` fields on top of PhotoCraft's default brush.
    fields: fn() -> Value,
    sample: Sample,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sample {
    Line(u8),
    Chalk,
    Nib,
    Spray,
    Pixel,
}

pub const BRUSHES: &[WigglyBrush] = &[
    WigglyBrush {
        name: "Marker",
        tip: "A smooth marker that boils gently",
        wiggle: 3.0,
        fields: || json!({"hardness": 1.0, "spacing": 0.1}),
        sample: Sample::Line(3),
    },
    WigglyBrush { name: "Shaky", tip: "Nervous lines", wiggle: 5.0, fields: || json!({"hardness": 1.0, "spacing": 0.1}), sample: Sample::Line(5) },
    WigglyBrush { name: "Rowdy", tip: "Lines that can't sit still", wiggle: 9.0, fields: || json!({"hardness": 1.0, "spacing": 0.1}), sample: Sample::Line(9) },
    WigglyBrush { name: "Steady", tip: "Holds perfectly still", wiggle: 0.0, fields: || json!({"hardness": 1.0, "spacing": 0.1}), sample: Sample::Line(0) },
    WigglyBrush {
        name: "Chalk",
        tip: "Dusty and broken",
        wiggle: 3.0,
        fields: || json!({"hardness": 0.5, "flow": 0.7, "noise": true, "scattering": {"enabled": true, "scatter": {"jitter": 0.4}, "count": 2}}),
        sample: Sample::Chalk,
    },
    WigglyBrush {
        name: "Nib",
        tip: "A slanted pen nib",
        wiggle: 2.0,
        fields: || json!({"hardness": 1.0, "roundness": 0.25, "angle": 45.0}),
        sample: Sample::Nib,
    },
    WigglyBrush {
        name: "Spray",
        tip: "Spray paint",
        wiggle: 2.0,
        fields: || json!({"hardness": 0.8, "spacing": 0.6, "scattering": {"enabled": true, "scatter": {"jitter": 2.5}, "bothAxes": true, "count": 4}}),
        sample: Sample::Spray,
    },
    WigglyBrush {
        name: "Pixel",
        tip: "Crisp, chunky pixels",
        wiggle: 2.0,
        fields: || json!({"hardness": 1.0, "aliased": true, "spacing": 0.1}),
        sample: Sample::Pixel,
    },
];

/// `tools.setBrush` params for brush `b` at `size` pixels.
pub fn params(b: &WigglyBrush, size: f32) -> Value {
    let mut v = (b.fields)();
    if let Some(o) = v.as_object_mut() {
        o.insert("reset".into(), json!(true));
        o.insert("size".into(), json!(if size.is_finite() { size.clamp(1.0, 5000.0) } else { 12.0 }));
    }
    v
}

/// Draw `b`'s sample mark in `r`.
pub fn paint_sample(p: &Painter, r: Rect, b: &WigglyBrush, look: &Look, seed: u64) {
    let ink = look.t.ink;
    let n = 12;
    let wave: Vec<Pos2> = (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            pos2(r.left() + r.width() * t, r.center().y + (t * std::f32::consts::TAU).sin() * r.height() * 0.22)
        })
        .collect();
    match b.sample {
        Sample::Line(w) => {
            let amp = f32::from(w) * 0.35;
            if w == 0 {
                p.add(Shape::line(wave, Stroke::new(3.0, ink)));
            } else {
                rough::line(p, &wave, Stroke::new(3.0, ink), seed, look.frame, amp);
            }
        }
        Sample::Chalk => {
            for (k, seg) in wave.windows(2).enumerate() {
                if k % 3 != 2 {
                    rough::line(p, seg, Stroke::new(3.0, ink.gamma_multiply(0.75)), seed ^ k as u64, look.frame, 1.0);
                }
            }
        }
        Sample::Nib => {
            for seg in wave.windows(2) {
                let (Some(a), Some(b2)) = (seg.first(), seg.last()) else { continue };
                // Thick where the stroke runs across the slant, thin along it.
                let d = (*b2 - *a).normalized();
                let w = 1.2 + 3.5 * (d.x - d.y).abs() / std::f32::consts::SQRT_2;
                p.add(Shape::line_segment([*a, *b2], Stroke::new(w, ink)));
            }
        }
        Sample::Spray => {
            for i in 0..40u64 {
                let a = rough::hash(seed ^ look.frame, i) * std::f32::consts::PI;
                let d = rough::hash(seed, i + 100).abs();
                let c = r.center() + Vec2::angled(a * 2.0) * d * r.width() * 0.4;
                p.circle_filled(c, 1.1, ink);
            }
        }
        Sample::Pixel => {
            let s = 3.0;
            for q in &wave {
                let sq = pos2((q.x / s).round() * s, (q.y / s).round() * s);
                p.rect_filled(Rect::from_min_size(sq, vec2(s, s)), 0.0, ink);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_engine::Session;

    #[test]
    fn every_brush_is_a_valid_photocraft_brush() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        for b in BRUSHES {
            s.execute("tools.setBrush", params(b, 12.0)).unwrap_or_else(|e| panic!("{}: {e}", b.name));
            assert_eq!(s.tools.brush.size, 12.0, "{}", b.name);
            s.execute("paint.stroke", json!({"points": [[5, 30], [60, 30]]})).unwrap_or_else(|e| panic!("{}: {e}", b.name));
        }
        assert!(s.tools.brush.aliased, "the last one is Pixel");
        assert_eq!(params(&BRUSHES[0], f32::NAN)["size"], json!(12.0), "a bad size falls back");
    }
}
