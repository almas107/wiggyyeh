//! Hand-drawn vector icons (no image assets): each is drawn in a unit square mapped onto its
//! button, in the ink colour with an accent for the paint-ish parts. Started from the earlier
//! WobbleWorks' icons; the tool icons are new.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Brush,
    Pencil,
    Eraser,
    Fill,
    Gradient,
    Smudge,
    Blur,
    Stamp,
    Lasso,
    Marquee,
    Wand,
    Move,
    Text,
    Rect,
    Ellipse,
    Line,
    Pick,
    Hand,
    Zoom,
    Crop,
    More,
    New,
    Folder,
    Save,
    Export,
    Undo,
    Redo,
    Sliders,
}

struct Pen<'a> {
    p: &'a Painter,
    r: Rect,
    s: Stroke,
    c: Color32,
}

impl Pen<'_> {
    fn at(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.r.min.x + x * self.r.width(), self.r.min.y + y * self.r.height())
    }
    fn line(&self, pts: &[(f32, f32)]) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.at(x, y)).collect();
        self.p.add(Shape::line(v, self.s));
    }
    fn closed(&self, pts: &[(f32, f32)], fill: bool) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.at(x, y)).collect();
        if fill {
            self.p.add(Shape::convex_polygon(v, self.c, self.s));
        } else {
            self.p.add(Shape::closed_line(v, self.s));
        }
    }
    fn dot(&self, x: f32, y: f32, r: f32) {
        self.p.circle_filled(self.at(x, y), r * self.r.width(), self.c);
    }
    fn ring(&self, x: f32, y: f32, r: f32) {
        self.p.circle_stroke(self.at(x, y), r * self.r.width(), self.s);
    }
}

