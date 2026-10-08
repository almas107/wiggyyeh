//! Sound, synthesized in code (no sound files): clicks, pops, a brush scratch that follows how
//! fast you draw, undo and redo blips, a chime for celebrations, a thud for big actions and a
//! boing for the mascot. [`render`] makes the samples; an [`AudioOut`] plays them (cpal on the
//! desktop, WebAudio in the browser; both live in the binary). Volume and mute are settings.

/// The sounds WobbleWorks makes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sound {
    /// A UI button.
    Click,
    /// A tool picked, a card opening.
    Pop,
    /// One short slice of brush-on-paper noise; `speed` 0–1 sets loudness and brightness.
    Scratch {
        speed: f32,
    },
    Undo,
    Redo,
    /// Saving, exporting, the first stroke.
    Chime,
    /// Clearing, deleting, merging.
    Thud,
    /// The mascot, poked.
    Boing,
}

/// Plays mono samples at its own rate.
pub trait AudioOut {
    fn rate(&self) -> u32;
    fn play(&mut self, samples: Vec<f32>);
}

/// Deterministic white noise in [-1, 1].
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

fn len(rate: u32, seconds: f32) -> usize {
    ((rate as f32 * seconds) as usize).min(rate as usize * 2)
}

/// A sine sweeping from `f0` to `f1` Hz with an exponential decay of `decay` per second.
fn sweep(rate: u32, seconds: f32, f0: f32, f1: f32, decay: f32) -> Vec<f32> {
    let n = len(rate, seconds);
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let f = f0 + (f1 - f0) * (t / seconds);
            phase += std::f32::consts::TAU * f / rate as f32;
            phase.sin() * (-t * decay).exp() * attack(t)
        })
        .collect()
}

/// A 3 ms fade-in so nothing clicks on.
fn attack(t: f32) -> f32 {
    (t / 0.003).min(1.0)
}

fn mix_into(dst: &mut Vec<f32>, src: &[f32], at: usize) {
    if dst.len() < at + src.len() {
        dst.resize(at + src.len(), 0.0);
    }
    for (d, s) in dst.iter_mut().skip(at).zip(src) {
        *d += s;
    }
}

/// The samples of `sound` at `rate` Hz, peaking below 1. `seed` varies the noisy ones.
pub fn render(sound: Sound, rate: u32, seed: u32) -> Vec<f32> {
    let rate = rate.clamp(8_000, 192_000);
    let mut noise = Noise(seed | 1);
    let out: Vec<f32> = match sound {
        Sound::Click => {
            let n = len(rate, 0.03);
            (0..n)
                .map(|i| {
                    let t = i as f32 / rate as f32;
                    ((t * 2400.0 * std::f32::consts::TAU).sin() * 0.6 + noise.next() * 0.4) * (-t * 160.0).exp() * attack(t) * 0.5
                })
                .collect()
        }
        Sound::Pop => sweep(rate, 0.09, 320.0, 980.0, 38.0).into_iter().map(|s| s * 0.6).collect(),
        Sound::Scratch { speed } => {
            let speed = if speed.is_finite() { speed.clamp(0.0, 1.0) } else { 0.0 };
            let n = len(rate, 0.06);
            // White noise through a one-pole low-pass: faster strokes sound brighter and louder.
            let k = 0.08 + 0.5 * speed;
            let mut y = 0.0;
            (0..n)
                .map(|i| {
                    let t = i as f32 / rate as f32;
                    y += k * (noise.next() - y);
                    let env = (t / 0.01).min(1.0) * ((0.06 - t) / 0.015).clamp(0.0, 1.0);
                    y * env * (0.15 + 0.5 * speed)
                })
                .collect()
        }
        Sound::Undo => {
            let mut v = sweep(rate, 0.07, 880.0, 820.0, 30.0);
            mix_into(&mut v, &sweep(rate, 0.09, 600.0, 520.0, 30.0), len(rate, 0.06));
            v.into_iter().map(|s| s * 0.45).collect()
        }
        Sound::Redo => {
            let mut v = sweep(rate, 0.07, 600.0, 640.0, 30.0);
            mix_into(&mut v, &sweep(rate, 0.09, 880.0, 940.0, 30.0), len(rate, 0.06));
            v.into_iter().map(|s| s * 0.45).collect()
        }
        Sound::Chime => {
            let mut v = Vec::new();
            for (k, f) in [523.25f32, 659.25, 783.99, 1046.5].iter().enumerate() {
                mix_into(&mut v, &sweep(rate, 0.35, *f, *f, 9.0), len(rate, 0.07 * k as f32));
            }
            v.into_iter().map(|s| s * 0.3).collect()
        }
        Sound::Thud => {
            let mut v = sweep(rate, 0.22, 110.0, 45.0, 14.0);
            let n = len(rate, 0.04);
            let crunch: Vec<f32> = (0..n)
                .map(|i| {
                    let t = i as f32 / rate as f32;
                    noise.next() * (-t * 90.0).exp() * attack(t) * 0.5
                })
                .collect();
            mix_into(&mut v, &crunch, 0);
            v.into_iter().map(|s| s * 0.7).collect()
        }
        Sound::Boing => {
            let n = len(rate, 0.35);
            let mut phase = 0.0f32;
            (0..n)
                .map(|i| {
                    let t = i as f32 / rate as f32;
                    let f = 260.0 + 220.0 * (1.0 - (-t * 10.0).exp()) + 35.0 * (t * 38.0).sin();
                    phase += std::f32::consts::TAU * f / rate as f32;
                    phase.sin() * (-t * 7.0).exp() * attack(t) * 0.5
                })
                .collect()
        }
    };
    out.into_iter().map(|s| if s.is_finite() { s.clamp(-0.95, 0.95) } else { 0.0 }).collect()
}

