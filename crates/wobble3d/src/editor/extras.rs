//! Smaller drawing aids asked for by Feather users (YouTube comments on its tutorials): guides
//! on the axis planes without drawing one, guides made from curves already drawn, stroke ends
//! that snap to the ends of other curves, and Feather's focus mode (the active group as a black
//! silhouette over the rest in grey, as its tutorial shows).

use std::sync::Arc;

use serde_json::{Value, json};

use super::{Editor, Event};
use crate::guide::{Guide, Primitive};
use crate::math::{Quat, Vec3};
use crate::model::{Point, ResourceState};

/// How close (screen pixels) a stroke's end must come to another curve's end to snap to it.
pub const SNAP_PX: f32 = 10.0;

impl Editor {
    /// A flat guide through the orbit point across an axis (`x`: the YZ plane, `y`: the floor,
    /// `z`: the XY wall) or facing the view: no guide drawing needed for flat work.
    pub fn axis_plane(&mut self, axis: &str) -> Result<u64, String> {
        let view = self.view();
        let normal = match axis.to_ascii_lowercase().as_str() {
            "x" | "yz" => Vec3::X,
            "y" | "xz" | "floor" | "ground" => Vec3::Y,
            "z" | "xy" | "wall" => Vec3::Z,
            "view" | "screen" => view.back,
            other => return Err(format!("unknown plane {other:?} (x, y, z or view)")),
        };
        let gid = self.scene.alloc_id();
        let mut g = Guide::primitive(gid, Primitive::Plane, 8)?;
        g.name = format!("{} plane {gid}", axis.to_ascii_uppercase());
        // The plane primitive lies flat (normal +Y), 4 m wide: turn it and make it 12 m.
        g.xform.rotation = Quat::from_to(Vec3::Y, normal);
        g.xform.scale = crate::math::v3(3.0, 3.0, 3.0);
        g.xform.translation = self.camera.target;
        if let Some(prev) = self.scene.active_guide() {
            g.opacity = prev.opacity;
        }
        g.rebuild()?;
        self.checkpoint("Axis plane guide");
        self.replace_active_guide();
        self.scene.guides.push(Arc::new(g));
        self.scene.set_guide_state(gid, ResourceState::Active);
        self.events.push(Event::GuideMade);
        self.status = "Draw on the plane (it goes through the orbit point)".into();
        self.changed();
        Ok(gid)
    }

    /// A guide extruded from a curve already drawn (the selected one, or `id`), along the view:
    /// as if the curve had been drawn with the guide tool from here. No redrawing.
    pub fn guide_from_curve(&mut self, id: Option<u64>) -> Result<u64, String> {
        let sid = id.or_else(|| {
            let mut v: Vec<u64> = self.selection.iter().copied().collect();
            v.sort_unstable();
            v.first().copied()
        });
        let sid = sid.ok_or("select a curve to make a guide from")?;
        let st = self.scene.stroke(sid).cloned().ok_or("that curve is gone")?;
        if st.is_fill() || st.points.len() < 2 {
            return Err("pick a curve (not a fill) to make a guide from".into());
        }
        let view = self.view();
        let screen: Vec<(f32, f32)> = st.points.iter().filter_map(|p| view.project(p.p).map(|q| (q.x, q.y))).collect();
        if screen.len() < 2 {
            return Err("the curve is not in view".into());
        }
        let gid = self.scene.alloc_id();
        let mut g = Guide::drawn(gid, &view, &screen, st.centre())?;
        if let Some(prev) = self.scene.active_guide() {
            g.opacity = prev.opacity;
        }
        self.checkpoint("Guide from curve");
        self.replace_active_guide();
        self.scene.guides.push(Arc::new(g));
        self.scene.set_guide_state(gid, ResourceState::Active);
        self.events.push(Event::GuideMade);
        self.status = "Guide made from the curve: draw on it, bend it, or close it".into();
        self.changed();
        Ok(gid)
    }

