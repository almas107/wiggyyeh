use photocraft_doc::{Model3dFit, PixelFormat};
use serde_json::json;
use wobbleworks_3d::camera::{Camera, Viewport};
use wobbleworks_3d::editor::Editor;

use super::*;

fn note() -> Editor {
    let mut ed = Editor::new();
    ed.set_viewport(640.0, 400.0);
    ed.stable = 0.0;
    let line: Vec<[f32; 2]> = (0..40).map(|i| [200.0 + i as f32 * 6.0, 200.0]).collect();
    ed.run("stroke.draw", &json!({"points": line})).expect("draw");
    ed
}

fn opaque(s: &Surface, w: i32, h: i32) -> usize {
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).filter(|&(x, y)| s.rgba(x, y)[3] > 0.0).count()
}

#[test]
fn tracing_never_touches_the_note() {
    let mut ed = note();
    ed.run("shot.add", &json!({})).expect("camera");
    let before = (ed.scene.clone(), ed.camera, ed.scene_revision);
    let cam = ed.camera;
    let pic = picture_at(&mut ed, &cam, 0, 300, 420, true).expect("render");
    assert_eq!((pic.width, pic.height), (300, 420));
    assert!(pic.rgba.chunks_exact(4).any(|p| p[3] > 0), "the stroke shows");
    let layer = new_layer(&mut ed, None, Model3dFit::Height, 120, 80, PixelFormat::RGBA8).expect("layer");
    let _ = refresh_layer(&mut ed, &layer, &layer.scene.clone(), 120, 80, PixelFormat::RGBA8).expect("refresh");
    // Making a layer gives the note its uid, and nothing else.
    let mut expect = before.0.clone();
    expect.uid = ed.scene.uid;
    assert!(ed.scene == expect, "the note is unchanged");
    assert_eq!(ed.camera, before.1, "the 3D view's camera is put back");
    assert_eq!(ed.scene_revision, before.2);
}

#[test]
fn a_known_camera_at_two_document_sizes() {
    let mut ed = note();
    let cam = ed.camera;
    for (w, h) in [(320, 200), (200, 320)] {
        let pic = picture_at(&mut ed, &cam, 0, w, h, true).expect("render");
        assert_eq!((pic.width, pic.height, pic.rgba.len()), (w, h, (w * h * 4) as usize));
    }
    for (w, h) in [(0, 10), (10, 0), (MAX_SIDE + 1, 10)] {
        assert!(picture_at(&mut ed, &cam, 0, w, h, true).is_err());
    }
    // A hostile camera is sanitised, not rendered as NaN.
    let wild = Camera { distance: f32::NAN, twist: f32::INFINITY, ..cam };
    assert!(picture_at(&mut ed, &wild, 0, 64, 64, true).is_ok());
}

#[test]
fn fit_width_keeps_the_horizontal_view_on_a_tall_picture() {
    let c = Camera { viewport: Viewport { width: 1600.0, height: 800.0 }, ..Camera::default() };
    let tall = fit_camera(&c, Model3dFit::Width, 800, 1600);
    // Horizontal half-extent ∝ aspect × tan(fov_y / 2) ∝ aspect / focal.
    let (wide, fitted) = (2.0 / c.focal_mm, 0.5 / tall.focal_mm);
    assert!((wide - fitted).abs() / wide < 1e-4, "{wide} vs {fitted}");
    assert_eq!(fit_camera(&c, Model3dFit::Height, 800, 1600).focal_mm, c.focal_mm);
    assert_eq!(fit_camera(&c, Model3dFit::Contain, 4000, 1000).focal_mm, c.focal_mm);
    assert!(fit_camera(&c, Model3dFit::Contain, 800, 1600).focal_mm < c.focal_mm);
}

