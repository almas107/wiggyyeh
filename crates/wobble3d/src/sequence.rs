//! Camera shots between each other: the camera at any point of a sequence (Feather's Sequence:
//! play flies between the saved shots, and the timeline scrubs through them), and a small
//! picture of each shot for the shot strip.

use crate::camera::{Camera, Viewport};
use crate::model::Sequence;

/// Thumbnails are at most this many pixels on a side.
pub const THUMB_MAX: u32 = 512;

/// The camera between shots at `t` (in shot-to-shot legs: 1.5 is halfway from the second shot
/// to the third), eased; `None` with fewer than two shots.
pub fn shot_camera(seq: &Sequence, t: f64, viewport: Viewport) -> Option<Camera> {
    let n = seq.shots.len();
    if n < 2 || !t.is_finite() {
        return None;
    }
    let t = t.clamp(0.0, (n - 1) as f64);
    let i = (t.floor() as usize).min(n - 2);
    let f = (t - i as f64).clamp(0.0, 1.0) as f32;
    let e = f * f * (3.0 - 2.0 * f);
    let (a, b) = (seq.shots.get(i)?.camera, seq.shots.get(i + 1)?.camera);
    let lerp = |x: f32, y: f32| x + (y - x) * e;
    // Angles take the short way round (a leg from +170° to -170° crosses ±180).
    let lerp_angle = |x: f32, y: f32| {
        let mut d = y - x;
        if d > 180.0 {
            d -= 360.0;
        } else if d < -180.0 {
            d += 360.0;
        }
        x + d * e
    };
    Some(Camera {
        target: a.target.lerp(b.target, e),
        yaw: lerp_angle(a.yaw, b.yaw),
        twist: crate::camera::wrap_degrees(lerp_angle(a.twist, b.twist)),
        pitch: lerp(a.pitch, b.pitch),
        distance: lerp(a.distance, b.distance),
        focal_mm: lerp(a.focal_mm, b.focal_mm),
        orthographic: if e < 0.5 { a.orthographic } else { b.orthographic },
        snapped_from_perspective: false,
        viewport,
    })
}

/// The whole sequence's length in legs (shots − 1), 0 with fewer than two shots.
pub fn legs(seq: &Sequence) -> f64 {
    seq.shots.len().saturating_sub(1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Shot;

    fn seq(yaws: &[f32]) -> Sequence {
        let mut s = Sequence::default();
        for (i, y) in yaws.iter().enumerate() {
            let c = Camera { yaw: *y, ..Camera::default() };
            s.shots.push(Shot { id: i as u64 + 1, name: format!("Shot {}", i + 1), camera: c });
        }
        s
    }

    #[test]
    fn the_camera_flies_between_shots_the_short_way() {
        let vp = Viewport { width: 100.0, height: 50.0 };
        assert!(shot_camera(&seq(&[0.0]), 0.0, vp).is_none(), "one shot is not a sequence");
        let s = seq(&[170.0, -170.0, 0.0]);
        let mid = shot_camera(&s, 0.5, vp).expect("mid");
        assert!((mid.yaw - 180.0).abs() < 1e-3, "across ±180, not the long way: {}", mid.yaw);
        assert_eq!(mid.viewport.width, 100.0);
        let end = shot_camera(&s, 99.0, vp).expect("clamped");
        assert!(end.yaw.abs() < 1e-3);
        assert!(shot_camera(&s, f64::NAN, vp).is_none());
        assert_eq!(legs(&s), 2.0);
        assert_eq!(legs(&Sequence::default()), 0.0);
    }

    #[test]
    fn twist_flies_between_shots_the_short_way_too() {
        let vp = Viewport { width: 100.0, height: 50.0 };
        let mut s = seq(&[0.0, 0.0]);
        s.shots[0].camera.twist = 170.0;
        s.shots[1].camera.twist = -170.0;
        let mid = shot_camera(&s, 0.5, vp).expect("mid");
        assert!((mid.twist.abs() - 180.0).abs() < 1e-3, "across ±180: {}", mid.twist);
        let quarter = shot_camera(&s, 0.25, vp).expect("quarter");
        assert!(quarter.twist > 170.0 || quarter.twist < -179.0, "{}", quarter.twist);
    }
}
