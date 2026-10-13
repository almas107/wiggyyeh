//! Tracing the 3D note in 2D: the picture's 3D layers (Clip Studio Paint's 3D layer as a
//! reference rig; design in `docs/wobbleworks-3d-trace.md`, the bridge in `wobbleworks-trace3d`).
//!
//! The model is Clip Studio's: the note keeps a camera list (its camera shots) and each 3D layer
//! names one (`main_camera_id`) or follows the note's view camera. "Shoot" appends the view to
//! the camera list and adds a 3D layer pointing at it, so you can orbit on freely and the layer
//! holds its angle; "Shoot live" makes a layer that follows the view. Leaving the 3D mode brings
//! the note's layers up to date. Rendering only reads the note.

use photocraft_doc::{Layer, LayerContent, LayerId, Model3dFit, Model3dLayer};
use photocraft_engine::Session;
use photocraft_raster::Surface;
use serde_json::{Value, json};
use wobbleworks_3d::editor::Editor;

/// A 3D layer's opacity when it is made: dim enough to draw over.
pub const TRACE_OPACITY: f32 = 0.5;

/// What a new 3D layer is seen through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shoot {
    /// The view, saved as a new camera in the note's list (the layer holds this angle).
    Pinned,
    /// The note's view camera, followed.
    Live,
    /// A camera already in the list (a camera shot), by id.
    Camera(u64),
}

pub fn fit_name(f: Model3dFit) -> &'static str {
    match f {
        Model3dFit::Height => "height",
        Model3dFit::Width => "width",
        Model3dFit::Contain => "contain",
    }
}

pub fn parse_fit(s: &str) -> Option<Model3dFit> {
    match s {
        "height" => Some(Model3dFit::Height),
        "width" => Some(Model3dFit::Width),
        "contain" => Some(Model3dFit::Contain),
        _ => None,
    }
}

/// The 3D layer data of a layer, if it is one.
pub fn model3d(layer: &Layer) -> Option<&Model3dLayer> {
    match &layer.content {
        LayerContent::Model3d(m) => Some(m),
        _ => None,
    }
}

fn doc_size(session: &Session) -> Result<(u32, u32, photocraft_doc::PixelFormat), String> {
    let st = session.active().ok_or("there is no picture to trace into: make one first")?;
    Ok((st.doc.size.width, st.doc.size.height, st.doc.pixel_format()))
}

/// Shoot: a 3D layer of the note above the active layer, dimmed, with an empty layer above it
/// to trace on (made active). One undo step in the picture's history; a pinned shoot also adds
/// the camera to the note (one undo step in the 3D history).
pub fn shoot(session: &mut Session, ed: &mut Editor, what: Shoot, fit: Model3dFit) -> Result<Value, String> {
    let (w, h, format) = doc_size(session)?;
    let (camera, name) = match what {
        Shoot::Live => (None, "3D (view)".to_string()),
        Shoot::Camera(id) => {
            let shot = ed.scene.sequence.shots.iter().find(|s| s.id == id).ok_or("that camera isn't in the note's camera list")?;
            (Some(id), format!("3D ({})", shot.name))
        }
        Shoot::Pinned => {
            let n = ed.scene.sequence.shots.iter().filter(|s| s.name.starts_with("Trace ")).count();
            let label = format!("Trace {}", n + 1);
            let id = ed.run("shot.add", &json!({"name": label}))?.as_u64().ok_or("the camera couldn't be saved")?;
            (Some(id), format!("3D ({label})"))
        }
    };
    let m = wobbleworks_trace3d::new_layer(ed, camera, fit, w, h, format)?;
    let (main, follow) = (m.main_camera_id, m.follow_view);
    let (layer, above) = session
        .edit("Shoot 3D", |doc, active| {
            let mut layer = Layer::new(name, LayerContent::Model3d(m));
            layer.opacity = TRACE_OPACITY;
            let id = doc.insert_above(*active, layer);
            let above = doc.insert_above(Some(id), Layer::new("Trace", LayerContent::Raster(Surface::new(format))));
            *active = Some(above);
            Ok((id, above))
        })
        .map_err(|e| e.to_string())?;
    // Every layer of the note shares the newest copy of it (the camera list may have grown);
    // what they show hasn't changed, so nothing renders again.
    sync(session, ed, false)?;
    Ok(json!({"layer": layer.0, "traceOn": above.0, "camera": main, "followView": follow, "size": [w, h]}))
}