    /// Pull a new curve's first and last points onto the nearest end of another visible curve
    /// within [`SNAP_PX`] on screen, so outlines close and lines meet. Only ends that are near
    /// in 3D too: an end that merely lines up on screen with a point far in front or behind
    /// would yank the curve's end out of its plane into a spike.
    pub(super) fn snap_curve_ends(&self, pts: &mut [Point]) {
        if !self.snap_ends || pts.len() < 2 {
            return;
        }
        let view = self.view();
        let ends: Vec<(f32, f32, Point)> = self
            .scene
            .strokes
            .iter()
            .filter(|s| !s.is_fill() && self.scene.group_shown(s.group))
            .flat_map(|s| [s.points.first().copied(), s.points.last().copied()])
            .flatten()
            .filter_map(|p| view.project(p.p).map(|q| (q.x, q.y, p)))
            .collect();
        if ends.is_empty() {
            return;
        }
        let last = pts.len() - 1;
        for i in [0, last] {
            let Some(q) = pts.get(i).and_then(|p| view.project(p.p)) else { continue };
            let reach = view.world_per_px(q.depth) * SNAP_PX * 2.0;
            let best = ends
                .iter()
                .map(|(x, y, p)| (((x - q.x).powi(2) + (y - q.y).powi(2)).sqrt(), p))
                .filter(|(d, _)| *d <= SNAP_PX)
                .filter(|(_, p)| pts.get(i).is_some_and(|own| (own.p - p.p).length() <= reach))
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let (Some((_, target)), Some(slot)) = (best, pts.get_mut(i)) {
                slot.p = target.p;
                if slot.n == Vec3::ZERO {
                    slot.n = target.n;
                }
            }
        }
    }

    /// Shift the selected curves' colours in hue (degrees), saturation and brightness (−1..1)
    /// and their opacity, each by the same amount: "brighten everything painted" keeps the
    /// differences between colours, as Feather's tutorial does to tune a scene's tone.
    pub fn adjust_colour(&mut self, hue: f32, sat: f32, bright: f32, opacity: f32) -> Result<usize, String> {
        if ![hue, sat, bright, opacity].iter().all(|v| v.is_finite()) {
            return Err("hue, saturation, brightness and opacity must be numbers".into());
        }
        if self.selection.is_empty() {
            return Err("select the curves to adjust first".into());
        }
        self.checkpoint("Adjust colour");
        let mut n = 0;
        for st in self.scene.strokes.iter_mut().filter(|s| self.selection.contains(&s.id)) {
            let m = Arc::make_mut(st);
            let (h, s, v) = m.brush.color.to_hsv();
            let a = m.brush.color.0[3];
            m.brush.color = crate::model::Rgba::from_hsv(h + hue, (s + sat).clamp(0.0, 1.0), (v + bright).clamp(0.0, 1.0), a);
            m.brush.opacity = (m.brush.opacity + opacity).clamp(0.0, 1.0);
            n += 1;
        }
        self.changed();
        Ok(n)
    }

    /// The extra commands (dispatched from [`Editor::run`]).
    pub(super) fn run_extras(&mut self, cmd: &str, p: &Value) -> Option<Result<Value, String>> {
        let out = match cmd {
            "guide.plane" => {
                let axis = p.get("axis").and_then(Value::as_str).unwrap_or("y");
                self.axis_plane(axis).map(|id| json!(id))
            }
            "guide.fromCurve" => {
                let id = p.get("id").and_then(Value::as_u64);
                self.guide_from_curve(id).map(|id| json!(id))
            }
            "edit.adjustColor" => {
                let g = |k: &str| p.get(k).and_then(Value::as_f64).map_or(0.0, |v| v as f32);
                self.adjust_colour(g("hue"), g("saturation"), g("brightness"), g("opacity")).map(|n| json!(n))
            }
            "view.focus" => {
                self.focus = p.get("on").and_then(Value::as_bool).unwrap_or(!self.focus);
                self.touch();
                Ok(json!(self.focus))
            }
            "snap.set" => match p.get("ends").and_then(Value::as_bool) {
                Some(v) => {
                    self.snap_ends = v;
                    self.touch();
                    Ok(json!(v))
                }
                None => Err("ends must be true or false".into()),
            },
            _ => return None,
        };
        Some(out)
    }
}
