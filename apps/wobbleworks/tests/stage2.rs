//! Stage 2: wiggle layers on PhotoCraft documents. Everything drawn boils (strokes land on every
//! boil frame), playback flips frames without touching history, and the boil exports as an
//! animated GIF, a PNG sequence or a layered PSD.

use std::cell::RefCell;
use std::rc::Rc;

use photocraft_engine::wiggle_cmds;
use photocraft_ui_egui::Services;
use serde_json::json;
use wobbleworks::WobbleApp;
use wobbleworks::io::codec_services;

type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

fn app() -> (WobbleApp, Written) {
    let written: Written = Rc::default();
    let sink = written.clone();
    let services = Services {
        write: Some(Box::new(move |path: &str, bytes: &[u8]| {
            sink.borrow_mut().push((path.to_string(), bytes.to_vec()));
            Ok(())
        })),
        pick_save: Some(Box::new(|suggested: &str| Some(suggested.to_string()))),
        ..codec_services()
    };
    (WobbleApp::new(services), written)
}

/// Ink (alpha) on each frame of the active wiggle layer.
fn ink_per_frame(w: &WobbleApp) -> Vec<f32> {
    let st = w.app.session.active().unwrap();
    let wig = wiggle_cmds::wiggle_of(&st.doc, st.active_layer.unwrap()).unwrap();
    wiggle_cmds::frames(wig)
        .iter()
        .map(|id| {
            let s = st.doc.layer(*id).unwrap().surface().unwrap();
            s.read_region(photocraft_geom::Rect::from_xywh(0, 0, 1200, 800)).chunks(4).map(|p| p[3]).sum()
        })
        .collect()
}

#[test]
fn new_pictures_start_on_a_wiggle_layer() {
    let (w, _) = app();
    assert!(w.on_wiggle_layer());
    let st = w.app.session.active().unwrap();
    assert_eq!(wiggle_cmds::frame_count(&st.doc), 3);
}

#[test]
fn strokes_boil_on_every_frame_and_undo_as_one() {
    let (mut w, _) = app();
    let before = w.app.session.active().unwrap().history.past_len();
    w.stroke(&[(100.0, 100.0), (400.0, 160.0)], "#ff2e88", 20.0).unwrap();
    let ink = ink_per_frame(&w);
    assert!(ink.iter().all(|&a| a > 100.0), "{ink:?}");
    assert_eq!(w.app.session.active().unwrap().history.past_len(), before + 1);
    w.run("edit.undo", json!({})).unwrap();
    assert!(ink_per_frame(&w).iter().all(|&a| a == 0.0));
}

#[test]
fn a_stroke_made_with_photocrafts_brush_is_spread_onto_every_frame() {
    let (mut w, _) = app();
    w.spread_to_frames();
    // What PhotoCraft's canvas does on release: one paint.stroke on the active frame.
    w.run("paint.stroke", json!({"points": [[50, 50], [300, 80]], "size": 16, "seed": 7})).unwrap();
    let ink = ink_per_frame(&w);
    assert!(ink[0] > 0.0 && ink[1] == 0.0, "only the active frame so far: {ink:?}");
    let before = w.app.session.active().unwrap().history.past_len();
    w.spread_to_frames();
    let ink = ink_per_frame(&w);
    assert!(ink.iter().all(|&a| a > 0.0), "spread: {ink:?}");
    assert_eq!(w.app.session.active().unwrap().history.past_len(), before, "still one undo step");
    // Spreading again does nothing new.
    w.spread_to_frames();
    assert_eq!(w.app.session.active().unwrap().history.past_len(), before);
}

#[test]
fn playback_flips_frames_without_history_or_unsaved_changes() {
    let (mut w, _) = app();
    w.save_psd(Some("boil.psd".into())).unwrap();
    let undo = w.app.session.active().unwrap().history.past_len();
    let shown = |w: &WobbleApp| -> usize {
        let st = w.app.session.active().unwrap();
        let wig = wiggle_cmds::all(&st.doc)[0];
        wiggle_cmds::frames(wig).iter().position(|id| st.doc.layer(*id).unwrap().visible).unwrap()
    };
    w.play_boil(0.0);
    let a = shown(&w);
    w.play_boil(1.0 / f64::from(w.boil_fps) + 0.001);
    let b = shown(&w);
    assert_ne!(a, b);
    assert_eq!(w.app.session.active().unwrap().history.past_len(), undo);
    assert!(!w.app.session.active().unwrap().is_dirty());
    w.boil_play = false;
    w.play_boil(9.0);
    assert_eq!(shown(&w), b, "paused");
    w.play_boil(f64::NAN);
}

#[test]
fn the_boil_exports_as_gif_png_frames_and_a_layered_psd() {
    let (mut w, written) = app();
    w.stroke(&[(100.0, 400.0), (1100.0, 420.0)], "#2f5bff", 30.0).unwrap();
    let gif = w.export_gif(Some("boil".into())).unwrap();
    assert_eq!(gif, "boil.gif");
    assert_eq!(&written.borrow().last().unwrap().1[..6], b"GIF89a");
    let pngs = w.export_png_sequence(Some("boil.png".into())).unwrap();
    assert_eq!(pngs, ["boil_1.png", "boil_2.png", "boil_3.png"]);
    let psd = w.save_psd(Some("boil".into())).unwrap();
    let bytes = written.borrow().iter().rev().find(|(p, _)| *p == psd).unwrap().1.clone();
    let (doc, _) = wobbleworks::io::import(&psd, &bytes).unwrap();
    let wigs = wiggle_cmds::all(&doc);
    assert_eq!(wigs.len(), 1, "the wiggle layer survives a PSD round trip");
    assert_eq!(wiggle_cmds::frames(wigs[0]).len(), 3);
}

#[test]
fn old_wobbleworks_projects_open_through_photocrafts_open() {
    let (mut w, _) = app();
    let wob = json!({"format": "wobbleworks", "version": 2, "w": 80, "h": 60, "settings": {"frames": 4},
        "layers": [{"name": "Lines", "strokes": [{"t": "marker", "c": "#2f5bff", "z": 5, "s": 9, "p": [5, 30, 75, 30]}]}]});
    w.app.open_bytes("old.wob", wob.to_string().as_bytes()).unwrap();
    let st = w.app.session.active().unwrap();
    assert_eq!((st.doc.size.width, st.doc.size.height), (80, 60));
    assert_eq!(wiggle_cmds::frame_count(&st.doc), 4);
    assert!(w.app.open_bytes("bad.wob", b"{").is_err());
}
