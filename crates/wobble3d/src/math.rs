//! Small 3D math: vectors and quaternions (f32). Every operation is total: normalising a zero
//! vector gives a zero vector, never NaN.

use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub const fn v3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

impl Vec3 {
    pub const ZERO: Vec3 = v3(0.0, 0.0, 0.0);
    pub const X: Vec3 = v3(1.0, 0.0, 0.0);
    pub const Y: Vec3 = v3(0.0, 1.0, 0.0);
    pub const Z: Vec3 = v3(0.0, 0.0, 1.0);

    pub fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn cross(self, o: Vec3) -> Vec3 {
        v3(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }
    pub fn length_sq(self) -> f32 {
        self.dot(self)
    }
    pub fn distance(self, o: Vec3) -> f32 {
        (self - o).length()
    }
    /// Unit vector, or zero for a (near) zero or non-finite vector.
    pub fn normalized(self) -> Vec3 {
        let l = self.length();
        if l > 1e-12 && l.is_finite() { self / l } else { Vec3::ZERO }
    }
    pub fn lerp(self, o: Vec3, t: f32) -> Vec3 {
        self + (o - self) * t
    }
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
    pub fn mul_elem(self, o: Vec3) -> Vec3 {
        v3(self.x * o.x, self.y * o.y, self.z * o.z)
    }
    pub fn min(self, o: Vec3) -> Vec3 {
        v3(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }
    pub fn max(self, o: Vec3) -> Vec3 {
        v3(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }
    /// Any unit vector perpendicular to this one.
    pub fn any_perpendicular(self) -> Vec3 {
        let a = if self.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
        self.cross(a).normalized()
    }
    pub fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }
    pub fn from_slice(s: &[f64]) -> Option<Vec3> {
        let v = v3(*s.first()? as f32, *s.get(1)? as f32, *s.get(2)? as f32);
        v.is_finite().then_some(v)
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        v3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl AddAssign for Vec3 {
    fn add_assign(&mut self, o: Vec3) {
        *self = *self + o;
    }
}
impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        v3(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl SubAssign for Vec3 {
    fn sub_assign(&mut self, o: Vec3) {
        *self = *self - o;
    }
}
impl Mul<f32> for Vec3 {
    type Output = Vec3;
    fn mul(self, s: f32) -> Vec3 {
        v3(self.x * s, self.y * s, self.z * s)
    }
}
impl Div<f32> for Vec3 {
    type Output = Vec3;
    fn div(self, s: f32) -> Vec3 {
        if s == 0.0 { Vec3::ZERO } else { v3(self.x / s, self.y / s, self.z / s) }
    }
}
impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        v3(-self.x, -self.y, -self.z)
    }
}

/// A rotation. Always kept normalised by the constructors.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Default for Quat {
    fn default() -> Self {
        Quat::IDENTITY
    }
}

impl Quat {
    pub const IDENTITY: Quat = Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 };

    /// Rotation by `angle` radians about `axis` (identity for a zero axis).
    pub fn from_axis_angle(axis: Vec3, angle: f32) -> Quat {
        let a = axis.normalized();
        if a == Vec3::ZERO || !angle.is_finite() {
            return Quat::IDENTITY;
        }
        let (s, c) = (angle * 0.5).sin_cos();
        Quat { x: a.x * s, y: a.y * s, z: a.z * s, w: c }
    }

    /// The shortest rotation taking direction `from` to direction `to`.
    pub fn from_to(from: Vec3, to: Vec3) -> Quat {
        let (f, t) = (from.normalized(), to.normalized());
        if f == Vec3::ZERO || t == Vec3::ZERO {
            return Quat::IDENTITY;
        }
        let d = f.dot(t).clamp(-1.0, 1.0);
        if d > 0.999_999 {
            return Quat::IDENTITY;
        }
        if d < -0.999_999 {
            return Quat::from_axis_angle(f.any_perpendicular(), std::f32::consts::PI);
        }
        Quat::from_axis_angle(f.cross(t), d.acos())
    }

    pub fn normalized(self) -> Quat {
        let l = (self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w).sqrt();
        if l > 1e-12 && l.is_finite() {
            Quat { x: self.x / l, y: self.y / l, z: self.z / l, w: self.w / l }
        } else {
            Quat::IDENTITY
        }
    }

    pub fn conjugate(self) -> Quat {
        Quat { x: -self.x, y: -self.y, z: -self.z, w: self.w }
    }

    pub fn rotate(self, v: Vec3) -> Vec3 {
        let q = v3(self.x, self.y, self.z);
        let t = q.cross(v) * 2.0;
        v + t * self.w + q.cross(t)
    }
}

impl Mul for Quat {
    type Output = Quat;
    /// `a * b` applies `b` first, then `a`.
    fn mul(self, b: Quat) -> Quat {
        let a = self;
        Quat {
            w: a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z,
            x: a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y,
            y: a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x,
            z: a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w,
        }
        .normalized()
    }
}

/// Position, rotation and per-axis scale. Applied as scale, then rotate, then translate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Xform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Xform {
    fn default() -> Self {
        Xform { translation: Vec3::ZERO, rotation: Quat::IDENTITY, scale: v3(1.0, 1.0, 1.0) }
    }
}