/// Volume, mute and the output, plus the scratch's pacing.
pub struct Audio {
    pub out: Option<Box<dyn AudioOut>>,
    /// 0–1.
    pub volume: f32,
    pub muted: bool,
    seed: u32,
    /// When the last scratch slice started (seconds).
    last_scratch: f64,
    /// The last few sounds asked for (played or not), newest last: for tests and debugging.
    pub recent: std::collections::VecDeque<Sound>,
}

impl Default for Audio {
    fn default() -> Self {
        Self { out: None, volume: 0.6, muted: false, seed: 0x5eed, last_scratch: f64::NEG_INFINITY, recent: Default::default() }
    }
}

impl Audio {
    pub fn play(&mut self, sound: Sound) {
        if self.recent.len() >= 16 {
            self.recent.pop_front();
        }
        self.recent.push_back(sound);
        let volume = if self.muted { 0.0 } else { self.volume.clamp(0.0, 1.0) };
        let Some(out) = self.out.as_mut() else { return };
        if volume <= 0.0 {
            return;
        }
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let gain = volume * volume; // perceptual-ish
        let samples = render(sound, out.rate(), self.seed).into_iter().map(|s| s * gain).collect();
        out.play(samples);
    }

    /// Keep the brush scratching while drawing: call every frame with the pointer speed (points per
    /// second); slices are paced so they overlap a little.
    pub fn scratch(&mut self, now: f64, speed: f32) {
        let due = (now - self.last_scratch).partial_cmp(&0.045).is_some_and(|o| o.is_ge());
        if !due || !speed.is_finite() || speed < 30.0 {
            return;
        }
        self.last_scratch = now;
        self.play(Sound::Scratch { speed: (speed / 1800.0).clamp(0.05, 1.0) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Capture(Arc<Mutex<Vec<Vec<f32>>>>);

    impl AudioOut for Capture {
        fn rate(&self) -> u32 {
            44_100
        }
        fn play(&mut self, samples: Vec<f32>) {
            self.0.lock().unwrap().push(samples);
        }
    }

    #[test]
    fn every_sound_is_short_audible_and_never_clips() {
        for s in [Sound::Click, Sound::Pop, Sound::Scratch { speed: 0.5 }, Sound::Undo, Sound::Redo, Sound::Chime, Sound::Thud, Sound::Boing] {
            let v = render(s, 44_100, 7);
            assert!(!v.is_empty() && v.len() <= 44_100, "{s:?}: {} samples", v.len());
            let peak = v.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(peak > 0.02 && peak <= 0.95, "{s:?} peaks at {peak}");
            assert!(v.first().is_some_and(|x| x.abs() < 0.05), "{s:?} starts without a click");
        }
        // Hostile input doesn't panic or blow up.
        assert!(render(Sound::Scratch { speed: f32::NAN }, 0, 0).iter().all(|x| x.is_finite()));
        assert!(render(Sound::Chime, u32::MAX, 1).len() <= 192_000 * 2);
    }

    #[test]
    fn faster_strokes_scratch_louder() {
        let rms = |v: Vec<f32>| (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt();
        assert!(rms(render(Sound::Scratch { speed: 1.0 }, 44_100, 3)) > rms(render(Sound::Scratch { speed: 0.05 }, 44_100, 3)) * 2.0);
    }

    #[test]
    fn mute_volume_and_scratch_pacing() {
        let played = Arc::new(Mutex::new(Vec::new()));
        let mut a = Audio { out: Some(Box::new(Capture(played.clone()))), ..Default::default() };
        a.play(Sound::Pop);
        assert_eq!(played.lock().unwrap().len(), 1);
        a.muted = true;
        a.play(Sound::Pop);
        a.muted = false;
        a.volume = 0.0;
        a.play(Sound::Pop);
        assert_eq!(played.lock().unwrap().len(), 1, "muted and silent play nothing");
        a.volume = 1.0;
        a.scratch(1.0, 900.0);
        a.scratch(1.01, 900.0);
        a.scratch(1.06, 900.0);
        a.scratch(1.2, 5.0);
        a.scratch(f64::NAN, 900.0);
        assert_eq!(played.lock().unwrap().len(), 3, "paced, and silent when barely moving");
        Audio::default().play(Sound::Click);
    }
}