/// Bring the active picture's 3D layers of the note in the editor up to date: each gets the
/// note's newest bytes (one shared copy), and is rendered again when `render` says so for it
/// (`true`: all; the shell passes which changed). One undo step; returns how many were rendered.
pub fn sync(session: &mut Session, ed: &mut Editor, render_all: bool) -> Result<usize, String> {
    sync_with(session, ed, |_, _| render_all)
}

/// [`sync`], rendering only the layers `render` picks.
pub fn sync_with(session: &mut Session, ed: &mut Editor, render: impl Fn(LayerId, &Model3dLayer) -> bool) -> Result<usize, String> {
    let uid = ed.scene.uid;
    if uid == 0 {
        return Ok(0);
    }
    let (w, h, format) = doc_size(session)?;
    let Some(st) = session.active() else { return Ok(0) };
    let mine: Vec<(LayerId, Model3dLayer)> =
        st.doc.walk().into_iter().filter_map(|(_, _, l)| model3d(l).filter(|m| m.scene_id == uid).map(|m| (l.id, m.clone()))).collect();
    if mine.is_empty() {
        return Ok(0);
    }
    let (_, bytes) = wobbleworks_trace3d::snapshot(ed)?;
    let mut updates = Vec::new();
    let mut rendered = 0;
    for (id, m) in mine {
        let wants = render(id, &m);
        let next = if wants {
            rendered += 1;
            wobbleworks_trace3d::refresh_layer(ed, &m, &bytes, w, h, format)?
        } else {
            Model3dLayer { scene: bytes.clone(), ..m.clone() }
        };
        if next != m {
            updates.push((id, next));
        }
    }
    if updates.is_empty() {
        return Ok(rendered);
    }
    session
        .edit("Update 3D layers", |doc, _| {
            for (id, m) in updates {
                if let Some(l) = doc.layer_mut(id) {
                    l.content = LayerContent::Model3d(m);
                }
            }
            Ok(())
        })
        .map_err(|e| e.to_string())?;
    Ok(rendered)
}

/// Render 3D layers again from their own saved notes (any note, open in 3D or not): one layer,
/// or every 3D layer of the active picture. One undo step; returns how many were rendered.
pub fn rerender_saved(session: &mut Session, only: Option<LayerId>) -> Result<usize, String> {
    let (w, h, format) = doc_size(session)?;
    let Some(st) = session.active() else { return Ok(0) };
    let mut updates = Vec::new();
    for (_, _, l) in st.doc.walk() {
        if only.is_some_and(|id| id != l.id) {
            continue;
        }
        let Some(m) = model3d(l) else { continue };
        updates.push((l.id, wobbleworks_trace3d::render_saved(m, w, h, format)?));
    }
    if updates.is_empty() {
        return match only {
            Some(_) => Err("that layer isn't a 3D layer".into()),
            None => Ok(0),
        };
    }
    let n = updates.len();
    session
        .edit("Update 3D layers", |doc, _| {
            for (id, m) in updates {
                if let Some(l) = doc.layer_mut(id) {
                    l.content = LayerContent::Model3d(m);
                }
            }
            Ok(())
        })
        .map_err(|e| e.to_string())?;
    Ok(n)
}

/// Whether the note in the editor is safe to replace: nothing drawn, or already kept in a 3D
/// layer of the picture (which is brought up to date first).
fn note_is_kept(session: &Session, ed: &Editor) -> bool {
    let sc = &ed.scene;
    let empty = sc.strokes.is_empty() && sc.guides.is_empty() && sc.images.is_empty() && sc.models.is_empty();
    empty || (sc.uid != 0 && session.active().is_some_and(|st| st.doc.walk().iter().any(|(_, _, l)| model3d(l).is_some_and(|m| m.scene_id == sc.uid))))
}

