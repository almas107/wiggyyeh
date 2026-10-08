//! Game juice for the whole window, applied to the frame's shapes after everything is drawn:
//!
//! - every window, menu, popup and tooltip PhotoCraft opens pops in with a springy scale;
//! - big actions (clear, delete, merge) shake the screen;
//! - paint splats and confetti burst out on fills, undo, saving and other celebrations.
//!
//! Reduce motion turns all of it off.

use std::collections::HashMap;

use egui::emath::TSTransform;
use egui::{Color32, Context, Id, LayerId, Order, Pos2, Rect, Vec2, pos2, vec2};

use crate::rough;

/// Seconds a window takes to pop in.
const POP_SECONDS: f64 = 0.22;
/// Seconds a shake lasts.
const SHAKE_SECONDS: f64 = 0.32;
/// Most particles alive at once.
const MAX_PARTICLES: usize = 400;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Particle {
    pos: Pos2,
    vel: Vec2,
    born: f64,
    life: f64,
    size: f32,
    colour: Color32,
    seed: u64,
    /// Confetti flutters (a rectangle that spins); splats are round and fall.
    confetti: bool,
}

/// Ease out with a little overshoot.
pub fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let (c1, c3) = (1.6, 2.6);
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

#[derive(Default)]
pub struct Juice {
    pub reduce_motion: bool,
    /// When each open layer first appeared.
    seen: HashMap<LayerId, f64>,
    shake: Option<(f64, f32)>,
    particles: Vec<Particle>,
    rng: u64,
}

impl Juice {
    fn rand(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Shake the screen by up to `strength` points.
    pub fn shake(&mut self, now: f64, strength: f32) {
        if !self.reduce_motion {
            self.shake = Some((now, strength.clamp(0.0, 20.0)));
        }
    }

    /// A burst of paint splats in `colours` at `at`.
    pub fn splat(&mut self, now: f64, at: Pos2, colours: &[Color32], count: usize) {
        self.burst(now, at, colours, count, false);
    }

    /// Confetti from `at`.
    pub fn confetti(&mut self, now: f64, at: Pos2, colours: &[Color32], count: usize) {
        self.burst(now, at, colours, count, true);
    }

    fn burst(&mut self, now: f64, at: Pos2, colours: &[Color32], count: usize, confetti: bool) {
        if self.reduce_motion || colours.is_empty() || !at.is_finite() {
            return;
        }
        for i in 0..count.min(MAX_PARTICLES) {
            let a = self.rand() * std::f32::consts::TAU;
            let speed = if confetti { 220.0 + 380.0 * self.rand() } else { 80.0 + 220.0 * self.rand() };
            let up = if confetti { -260.0 } else { -60.0 };
            let colour = colours.get(i % colours.len()).copied().unwrap_or(Color32::WHITE);
            let p = Particle {
                pos: at,
                vel: Vec2::angled(a) * speed + vec2(0.0, up),
                born: now,
                life: 0.6 + 0.6 * f64::from(self.rand()),
                size: if confetti { 4.0 + 4.0 * self.rand() } else { 3.0 + 6.0 * self.rand() },
                colour,
                seed: self.rng,
                confetti,
            };
            self.particles.push(p);
        }
        let extra = self.particles.len().saturating_sub(MAX_PARTICLES);
        self.particles.drain(..extra);
    }

    /// Is anything still moving (so the caller keeps frames coming)?
    pub fn busy(&self, now: f64) -> bool {
        !self.particles.is_empty() || self.shake.is_some_and(|(t, _)| now - t < SHAKE_SECONDS) || self.seen.values().any(|t| now - t < POP_SECONDS)
    }

    /// Apply pops and shake to this frame's shapes and draw the particles. `skip` names layers
    /// with their own entrance (WobbleWorks' colour card).
    pub fn apply(&mut self, ctx: &Context, now: f64, skip: &[Id]) {
        let layers: Vec<LayerId> = ctx.memory(|m| m.layer_ids().collect());
        // Forget closed windows, so they pop again when reopened.
        self.seen.retain(|l, _| layers.contains(l));
        let shake = match self.shake {
            Some((t, s)) if now - t < SHAKE_SECONDS && !self.reduce_motion => {
                let k = 1.0 - ((now - t) / SHAKE_SECONDS) as f32;
                let f = (now * 60.0) as u64;
                Some(vec2(rough::hash(f, 1), rough::hash(f, 2)) * s * k * k)
            }
            _ => {
                self.shake = None;
                None
            }
        };
        let mut pops = Vec::new();
        for &layer in &layers {
            let popping = matches!(layer.order, Order::Middle | Order::Foreground | Order::Tooltip) && !skip.contains(&layer.id);
            if popping {
                let first = *self.seen.entry(layer).or_insert(now);
                let age = now - first;
                if age < POP_SECONDS && !self.reduce_motion {
                    pops.push((layer, ease_out_back((age / POP_SECONDS) as f32)));
                }
            }
        }
        if shake.is_none() && pops.is_empty() {
            self.particles_draw(ctx, now);
            return;
        }
        ctx.graphics_mut(|g| {
            for (layer, e) in &pops {
                let Some(list) = g.get_mut(*layer) else { continue };
                let bounds = list.all_entries().fold(Rect::NOTHING, |r, s| r.union(s.shape.visual_bounding_rect()));
                if !bounds.is_finite() || bounds.width() <= 0.0 {
                    continue;
                }
                let scale = 0.82 + 0.18 * e;
                let c = bounds.center().to_vec2();
                list.transform(TSTransform::from_translation(c) * TSTransform::from_scaling(scale) * TSTransform::from_translation(-c));
            }
            if let Some(off) = shake {
                for &layer in &layers {
                    if let Some(list) = g.get_mut(layer) {
                        list.transform(TSTransform::from_translation(off));
                    }
                }
            }
        });
        if !pops.is_empty() || shake.is_some() {
            ctx.request_repaint();
        }
        self.particles_draw(ctx, now);
    }

    fn particles_draw(&mut self, ctx: &Context, now: f64) {
        self.particles.retain(|p| now - p.born < p.life);
        if self.particles.is_empty() {
            return;
        }
        let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("wobble-particles")));
        let gravity = 900.0;
        for p in &self.particles {
            let t = (now - p.born) as f32;
            let k = 1.0 - (now - p.born) as f32 / p.life as f32;
            let drag = if p.confetti { 0.55 } else { 0.85 };
            let pos = p.pos + p.vel * t * drag + vec2(0.0, 0.5 * gravity * t * t * if p.confetti { 0.35 } else { 1.0 });
            let alpha = (k * 1.6).clamp(0.0, 1.0);
            let colour = p.colour.gamma_multiply(alpha);
            if p.confetti {
                let a = t * 9.0 + (p.seed % 7) as f32;
                let d = Vec2::angled(a) * p.size;
                let n = Vec2::angled(a + 1.57) * p.size * 0.45 * (t * 13.0).cos().abs().max(0.2);
                painter.add(egui::Shape::convex_polygon(vec![pos - d - n, pos + d - n, pos + d + n, pos - d + n], colour, egui::Stroke::NONE));
            } else {
                rough::blob(
                    &painter,
                    pos,
                    p.size * (0.6 + 0.4 * k),
                    &rough::Paint { fill: colour, ink: egui::Stroke::NONE, shadow: None, radius: 0.0, wobble: 0.0 },
                    p.seed,
                    (now * 8.0) as u64,
                );
            }
        }
        ctx.request_repaint();
    }

    /// How many particles are alive (for tests).
    pub fn particle_count(&self) -> usize {
        self.particles.len()
    }
}