impl Xform {
    pub fn apply(&self, p: Vec3) -> Vec3 {
        self.rotation.rotate(p.mul_elem(self.scale)) + self.translation
    }
    /// Directions (normals) under this transform, renormalised.
    pub fn apply_normal(&self, n: Vec3) -> Vec3 {
        let s = self.scale;
        let inv = v3(safe_recip(s.x), safe_recip(s.y), safe_recip(s.z));
        self.rotation.rotate(n.mul_elem(inv)).normalized()
    }
}

fn safe_recip(x: f32) -> f32 {
    if x.abs() < 1e-12 { 0.0 } else { 1.0 / x }
}

/// Ray / triangle intersection (Möller–Trumbore), both sides. Returns the distance along the
/// ray and the barycentric (u, v) of the hit.
pub fn ray_triangle(origin: Vec3, dir: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<(f32, f32, f32)> {
    let e1 = b - a;
    let e2 = c - a;
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - a;
    let u = s.dot(p) * inv;
    if !(-1e-6..=1.0 + 1e-6).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = dir.dot(q) * inv;
    if v < -1e-6 || u + v > 1.0 + 1e-6 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 1e-6 && t.is_finite()).then_some((t, u, v))
}

/// Ray / plane intersection distance (None when parallel or behind the origin).
pub fn ray_plane(origin: Vec3, dir: Vec3, point: Vec3, normal: Vec3) -> Option<f32> {
    let d = dir.dot(normal);
    if d.abs() < 1e-9 {
        return None;
    }
    let t = (point - origin).dot(normal) / d;
    (t > 0.0 && t.is_finite()).then_some(t)
}

/// Ray / axis-aligned box: true when the ray passes through it.
pub fn ray_hits_box(origin: Vec3, dir: Vec3, lo: Vec3, hi: Vec3) -> bool {
    let mut tmin = f32::NEG_INFINITY;
    let mut tmax = f32::INFINITY;
    for (o, d, l, h) in [(origin.x, dir.x, lo.x, hi.x), (origin.y, dir.y, lo.y, hi.y), (origin.z, dir.z, lo.z, hi.z)] {
        if d.abs() < 1e-12 {
            if o < l || o > h {
                return false;
            }
        } else {
            let (a, b) = ((l - o) / d, (h - o) / d);
            tmin = tmin.max(a.min(b));
            tmax = tmax.min(a.max(b));
        }
    }
    tmax >= tmin.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-4
    }

    #[test]
    fn quaternions_rotate_and_compose() {
        let q = Quat::from_axis_angle(Vec3::Y, std::f32::consts::FRAC_PI_2);
        assert!(close(q.rotate(Vec3::X), v3(0.0, 0.0, -1.0)));
        let r = Quat::from_to(Vec3::X, Vec3::Z);
        assert!(close(r.rotate(Vec3::X), Vec3::Z));
        let back = Quat::from_to(Vec3::X, -Vec3::X);
        assert!(close(back.rotate(Vec3::X), -Vec3::X));
        assert!(close((q * q.conjugate()).rotate(Vec3::Y), Vec3::Y));
    }

    #[test]
    fn degenerate_input_is_harmless() {
        assert_eq!(Vec3::ZERO.normalized(), Vec3::ZERO);
        assert_eq!(v3(f32::NAN, 0.0, 0.0).normalized(), Vec3::ZERO);
        assert_eq!(Quat::from_axis_angle(Vec3::ZERO, 1.0), Quat::IDENTITY);
        assert_eq!(Vec3::X / 0.0, Vec3::ZERO);
        assert!(Vec3::from_slice(&[1.0]).is_none());
        assert!(Vec3::from_slice(&[1.0, f64::NAN, 2.0]).is_none());
    }

    #[test]
    fn rays_hit_triangles_and_planes() {
        let hit = ray_triangle(v3(0.2, 0.2, 5.0), -Vec3::Z, Vec3::ZERO, Vec3::X, Vec3::Y);
        assert!(hit.is_some_and(|(t, _, _)| (t - 5.0).abs() < 1e-5));
        assert!(ray_triangle(v3(2.0, 2.0, 5.0), -Vec3::Z, Vec3::ZERO, Vec3::X, Vec3::Y).is_none());
        assert_eq!(ray_plane(v3(0.0, 3.0, 0.0), -Vec3::Y, Vec3::ZERO, Vec3::Y), Some(3.0));
        assert!(ray_hits_box(v3(0.0, 0.0, 5.0), -Vec3::Z, v3(-1.0, -1.0, -1.0), v3(1.0, 1.0, 1.0)));
        assert!(!ray_hits_box(v3(3.0, 0.0, 5.0), -Vec3::Z, v3(-1.0, -1.0, -1.0), v3(1.0, 1.0, 1.0)));
    }

    #[test]
    fn xform_applies_scale_rotate_translate() {
        let x = Xform { translation: v3(1.0, 0.0, 0.0), rotation: Quat::from_axis_angle(Vec3::Z, std::f32::consts::FRAC_PI_2), scale: v3(2.0, 2.0, 2.0) };
        assert!(close(x.apply(Vec3::X), v3(1.0, 2.0, 0.0)));
    }
}
