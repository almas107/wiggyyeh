//! Hand-drawn vector icons (no font or image assets needed). Each icon is drawn in a unit square
//! mapped onto the button.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2};

use crate::model::{Brush, Tip};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Brush(Brush),
    Tip(Tip),
    Line,
    Rect,
    Ellipse,
    Fill,
    Lasso,
    Move,
    Pick,
    Hand,
    Undo,
    Redo,
    Play,
    Pause,
    Gear,
    Help,
    Eye,
    EyeOff,
    Lock,
    Clip,
    Plus,
    Trash,
    Up,
    Down,
    Copy,
    Image,
    Save,
    Folder,
    ZoomIn,
    ZoomOut,
    Fit,
    Focus,
    FlipH,
    FlipV,
    Merge,
    Clear,
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
    fn wave(&self, amp: f32, n: usize, y: f32) {
        let pts: Vec<(f32, f32)> = (0..=n).map(|i| (0.12 + 0.76 * i as f32 / n as f32, y + if i % 2 == 0 { -amp } else { amp })).collect();
        self.line(&pts);
    }
}

fn tip_poly(t: Tip) -> Vec<(f32, f32)> {
    match t {
        Tip::Round => (0..24).map(|i| (i as f32 / 24.0 * std::f32::consts::TAU).sin_cos()).map(|(s, c)| (0.5 + 0.3 * c, 0.5 + 0.3 * s)).collect(),
        Tip::Square => vec![(0.22, 0.22), (0.78, 0.22), (0.78, 0.78), (0.22, 0.78)],
        Tip::Diamond => vec![(0.5, 0.16), (0.84, 0.5), (0.5, 0.84), (0.16, 0.5)],
        Tip::Star => (0..10)
            .map(|i| {
                let a = i as f32 / 10.0 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                let r = if i % 2 == 0 { 0.36 } else { 0.16 };
                (0.5 + r * a.cos(), 0.53 + r * a.sin())
            })
            .collect(),
        Tip::Heart => (0..40)
            .map(|i| {
                let t = i as f32 / 40.0 * std::f32::consts::TAU;
                let x = 16.0 * t.sin().powi(3);
                let y = 13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
                (0.5 + x * 0.02, 0.48 - y * 0.02)
            })
            .collect(),
    }
}

