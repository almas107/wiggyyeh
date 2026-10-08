//! PhotoCraft's icons, hand-drawn: every Lucide icon's SVG is parsed into polylines once, then
//! drawn as a soft marker line that wobbles and boils like the rest of the UI, with closed round
//! shapes filled with a pale wash. Installed through `photocraft_ui_egui::icons::set_painter`, so
//! the toolbar, panels and menus all get them. Supports the SVG subset Lucide uses: `path` (all
//! commands, arcs included), `circle`, `ellipse`, `line`, `rect`, `polyline` and `polygon`.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};

use crate::rough;
use crate::theme::mix;

/// One stroke of an icon in its 24 × 24 box; `fill` marks closed convex shapes (circles, rects).
#[derive(Clone, Debug, PartialEq)]
pub struct Poly {
    pub pts: Vec<Pos2>,
    pub closed: bool,
    pub fill: bool,
}

/// Every icon, parsed once.
fn library() -> &'static HashMap<&'static str, Vec<Poly>> {
    static LIB: OnceLock<HashMap<&'static str, Vec<Poly>>> = OnceLock::new();
    LIB.get_or_init(|| {
        photocraft_ui_egui::icons::names().filter_map(|n| photocraft_ui_egui::icons::svg(n).map(|b| (n, parse(&String::from_utf8_lossy(b))))).collect()
    })
}

/// The polylines of an SVG document (attributes in Lucide's style: `name="value"`).
pub fn parse(svg: &str) -> Vec<Poly> {
    let mut out = Vec::new();
    for el in svg.split('<').skip(1) {
        let name: String = el.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        let attr = |k: &str| -> Option<&str> {
            let key = format!(" {k}=\"");
            let i = el.find(&key)? + key.len();
            let rest = el.get(i..)?;
            rest.get(..rest.find('"')?)
        };
        let num = |k: &str| attr(k).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(0.0);
        match name.as_str() {
            "path" => out.extend(path(attr("d").unwrap_or(""))),
            "circle" => out.push(ellipse(num("cx"), num("cy"), num("r"), num("r"))),
            "ellipse" => out.push(ellipse(num("cx"), num("cy"), num("rx"), num("ry"))),
            "line" => out.push(Poly { pts: vec![pos2(num("x1"), num("y1")), pos2(num("x2"), num("y2"))], closed: false, fill: false }),
            "rect" => {
                let (x, y, w, h) = (num("x"), num("y"), num("width"), num("height"));
                let r = num("rx").max(num("ry"));
                let pts = rough::rounded_outline(Rect::from_min_size(pos2(x, y), vec2(w, h)), r, 1.0);
                out.push(Poly { pts, closed: true, fill: true });
            }
            "polyline" | "polygon" => {
                let v = numbers(attr("points").unwrap_or(""));
                let pts: Vec<Pos2> = v.chunks_exact(2).map(|c| pos2(c[0], c[1])).collect();
                out.push(Poly { pts, closed: name == "polygon", fill: false });
            }
            _ => {}
        }
    }
    out.retain(|p| p.pts.len() >= 2 && p.pts.iter().all(|q| q.is_finite()));
    out
}

fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32) -> Poly {
    let n = 28;
    let pts = (0..n).map(|i| i as f32 / n as f32 * std::f32::consts::TAU).map(|a| pos2(cx + rx * a.cos(), cy + ry * a.sin())).collect();
    Poly { pts, closed: true, fill: true }
}

