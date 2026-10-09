//! Pictures of the 3D note: a PNG of the view (1–4×, optionally transparent), the boil as a
//! looping GIF, and a 360° turntable GIF around the orbit point (Feather's Quick Export). Exports
//! hide guides, the grid and the orbit point, as Feather's do.

use std::collections::HashMap;

use wobbleworks_3d::camera::Camera;
use wobbleworks_3d::editor::Editor;
use wobbleworks_3d::raster::{MAX_SIDE, Picture, Textures, rasterize};

/// Most frames in an exported GIF.
pub const GIF_FRAMES_MAX: usize = 120;

/// The view at `scale` times the viewport size.
pub fn picture(ed: &mut Editor, camera: &Camera, frame: u32, scale: u32, transparent: bool) -> Result<Picture, String> {
    let scale = scale.clamp(1, 4) as f32;
    let vp = camera.viewport.sane();
    let (w, h) = ((vp.width * scale).round(), (vp.height * scale).round());
    if w < 1.0 || h < 1.0 || w > MAX_SIDE as f32 || h > MAX_SIDE as f32 {
        return Err(format!("{w} × {h} is too big to export"));
    }
    let mut cam = *camera;
    cam.viewport = wobbleworks_3d::camera::Viewport { width: w, height: h };
    let saved = ed.camera;
    ed.camera = cam;
    let f = ed.render(frame, false);
    ed.camera = saved;
    let images: HashMap<u64, (u32, u32, &[u8])> =
        ed.scene.images.iter().chain(ed.scene.environment.background_image.iter()).map(|i| (i.id, (i.width, i.height, i.rgba.as_slice()))).collect();
    let tex = Textures { atlas: &ed.atlas, images };
    let env = &ed.scene.environment;
    let effects = env.render_mode.then_some(&env.effects);
    rasterize(&f, &tex, w as u32, h as u32, (!transparent).then_some(env.background), if transparent { None } else { effects })
}

pub fn png(ed: &mut Editor, frame: u32, scale: u32, transparent: bool) -> Result<Vec<u8>, String> {
    let cam = ed.camera;
    let pic = picture(ed, &cam, frame, scale, transparent)?;
    crate::io::encode_png(pic.width, pic.height, &pic.rgba)
}

fn gif(frames: Vec<Picture>, fps: f32) -> Result<Vec<u8>, String> {
    use image::codecs::gif::{GifEncoder, Repeat};
    let fps = if fps.is_finite() { fps.clamp(1.0, 50.0) } else { 8.0 };
    let delay = image::Delay::from_numer_denom_ms((1000.0 / fps).round() as u32, 1);
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new_with_speed(&mut out, 10);
        enc.set_repeat(Repeat::Infinite).map_err(|e| e.to_string())?;
        let frames: Result<Vec<image::Frame>, String> = frames
            .into_iter()
            .map(|p| {
                image::RgbaImage::from_raw(p.width, p.height, p.rgba)
                    .map(|img| image::Frame::from_parts(img, 0, 0, delay))
                    .ok_or_else(|| "a frame came out the wrong size".to_string())
            })
            .collect();
        enc.encode_frames(frames?).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// What an animated export shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anim {
    /// The boil frames (looping).
    Boil,
    /// A 360° turn around the orbit point, boiling as it turns.
    Turntable,
    /// The camera flying through the shots (Sequence), as the Shots tab plays them.
    Shots,
}

impl Anim {
    /// File name suffix.
    pub fn suffix(self) -> &'static str {
        match self {
            Anim::Boil => "",
            Anim::Turntable => " turntable",
            Anim::Shots => " shots",
        }
    }
}

/// How an animation is saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Video {
    Gif,
    /// AV1 in an MP4 ([`super::mp4`]).
    Mp4,
}

impl Video {
    pub fn ext(self) -> &'static str {
        match self {
            Video::Gif => "gif",
            Video::Mp4 => "mp4",
        }
    }
}

/// A video lasts at least this long: a three-frame boil loop is repeated to fill it.
const VIDEO_SECONDS_MIN: f32 = 4.0;

/// An animation as a GIF or an MP4 (`scale` times the view's size).
pub fn animation(ed: &mut Editor, anim: Anim, video: Video, scale: u32) -> Result<Vec<u8>, String> {
    let (frames, fps) = frames(ed, anim, video, scale)?;
    match video {
        Video::Gif => gif(frames, fps),
        Video::Mp4 => super::mp4::encode(&frames, fps),
    }
}

/// The boil frames as a looping GIF.
pub fn boil_gif(ed: &mut Editor, scale: u32) -> Result<Vec<u8>, String> {
    animation(ed, Anim::Boil, Video::Gif, scale)
}

/// A 360° turn around the orbit point (36 steps), boiling as it turns.
pub fn turntable_gif(ed: &mut Editor, scale: u32) -> Result<Vec<u8>, String> {
    animation(ed, Anim::Turntable, Video::Gif, scale)
}

/// The camera between shots at `t` (in shot-to-shot legs: 1.5 is halfway from the second shot
/// to the third), eased; `None` with fewer than two shots.
pub fn shot_camera(seq: &wobbleworks_3d::model::Sequence, t: f64, viewport: wobbleworks_3d::camera::Viewport) -> Option<Camera> {
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
    let mut dyaw = b.yaw - a.yaw;
    if dyaw > 180.0 {
        dyaw -= 360.0;
    } else if dyaw < -180.0 {
        dyaw += 360.0;
    }
    Some(Camera {
        target: a.target.lerp(b.target, e),
        yaw: a.yaw + dyaw * e,
        pitch: lerp(a.pitch, b.pitch),
        distance: lerp(a.distance, b.distance),
        focal_mm: lerp(a.focal_mm, b.focal_mm),
        orthographic: if e < 0.5 { a.orthographic } else { b.orthographic },
        snapped_from_perspective: false,
        viewport,
    })
}