/// Draw `icon` into `rect` in `color`; `accent` tints the paint-ish parts.
pub fn paint(p: &Painter, rect: Rect, icon: Icon, color: Color32, accent: Color32) {
    let w = (rect.width() * 0.085).max(1.4);
    let pen = Pen { p, r: rect, s: Stroke::new(w, color), c: color };
    let acc = Pen { p, r: rect, s: Stroke::new(w * 1.6, accent), c: accent };
    match icon {
        Icon::Brush(b) => match b {
            Brush::Marker => acc.wave(0.08, 4, 0.5),
            Brush::Dither => {
                for y in 0..4 {
                    for x in 0..4 {
                        if (x + y) % 2 == 0 {
                            acc.dot(0.26 + x as f32 * 0.16, 0.26 + y as f32 * 0.16, 0.055);
                        }
                    }
                }
            }
            Brush::Fuzz => {
                for y in 0..3 {
                    for x in 0..4 {
                        acc.dot(0.24 + x as f32 * 0.17, 0.32 + y as f32 * 0.18, 0.035);
                    }
                }
            }
            Brush::Shaky => acc.wave(0.1, 8, 0.5),
            Brush::Rowdy => acc.wave(0.22, 5, 0.5),
            Brush::Sketch => {
                for (k, off) in [-0.06f32, 0.0, 0.06].iter().enumerate() {
                    pen.line(&[(0.14, 0.62 + off), (0.86, 0.38 + off * (k as f32 - 1.0))]);
                }
            }
            Brush::Ribbon => {
                for i in 0..7 {
                    let x = 0.16 + i as f32 * 0.115;
                    let r = 0.03 + 0.07 * (i as f32 * 0.9).sin().abs();
                    acc.dot(x, 0.5 + 0.12 * (i as f32 * 0.9).sin(), r);
                }
            }
            Brush::Spray => {
                for i in 0..22 {
                    let a = i as f32 * 2.399;
                    let r = 0.3 * ((i as f32 + 0.5) / 22.0).sqrt();
                    acc.dot(0.5 + r * a.cos(), 0.5 + r * a.sin(), 0.028);
                }
            }
            Brush::Beads => {
                for (i, r) in [0.07f32, 0.1, 0.06, 0.09].iter().enumerate() {
                    acc.dot(0.2 + i as f32 * 0.2, 0.5, *r);
                }
            }
            Brush::Chalk => {
                for i in 0..14 {
                    if i % 3 != 1 {
                        acc.dot(0.15 + i as f32 * 0.05, 0.58 - i as f32 * 0.012, 0.05);
                    }
                }
            }
            Brush::Nib => {
                acc.closed(&[(0.2, 0.75), (0.32, 0.8), (0.82, 0.3), (0.7, 0.24)], true);
            }
            Brush::Blob => {
                let pts: Vec<(f32, f32)> = (0..20)
                    .map(|i| {
                        let a = i as f32 / 20.0 * std::f32::consts::TAU;
                        let r = 0.3 + 0.05 * (a * 3.0).sin();
                        (0.5 + r * a.cos(), 0.5 + r * a.sin())
                    })
                    .collect();
                let v: Vec<Pos2> = pts.iter().map(|&(x, y)| pen.at(x, y)).collect();
                p.add(Shape::convex_polygon(v, accent, pen.s));
            }
            Brush::Steady => acc.line(&[(0.15, 0.5), (0.85, 0.5)]),
            Brush::Eraser => {
                pen.closed(&[(0.2, 0.62), (0.52, 0.3), (0.8, 0.58), (0.48, 0.9)], false);
                pen.line(&[(0.36, 0.46), (0.64, 0.74)]);
                pen.line(&[(0.1, 0.9), (0.9, 0.9)]);
            }
        },
        Icon::Tip(t) => {
            let v: Vec<Pos2> = tip_poly(t).iter().map(|&(x, y)| pen.at(x, y)).collect();
            p.add(Shape::Path(egui::epaint::PathShape { points: v, closed: true, fill: accent, stroke: pen.s.into() }));
        }
        Icon::Line => acc.line(&[(0.18, 0.82), (0.82, 0.18)]),
        Icon::Rect => pen.closed(&[(0.18, 0.26), (0.82, 0.26), (0.82, 0.74), (0.18, 0.74)], false),
        Icon::Ellipse => {
            let pts: Vec<(f32, f32)> =
                (0..32).map(|i| (i as f32 / 32.0 * std::f32::consts::TAU).sin_cos()).map(|(s, c)| (0.5 + 0.34 * c, 0.5 + 0.24 * s)).collect();
            pen.closed(&pts, false);
        }
        Icon::Fill => {
            pen.closed(&[(0.22, 0.45), (0.5, 0.2), (0.78, 0.45), (0.5, 0.75)], false);
            acc.dot(0.82, 0.75, 0.08);
            acc.line(&[(0.78, 0.45), (0.82, 0.66)]);
        }
        Icon::Lasso => {
            let pts: Vec<(f32, f32)> =
                (0..18).map(|i| (i as f32 / 18.0 * std::f32::consts::TAU).sin_cos()).map(|(s, c)| (0.52 + 0.32 * c, 0.42 + 0.22 * s)).collect();
            for (i, w) in pts.windows(2).enumerate() {
                if i % 2 == 0 {
                    pen.line(w);
                }
            }
            pen.line(&[(0.3, 0.58), (0.26, 0.86)]);
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
        Icon::Pick => {
            pen.line(&[(0.18, 0.82), (0.58, 0.42)]);
            acc.dot(0.68, 0.32, 0.13);
            pen.ring(0.68, 0.32, 0.13);
        }
        Icon::Hand => {
            pen.closed(
                &[(0.3, 0.88), (0.25, 0.55), (0.3, 0.3), (0.42, 0.3), (0.45, 0.18), (0.56, 0.18), (0.6, 0.3), (0.72, 0.32), (0.74, 0.62), (0.66, 0.88)],
                false,
            );
        }
        Icon::Undo | Icon::Redo => {
            let flip = |x: f32| if icon == Icon::Redo { 1.0 - x } else { x };
            let arc: Vec<(f32, f32)> =
                (0..=12).map(|i| i as f32 / 12.0 * std::f32::consts::PI).map(|a| (flip(0.5 + 0.28 * a.cos()), 0.55 - 0.25 * a.sin())).collect();
            pen.line(&arc);
            pen.line(&[(flip(0.1), 0.42), (flip(0.22), 0.6), (flip(0.38), 0.46)]);
        }
        Icon::Play => pen.closed(&[(0.3, 0.2), (0.8, 0.5), (0.3, 0.8)], true),
        Icon::Pause => {
            pen.closed(&[(0.26, 0.22), (0.42, 0.22), (0.42, 0.78), (0.26, 0.78)], true);
            pen.closed(&[(0.58, 0.22), (0.74, 0.22), (0.74, 0.78), (0.58, 0.78)], true);
        }
        Icon::Gear => {
            for i in 0..8 {
                let a = i as f32 / 8.0 * std::f32::consts::TAU;
                pen.line(&[(0.5 + 0.22 * a.cos(), 0.5 + 0.22 * a.sin()), (0.5 + 0.38 * a.cos(), 0.5 + 0.38 * a.sin())]);
            }
            pen.ring(0.5, 0.5, 0.24);
            pen.ring(0.5, 0.5, 0.08);
        }
        Icon::Help => {
            let arc: Vec<(f32, f32)> = (0..=10)
                .map(|i| -std::f32::consts::PI + i as f32 / 10.0 * 1.5 * std::f32::consts::PI)
                .map(|a| (0.5 + 0.18 * a.cos(), 0.36 + 0.16 * a.sin()))
                .collect();
            pen.line(&arc);
            pen.line(&[(0.5, 0.52), (0.5, 0.64)]);
            pen.dot(0.5, 0.8, 0.05);
        }
        Icon::Eye | Icon::EyeOff => {
            let top: Vec<(f32, f32)> = (0..=12).map(|i| i as f32 / 12.0).map(|t| (0.12 + 0.76 * t, 0.5 - 0.26 * (t * std::f32::consts::PI).sin())).collect();
            let bot: Vec<(f32, f32)> = top.iter().map(|&(x, y)| (x, 1.0 - y)).collect();
            pen.line(&top);
            pen.line(&bot);
            if icon == Icon::Eye {
                pen.dot(0.5, 0.5, 0.11);
            } else {
                pen.line(&[(0.16, 0.84), (0.84, 0.16)]);
            }
        }
        Icon::Lock => {
            pen.closed(&[(0.24, 0.46), (0.76, 0.46), (0.76, 0.84), (0.24, 0.84)], false);
            let arc: Vec<(f32, f32)> = (0..=10)
                .map(|i| std::f32::consts::PI + i as f32 / 10.0 * std::f32::consts::PI)
                .map(|a| (0.5 + 0.17 * a.cos(), 0.46 + 0.22 * a.sin()))
                .collect();
            pen.line(&arc);
        }
        Icon::Clip => {
            pen.line(&[(0.3, 0.18), (0.3, 0.62), (0.7, 0.62)]);
            pen.line(&[(0.58, 0.5), (0.7, 0.62), (0.58, 0.74)]);
        }
        Icon::Plus => {
            pen.line(&[(0.5, 0.2), (0.5, 0.8)]);
            pen.line(&[(0.2, 0.5), (0.8, 0.5)]);
        }
        Icon::Trash => {
            pen.closed(&[(0.28, 0.3), (0.72, 0.3), (0.66, 0.86), (0.34, 0.86)], false);
            pen.line(&[(0.18, 0.3), (0.82, 0.3)]);
            pen.line(&[(0.42, 0.18), (0.58, 0.18)]);
        }
        Icon::Up => pen.line(&[(0.24, 0.62), (0.5, 0.34), (0.76, 0.62)]),
        Icon::Down => pen.line(&[(0.24, 0.38), (0.5, 0.66), (0.76, 0.38)]),
        Icon::Copy => {
            pen.closed(&[(0.18, 0.18), (0.6, 0.18), (0.6, 0.6), (0.18, 0.6)], false);
            pen.closed(&[(0.4, 0.4), (0.82, 0.4), (0.82, 0.82), (0.4, 0.82)], false);
        }
        Icon::Image => {
            pen.closed(&[(0.14, 0.2), (0.86, 0.2), (0.86, 0.8), (0.14, 0.8)], false);
            acc.closed(&[(0.2, 0.74), (0.42, 0.44), (0.58, 0.62), (0.66, 0.52), (0.8, 0.74)], true);
            acc.dot(0.7, 0.34, 0.06);
        }
        Icon::Save => {
            pen.line(&[(0.5, 0.14), (0.5, 0.62)]);
            pen.line(&[(0.3, 0.44), (0.5, 0.64), (0.7, 0.44)]);
            pen.line(&[(0.16, 0.66), (0.16, 0.84), (0.84, 0.84), (0.84, 0.66)]);
        }
        Icon::Folder => pen.closed(&[(0.12, 0.26), (0.4, 0.26), (0.48, 0.36), (0.88, 0.36), (0.88, 0.8), (0.12, 0.8)], false),
        Icon::ZoomIn | Icon::ZoomOut => {
            pen.ring(0.42, 0.42, 0.24);
            pen.line(&[(0.6, 0.6), (0.86, 0.86)]);
            pen.line(&[(0.3, 0.42), (0.54, 0.42)]);
            if icon == Icon::ZoomIn {
                pen.line(&[(0.42, 0.3), (0.42, 0.54)]);
            }
        }
        Icon::Fit => {
            for (a, b, c) in [
                ((0.16, 0.36), (0.16, 0.16), (0.36, 0.16)),
                ((0.64, 0.16), (0.84, 0.16), (0.84, 0.36)),
                ((0.84, 0.64), (0.84, 0.84), (0.64, 0.84)),
                ((0.36, 0.84), (0.16, 0.84), (0.16, 0.64)),
            ] {
                pen.line(&[a, b, c]);
            }
        }
        Icon::Focus => {
            pen.closed(&[(0.14, 0.2), (0.86, 0.2), (0.86, 0.8), (0.14, 0.8)], false);
            acc.dot(0.5, 0.5, 0.12);
        }
        Icon::FlipH => {
            pen.line(&[(0.5, 0.12), (0.5, 0.88)]);
            pen.closed(&[(0.4, 0.3), (0.4, 0.72), (0.12, 0.72)], true);
            pen.closed(&[(0.6, 0.3), (0.6, 0.72), (0.88, 0.72)], false);
        }
        Icon::FlipV => {
            pen.line(&[(0.12, 0.5), (0.88, 0.5)]);
            pen.closed(&[(0.3, 0.4), (0.72, 0.4), (0.72, 0.12)], true);
            pen.closed(&[(0.3, 0.6), (0.72, 0.6), (0.72, 0.88)], false);
        }
        Icon::Merge => {
            pen.line(&[(0.3, 0.14), (0.3, 0.36), (0.5, 0.56)]);
            pen.line(&[(0.7, 0.14), (0.7, 0.36), (0.5, 0.56), (0.5, 0.86)]);
        }
        Icon::Clear => {
            pen.closed(&[(0.16, 0.2), (0.84, 0.2), (0.84, 0.8), (0.16, 0.8)], false);
            pen.line(&[(0.34, 0.36), (0.66, 0.64)]);
            pen.line(&[(0.66, 0.36), (0.34, 0.64)]);
        }
    }
}