#[test]
fn layers_name_a_camera_in_the_notes_list() {
    let mut ed = note();
    let shot = ed.run("shot.add", &json!({})).expect("camera").as_u64().expect("the camera's id");
    let pinned = new_layer(&mut ed, Some(shot), Model3dFit::Height, 160, 100, PixelFormat::RGBA8).expect("pinned");
    let live = new_layer(&mut ed, None, Model3dFit::Height, 160, 100, PixelFormat::RGBA8).expect("live");
    assert_eq!((pinned.main_camera_id, pinned.follow_view), (shot, false));
    assert!(live.follow_view);
    assert_eq!(pinned.scene_id, live.scene_id);
    assert!(Arc::ptr_eq(&pinned.scene, &pinned.scene.clone()));
    assert!(opaque(&pinned.cache, 160, 100) > 10);
    // Orbit away: the live layer follows, the pinned one holds its camera's picture.
    ed.run("camera.orbit", &json!({"dx": 300.0})).expect("orbit");
    let (_, bytes) = snapshot(&mut ed).expect("note");
    let p2 = refresh_layer(&mut ed, &pinned, &bytes, 160, 100, PixelFormat::RGBA8).expect("pinned");
    let l2 = refresh_layer(&mut ed, &live, &bytes, 160, 100, PixelFormat::RGBA8).expect("live");
    assert!(p2.cache == pinned.cache, "the pinned angle holds");
    assert!(l2.cache != live.cache, "the live layer follows the view");
    // Delete the camera: the layer keeps its picture, marked stale.
    ed.run("shot.delete", &json!({"ids": [shot]})).expect("delete");
    let p3 = refresh_layer(&mut ed, &pinned, &bytes, 160, 100, PixelFormat::RGBA8).expect("kept");
    assert!(p3.dirty && p3.cache == pinned.cache);
    // A note with another uid is refused.
    let mut other = note();
    assert!(refresh_layer(&mut other, &pinned, &bytes, 160, 100, PixelFormat::RGBA8).is_err());
}

#[test]
fn a_saved_layer_renders_without_its_note_open() {
    let mut ed = note();
    let layer = new_layer(&mut ed, None, Model3dFit::Height, 200, 120, PixelFormat::RGBA8).expect("layer");
    let again = render_saved(&layer, 200, 120, PixelFormat::RGBA8).expect("render");
    assert!(opaque(&again.cache, 200, 120) > 10);
    let bigger = render_saved(&layer, 400, 240, PixelFormat::RGBA8).expect("render");
    assert!(opaque(&bigger.cache, 400, 240) > opaque(&again.cache, 200, 120));
    let broken = Model3dLayer { scene: Arc::new(b"not a note".to_vec()), ..layer };
    assert!(render_saved(&broken, 10, 10, PixelFormat::RGBA8).is_err());
}

#[test]
fn surfaces_check_their_size_and_take_the_documents_format() {
    let ok = Picture { width: 2, height: 1, rgba: vec![255, 0, 0, 128, 0, 0, 255, 255] };
    let s = to_surface(&ok, PixelFormat::RGBA8).expect("fits");
    let p = s.rgba(0, 0);
    assert!((p[0] - 1.0).abs() < 1e-3 && (p[3] - 128.0 / 255.0).abs() < 1e-3, "straight alpha in, straight alpha kept: {p:?}");
    assert!(to_surface(&Picture { width: 3, height: 1, rgba: ok.rgba.clone() }, PixelFormat::RGBA8).is_err());
    assert!(to_surface(&Picture { width: 0, height: 0, rgba: Vec::new() }, PixelFormat::RGBA8).is_err());
    let s16 = to_surface(&ok, PixelFormat { sample: photocraft_doc::SampleType::U16, ..PixelFormat::RGBA8 }).expect("16-bit");
    assert!((s16.rgba(1, 0)[2] - 1.0).abs() < 1e-3);
}

#[test]
fn big_documents_render_at_the_limit_and_scale_up() {
    let mut ed = note();
    let cam = ed.camera;
    let s = render_surface(&mut ed, &cam, Model3dFit::Height, MAX_SIDE * 2, 16, PixelFormat::RGBA8).expect("render");
    assert!(s.content_bounds().x1 <= (MAX_SIDE * 2) as i32);
}
