//! Stage 1: WobbleWorks opens, draws and saves PSDs through PhotoCraft's engine, and never panics
//! on bad input.

use std::cell::RefCell;
use std::rc::Rc;

use photocraft_engine::Session;
use photocraft_ui_egui::Services;
use photocraft_ui_egui::state::Tool;
use serde_json::{Value, json};
use wobbleworks_app::WobbleApp;
use wobbleworks_app::io::codec_services;

type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// A WobbleWorks app whose saves land in memory, and the files it wrote.
fn app() -> (WobbleApp, Written) {
    let written: Written = Rc::default();
    let sink = written.clone();
    let services = Services {
        write: Some(Box::new(move |path: &str, bytes: &[u8]| {
            sink.borrow_mut().push((path.to_string(), bytes.to_vec()));
            Ok(())
        })),
        pick_save: Some(Box::new(|suggested: &str| Some(format!("picked/{suggested}")))),
        ..codec_services()
    };
    (WobbleApp::new(services), written)
}

/// The composite pixel of `doc` at (x, y), as RGBA floats.
fn pixel(doc: photocraft_doc::Document, x: i32, y: i32) -> Vec<f64> {
    let mut s = Session::new();
    s.add_document(doc, None);
    let px = s.execute("document.pixel", json!({"x": x, "y": y})).unwrap();
    px.as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect()
}

fn is(px: &[f64], rgb: [f64; 3]) -> bool {
    px.iter().zip(rgb).all(|(a, b)| (a - b).abs() < 0.02)
}

#[test]
fn starts_with_a_blank_picture_and_the_brush() {
    let (w, _) = app();
    let st = w.app.session.active().unwrap();
    assert_eq!((st.doc.size.width, st.doc.size.height), wobbleworks_app::shell::NEW_SIZE);
    assert_eq!(w.app.ui.tool, Tool::Brush);
    assert!(!w.app.ui.status_error, "{}", w.app.ui.status);
}

#[test]
fn draw_save_psd_and_reopen() {
    let (mut w, written) = app();
    w.stroke(&[(100.0, 100.0), (300.0, 120.0), (500.0, 100.0)], "#ff0000", 30.0).unwrap();
    assert!(w.app.session.is_enabled("edit.undo"), "the stroke is one history step");
    let path = w.save_psd(Some("out/wobble".into())).unwrap();
    assert_eq!(path, "out/wobble.psd");
    let (name, bytes) = written.borrow().last().cloned().unwrap();
    assert_eq!(name, "out/wobble.psd");
    assert_eq!(&bytes[..4], b"8BPS", "a Photoshop document");
    assert!(!w.app.session.active().unwrap().is_dirty(), "saved");

    let (doc, _) = wobbleworks_app::io::import(&name, &bytes).unwrap();
    assert_eq!((doc.size.width, doc.size.height), (1200, 800));
    assert!(is(&pixel(doc.clone(), 300, 120), [1.0, 0.0, 0.0]), "the stroke survived the round trip");
    assert!(is(&pixel(doc, 1100, 700), [1.0, 1.0, 1.0]), "the paper stays white");

    // Open the saved PSD, draw on it, and save it again: through the save dialog this time.
    w.app.open_bytes("wobble.psd", &bytes).unwrap();
    assert_eq!(w.app.session.documents().len(), 2);
    w.stroke(&[(100.0, 600.0), (400.0, 600.0)], "#0000ff", 20.0).unwrap();
    let path = w.save_psd(None).unwrap();
    assert_eq!(path, "picked/wobble.psd");
    let (_, bytes) = written.borrow().last().cloned().unwrap();
    let (doc, _) = wobbleworks_app::io::import(&path, &bytes).unwrap();
    assert!(is(&pixel(doc.clone(), 250, 600), [0.0, 0.0, 1.0]));
    assert!(is(&pixel(doc, 300, 120), [1.0, 0.0, 0.0]));
}

