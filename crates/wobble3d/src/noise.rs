//! Deterministic hash noise: the same seed always wiggles the same way, on every platform.

/// A well-mixed 32-bit hash (lowbias32).
pub fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

pub fn hash2(a: u32, b: u32) -> u32 {
    hash(a ^ hash(b).wrapping_add(0x9e37_79b9))
}

/// Uniform in [0, 1).
pub fn unit(seed: u32) -> f32 {
    (hash(seed) >> 8) as f32 / (1u32 << 24) as f32
}

/// Uniform in [-1, 1).
pub fn signed(seed: u32) -> f32 {
    unit(seed) * 2.0 - 1.0
}

/// `floor` without the libm call (exact for |x| < 2³¹; beyond that the noise just goes flat).
#[inline]
fn ffloor(x: f32) -> f32 {
    let i = x as i32 as f32;
    if i > x { i - 1.0 } else { i }
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Smooth 1D value noise in [-1, 1].
pub fn value1(seed: u32, x: f32) -> f32 {
    if !x.is_finite() {
        return 0.0;
    }
    let xf = ffloor(x);
    let i = xf as i64 as u32;
    let t = smooth(x - xf);
    let a = signed(hash2(seed, i));
    let b = signed(hash2(seed, i.wrapping_add(1)));
    a + (b - a) * t
}

/// Smooth 2D value noise in [-1, 1].
pub fn value2(seed: u32, x: f32, y: f32) -> f32 {
    if !(x.is_finite() && y.is_finite()) {
        return 0.0;
    }
    let (xf, yf) = (ffloor(x), ffloor(y));
    let (i, j) = (xf as i64 as u32, yf as i64 as u32);
    let (tx, ty) = (smooth(x - xf), smooth(y - yf));
    let h = |a: u32, b: u32| signed(hash2(hash2(seed, a), b));
    let top = h(i, j) + (h(i.wrapping_add(1), j) - h(i, j)) * tx;
    let bot = h(i, j.wrapping_add(1)) + (h(i.wrapping_add(1), j.wrapping_add(1)) - h(i, j.wrapping_add(1))) * tx;
    top + (bot - top) * ty
}

/// Two octaves of 1D noise.
pub fn fbm1(seed: u32, x: f32) -> f32 {
    (value1(seed, x) * 0.7 + value1(seed ^ 0x5bd1_e995, x * 2.3) * 0.3).clamp(-1.0, 1.0)
}

/// Two octaves of 1D noise along a line, sampled at slowly increasing positions: the lattice
/// values of the current cells are kept, so most samples cost no hashing. Same values as
/// [`fbm1`].
#[derive(Debug, Clone, Copy)]
pub struct Track {
    seed: u32,
    cells: [(i64, f32, f32); 2],
}

impl Track {
    pub fn new(seed: u32) -> Track {
        Track { seed, cells: [(i64::MIN, 0.0, 0.0); 2] }
    }

    fn octave(&mut self, k: usize, seed: u32, x: f32) -> f32 {
        let xf = ffloor(x);
        let i = xf as i64;
        let Some(cell) = self.cells.get_mut(k) else { return 0.0 };
        if cell.0 != i {
            let iu = i as u32;
            *cell = (i, signed(hash2(seed, iu)), signed(hash2(seed, iu.wrapping_add(1))));
        }
        let t = smooth(x - xf);
        cell.1 + (cell.2 - cell.1) * t
    }

    /// `fbm1(seed, x)`.
    pub fn at(&mut self, x: f32) -> f32 {
        if !x.is_finite() {
            return 0.0;
        }
        let s = self.seed;
        (self.octave(0, s, x) * 0.7 + self.octave(1, s ^ 0x5bd1_e995, x * 2.3) * 0.3).clamp(-1.0, 1.0)
    }
}

/// Periodic 1D value noise: repeats every `period` units (period ≥ 1, rounded).
pub fn periodic1(seed: u32, x: f32, period: u32) -> f32 {
    if !x.is_finite() {
        return 0.0;
    }
    let p = period.max(1);
    let xf = ffloor(x);
    let i = (xf as i64).rem_euclid(p as i64) as u32;
    let j = (i + 1) % p;
    let t = smooth(x - xf);
    let a = signed(hash2(seed, i));
    let b = signed(hash2(seed, j));
    a + (b - a) * t
}

/// Periodic 1D noise with straight segments between its points: angular, like torn paper or a
/// dragged brush, in [-1, 1].
pub fn periodic1_linear(seed: u32, x: f32, period: u32) -> f32 {
    if !x.is_finite() {
        return 0.0;
    }
    let p = period.max(1);
    let xf = ffloor(x);
    let i = (xf as i64).rem_euclid(p as i64) as u32;
    let j = (i + 1) % p;
    let t = x - xf;
    let a = signed(hash2(seed, i));
    let b = signed(hash2(seed, j));
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_bounded_smooth_and_deterministic() {
        for i in 0..2000 {
            let x = i as f32 * 0.037 - 20.0;
            let v = value1(7, x);
            assert!((-1.0..=1.0).contains(&v));
            assert!((value1(7, x + 0.001) - v).abs() < 0.01);
            assert!((-1.0..=1.0).contains(&value2(3, x, x * 0.7)));
            assert!((periodic1(5, x, 8) - periodic1(5, x + 8.0, 8)).abs() < 1e-4);
        }
        assert_eq!(value1(1, 3.3), value1(1, 3.3));
        assert_ne!(value1(1, 3.3), value1(2, 3.3));
        assert_eq!(value1(1, f32::NAN), 0.0);
        let mut t = Track::new(9);
        for i in 0..500 {
            let x = i as f32 * 0.013;
            assert_eq!(t.at(x), fbm1(9, x));
        }
        assert_eq!(value2(1, f32::INFINITY, 0.0), 0.0);
    }
}
