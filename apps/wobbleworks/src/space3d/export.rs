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

/// The boil frames as a looping GIF.
pub fn boil_gif(ed: &mut Editor, scale: u32) -> Result<Vec<u8>, String> {
    let n = if ed.scene.boil.enabled { ed.scene.boil.frames.clamp(2, 12) } else { 1 };
    let cam = ed.camera;
    let frames = (0..n).map(|i| picture(ed, &cam, i, scale, false)).collect::<Result<Vec<_>, _>>()?;
    gif(frames, ed.scene.boil.fps)
}

/// A 360° turn around the orbit point (36 steps), boiling as it turns.
pub fn turntable_gif(ed: &mut Editor, scale: u32) -> Result<Vec<u8>, String> {
    let steps = 36usize.min(GIF_FRAMES_MAX);
    let base = ed.camera;
    let n = ed.scene.boil.frames.clamp(2, 12);
    let mut frames = Vec::with_capacity(steps);
    for i in 0..steps {
        let mut cam = base;
        cam.yaw = base.yaw + 360.0 * i as f32 / steps as f32;
        cam.sanitize();
        frames.push(picture(ed, &cam, (i as u32) % n, scale, false)?);
    }
    gif(frames, 12.0)
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
        assert_eq!(ed.camera.viewport.width, 160.0, "the view is left as it was");
    }
}