#[test]
fn palette_sets_the_foreground_and_new_starts_over() {
    let (mut w, _) = app();
    w.pick_colour(1).unwrap();
    assert_eq!(w.foreground(), egui::Color32::from_rgb(0xff, 0x2e, 0x88));
    w.new_picture().unwrap();
    assert_eq!(w.app.session.documents().len(), 2);
    // The brush paints the palette colour when the stroke names none.
    w.run("paint.stroke", json!({"points": [[50, 50], [200, 50]], "size": 20})).unwrap();
    let doc = (*w.app.session.active().unwrap().doc).clone();
    assert!(is(&pixel(doc, 120, 50), [1.0, 0.18, 0.533]), "pink");
}

#[test]
fn bad_input_is_an_error_not_a_panic() {
    let (mut w, _) = app();
    assert!(w.stroke(&[], "#ff0000", 10.0).is_err());
    assert!(w.stroke(&[(f64::NAN, 0.0)], "#ff0000", 10.0).is_err());
    assert!(w.stroke(&[(1e300, 0.0), (0.0, 0.0)], "#ff0000", 10.0).is_err());
    assert!(w.pick_colour(99).is_err());
    assert!(w.app.open_bytes("bad.psd", b"8BPS\0\x01 not really").is_err());
    assert!(w.app.open_bytes("empty.png", &[]).is_err());
    assert!(w.run("no.such.command", json!({})).is_err());
    assert!(w.run("paint.stroke", Value::Null).is_err());
    // No pictures open: drawing and saving fail politely.
    while w.app.session.active_index().is_some() {
        let i = w.app.session.active_index().unwrap();
        w.app.session.close(i);
    }
    assert!(w.save_psd(Some("x.psd".into())).is_err());
    assert!(w.stroke(&[(1.0, 1.0)], "#000000", 4.0).is_err());
    // A save dialog that is cancelled is not written.
    let mut w = WobbleApp::new(Services { pick_save: Some(Box::new(|_: &str| None)), ..codec_services() });
    assert_eq!(w.save_psd(None), Err("cancelled".into()));
}

