//! The turntable camera, as in Feather: one-finger swipe orbits about the orbit point, pinch
//! zooms, two-finger swipe pans, a double tap snaps to the nearest "perfect view" (front, back,
//! left, right, top, bottom; orthographic), a three-finger double tap toggles perspective and
//! orthographic, and a three-finger swipe changes the field of view (10–500 mm lens).
//!
//! Y is up. Screen coordinates are pixels with the origin at the top left of the viewport.

use serde::{Deserialize, Serialize};

use crate::math::{Vec3, v3};

/// Lens range, millimetres on a 35 mm frame (Feather's FOV slider).
pub const FOCAL_MIN: f32 = 10.0;
pub const FOCAL_MAX: f32 = 500.0;
/// Vertical size of the 35 mm frame the focal length is measured against.
const FRAME_MM: f32 = 24.0;
pub const DISTANCE_MIN: f32 = 0.02;
pub const DISTANCE_MAX: f32 = 10_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport { width: 1280.0, height: 800.0 }
    }
}

impl Viewport {
    /// A usable viewport: at least 1 × 1 and finite.
    pub fn sane(self) -> Viewport {
        let f = |v: f32| if v.is_finite() { v.clamp(1.0, 16384.0) } else { 1.0 };
        Viewport { width: f(self.width), height: f(self.height) }
    }
}

/// The named views a double tap snaps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PerfectView {
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
}

impl PerfectView {
    pub const ALL: [PerfectView; 6] = [PerfectView::Front, PerfectView::Back, PerfectView::Left, PerfectView::Right, PerfectView::Top, PerfectView::Bottom];

    /// (yaw, pitch) in degrees.
    pub fn angles(self) -> (f32, f32) {
        match self {
            PerfectView::Front => (0.0, 0.0),
            PerfectView::Back => (180.0, 0.0),
            PerfectView::Right => (90.0, 0.0),
            PerfectView::Left => (-90.0, 0.0),
            PerfectView::Top => (0.0, 90.0),
            PerfectView::Bottom => (0.0, -90.0),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            PerfectView::Front => "front",
            PerfectView::Back => "back",
            PerfectView::Left => "left",
            PerfectView::Right => "right",
            PerfectView::Top => "top",
            PerfectView::Bottom => "bottom",
        }
    }

    pub fn parse(s: &str) -> Option<PerfectView> {
        PerfectView::ALL.into_iter().find(|v| v.name().eq_ignore_ascii_case(s))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    /// The orbit point.
    pub target: Vec3,
    /// Degrees about the world up axis.
    pub yaw: f32,
    /// Degrees above the horizon, -90..=90.
    pub pitch: f32,
    /// Distance from the eye to the orbit point.
    pub distance: f32,
    /// Lens focal length in millimetres (FOCAL_MIN..=FOCAL_MAX).
    pub focal_mm: f32,
    pub orthographic: bool,
    /// The view was snapped to a perfect view; orbiting away restores perspective when the
    /// camera was in perspective before the snap.
    #[serde(default)]
    pub snapped_from_perspective: bool,
    #[serde(default)]
    pub viewport: Viewport,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            target: Vec3::ZERO,
            yaw: 30.0,
            pitch: 18.0,
            distance: 8.0,
            focal_mm: 50.0,
            orthographic: false,
            snapped_from_perspective: false,
            viewport: Viewport::default(),
        }
    }
}

/// The camera's frame for one moment: everything projection needs.
#[derive(Debug, Clone, Copy)]
pub struct View {
    pub eye: Vec3,
    /// Unit vector from the target towards the eye (the camera looks along `-back`).
    pub back: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    /// 1 / tan(fov / 2).
    pub f: f32,
    pub aspect: f32,
    pub orthographic: bool,
    /// Half the view's height in world units at the target (orthographic size).
    pub half_height: f32,
    pub viewport: Viewport,
    pub near: f32,
}

/// A point projected to the screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projected {
    pub x: f32,
    pub y: f32,
    /// Distance in front of the camera along the view axis.
    pub depth: f32,
    /// Screen pixels per world unit at this depth.
    pub scale: f32,
}

impl Camera {
    pub fn fov_y(&self) -> f32 {
        2.0 * (FRAME_MM / (2.0 * self.focal_mm.clamp(FOCAL_MIN, FOCAL_MAX))).atan()
    }

    /// Clamp everything into range and replace non-finite values, so no input can wreck the view.
    pub fn sanitize(&mut self) {
        let d = Camera::default();
        if !self.target.is_finite() {
            self.target = d.target;
        }
        self.yaw = if self.yaw.is_finite() { self.yaw.rem_euclid(360.0) } else { d.yaw };
        if self.yaw > 180.0 {
            self.yaw -= 360.0;
        }
        self.pitch = if self.pitch.is_finite() { self.pitch.clamp(-90.0, 90.0) } else { d.pitch };
        self.distance = if self.distance.is_finite() { self.distance.clamp(DISTANCE_MIN, DISTANCE_MAX) } else { d.distance };
        self.focal_mm = if self.focal_mm.is_finite() { self.focal_mm.clamp(FOCAL_MIN, FOCAL_MAX) } else { d.focal_mm };
        self.viewport = self.viewport.sane();
    }