/// Open a 3D layer's note in the 3D mode (Clip Studio: double-click a 3D layer to edit it), with
/// the view on the layer's camera. Refused when that would lose a note kept nowhere else.
pub fn edit_layer(session: &mut Session, ed: &mut Editor, id: LayerId) -> Result<Value, String> {
    let st = session.active().ok_or("there is no picture")?;
    let m = st.doc.layer(id).and_then(model3d).ok_or("that layer isn't a 3D layer")?.clone();
    if m.scene_id != ed.scene.uid {
        if !note_is_kept(session, ed) {
            return Err("the 3D note open now isn't kept in this picture: shoot it or save it (File › Save note) first".into());
        }
        // Keep the note open now in its layers before it is replaced.
        sync(session, ed, false)?;
        let (scene, view) = wobbleworks_3d::io::load(&m.scene)?;
        ed.open(scene, view);
    }
    if !m.follow_view
        && let Some(shot) = ed.scene.sequence.shots.iter().find(|s| s.id == m.main_camera_id)
    {
        let vp = ed.camera.viewport;
        ed.camera = shot.camera;
        ed.camera.viewport = vp;
        ed.camera.sanitize();
    }
    Ok(json!({"note": ed.scene.uid, "camera": if m.follow_view { Value::Null } else { json!(m.main_camera_id) }}))
}