#[test]
fn recent_colours_follow_painting_and_the_palette_is_editable() {
    let (mut w, _) = app();
    w.track_recent();
    assert!(w.recent.is_empty(), "nothing painted yet");
    w.pick_colour(2).unwrap();
    w.track_recent();
    assert!(w.recent.is_empty(), "picking a colour isn't painting");
    w.run("paint.stroke", json!({"points": [[10, 10], [60, 10]], "size": 8})).unwrap();
    w.track_recent();
    assert_eq!(w.recent.first(), Some(&w.foreground()));
    w.run("paint.stroke", json!({"points": [[10, 30], [60, 30]], "size": 8})).unwrap();
    w.track_recent();
    assert_eq!(w.recent.len(), 1, "no duplicates");

    let before = w.palette.len();
    assert!(!w.add_to_palette(), "already in the palette");
    w.set_foreground(egui::Color32::from_rgb(1, 2, 3)).unwrap();
    assert!(w.add_to_palette());
    assert_eq!(w.palette.len(), before + 1);

    // Saved and restored between sessions; junk is ignored.
    let saved = w.colours_json();
    let (mut w2, _) = app();
    w2.restore_colours(&saved);
    assert_eq!(w2.palette, w.palette);
    assert_eq!(w2.recent, w.recent);
    w2.restore_colours("not json");
    w2.restore_colours(r##"{"palette": [], "recent": [42, "#zz", "#010203"]}"##);
    assert_eq!(w2.palette, w.palette, "an empty palette is not restored");
    assert_eq!(w2.recent, vec![egui::Color32::from_rgb(1, 2, 3)]);
}

#[test]
fn save_export_and_new_go_through_photocraft() {
    let (mut w, written) = app();
    let png = w.export_png(Some("out/pic".into())).unwrap();
    assert_eq!(png, "out/pic.png");
    assert_eq!(&written.borrow().last().unwrap().1[1..4], b"PNG");
    assert!(w.app.session.active().unwrap().path.is_none(), "export leaves the picture's own file alone");
}

mod ui {
    use super::*;
    use egui_kittest::Harness;

    fn harness(size: egui::Vec2) -> Harness<'static, WobbleApp> {
        Harness::builder().with_size(size).with_max_steps(16).build_eframe(|cc| {
            photocraft_ui_egui::PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            app().0
        })
    }

    fn press(h: &mut Harness<'_, WobbleApp>, pos: egui::Pos2, pressed: bool) {
        h.input_mut().events.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
    }

    /// A real pointer drag on the canvas paints through PhotoCraft's Brush tool.
    #[test]
    fn pointer_drag_paints_with_the_brush() {
        let mut h = harness(egui::vec2(1280.0, 820.0));
        h.run_steps(4);
        let rev = h.state().app.session.active().unwrap().revision;
        let start = egui::pos2(500.0, 420.0);
        h.input_mut().events.push(egui::Event::PointerMoved(start));
        h.run_steps(1);
        press(&mut h, start, true);
        h.run_steps(1);
        for i in 1..=12 {
            h.input_mut().events.push(egui::Event::PointerMoved(start + egui::vec2(i as f32 * 10.0, i as f32 * 4.0)));
            h.run_steps(1);
        }
        press(&mut h, start + egui::vec2(120.0, 48.0), false);
        h.run_steps(3);
        let st = h.state().app.session.active().unwrap();
        assert!(st.revision > rev, "the drag painted (status: {})", h.state().app.ui.status);
        assert!(h.state().app.session.is_enabled("edit.undo"));
    }

    /// The editor runs headless at desktop and phone widths, with the mixer open, without
    /// panicking; the pixel font gets installed.
    #[test]
    fn renders_at_desktop_and_phone_width() {
        for size in [egui::vec2(1280.0, 820.0), egui::vec2(390.0, 760.0)] {
            let mut h = harness(size);
            h.run_steps(4);
            h.state_mut().show_mixer = true;
            h.run_steps(4);
            let has_font = h.ctx.fonts(|f| f.definitions().font_data.contains_key(wobbleworks_app::ttf::FONT_NAME));
            assert!(has_font);
        }
    }

    /// Offscreen screenshot through wgpu, for looking at the UI:
    /// `WOBBLE_SNAPSHOT=shot.png` (and `WOBBLE_MIXER=1`, `WOBBLE_WIDTH=390`, `WOBBLE_HOVER=x,y`):
    /// `cargo test -p wobbleworks-app snapshot -- --ignored`.
    #[test]
    #[ignore = "needs a GPU or software renderer; run on demand"]
    fn snapshot() {
        let out = std::env::var("WOBBLE_SNAPSHOT").unwrap_or_else(|_| "wobbleworks-app.png".into());
        let w: f32 = std::env::var("WOBBLE_WIDTH").ok().and_then(|s| s.parse().ok()).unwrap_or(1280.0);
        let mut h = Harness::builder().with_size(egui::vec2(w, 820.0)).with_pixels_per_point(1.0).with_max_steps(32).wgpu().build_eframe(|cc| {
            photocraft_ui_egui::PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut a = app().0;
            a.hand_drawn = std::env::var("WOBBLE_FLAT").is_err();
            a.pixel_font = std::env::var("WOBBLE_PLAIN_FONT").is_err();
            a.custom_icons = std::env::var("WOBBLE_SVG_ICONS").is_err();
            if let Some(rs) = cc.wgpu_render_state.clone() {
                a.app.set_wgpu(rs);
            }
            a
        });
        h.run_steps(3);
        h.state_mut().stroke(&[(150.0, 200.0), (400.0, 300.0), (700.0, 220.0), (1000.0, 380.0)], "#ff2e88", 28.0).unwrap();
        h.state_mut().stroke(&[(200.0, 600.0), (600.0, 520.0), (900.0, 640.0)], "#2f7bff", 18.0).unwrap();
        h.state_mut().show_mixer = std::env::var("WOBBLE_MIXER").is_ok();
        h.state_mut().track_recent();
        h.run_steps(6);
        if let Some((x, y)) = std::env::var("WOBBLE_HOVER").ok().and_then(|s| s.split_once(',').and_then(|(x, y)| Some((x.parse().ok()?, y.parse().ok()?)))) {
            h.input_mut().events.push(egui::Event::PointerMoved(egui::pos2(x, y)));
            for _ in 0..40 {
                h.step();
            }
        }
        h.render().unwrap().save(&out).unwrap();
    }
}