/// Draw `icon` into `rect` in `color`; `accent` tints the paint-ish parts.
pub fn paint(p: &Painter, rect: Rect, icon: Icon, color: Color32, accent: Color32) {
    use std::f32::consts::{PI, TAU};
    let w = (rect.width() * 0.08).max(1.4);
    let pen = Pen { p, r: rect, s: Stroke::new(w, color), c: color };
    let acc = Pen { p, r: rect, s: Stroke::new(w, color), c: accent };
    let paint = Pen { p, r: rect, s: Stroke::new(w * 1.7, accent), c: accent };
    match icon {
        Icon::Brush => {
            acc.closed(&[(0.18, 0.86), (0.22, 0.66), (0.36, 0.62), (0.4, 0.76)], true);
            pen.line(&[(0.36, 0.62), (0.78, 0.18), (0.86, 0.26), (0.42, 0.7)]);
        }
        Icon::Pencil => {
            pen.closed(&[(0.18, 0.82), (0.24, 0.62), (0.7, 0.16), (0.84, 0.3), (0.38, 0.76)], false);
            acc.closed(&[(0.18, 0.82), (0.24, 0.62), (0.38, 0.76)], true);
            pen.line(&[(0.6, 0.26), (0.74, 0.4)]);
        }
        Icon::Eraser => {
            acc.closed(&[(0.16, 0.6), (0.48, 0.28), (0.84, 0.6), (0.52, 0.9)], true);
            pen.line(&[(0.32, 0.44), (0.68, 0.76)]);
            pen.line(&[(0.1, 0.92), (0.9, 0.92)]);
        }
        Icon::Fill => {
            pen.closed(&[(0.2, 0.46), (0.48, 0.18), (0.76, 0.46), (0.48, 0.74)], false);
            paint.line(&[(0.76, 0.46), (0.82, 0.66)]);
            acc.dot(0.82, 0.76, 0.08);
        }
        Icon::Gradient => {
            for i in 0..5 {
                let x = 0.16 + i as f32 * 0.14;
                let c = crate::theme::mix(accent, Color32::WHITE, i as f32 / 5.0);
                Pen { p, r: rect, s: Stroke::NONE, c }.closed(&[(x, 0.22), (x + 0.14, 0.22), (x + 0.14, 0.78), (x, 0.78)], true);
            }
            pen.closed(&[(0.16, 0.22), (0.86, 0.22), (0.86, 0.78), (0.16, 0.78)], false);
        }
        Icon::Smudge => {
            paint.line(&[(0.14, 0.78), (0.3, 0.66), (0.5, 0.7), (0.66, 0.6)]);
            pen.closed(&[(0.56, 0.62), (0.62, 0.2), (0.76, 0.2), (0.74, 0.62)], false);
        }
        Icon::Blur => {
            acc.closed(&[(0.5, 0.14), (0.74, 0.52), (0.72, 0.7), (0.6, 0.84), (0.4, 0.84), (0.28, 0.7), (0.26, 0.52)], true);
        }
        Icon::Stamp => {
            pen.closed(&[(0.4, 0.16), (0.6, 0.16), (0.6, 0.5), (0.4, 0.5)], false);
            acc.closed(&[(0.18, 0.56), (0.82, 0.56), (0.82, 0.72), (0.18, 0.72)], true);
            pen.line(&[(0.18, 0.86), (0.82, 0.86)]);
        }
        Icon::Lasso => {
            let pts: Vec<(f32, f32)> = (0..18).map(|i| (i as f32 / 18.0 * TAU).sin_cos()).map(|(s, c)| (0.52 + 0.32 * c, 0.42 + 0.22 * s)).collect();
            for (i, seg) in pts.windows(2).enumerate() {
                if i % 2 == 0 {
                    pen.line(seg);
                }
            }
            pen.line(&[(0.3, 0.58), (0.26, 0.86)]);
        }
        Icon::Marquee => {
            let c = [(0.16, 0.22), (0.84, 0.22), (0.84, 0.78), (0.16, 0.78), (0.16, 0.22)];
            for seg in c.windows(2) {
                if let [(x0, y0), (x1, y1)] = seg {
                    for k in 0..4 {
                        let (a, b) = (k as f32 / 4.0, (k as f32 + 0.55) / 4.0);
                        pen.line(&[(x0 + (x1 - x0) * a, y0 + (y1 - y0) * a), (x0 + (x1 - x0) * b, y0 + (y1 - y0) * b)]);
                    }
                }
            }
        }
        Icon::Wand => {
            pen.line(&[(0.18, 0.84), (0.6, 0.42)]);
            for (x, y) in [(0.72, 0.16), (0.86, 0.3), (0.84, 0.12), (0.62, 0.22)] {
                acc.dot(x, y, 0.05);
            }
        }
        Icon::Move => {
            pen.line(&[(0.5, 0.12), (0.5, 0.88)]);
            pen.line(&[(0.12, 0.5), (0.88, 0.5)]);
            for (a, b, c) in [
                ((0.38, 0.24), (0.5, 0.12), (0.62, 0.24)),
                ((0.38, 0.76), (0.5, 0.88), (0.62, 0.76)),
                ((0.24, 0.38), (0.12, 0.5), (0.24, 0.62)),
                ((0.76, 0.38), (0.88, 0.5), (0.76, 0.62)),
            ] {
                pen.line(&[a, b, c]);
            }
        }
        Icon::Text => {
            pen.line(&[(0.2, 0.2), (0.8, 0.2)]);
            pen.line(&[(0.5, 0.2), (0.5, 0.84)]);
            pen.line(&[(0.38, 0.84), (0.62, 0.84)]);
        }
        Icon::Rect => acc.closed(&[(0.18, 0.26), (0.82, 0.26), (0.82, 0.74), (0.18, 0.74)], true),
        Icon::Ellipse => {
            let pts: Vec<(f32, f32)> = (0..32).map(|i| (i as f32 / 32.0 * TAU).sin_cos()).map(|(s, c)| (0.5 + 0.34 * c, 0.5 + 0.26 * s)).collect();
            acc.closed(&pts, true);
        }
        Icon::Line => paint.line(&[(0.18, 0.82), (0.82, 0.18)]),
        Icon::Pick => {
            pen.line(&[(0.18, 0.82), (0.58, 0.42)]);
            acc.dot(0.68, 0.32, 0.13);
            pen.ring(0.68, 0.32, 0.13);
        }
        Icon::Hand => pen.closed(
            &[(0.3, 0.88), (0.25, 0.55), (0.3, 0.3), (0.42, 0.3), (0.45, 0.18), (0.56, 0.18), (0.6, 0.3), (0.72, 0.32), (0.74, 0.62), (0.66, 0.88)],
            false,
        ),
        Icon::Zoom => {
            pen.ring(0.42, 0.42, 0.24);
            pen.line(&[(0.6, 0.6), (0.86, 0.86)]);
            pen.line(&[(0.3, 0.42), (0.54, 0.42)]);
            pen.line(&[(0.42, 0.3), (0.42, 0.54)]);
        }
        Icon::Crop => {
            pen.line(&[(0.3, 0.1), (0.3, 0.7), (0.9, 0.7)]);
            pen.line(&[(0.1, 0.3), (0.7, 0.3), (0.7, 0.9)]);
        }
        Icon::More => {
            for x in [0.25, 0.5, 0.75] {
                acc.dot(x, 0.5, 0.08);
            }
        }
        Icon::New => {
            pen.closed(&[(0.22, 0.12), (0.62, 0.12), (0.78, 0.28), (0.78, 0.88), (0.22, 0.88)], false);
            paint.line(&[(0.5, 0.4), (0.5, 0.7)]);
            paint.line(&[(0.35, 0.55), (0.65, 0.55)]);
        }
        Icon::Folder => {
            acc.closed(&[(0.12, 0.26), (0.4, 0.26), (0.48, 0.36), (0.88, 0.36), (0.88, 0.8), (0.12, 0.8)], true);
        }
        Icon::Save => {
            pen.line(&[(0.5, 0.14), (0.5, 0.62)]);
            pen.line(&[(0.3, 0.44), (0.5, 0.64), (0.7, 0.44)]);
            paint.line(&[(0.16, 0.66), (0.16, 0.84), (0.84, 0.84), (0.84, 0.66)]);
        }
        Icon::Export => {
            pen.closed(&[(0.14, 0.3), (0.62, 0.3), (0.62, 0.84), (0.14, 0.84)], false);
            acc.closed(&[(0.2, 0.78), (0.32, 0.58), (0.42, 0.7), (0.5, 0.62), (0.56, 0.78)], true);
            paint.line(&[(0.48, 0.48), (0.86, 0.12)]);
            pen.line(&[(0.62, 0.12), (0.86, 0.12), (0.86, 0.36)]);
        }
        Icon::Undo | Icon::Redo => {
            let flip = |x: f32| if icon == Icon::Redo { 1.0 - x } else { x };
            let arc: Vec<(f32, f32)> = (0..=12).map(|i| i as f32 / 12.0 * PI).map(|a| (flip(0.5 + 0.28 * a.cos()), 0.55 - 0.25 * a.sin())).collect();
            pen.line(&arc);
            pen.line(&[(flip(0.1), 0.42), (flip(0.22), 0.6), (flip(0.38), 0.46)]);
        }
        Icon::Sliders => {
            for (y, x) in [(0.26, 0.66), (0.5, 0.34), (0.74, 0.58)] {
                pen.line(&[(0.14, y), (0.86, y)]);
                acc.dot(x, y, 0.08);
                pen.ring(x, y, 0.08);
            }
        }
    }
}