/// The pictures of an animation and their rate.
pub fn frames(ed: &mut Editor, anim: Anim, video: Video, scale: u32) -> Result<(Vec<Picture>, f32), String> {
    let max = match video {
        Video::Gif => GIF_FRAMES_MAX,
        Video::Mp4 => super::mp4::FRAMES_MAX,
    };
    let boil = ed.scene.boil;
    let boil_n = if boil.enabled { boil.frames.clamp(2, 12) } else { 1 };
    let boil_fps = if boil.fps.is_finite() { boil.fps.clamp(1.0, 50.0) } else { 8.0 };
    // The boil frame showing at `secs` into the animation.
    let boil_at = |secs: f32| ((secs * boil_fps).max(0.0) as u32) % boil_n;
    let base = ed.camera;
    let mut out = Vec::new();
    let fps = match anim {
        Anim::Boil => {
            let reps = match video {
                Video::Gif => 1,
                Video::Mp4 => ((VIDEO_SECONDS_MIN * boil_fps / boil_n as f32).ceil() as usize).max(1),
            };
            let one: Vec<Picture> = (0..boil_n).map(|i| picture(ed, &base, i, scale, false)).collect::<Result<_, _>>()?;
            for _ in 0..reps {
                if out.len() + one.len() > max {
                    break;
                }
                out.extend(one.iter().cloned());
            }
            boil_fps
        }
        Anim::Turntable => {
            let (steps, fps) = match video {
                Video::Gif => (36usize, 12.0),
                Video::Mp4 => (120, 30.0),
            };
            for i in 0..steps.min(max) {
                let mut cam = base;
                cam.yaw = base.yaw + 360.0 * i as f32 / steps as f32;
                cam.sanitize();
                out.push(picture(ed, &cam, boil_at(i as f32 / fps), scale, false)?);
            }
            fps
        }
        Anim::Shots => {
            let seq = ed.scene.sequence.clone();
            if seq.shots.len() < 2 {
                return Err("add two shots or more to export them".into());
            }
            let legs = (seq.shots.len() - 1) as f32;
            let per = (seq.seconds_per_shot / seq.speed.max(0.1)).clamp(0.1, 60.0);
            let fps = match video {
                Video::Gif => 12.0,
                Video::Mp4 => 30.0,
            };
            let n = ((legs * per * fps).ceil() as usize + 1).clamp(2, max);
            for k in 0..n {
                let t = f64::from(legs) * k as f64 / (n - 1) as f64;
                let cam = shot_camera(&seq, t, base.viewport).ok_or("the shots changed")?;
                out.push(picture(ed, &cam, boil_at(k as f32 / fps), scale, false)?);
            }
            fps
        }
    };
    Ok((out, fps))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exports_make_real_files() {
        let mut ed = Editor::new();
        ed.set_viewport(160.0, 100.0);
        ed.run("stroke.add", &json!({"points": [[-1,0,0],[1,0.5,0]]})).expect("curve");
        let png = png(&mut ed, 0, 2, true).expect("png");
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
        let g = boil_gif(&mut ed, 1).expect("gif");
        assert!(g.starts_with(b"GIF89a"));
        let t = turntable_gif(&mut ed, 1).expect("turntable");
        assert!(t.starts_with(b"GIF89a"));
        let v = animation(&mut ed, Anim::Boil, Video::Mp4, 1).expect("mp4");
        assert_eq!(&v[4..8], b"ftyp");
        assert!(animation(&mut ed, Anim::Shots, Video::Gif, 1).is_err(), "no shots yet");
        ed.run("shot.add", &json!({})).expect("shot");
        ed.run("camera.orbit", &json!({"dx": 90, "dy": 0})).expect("orbit");
        ed.run("shot.add", &json!({})).expect("shot");
        let v = animation(&mut ed, Anim::Shots, Video::Mp4, 1).expect("shots mp4");
        assert_eq!(&v[4..8], b"ftyp");
        assert_eq!(ed.camera.viewport.width, 160.0, "the view is left as it was");
    }

    /// Writes a turntable MP4 for checking with other tools:
    /// `WOBBLE3D_MP4=t.mp4 cargo test -p wobbleworks --lib sample_mp4 -- --ignored`, then
    /// `ffprobe t.mp4` / `ffmpeg -i t.mp4 -f null -`.
    #[test]
    #[ignore = "writes a file for external players"]
    fn sample_mp4() {
        let mut ed = Editor::new();
        ed.set_viewport(480.0, 320.0);
        for (i, c) in ["#e8344e", "#2f5bff", "#2ec27e"].iter().enumerate() {
            ed.run("brush.set", &json!({"kind": "oil", "color": c, "size": 40})).expect("brush");
            let y = i as f32 * 0.4 - 0.4;
            ed.run("stroke.add", &json!({"points": (0..30).map(|k| { let t = k as f32 / 29.0; [t * 2.0 - 1.0, y + (t * 6.0).sin() * 0.2, (t * 3.0).cos() * 0.5] }).collect::<Vec<_>>()})).expect("curve");
        }
        let t0 = std::time::Instant::now();
        let (frames, fps) = frames(&mut ed, Anim::Turntable, Video::Mp4, 1).expect("frames");
        let t1 = std::time::Instant::now();
        let v = super::super::mp4::encode(&frames, fps).expect("mp4");
        eprintln!("{} frames: drawn in {:?}, encoded in {:?}", frames.len(), t1 - t0, t1.elapsed());
        std::fs::write(std::env::var("WOBBLE3D_MP4").unwrap_or_else(|_| "turntable.mp4".into()), v).expect("write");
    }
}
