//! WobbleWorks 3D notes as PhotoCraft 3D layers (Clip Studio Paint's 3D layer as a tracing
//! reference; design: `docs/wobbleworks-3d-trace.md`). The only place the two worlds meet:
//! `photocraft-doc` knows nothing of 3D, `wobbleworks-3d` nothing of documents.
//!
//! The model is Clip Studio's (section 7.2 of the design): the note keeps a **camera list** (its
//! camera shots) and each [`Model3dLayer`] names one by id (`main_camera_id`, CSP's
//! `MainCameraID`), or follows the note's view camera (`follow_view`). Several angles of one
//! note cost one camera each, and the bytes of the note are shared between its layers.
//!
//! Rendering only reads the note: [`picture_at`] puts the live camera back, so tracing never
//! changes the 3D canvas.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::HashMap;
use std::sync::Arc;

use photocraft_doc::{Model3dFit, Model3dLayer, PixelFormat, Rect};
use photocraft_raster::Surface;
use wobbleworks_3d::camera::{Camera, Viewport};
use wobbleworks_3d::editor::Editor;
use wobbleworks_3d::model::Scene;
use wobbleworks_3d::raster::{MAX_SIDE, Picture, Textures, rasterize};

/// The note seen through `camera` at exactly `width` × `height` pixels. The camera's vertical
/// field of view is kept (fit it first for anything else, see [`fit_camera`]). `ed.scene` is only
/// read and the live camera is put back: the 3D view is left exactly as it was.
pub fn picture_at(ed: &mut Editor, camera: &Camera, frame: u32, width: u32, height: u32, transparent: bool) -> Result<Picture, String> {
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(format!("{width} × {height} can't be rendered (1 to {MAX_SIDE} pixels a side)"));
    }
    let mut cam = *camera;
    cam.viewport = Viewport { width: width as f32, height: height as f32 };
    // A hostile camera (from a file, a param) can't reach the renderer.
    cam.sanitize();
    let saved = ed.camera;
    ed.camera = cam;
    let f = ed.render(frame, false);
    ed.camera = saved;
    let images: HashMap<u64, (u32, u32, &[u8])> =
        ed.scene.images.iter().chain(ed.scene.environment.background_image.iter()).map(|i| (i.id, (i.width, i.height, i.rgba.as_slice()))).collect();
    let tex = Textures { atlas: &ed.atlas, images };
    let env = &ed.scene.environment;
    let effects = env.render_mode.then_some(&env.effects);
    rasterize(&f, &tex, width, height, (!transparent).then_some(env.background), if transparent { None } else { effects })
}

/// `camera` made for a `width` × `height` picture: its viewport set, and its lens changed so the
/// framing it had in its own viewport is kept as `fit` says.
pub fn fit_camera(camera: &Camera, fit: Model3dFit, width: u32, height: u32) -> Camera {
    let shot = camera.viewport.sane();
    let shot_aspect = shot.width / shot.height;
    let mut c = *camera;
    c.viewport = Viewport { width: width.max(1) as f32, height: height.max(1) as f32 };
    c.sanitize();
    let picture = c.viewport.width / c.viewport.height;
    // tan(fov_y / 2) is proportional to 1 / focal, so keeping the horizontal field of view on a
    // narrower picture means a shorter lens by picture / shot.
    let keep_width = match fit {
        Model3dFit::Height => false,
        Model3dFit::Width => true,
        Model3dFit::Contain => picture < shot_aspect,
    };
    if keep_width && shot_aspect.is_finite() && shot_aspect > 0.0 {
        c.focal_mm *= picture / shot_aspect;
        c.sanitize();
    }
    c
}

/// The camera a layer is seen through in `scene`: the note's view camera when it follows the
/// view, else the listed camera (camera shot) it names. None when that camera was deleted.
pub fn layer_camera(scene: &Scene, view: &Camera, layer: &Model3dLayer) -> Option<Camera> {
    if layer.follow_view {
        return Some(*view);
    }
    scene.sequence.shots.iter().find(|s| s.id == layer.main_camera_id).map(|s| s.camera)
}

/// A picture as a layer surface in `format`. Both sides are straight alpha, so the bytes go in
/// as they are; the length is checked first (a mismatch would otherwise panic in the surface).
pub fn to_surface(pic: &Picture, format: PixelFormat) -> Result<Surface, String> {
    let need = (pic.width as usize).checked_mul(pic.height as usize).and_then(|n| n.checked_mul(4)).ok_or("the picture is too big")?;
    if pic.width == 0 || pic.height == 0 || pic.rgba.len() != need {
        return Err("the 3D picture came out the wrong size".into());
    }
    let w = i32::try_from(pic.width).map_err(|_| "the picture is too wide")?;
    let h = i32::try_from(pic.height).map_err(|_| "the picture is too tall")?;
    let s = Surface::from_interleaved(PixelFormat::RGBA8, Rect::new(0, 0, w, h), &pic.rgba);
    Ok(if format == PixelFormat::RGBA8 { s } else { s.convert(format) })
}