    pub fn view(&self) -> View {
        let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
        let back = v3(pitch.cos() * yaw.sin(), pitch.sin(), pitch.cos() * yaw.cos()).normalized();
        let right = v3(yaw.cos(), 0.0, -yaw.sin()).normalized();
        let up = back.cross(right).normalized();
        let fov = self.fov_y();
        let f = 1.0 / (fov * 0.5).tan();
        let vp = self.viewport.sane();
        View {
            eye: self.target + back * self.distance,
            back,
            right,
            up,
            f,
            aspect: vp.width / vp.height,
            orthographic: self.orthographic,
            half_height: self.distance / f,
            viewport: vp,
            near: (self.distance * 0.002).max(1e-4),
        }
    }

    /// Orbit by a screen drag of (dx, dy) pixels, turntable style.
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        if !(dx.is_finite() && dy.is_finite()) {
            return;
        }
        self.yaw -= dx * 0.35;
        self.pitch += dy * 0.35;
        if self.snapped_from_perspective && (dx != 0.0 || dy != 0.0) {
            self.orthographic = false;
            self.snapped_from_perspective = false;
        }
        self.sanitize();
    }

    /// Pan by a screen drag: the scene follows the pointer.
    pub fn pan(&mut self, dx: f32, dy: f32) {
        if !(dx.is_finite() && dy.is_finite()) {
            return;
        }
        let v = self.view();
        let per_px = 2.0 * v.half_height / v.viewport.height;
        self.target = self.target - v.right * (dx * per_px) + v.up * (dy * per_px);
        self.sanitize();
    }

    /// Zoom by a factor (> 1 zooms in).
    pub fn zoom(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.distance /= factor;
        }
        self.sanitize();
    }

    /// Change the lens, keeping what's at the orbit point the same size (a dolly zoom).
    pub fn set_focal(&mut self, mm: f32) {
        if !mm.is_finite() {
            return;
        }
        let before = self.view().half_height;
        self.focal_mm = mm.clamp(FOCAL_MIN, FOCAL_MAX);
        let f = 1.0 / (self.fov_y() * 0.5).tan();
        self.distance = before * f;
        self.sanitize();
    }

    /// The perfect view nearest to the current direction.
    pub fn nearest_perfect_view(&self) -> PerfectView {
        let back = self.view().back;
        let mut best = (PerfectView::Front, f32::NEG_INFINITY);
        for pv in PerfectView::ALL {
            let mut c = *self;
            (c.yaw, c.pitch) = pv.angles();
            let d = c.view().back.dot(back);
            if d > best.1 {
                best = (pv, d);
            }
        }
        best.0
    }

    /// Snap to a perfect view, orthographic. Orbiting away returns to perspective if the camera
    /// was in perspective.
    pub fn snap(&mut self, pv: PerfectView) {
        (self.yaw, self.pitch) = pv.angles();
        if !self.orthographic {
            self.snapped_from_perspective = true;
        }
        self.orthographic = true;
    }

    pub fn toggle_projection(&mut self) {
        self.orthographic = !self.orthographic;
        self.snapped_from_perspective = false;
    }

    /// Move the orbit point to `p` without moving the eye.
    pub fn set_orbit_point(&mut self, p: Vec3) {
        if !p.is_finite() {
            return;
        }
        let eye = self.view().eye;
        let d = eye - p;
        let len = d.length();
        if len < DISTANCE_MIN {
            self.target = p;
            return;
        }
        let back = d / len;
        self.target = p;
        self.distance = len;
        self.pitch = back.y.clamp(-1.0, 1.0).asin().to_degrees();
        if back.x.abs() + back.z.abs() > 1e-6 {
            self.yaw = back.x.atan2(back.z).to_degrees();
        }
        self.sanitize();
    }

    /// Frame a bounding box (all of it visible).
    pub fn frame(&mut self, lo: Vec3, hi: Vec3) {
        if !(lo.is_finite() && hi.is_finite()) {
            return;
        }
        self.target = (lo + hi) * 0.5;
        let radius = ((hi - lo).length() * 0.5).max(0.25);
        let v = self.view();
        let fit = v.f.min(v.f * v.aspect);
        self.distance = radius * fit * 1.15 + radius;
        self.sanitize();
    }
}