/// Whether the active picture has 3D layers of the note in the editor; `following` = only those
/// that follow its view camera.
pub fn has_layers_of(session: &Session, ed: &Editor, following: bool) -> bool {
    let uid = ed.scene.uid;
    uid != 0
        && session
            .active()
            .is_some_and(|st| st.doc.walk().iter().any(|(_, _, l)| model3d(l).is_some_and(|m| m.scene_id == uid && (!following || m.follow_view))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, Document, SampleType, Size};

    fn note() -> Editor {
        let mut ed = Editor::new();
        ed.set_viewport(640.0, 400.0);
        ed.stable = 0.0;
        let line: Vec<[f32; 2]> = (0..40).map(|i| [200.0 + i as f32 * 6.0, 200.0]).collect();
        ed.run("stroke.draw", &json!({"points": line})).expect("draw");
        ed
    }

    fn session(w: u32, h: u32) -> Session {
        let mut s = Session::new();
        s.add_document(Document::new("t", Size::new(w, h), ColorMode::Rgb, SampleType::U8), None);
        s
    }

    fn layer(s: &Session, id: u64) -> Model3dLayer {
        s.active().and_then(|st| st.doc.layer(LayerId(id))).and_then(model3d).cloned().expect("3D layer")
    }

    #[test]
    fn shoot_adds_a_camera_to_the_note_and_a_3d_layer_naming_it() {
        let mut ed = note();
        let mut s = session(256, 160);
        let r = shoot(&mut s, &mut ed, Shoot::Pinned, Model3dFit::Height).expect("shoot");
        let cam = r["camera"].as_u64().expect("camera id");
        let shot = ed.scene.sequence.shots.iter().find(|x| x.id == cam).expect("in the camera list");
        assert_eq!(shot.name, "Trace 1");
        let m = layer(&s, r["layer"].as_u64().expect("id"));
        assert_eq!((m.main_camera_id, m.follow_view, m.scene_id), (cam, false, ed.scene.uid));
        let opaque = (0..160).flat_map(|y| (0..256).map(move |x| (x, y))).filter(|&(x, y)| m.cache.rgba(x, y)[3] > 0.0).count();
        assert!(opaque > 20, "the stroke is in the layer: {opaque}");
        let st = s.active().expect("doc");
        assert_eq!(st.active_layer.map(|l| l.0), r["traceOn"].as_u64());
        assert_eq!(st.doc.layer(LayerId(r["layer"].as_u64().expect("id"))).map(|l| l.opacity), Some(TRACE_OPACITY));
        // One undo step takes both layers away; the 3D history has the camera.
        let n = st.doc.walk().len();
        assert!(s.undo());
        assert_eq!(s.active().expect("doc").doc.walk().len(), n - 2);
        ed.run("edit.undo", &Value::Null).expect("undo");
        assert!(ed.scene.sequence.shots.iter().all(|x| x.id != cam));
        assert_ne!(ed.scene.uid, 0, "undo keeps the note's uid");
    }

    #[test]
    fn pinned_layers_hold_their_angle_and_live_ones_follow() {
        let mut ed = note();
        let mut s = session(200, 120);
        let pinned = shoot(&mut s, &mut ed, Shoot::Pinned, Model3dFit::Height).expect("pinned")["layer"].as_u64().expect("id");
        let live = shoot(&mut s, &mut ed, Shoot::Live, Model3dFit::Height).expect("live")["layer"].as_u64().expect("id");
        assert!(has_layers_of(&s, &ed, true));
        let (p0, l0) = (layer(&s, pinned).cache, layer(&s, live).cache);
        ed.run("camera.orbit", &json!({"dx": -100.0})).expect("orbit");
        assert_eq!(sync(&mut s, &mut ed, true).expect("sync"), 2);
        assert!(layer(&s, pinned).cache == p0, "the pinned angle holds");
        assert!(layer(&s, live).cache != l0, "the live layer follows");
        // Both share one copy of the note.
        assert!(std::sync::Arc::ptr_eq(&layer(&s, pinned).scene, &layer(&s, live).scene));
        // Shoot from a listed camera; an unknown one is refused.
        let cam = layer(&s, pinned).main_camera_id;
        assert!(shoot(&mut s, &mut ed, Shoot::Camera(cam), Model3dFit::Width).is_ok());
        assert!(shoot(&mut s, &mut ed, Shoot::Camera(999_999), Model3dFit::Height).is_err());
    }

    #[test]
    fn editing_a_layer_opens_its_note_without_losing_the_open_one() {
        let mut ed = note();
        let mut s = session(160, 100);
        let r = shoot(&mut s, &mut ed, Shoot::Pinned, Model3dFit::Height).expect("shoot");
        let id = LayerId(r["layer"].as_u64().expect("id"));
        let uid = ed.scene.uid;
        // A different, unsaved note is open: refused.
        let mut other = note();
        assert!(edit_layer(&mut s, &mut other, id).is_err());
        assert_eq!(other.scene.strokes.len(), 1);
        // An empty note gives way; the view goes to the layer's camera.
        let mut fresh = Editor::new();
        fresh.set_viewport(640.0, 400.0);
        edit_layer(&mut s, &mut fresh, id).expect("opens");
        assert_eq!((fresh.scene.uid, fresh.scene.strokes.len()), (uid, 1));
        let cam = fresh.scene.sequence.shots.iter().find(|x| Some(x.id) == r["camera"].as_u64()).map(|x| x.camera).expect("camera");
        assert_eq!((fresh.camera.yaw, fresh.camera.pitch), (cam.yaw, cam.pitch));
        assert!(edit_layer(&mut s, &mut fresh, LayerId(424_242)).is_err());
    }

    #[test]
    fn saved_layers_render_again_without_the_note_open() {
        let mut ed = note();
        let mut s = session(120, 80);
        let id = shoot(&mut s, &mut ed, Shoot::Live, Model3dFit::Height).expect("shoot")["layer"].as_u64().expect("id");
        assert_eq!(rerender_saved(&mut s, None).expect("render"), 1);
        assert_eq!(rerender_saved(&mut s, Some(LayerId(id))).expect("render"), 1);
        let plain = s.active().and_then(|st| st.active_layer).expect("trace-on layer");
        assert!(rerender_saved(&mut s, Some(plain)).is_err());
        assert!(fit_name(Model3dFit::Contain) == "contain" && parse_fit("width") == Some(Model3dFit::Width) && parse_fit("x").is_none());
    }
}