/// Render `camera` at a document's size into a surface: sides over [`MAX_SIDE`] are rendered
/// at that size and scaled up (nearest, as a reference to trace over needs no more).
pub fn render_surface(ed: &mut Editor, camera: &Camera, fit: Model3dFit, width: u32, height: u32, format: PixelFormat) -> Result<Surface, String> {
    if width == 0 || height == 0 {
        return Err("the picture is empty".into());
    }
    let longest = width.max(height);
    let (rw, rh) = if longest > MAX_SIDE {
        let k = f64::from(MAX_SIDE) / f64::from(longest);
        (((f64::from(width) * k).round() as u32).clamp(1, MAX_SIDE), ((f64::from(height) * k).round() as u32).clamp(1, MAX_SIDE))
    } else {
        (width, height)
    };
    let cam = fit_camera(camera, fit, rw, rh);
    let pic = picture_at(ed, &cam, 0, rw, rh, true)?;
    let pic = if (rw, rh) == (width, height) { pic } else { upscale(&pic, width, height)? };
    to_surface(&pic, format)
}

fn upscale(pic: &Picture, width: u32, height: u32) -> Result<Picture, String> {
    let bytes = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(4)).ok_or("the picture is too big")?;
    let mut rgba = vec![0u8; bytes];
    for y in 0..height as usize {
        let sy = (y * pic.height as usize / height as usize).min((pic.height as usize).saturating_sub(1));
        for x in 0..width as usize {
            let sx = (x * pic.width as usize / width as usize).min((pic.width as usize).saturating_sub(1));
            let (Some(src), Some(dst)) = (
                pic.rgba.get((sy * pic.width as usize + sx) * 4..).and_then(|s| s.get(..4)),
                rgba.get_mut((y * width as usize + x) * 4..).and_then(|d| d.get_mut(..4)),
            ) else {
                return Err("the 3D picture came out the wrong size".into());
            };
            dst.copy_from_slice(src);
        }
    }
    Ok(Picture { width, height, rgba })
}

/// A seed for a note's uid: a hash of its saved bytes (FNV-1a).
fn seed_of(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

/// The note in the editor, saved (with its view camera) for a layer, and its uid (given one if
/// it had none, so every layer made from it can find it again).
pub fn snapshot(ed: &mut Editor) -> Result<(u64, Arc<Vec<u8>>), String> {
    if ed.scene.uid == 0 {
        let bytes = wobbleworks_3d::io::save(&ed.scene, &ed.camera)?;
        ed.note_uid(seed_of(&bytes));
    }
    let bytes = wobbleworks_3d::io::save(&ed.scene, &ed.camera)?;
    Ok((ed.scene.uid, Arc::new(bytes)))
}

/// A new 3D layer of the note in the editor at a document's size: seen through the listed camera
/// `main_camera_id`, or following the view camera.
pub fn new_layer(ed: &mut Editor, main_camera_id: Option<u64>, fit: Model3dFit, width: u32, height: u32, format: PixelFormat) -> Result<Model3dLayer, String> {
    let (scene_id, scene) = snapshot(ed)?;
    let mut layer = Model3dLayer {
        scene,
        scene_id,
        main_camera_id: main_camera_id.unwrap_or(0),
        follow_view: main_camera_id.is_none(),
        fit,
        cache: Surface::new(format),
        dirty: true,
    };
    let cam = layer_camera(&ed.scene, &ed.camera, &layer).ok_or("that camera isn't in the note's camera list")?;
    layer.cache = render_surface(ed, &cam, fit, width, height, format)?;
    layer.dirty = false;
    Ok(layer)
}

/// `layer` brought up to date with the note in the editor (which must be the layer's note:
/// same uid): the note's bytes replaced by `scene` and the picture rendered again. When its
/// camera is gone from the list, the layer keeps its last picture and is marked dirty.
pub fn refresh_layer(
    ed: &mut Editor,
    layer: &Model3dLayer,
    scene: &Arc<Vec<u8>>,
    width: u32,
    height: u32,
    format: PixelFormat,
) -> Result<Model3dLayer, String> {
    if layer.scene_id != ed.scene.uid {
        return Err("that 3D layer was made from another note".into());
    }
    let mut out = Model3dLayer { scene: scene.clone(), ..layer.clone() };
    match layer_camera(&ed.scene, &ed.camera, layer) {
        Some(cam) => {
            out.cache = render_surface(ed, &cam, layer.fit, width, height, format)?;
            out.dirty = false;
        }
        None => out.dirty = true,
    }
    Ok(out)
}

/// Render a 3D layer from its own saved note, without the note being open in the 3D mode (a
/// picture just opened, or resized). Uses a scratch editor, so nothing live is touched.
pub fn render_saved(layer: &Model3dLayer, width: u32, height: u32, format: PixelFormat) -> Result<Model3dLayer, String> {
    let (scene, view) = wobbleworks_3d::io::load(&layer.scene)?;
    let mut ed = Editor::new();
    ed.open(scene, view);
    ed.camera = view;
    ed.camera.sanitize();
    let mut out = layer.clone();
    match layer_camera(&ed.scene, &view, layer) {
        Some(cam) => {
            out.cache = render_surface(&mut ed, &cam, layer.fit, width, height, format)?;
            out.dirty = false;
        }
        None => out.dirty = true,
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