/// Numbers in an SVG attribute (`1.5-2` is two numbers, `.5.5` too).
fn numbers(s: &str) -> Vec<f32> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<f32>| {
        if let Ok(v) = cur.parse() {
            out.push(v);
        }
        cur.clear();
    };
    for c in s.chars() {
        match c {
            '0'..='9' => cur.push(c),
            '.' if cur.contains('.') => {
                flush(&mut cur, &mut out);
                cur.push('.');
            }
            '.' => cur.push(c),
            '-' | '+' if !(cur.ends_with('e') || cur.ends_with('E')) => {
                flush(&mut cur, &mut out);
                cur.push(c);
            }
            'e' | 'E' | '-' | '+' => cur.push(c),
            _ => flush(&mut cur, &mut out),
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// Arc numbers: rx ry rotation large-arc sweep x y, with the two flags possibly glued together.
fn arc_numbers(s: &str) -> Vec<f32> {
    let mut out = Vec::new();
    let mut rest = s.trim_start_matches([' ', ',']);
    let mut field = 0usize;
    while !rest.is_empty() {
        if field % 7 == 3 || field % 7 == 4 {
            let mut chars = rest.chars();
            match chars.next() {
                Some(c @ ('0' | '1')) => {
                    out.push(if c == '1' { 1.0 } else { 0.0 });
                    rest = chars.as_str().trim_start_matches([' ', ',']);
                    field += 1;
                    continue;
                }
                _ => break,
            }
        }
        // One number.
        let mut end = 0;
        let mut seen_dot = false;
        for (i, c) in rest.char_indices() {
            let ok = c.is_ascii_digit() || (c == '.' && !seen_dot) || ((c == '-' || c == '+') && i == 0);
            if c == '.' {
                seen_dot = true;
            }
            if !ok {
                break;
            }
            end = i + c.len_utf8();
        }
        if end == 0 {
            break;
        }
        match rest.get(..end).and_then(|n| n.parse::<f32>().ok()) {
            Some(v) => out.push(v),
            None => break,
        }
        rest = rest.get(end..).unwrap_or("").trim_start_matches([' ', ',']);
        field += 1;
    }
    out
}

fn path(d: &str) -> Vec<Poly> {
    // Split into (command, numbers) groups; arcs get their own number parser.
    let mut groups: Vec<(char, Vec<f32>)> = Vec::new();
    let mut start = None;
    let mut cmd = ' ';
    for (i, c) in d.char_indices().chain(std::iter::once((d.len(), 'M'))) {
        if c.is_ascii_alphabetic() && c != 'e' && c != 'E' {
            if let Some(s) = start {
                let body = d.get(s..i).unwrap_or("");
                let nums = if matches!(cmd, 'a' | 'A') { arc_numbers(body) } else { numbers(body) };
                groups.push((cmd, nums));
            }
            cmd = c;
            start = Some(i + c.len_utf8());
        }
    }
    let mut polys = Vec::new();
    let mut cur: Vec<Pos2> = Vec::new();
    let mut p = Pos2::ZERO;
    let mut sub_start = Pos2::ZERO;
    let mut last_ctrl: Option<Pos2> = None;
    let mut last_q: Option<Pos2> = None;
    let finish = |cur: &mut Vec<Pos2>, polys: &mut Vec<Poly>, closed: bool| {
        if cur.len() >= 2 {
            polys.push(Poly { pts: std::mem::take(cur), closed, fill: false });
        } else {
            cur.clear();
        }
    };
    for (c, nums) in groups {
        let rel = c.is_ascii_lowercase();
        let base = |p: Pos2| if rel { p.to_vec2() } else { Vec2::ZERO };
        let arity = match c.to_ascii_uppercase() {
            'M' | 'L' | 'T' => 2,
            'H' | 'V' => 1,
            'C' => 6,
            'S' | 'Q' => 4,
            'A' => 7,
            _ => 0,
        };
        if arity == 0 {
            // Z: close the subpath.
            if !cur.is_empty() {
                cur.push(sub_start);
                finish(&mut cur, &mut polys, true);
            }
            p = sub_start;
            continue;
        }
        for (k, a) in nums.chunks_exact(arity).enumerate() {
            let o = base(p);
            let (mut ctrl, mut q) = (None, None);
            match c.to_ascii_uppercase() {
                'M' => {
                    let np = pos2(a[0], a[1]) + o;
                    if k == 0 {
                        finish(&mut cur, &mut polys, false);
                        cur.push(np);
                        sub_start = np;
                    } else {
                        cur.push(np);
                    }
                    p = np;
                }
                'L' => {
                    p = pos2(a[0], a[1]) + o;
                    cur.push(p);
                }
                'H' => {
                    p = pos2(a[0] + if rel { p.x } else { 0.0 }, p.y);
                    cur.push(p);
                }
                'V' => {
                    p = pos2(p.x, a[0] + if rel { p.y } else { 0.0 });
                    cur.push(p);
                }
                'C' | 'S' => {
                    let (c1, c2, e) = if c.eq_ignore_ascii_case(&'C') {
                        (pos2(a[0], a[1]) + o, pos2(a[2], a[3]) + o, pos2(a[4], a[5]) + o)
                    } else {
                        let c1 = last_ctrl.map_or(p, |lc| p + (p - lc));
                        (c1, pos2(a[0], a[1]) + o, pos2(a[2], a[3]) + o)
                    };
                    cubic(&mut cur, p, c1, c2, e);
                    ctrl = Some(c2);
                    p = e;
                }
                'Q' | 'T' => {
                    let (c1, e) = if c.eq_ignore_ascii_case(&'Q') {
                        (pos2(a[0], a[1]) + o, pos2(a[2], a[3]) + o)
                    } else {
                        (last_q.map_or(p, |lq| p + (p - lq)), pos2(a[0], a[1]) + o)
                    };
                    let (c1c, c2c) = (p + (c1 - p) * (2.0 / 3.0), e + (c1 - e) * (2.0 / 3.0));
                    cubic(&mut cur, p, c1c, c2c, e);
                    q = Some(c1);
                    p = e;
                }
                'A' => {
                    let e = pos2(a[5], a[6]) + o;
                    arc(&mut cur, p, a[0], a[1], a[2], a[3] != 0.0, a[4] != 0.0, e);
                    p = e;
                }
                _ => {}
            }
            last_ctrl = ctrl;
            last_q = q;
        }
    }
    finish(&mut cur, &mut polys, false);
    polys
}

fn cubic(out: &mut Vec<Pos2>, p0: Pos2, p1: Pos2, p2: Pos2, p3: Pos2) {
    let n = 10;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let u = 1.0 - t;
        let v = p0.to_vec2() * (u * u * u) + p1.to_vec2() * (3.0 * u * u * t) + p2.to_vec2() * (3.0 * u * t * t) + p3.to_vec2() * (t * t * t);
        out.push(v.to_pos2());
    }
}

/// An SVG elliptical arc (endpoint parameterisation, SVG 1.1 appendix F.6) as points.
#[allow(clippy::too_many_arguments)]
fn arc(out: &mut Vec<Pos2>, p0: Pos2, rx: f32, ry: f32, rot_deg: f32, large: bool, sweep: bool, p1: Pos2) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx < 1e-4 || ry < 1e-4 || (p0 - p1).length() < 1e-4 {
        out.push(p1);
        return;
    }
    let phi = rot_deg.to_radians();
    let (s, c) = phi.sin_cos();
    let d = (p0 - p1) / 2.0;
    let x1 = c * d.x + s * d.y;
    let y1 = -s * d.x + c * d.y;
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut k = if den > 0.0 { (num / den).max(0.0).sqrt() } else { 0.0 };
    if large == sweep {
        k = -k;
    }
    let cx1 = k * rx * y1 / ry;
    let cy1 = -k * ry * x1 / rx;
    let mid = (p0.to_vec2() + p1.to_vec2()) / 2.0;
    let cx = c * cx1 - s * cy1 + mid.x;
    let cy = s * cx1 + c * cy1 + mid.y;
    let angle = |ux: f32, uy: f32, vx: f32, vy: f32| {
        let a = (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
        if a.is_finite() { a } else { 0.0 }
    };
    let t1 = angle(1.0, 0.0, (x1 - cx1) / rx, (y1 - cy1) / ry);
    let mut dt = angle((x1 - cx1) / rx, (y1 - cy1) / ry, (-x1 - cx1) / rx, (-y1 - cy1) / ry);
    if !sweep && dt > 0.0 {
        dt -= std::f32::consts::TAU;
    } else if sweep && dt < 0.0 {
        dt += std::f32::consts::TAU;
    }
    let n = ((dt.abs() / 0.35).ceil() as usize).clamp(2, 48);
    for i in 1..=n {
        let t = t1 + dt * i as f32 / n as f32;
        let (st, ct) = t.sin_cos();
        out.push(pos2(cx + rx * ct * c - ry * st * s, cy + rx * ct * s + ry * st * c));
    }
}

/// The colours icons are drawn in.
#[derive(Clone, Copy, Debug)]
pub struct IconInk {
    pub ink: Color32,
    pub wash: Color32,
    pub card: Color32,
}

/// Draw icon `name` in `rect`: a marker line in a calm version of `tint`, with a pale wash in
/// closed round shapes. Returns `false` for unknown icons. `frame` is the boil frame.
pub fn paint(painter: &Painter, rect: Rect, name: &str, tint: Color32, colours: IconInk, frame: u64) -> bool {
    let Some(polys) = library().get(name) else { return false };
    if !rect.is_finite() || rect.width() < 2.0 {
        return true;
    }
    // Strong accents (pink, blue) read as noise in a toolbar: pull them toward the ink.
    let calm = calm(tint, colours.ink);
    let s = rect.width() / 24.0;
    let width = (1.9 * s).clamp(1.1, 3.0);
    let seed = name.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    let map = |p: Pos2| rect.min + p.to_vec2() * s;
    for (i, poly) in polys.iter().enumerate() {
        let pts: Vec<Pos2> = poly.pts.iter().map(|&p| map(p)).collect();
        let extent = pts.iter().fold(Rect::NOTHING, |r, &p| r.union(Rect::from_center_size(p, Vec2::ZERO)));
        let size = extent.width().max(extent.height());
        // Tiny parts (dots) are just dots: wobbling them would fold them over themselves, and
        // egui's mitred joins turn folded corners into long spikes.
        if size < 2.5 {
            painter.circle_filled(extent.center(), (width * 0.5 + size * 0.5).max(1.0), calm);
            continue;
        }
        let amp = (0.55 * s.min(1.2)).min(size * 0.06);
        let mut pts = dedup(rough::jitter_line(&pts, seed ^ i as u64, frame, amp), 0.4);
        // A closed outline that repeats its first point would close on a zero-length edge.
        if poly.closed && pts.len() > 2 && pts.first().zip(pts.last()).is_some_and(|(a, b)| (*a - *b).length() < 0.8) {
            pts.pop();
        }
        if pts.len() < 2 {
            continue;
        }
        if poly.fill && poly.closed && pts.len() >= 3 {
            painter.add(Shape::convex_polygon(pts.clone(), colours.wash, Stroke::NONE));
        }
        let stroke = Stroke::new(width, calm);
        if poly.closed {
            painter.add(Shape::closed_line(pts.clone(), stroke));
        } else {
            painter.add(Shape::line(pts.clone(), stroke));
            // Round caps.
            for end in [pts.first(), pts.last()].into_iter().flatten() {
                painter.circle_filled(*end, width / 2.0, calm);
            }
        }
    }
    let _ = colours.card;
    true
}

/// `pts` without points closer than `min` to the one before (zero-length edges give egui's
/// joins nothing to aim along).
fn dedup(pts: Vec<Pos2>, min: f32) -> Vec<Pos2> {
    let mut out: Vec<Pos2> = Vec::with_capacity(pts.len());
    for p in pts {
        if out.last().is_none_or(|q| (p - *q).length() >= min) {
            out.push(p);
        }
    }
    out
}

/// `tint`, desaturated toward `ink` when it is a loud accent (keeps light tints on dark buttons).
pub fn calm(tint: Color32, ink: Color32) -> Color32 {
    let (r, g, b) = (f32::from(tint.r()), f32::from(tint.g()), f32::from(tint.b()));
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let sat = if max > 0.0 { (max - min) / max } else { 0.0 };
    if sat > 0.35 { mix(tint, ink, 0.55) } else { tint }
}

/// The painter to install with `photocraft_ui_egui::icons::set_painter`; it reads the boil frame
/// and colours from `state` each time it draws.
pub fn painter(state: Arc<std::sync::Mutex<(u64, IconInk)>>) -> photocraft_ui_egui::icons::IconPainter {
    Arc::new(move |p: &Painter, rect: Rect, name: &str, tint: Color32| {
        let (frame, colours) = *state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        paint(p, rect, name, tint, colours, frame)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_lucide_icon_parses_inside_its_box() {
        let lib = library();
        assert!(lib.len() >= 100);
        for (name, polys) in lib {
            assert!(!polys.is_empty(), "{name} parsed to nothing");
            for p in polys {
                assert!(p.pts.iter().all(|q| (-2.0..=26.0).contains(&q.x) && (-2.0..=26.0).contains(&q.y)), "{name} strays out of its box: {:?}", p.pts);
            }
        }
    }

    #[test]
    fn path_commands_and_numbers() {
        assert_eq!(numbers("1.5-2.5.5,3e1"), vec![1.5, -2.5, 0.5, 30.0]);
        assert_eq!(arc_numbers("1 1 0 01.5 2"), vec![1.0, 1.0, 0.0, 0.0, 1.0, 0.5, 2.0]);
        let p = path("M2 2h4v4H2z");
        assert_eq!(p.len(), 1);
        assert!(p[0].closed);
        assert_eq!(p[0].pts, vec![pos2(2.0, 2.0), pos2(6.0, 2.0), pos2(6.0, 6.0), pos2(2.0, 6.0), pos2(2.0, 2.0)]);
        // A half circle arc ends where it should.
        let a = path("M4 12a8 8 0 0 1 16 0");
        let last = *a[0].pts.last().unwrap();
        assert!((last - pos2(20.0, 12.0)).length() < 0.01);
        assert!(a[0].pts.iter().any(|q| q.y < 5.0), "bulges upward");
        // Garbage doesn't panic.
        let _ = path("M a z Q 1 L");
        let _ = parse("<svg><path d=\"M1 1 A 0 0 0 1 1 NaN 3\"/><circle r=\"x\"/></svg>");
    }

    #[test]
    fn loud_tints_are_calmed_and_greys_kept() {
        let ink = Color32::from_rgb(23, 22, 28);
        assert_eq!(calm(Color32::from_gray(80), ink), Color32::from_gray(80));
        let pink = calm(Color32::from_rgb(255, 46, 136), ink);
        assert!(pink.r() < 160, "{pink:?}");
    }
}

#[cfg(test)]
mod paint_tests {
    use super::*;

    /// Every icon's drawing stays inside its button (a stray point would streak across panels).
    #[test]
    fn painted_icons_stay_in_their_box() {
        let ctx = egui::Context::default();
        let colours = IconInk { ink: Color32::BLACK, wash: Color32::LIGHT_BLUE, card: Color32::WHITE };
        let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(16.0, 16.0));
        for name in photocraft_ui_egui::icons::names() {
            let painter = egui::Painter::new(ctx.clone(), egui::LayerId::debug(), Rect::EVERYTHING);
            let layer = egui::LayerId::debug();
            ctx.graphics_mut(|g| {
                let l = g.entry(layer);
                for i in 0..l.next_idx().0 {
                    l.mutate_shape(egui::layers::ShapeIdx(i), |s| s.shape = Shape::Noop);
                }
            });
            for frame in 0..3 {
                assert!(paint(&painter, rect, name, Color32::BLACK, colours, frame));
            }
            let bounds = ctx.graphics(|g| g.get(layer).map(|l| l.all_entries().fold(Rect::NOTHING, |r, s| r.union(s.shape.visual_bounding_rect()))));
            let b = bounds.unwrap_or(Rect::NOTHING);
            assert!(rect.expand(4.0).contains_rect(b), "{name}: {b:?}");
        }
    }
}