/// A point to burst from when there's no pointer: the middle of `rect`.
pub fn centre_or(pointer: Option<Pos2>, rect: Rect) -> Pos2 {
    pointer.filter(|p| p.is_finite()).unwrap_or_else(|| if rect.is_finite() { rect.center() } else { pos2(200.0, 200.0) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bursts_live_then_go_and_reduce_motion_stops_everything() {
        let mut j = Juice::default();
        j.splat(0.0, pos2(10.0, 10.0), &[Color32::RED], 20);
        j.confetti(0.0, pos2(10.0, 10.0), &[Color32::RED, Color32::BLUE], 10_000);
        assert_eq!(j.particle_count(), MAX_PARTICLES);
        assert!(j.busy(0.1));
        let ctx = Context::default();
        ctx.run_ui(egui::RawInput::default(), |_| {
            j.apply(&ctx, 5.0, &[]);
        })
        .textures_delta
        .clear();
        assert_eq!(j.particle_count(), 0, "all gone after their life");
        j.reduce_motion = true;
        j.splat(0.0, pos2(1.0, 1.0), &[Color32::RED], 5);
        j.shake(0.0, 8.0);
        j.splat(0.0, pos2(f32::NAN, 1.0), &[Color32::RED], 5);
        assert_eq!(j.particle_count(), 0);
        assert!(!j.busy(0.0));
    }

    #[test]
    fn windows_pop_in_with_an_overshoot() {
        assert!(ease_out_back(0.0).abs() < 1e-5);
        assert!((ease_out_back(1.0) - 1.0).abs() < 1e-5);
        assert!((1..10).any(|i| ease_out_back(i as f32 / 10.0) > 1.0));
        let mut j = Juice::default();
        let ctx = Context::default();
        let mut first = None;
        for (i, now) in [0.0, 0.05, 0.5].into_iter().enumerate() {
            ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::Window::new("pop").show(ui.ctx(), |ui| ui.label("hi"));
                j.apply(&ctx, now, &[]);
            })
            .textures_delta
            .clear();
            if i == 0 {
                first = j.seen.values().next().copied();
            }
        }
        assert_eq!(first, Some(0.0), "the window was seen opening");
        assert!(!j.busy(0.5));
    }
}
