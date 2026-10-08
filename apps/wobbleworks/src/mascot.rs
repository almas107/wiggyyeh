//! Wob, the mascot: a little hand-drawn blob on the corner of the paper that reacts to what you
//! do. It idles (breathing, eyes following the pointer), cheers (first stroke, saving,
//! exporting), looks surprised on undo, falls asleep when you leave it alone, and says something
//! cheerful in a speech bubble. Poke it for a boing.

use egui::{Color32, Context, Id, LayerId, Order, Pos2, Rect, Sense, Shape, Stroke, Vec2, pos2, vec2};

use crate::pixfont;
use crate::rough::{self, Paint};
use crate::widgets::Look;

/// Seconds without input before Wob nods off.
pub const SLEEP_AFTER: f64 = 30.0;
/// How long a reaction lasts.
const REACT_SECONDS: f64 = 1.4;
/// How long a speech bubble stays.
const SAY_SECONDS: f64 = 2.6;
/// Wob's size (points).
pub const SIZE: f32 = 58.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    Idle,
    Cheer,
    Surprised,
    Sleepy,
}

/// Things Wob reacts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    FirstStroke,
    Saved,
    Exported,
    Undo,
    Poked,
    BigAction,
}

pub struct Mascot {
    pub enabled: bool,
    pub reduce_motion: bool,
    mood: Mood,
    mood_until: f64,
    said: Option<(String, f64)>,
    last_input: f64,
    jump_at: f64,
    lines: u64,
}

impl Default for Mascot {
    fn default() -> Self {
        Self { enabled: true, reduce_motion: false, mood: Mood::Idle, mood_until: 0.0, said: None, last_input: 0.0, jump_at: f64::NEG_INFINITY, lines: 0 }
    }
}

const FIRST: &[&str] = &["Ooh, a line!", "Wiggly! I love it.", "Look at it boil!"];
const SAVED: &[&str] = &["Saved! Safe & sound.", "Tucked away!", "Saved. Nice one!"];
const EXPORTED: &[&str] = &["Ta-da!", "Off it goes!", "Frame it!"];
const UNDO: &[&str] = &["Whoops!", "Never happened.", "Poof!"];
const POKED: &[&str] = &["Hee hee!", "Boing!", "Keep drawing!", "I'm Wob!", "Wiggle wiggle."];
const BIG: &[&str] = &["Whoa!", "Big change!"];

impl Mascot {
    pub fn mood(&self, now: f64) -> Mood {
        if now < self.mood_until {
            self.mood
        } else if now - self.last_input > SLEEP_AFTER {
            Mood::Sleepy
        } else {
            Mood::Idle
        }
    }

    /// What Wob is saying right now.
    pub fn saying(&self, now: f64) -> Option<&str> {
        self.said.as_ref().filter(|(_, until)| now < *until).map(|(s, _)| s.as_str())
    }

    /// Someone did something: Wob wakes up.
    pub fn input(&mut self, now: f64) {
        self.last_input = now;
    }

    fn say(&mut self, now: f64, lines: &[&str]) {
        self.lines = self.lines.wrapping_add(1);
        let i = (self.lines as usize).wrapping_mul(7) % lines.len().max(1);
        if let Some(l) = lines.get(i) {
            self.said = Some(((*l).to_string(), now + SAY_SECONDS));
        }
    }

    pub fn react(&mut self, now: f64, e: Event) {
        self.last_input = now;
        let (mood, lines): (Mood, &[&str]) = match e {
            Event::FirstStroke => (Mood::Cheer, FIRST),
            Event::Saved => (Mood::Cheer, SAVED),
            Event::Exported => (Mood::Cheer, EXPORTED),
            Event::Undo => (Mood::Surprised, UNDO),
            Event::Poked => (Mood::Cheer, POKED),
            Event::BigAction => (Mood::Surprised, BIG),
        };
        self.mood = mood;
        self.mood_until = now + REACT_SECONDS;
        self.jump_at = now;
        self.say(now, lines);
    }