impl View {
    /// Project a world point. None when it is behind the near plane.
    pub fn project(&self, p: Vec3) -> Option<Projected> {
        let rel = p - self.eye;
        let x = rel.dot(self.right);
        let y = rel.dot(self.up);
        let depth = -rel.dot(self.back);
        let (w, h) = (self.viewport.width, self.viewport.height);
        if self.orthographic {
            let scale = h * 0.5 / self.half_height;
            return Some(Projected { x: w * 0.5 + x * scale, y: h * 0.5 - y * scale, depth, scale });
        }
        if depth < self.near {
            return None;
        }
        let scale = self.f * h * 0.5 / depth;
        Some(Projected { x: w * 0.5 + x * scale, y: h * 0.5 - y * scale, depth, scale })
    }

    /// The ray through a screen point: (origin, unit direction).
    pub fn ray(&self, sx: f32, sy: f32) -> (Vec3, Vec3) {
        let (w, h) = (self.viewport.width, self.viewport.height);
        let nx = (sx - w * 0.5) / (h * 0.5);
        let ny = (h * 0.5 - sy) / (h * 0.5);
        if self.orthographic {
            // Start well behind the eye so geometry behind an orthographic eye is still hit.
            let origin = self.eye + self.right * (nx * self.half_height) + self.up * (ny * self.half_height) + self.back * (self.half_height * 50.0);
            return (origin, -self.back);
        }
        let dir = (-self.back + self.right * (nx / self.f) + self.up * (ny / self.f)).normalized();
        (self.eye, dir)
    }

    /// The point on the plane through `anchor` facing the camera, under a screen point.
    pub fn unproject_at(&self, sx: f32, sy: f32, anchor: Vec3) -> Vec3 {
        let (o, d) = self.ray(sx, sy);
        match crate::math::ray_plane(o, d, anchor, self.back) {
            Some(t) => o + d * t,
            None => anchor,
        }
    }

    /// The direction the camera looks.
    pub fn forward(&self) -> Vec3 {
        -self.back
    }

    /// World units per screen pixel at a depth.
    pub fn world_per_px(&self, depth: f32) -> f32 {
        if self.orthographic { 2.0 * self.half_height / self.viewport.height } else { 2.0 * depth.max(self.near) / (self.f * self.viewport.height) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_and_unprojects_consistently() {
        for ortho in [false, true] {
            let mut c = Camera { orthographic: ortho, ..Camera::default() };
            c.sanitize();
            let v = c.view();
            let p = v3(0.7, -0.3, 0.4);
            let s = v.project(p).expect("in front");
            let back = v.unproject_at(s.x, s.y, p);
            assert!(back.distance(p) < 1e-3, "{ortho}: {back:?}");
            // The target is at the centre of the screen.
            let t = v.project(c.target).expect("target");
            assert!((t.x - 640.0).abs() < 1e-3 && (t.y - 400.0).abs() < 1e-3);
        }
    }

    #[test]
    fn perfect_views_snap_and_release() {
        let mut c = Camera { yaw: 80.0, pitch: 10.0, ..Camera::default() };
        assert_eq!(c.nearest_perfect_view(), PerfectView::Right);
        c.snap(PerfectView::Right);
        assert!(c.orthographic);
        c.orbit(10.0, 0.0);
        assert!(!c.orthographic, "orbiting away restores perspective");
        c.yaw = 5.0;
        c.pitch = 80.0;
        assert_eq!(c.nearest_perfect_view(), PerfectView::Top);
        c.snap(PerfectView::Top);
        let v = c.view();
        assert!(v.up.is_finite() && v.right.is_finite());
        assert!(v.project(Vec3::ZERO).is_some());
    }

    #[test]
    fn hostile_numbers_keep_the_camera_usable() {
        let mut c = Camera::default();
        c.orbit(f32::NAN, 1.0);
        c.pan(f32::INFINITY, 0.0);
        c.zoom(0.0);
        c.zoom(-3.0);
        c.zoom(1e30);
        c.set_focal(f32::NAN);
        c.set_focal(1e9);
        c.set_orbit_point(v3(f32::NAN, 0.0, 0.0));
        c.viewport = Viewport { width: f32::NAN, height: -5.0 };
        c.sanitize();
        let v = c.view();
        assert!(v.eye.is_finite() && v.f.is_finite() && v.half_height.is_finite());
        assert_eq!(c.focal_mm, FOCAL_MAX);
    }

    #[test]
    fn dolly_zoom_keeps_the_target_size() {
        let mut c = Camera::default();
        let before = c.view().half_height;
        c.set_focal(200.0);
        assert!((c.view().half_height - before).abs() < 1e-3);
        assert!(c.distance > 8.0);
    }

    #[test]
    fn orbit_point_moves_without_moving_the_eye() {
        let mut c = Camera::default();
        let eye = c.view().eye;
        c.set_orbit_point(v3(1.0, 0.5, -0.5));
        assert!(c.view().eye.distance(eye) < 1e-3);
    }
}