    /// Draw Wob with its feet at `foot` (bottom-right corner of the paper). Returns whether it was
    /// poked this frame.
    pub fn show(&mut self, ctx: &Context, look: &Look, now: f64, foot: Pos2) -> bool {
        if !self.enabled || !foot.is_finite() {
            return false;
        }
        let mood = self.mood(now);
        let motion = !self.reduce_motion;
        // Breathing, and a hop when reacting.
        let breathe = if motion { (now * if mood == Mood::Sleepy { 1.6 } else { 3.0 }).sin() as f32 } else { 0.0 };
        let since = (now - self.jump_at) as f32;
        let hop = if motion && since < 0.6 { (since / 0.6 * std::f32::consts::PI).sin() * 16.0 } else { 0.0 };
        let w = SIZE * (1.0 + 0.03 * breathe);
        let h = SIZE * 0.86 * (1.0 - 0.03 * breathe);
        let centre = foot - vec2(0.0, h / 2.0 + hop);
        let body = Rect::from_center_size(centre, vec2(w, h));
        let mut poked = false;
        egui::Area::new(Id::new("wobble-mascot")).order(Order::Foreground).fixed_pos(body.min - vec2(30.0, 30.0)).constrain(false).interactable(true).show(
            ctx,
            |ui| {
                let (_, resp) = ui.allocate_exact_size(body.size() + vec2(60.0, 60.0), Sense::click());
                if resp.clicked() {
                    poked = true;
                }
                resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            },
        );
        let p = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("wobble-mascot-paint")));
        let frame = look.frame;
        // Shadow on the paper.
        p.add(Shape::ellipse_filled(foot + vec2(3.0, 2.0), vec2(w * 0.45 * (1.0 - hop / 40.0), 5.0), look.t.shadow.gamma_multiply(0.35)));
        // The body: a lumpy blob with a soft belly.
        let fill = if mood == Mood::Sleepy { crate::theme::mix(look.t.cool, look.t.paper, 0.35) } else { look.t.cool };
        let outline: Vec<Pos2> = (0..48).map(|i| centre + Vec2::angled(i as f32 / 48.0 * std::f32::consts::TAU) * vec2(w / 2.0, h / 2.0)).collect();
        let edge = rough::wobble(&outline, centre, 0xb0b, frame, 1.6);
        p.add(Shape::convex_polygon(edge.clone(), fill, Stroke::NONE));
        p.add(Shape::ellipse_filled(centre + vec2(0.0, h * 0.18), vec2(w * 0.28, h * 0.2), crate::theme::mix(fill, Color32::WHITE, 0.45)));
        p.add(Shape::closed_line(edge, look.ink(2.5)));
        // Arms: up when cheering.
        let arm_up = matches!(mood, Mood::Cheer);
        for side in [-1.0f32, 1.0] {
            let shoulder = centre + vec2(side * w * 0.45, h * 0.05);
            let hand = shoulder + if arm_up { vec2(side * 12.0, -18.0) } else { vec2(side * 9.0, 8.0) };
            rough::line(&p, &rough::segment(shoulder, hand, 4), look.ink(2.5), 0xa77 ^ side.to_bits() as u64, frame, 0.6);
        }
        // Eyes: follow the pointer; wide when surprised; shut when sleepy.
        let pointer = ctx.pointer_hover_pos().filter(|q| q.is_finite());
        // Blink every few seconds; when the pointer is still, glance around.
        let blink = motion && (now % 4.3) < 0.12 && mood != Mood::Surprised;
        let still = ctx.input(|i| i.pointer.time_since_last_movement()) > 3.0;
        let glance = Vec2::angled((now * 0.7).sin() as f32 * 2.2) * 2.4;
        for side in [-1.0f32, 1.0] {
            let eye = centre + vec2(side * w * 0.17, -h * 0.12);
            match mood {
                _ if blink => {
                    rough::line(&p, &rough::segment(eye - vec2(5.0, 0.0), eye + vec2(5.0, 0.0), 3), look.ink(2.0), 0xb11c, frame, 0.3);
                }
                Mood::Sleepy => {
                    rough::line(&p, &rough::segment(eye - vec2(5.0, 0.0), eye + vec2(5.0, 0.0), 3), look.ink(2.0), 0xe7e, frame, 0.4);
                }
                _ => {
                    let r = if mood == Mood::Surprised { 7.5 } else { 6.0 };
                    p.add(Shape::circle_filled(eye, r, Color32::WHITE));
                    p.add(Shape::circle_stroke(eye, r, look.ink(1.8)));
                    let look_at = match pointer {
                        Some(q) if !(still && motion) => (q - eye).normalized() * (r * 0.4),
                        _ if motion => glance,
                        _ => Vec2::ZERO,
                    };
                    let look_at = if look_at.is_finite() { look_at } else { Vec2::ZERO };
                    p.add(Shape::circle_filled(eye + look_at, r * 0.45, look.t.ink));
                    p.add(Shape::circle_filled(eye + look_at - vec2(1.2, 1.2), 1.1, Color32::WHITE));
                }
            }
        }
        // Mouth.
        let mouth = centre + vec2(0.0, h * 0.12);
        match mood {
            Mood::Surprised => {
                p.add(Shape::ellipse_filled(mouth, vec2(4.5, 6.0), look.t.ink));
            }
            Mood::Cheer => {
                let pts: Vec<Pos2> = (0..=10).map(|i| mouth + Vec2::angled(i as f32 / 10.0 * std::f32::consts::PI) * vec2(9.0, 8.0) - vec2(0.0, 2.0)).collect();
                p.add(Shape::convex_polygon(pts, look.t.ink, Stroke::NONE));
            }
            _ => {
                let pts: Vec<Pos2> = (0..=8).map(|i| mouth + Vec2::angled(0.35 + i as f32 / 8.0 * 2.4) * 6.0 - vec2(0.0, 4.0)).collect();
                p.add(Shape::line(pts, look.ink(2.0)));
            }
        }
        // Cheeks.
        for side in [-1.0f32, 1.0] {
            p.add(Shape::circle_filled(centre + vec2(side * w * 0.3, h * 0.05), 3.5, look.t.hot.gamma_multiply(0.35)));
        }
        // Zzz, or a speech bubble.
        let text = match (mood, self.saying(now)) {
            (_, Some(s)) => Some(s.to_string()),
            (Mood::Sleepy, None) => Some(if ((now * 1.5) as u64).is_multiple_of(2) { "z z" } else { "z Z z" }.to_string()),
            _ => None,
        };
        if let Some(text) = text {
            let size = pixfont::size(&text, 2.0) + vec2(20.0, 16.0);
            let mut r = Rect::from_min_size(pos2(centre.x - size.x + 10.0, body.min.y - size.y - 14.0), size);
            let screen = ctx.content_rect();
            if r.min.x < screen.min.x + 4.0 {
                r = r.translate(vec2(screen.min.x + 4.0 - r.min.x, 0.0));
            }
            let style = Paint { fill: look.t.card, ink: look.ink(2.0), shadow: Some((vec2(3.0, 3.0), look.t.shadow)), radius: 10.0, wobble: 1.0 };
            rough::boxed(&p, r, &style, 0xb0bb1e, frame);
            let tail = [pos2(r.max.x - 22.0, r.max.y - 1.0), pos2(r.max.x - 10.0, r.max.y - 1.0), pos2(centre.x - 4.0, body.min.y - 4.0)];
            p.add(Shape::convex_polygon(tail.to_vec(), look.t.card, Stroke::NONE));
            p.add(Shape::line(vec![tail[0], tail[2], tail[1]], look.ink(2.0)));
            pixfont::paint_centered(&p, r.center(), &text, 2.0, look.t.ink, None);
        }
        poked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moods_follow_events_and_idleness() {
        let mut m = Mascot::default();
        assert_eq!(m.mood(1.0), Mood::Idle);
        m.react(1.0, Event::Undo);
        assert_eq!(m.mood(1.5), Mood::Surprised);
        assert!(UNDO.contains(&m.saying(1.5).unwrap()));
        assert_eq!(m.mood(3.0), Mood::Idle);
        assert!(m.saying(5.0).is_none());
        assert_eq!(m.mood(1.0 + SLEEP_AFTER + 1.0), Mood::Sleepy);
        m.input(40.0);
        assert_eq!(m.mood(41.0), Mood::Idle);
        m.react(50.0, Event::Saved);
        assert_eq!(m.mood(50.1), Mood::Cheer);
    }

    #[test]
    fn draws_without_panicking_anywhere() {
        let ctx = Context::default();
        let look = Look { t: crate::theme::BUBBLEGUM, frame: 1 };
        let mut m = Mascot::default();
        for (now, foot) in [(0.0, pos2(300.0, 300.0)), (100.0, pos2(-50.0, 5.0)), (2.0, pos2(f32::NAN, 1.0))] {
            m.react(now, Event::Poked);
            ctx.run_ui(egui::RawInput::default(), |_| {
                m.show(&ctx, &look, now, foot);
            })
            .textures_delta
            .clear();
        }
    }
}
