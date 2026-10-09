//! The 3D mode inside WobbleWorks, driven through real egui input: drawing with the mouse,
//! Blender's keys (G, X, typing a number, Enter), middle-drag orbit, guides, every panel and
//! menu, and files.

use std::cell::RefCell;
use std::rc::Rc;

use egui::{Event, Key, Modifiers, PointerButton, Pos2, pos2};
use egui_kittest::Harness;
use photocraft_ui_egui::Services;
use serde_json::json;
use wobbleworks::WobbleApp;
use wobbleworks::io::codec_services;
use wobbleworks::space3d::{Menu, SideTab, StageTab};

type Saved = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

fn app() -> (WobbleApp, Saved) {
    let mut w = WobbleApp::new(Services { ..codec_services() });
    let saved: Saved = Rc::default();
    let sink = saved.clone();
    w.space.files.save = Some(Box::new(move |name: &str, bytes: &[u8]| {
        sink.borrow_mut().push((name.to_string(), bytes.to_vec()));
        Ok(Some(format!("saved/{name}")))
    }));
    w.set_three_d(true);
    (w, saved)
}

fn harness() -> Harness<'static, WobbleApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_pixels_per_point(1.0).with_max_steps(16).build_eframe(|cc| {
        photocraft_ui_egui::PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app().0
    });
    h.run_steps(4);
    h
}

fn press(h: &mut Harness<'static, WobbleApp>, key: Key, modifiers: Modifiers) {
    h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    h.step();
    h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
    h.step();
}

fn button(h: &mut Harness<'static, WobbleApp>, p: Pos2, button: PointerButton, pressed: bool) {
    h.input_mut().events.push(Event::PointerButton { pos: p, button, pressed, modifiers: Modifiers::NONE });
    h.step();
}

fn drag(h: &mut Harness<'static, WobbleApp>, pts: &[Pos2], b: PointerButton) {
    let Some(first) = pts.first().copied() else { return };
    h.input_mut().events.push(Event::PointerMoved(first));
    h.step();
    button(h, first, b, true);
    for p in pts.iter().skip(1) {
        h.input_mut().events.push(Event::PointerMoved(*p));
        h.step();
    }
    button(h, pts.last().copied().unwrap_or(first), b, false);
}

fn view_centre(h: &Harness<'static, WobbleApp>) -> Pos2 {
    h.state().space.view.rect.center()
}

#[test]
fn starts_in_2d_and_the_3d_button_switches() {
    let w = WobbleApp::new(Services { ..codec_services() });
    assert!(!w.three_d);
    let (w, _) = app();
    assert!(w.three_d);
}

#[test]
fn drawing_with_the_mouse_makes_a_3d_curve() {
    let mut h = harness();
    let c = view_centre(&h);
    assert!(h.state().space.view.rect.width() > 300.0, "{:?}", h.state().space.view.rect);
    let pts: Vec<Pos2> = (0..30).map(|i| pos2(c.x - 150.0 + i as f32 * 10.0, c.y + (i as f32 * 0.3).sin() * 40.0)).collect();
    drag(&mut h, &pts, PointerButton::Primary);
    h.run_steps(2);
    let ed = &h.state().space.ed;
    assert_eq!(ed.scene.strokes.len(), 1, "{}", ed.status);
    assert!(ed.scene.strokes[0].points.len() > 10);
}

#[test]
fn blender_keys_move_the_selection_exactly() {
    let mut h = harness();
    h.state_mut().space.ed.run("stroke.add", &json!({"points": [[0,0,0],[0.5,0,0]]})).unwrap();
    let c = view_centre(&h);
    h.input_mut().events.push(Event::PointerMoved(c));
    h.step();
    press(&mut h, Key::A, Modifiers::NONE);
    assert_eq!(h.state().space.ed.selection.len(), 1);
    press(&mut h, Key::G, Modifiers::NONE);
    assert!(h.state().space.ed.in_modal());
    press(&mut h, Key::X, Modifiers::NONE);
    press(&mut h, Key::Num2, Modifiers::NONE);
    press(&mut h, Key::Enter, Modifiers::NONE);
    let ed = &h.state().space.ed;
    assert!(!ed.in_modal());
    let p = ed.scene.strokes[0].points[0].p;
    assert!((p.x - 2.0).abs() < 1e-4 && p.y.abs() < 1e-4, "{p:?}");
    // Ctrl+Z undoes the move.
    press(&mut h, Key::Z, Modifiers::COMMAND);
    assert!(h.state().space.ed.scene.strokes[0].points[0].p.x.abs() < 1e-4);
}

#[test]
fn middle_drag_orbits_and_numpad_keys_snap_views() {
    let mut h = harness();
    let c = view_centre(&h);
    let yaw = h.state().space.ed.camera.yaw;
    drag(&mut h, &[c, pos2(c.x + 40.0, c.y), pos2(c.x + 90.0, c.y + 10.0)], PointerButton::Middle);
    assert!((h.state().space.ed.camera.yaw - yaw).abs() > 5.0);
    assert!(h.state().space.ed.scene.strokes.is_empty(), "navigation does not draw");
    press(&mut h, Key::Num1, Modifiers::NONE);
    let cam = h.state().space.ed.camera;
    assert!(cam.orthographic && cam.yaw.abs() < 1e-4 && cam.pitch.abs() < 1e-4);
    press(&mut h, Key::Num7, Modifiers::CTRL);
    assert!((h.state().space.ed.camera.pitch + 90.0).abs() < 1e-4, "Ctrl+7: bottom");
}

#[test]
fn q_draws_a_guide_and_strokes_land_on_it() {
    let mut h = harness();
    press(&mut h, Key::Num1, Modifiers::NONE);
    let c = view_centre(&h);
    press(&mut h, Key::Q, Modifiers::NONE);
    let circle: Vec<Pos2> = (0..=48).map(|i| {
        let a = std::f32::consts::TAU * i as f32 / 48.0;
        pos2(c.x + 120.0 * a.cos(), c.y + 120.0 * a.sin())
    })
    .collect();
    drag(&mut h, &circle, PointerButton::Primary);
    assert!(h.state().space.ed.scene.active_guide().is_some(), "{}", h.state().space.ed.status);
    // After the guide, the tool is Draw again; draw across it from the side.
    press(&mut h, Key::Num3, Modifiers::NONE);
    let line: Vec<Pos2> = (0..20).map(|i| pos2(c.x, c.y - 100.0 + i as f32 * 10.0)).collect();
    drag(&mut h, &line, PointerButton::Primary);
    assert_eq!(h.state().space.ed.scene.strokes.len(), 1);
    // Esc closes the guide.
    press(&mut h, Key::Escape, Modifiers::NONE);
    assert!(h.state().space.ed.scene.active_guide().is_none());
}

#[test]
fn every_panel_tab_and_menu_shows_without_trouble() {
    let mut h = harness();
    h.state_mut().space.ed.run("stroke.add", &json!({"points": [[0,0,0],[1,1,0]]})).unwrap();
    h.state_mut().space.ed.run("select.all", &json!(null)).unwrap();
    for tab in [SideTab::Stage, SideTab::Boil, SideTab::Shots, SideTab::Item, SideTab::History, SideTab::Keys, SideTab::Help] {
        h.state_mut().space.side = tab;
        for st in [StageTab::Groups, StageTab::Resources, StageTab::Environment] {
            h.state_mut().space.stage = st;
            h.run_steps(2);
        }
    }
    let at = view_centre(&h);
    for m in [
        Menu::Add,
        Menu::Delete,
        Menu::Flip,
        Menu::MoveToGroup,
        Menu::ViewPie,
        Menu::Search(String::new()),
        Menu::Size,
        Menu::Opacity,
        Menu::Rename(1, "G".into()),
        Menu::Context,
    ] {
        h.state_mut().space.menu = Some((m, at));
        h.run_steps(2);
    }
    for (render, left) in [(true, true), (false, false)] {
        h.state_mut().space.ed.run("env.set", &json!({"render": render, "fog": render, "lighting": {"groundShadow": true, "toon": true}})).unwrap();
        h.state_mut().space.left_handed = left;
        h.state_mut().space.hide_ui = left;
        h.run_steps(2);
    }
    for tool in ["draw", "shape", "erase", "vacuum", "select", "guide", "loft", "primitive", "injector", "eyedropper", "liquify"] {
        let _ = h.state_mut().space.ed.run("tool.set", &json!({"tool": tool}));
        h.run_steps(2);
    }
}

#[test]
fn save_open_and_exports_go_through_the_platform() {
    let (mut w, saved) = app();
    w.space.ed.set_viewport(200.0, 120.0);
    w.space.ed.run("stroke.add", &json!({"points": [[0,0,0],[1,1,0]]})).unwrap();
    w.space.save(false);
    w.space.export_png();
    w.space.export_gif(false);
    w.space.export_obj();
    w.space.export_glb();
    let names: Vec<String> = saved.borrow().iter().map(|(n, _)| n.clone()).collect();
    assert_eq!(names, vec!["Note.wob3d", "WobbleWorks 3D.png", "WobbleWorks 3D.gif", "WobbleWorks 3D.obj", "WobbleWorks 3D.glb"]);
    let note = saved.borrow()[0].1.clone();
    let mut w2 = app().0;
    w2.space.receive("open", "n.wob3d", &note).unwrap();
    assert_eq!(w2.space.ed.scene.strokes.len(), 1);
    // Settings survive a restart, keymap included.
    w.space.ed.run("keymap.rebind", &json!({"action": "transform.grab", "chord": "Ctrl+G"})).unwrap();
    let text = w.settings_json();
    let mut w3 = app().0;
    w3.restore_settings(&text);
    assert_eq!(w3.space.ed.keymap.chord("transform.grab"), Some("Ctrl+G"));
}

/// Offscreen screenshot of the 3D mode through wgpu:
/// `WOBBLE3D_SNAPSHOT=shot.png cargo test -p wobbleworks --test three_d snapshot -- --ignored`
/// (`WOBBLE3D_RENDER=1` for render mode, `WOBBLE3D_WIDTH` for the window width).
#[test]
#[ignore = "needs a GPU or software renderer; run on demand"]
fn snapshot() {
    let out = std::env::var("WOBBLE3D_SNAPSHOT").unwrap_or_else(|_| "wobble3d.png".into());
    let width: f32 = std::env::var("WOBBLE3D_WIDTH").ok().and_then(|s| s.parse().ok()).unwrap_or(1440.0);
    let mut h = Harness::builder().with_size(egui::vec2(width, 900.0)).with_pixels_per_point(1.0).with_max_steps(32).wgpu().build_eframe(|cc| {
        photocraft_ui_egui::PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut a = app().0;
        if let Some(rs) = cc.wgpu_render_state.clone() {
            if std::env::var("WOBBLE3D_MESHES").is_err() {
                a.space.set_gpu(&rs);
            }
            a.app.set_wgpu(rs);
        }
        a
    });
    h.run_steps(4);
    {
        let ed = &mut h.state_mut().space.ed;
        let r = |ed: &mut wobbleworks_3d::editor::Editor, c: &str, p: serde_json::Value| ed.run(c, &p).unwrap();
        r(ed, "camera.view", json!({"view": "front"}));
        let vp = ed.camera.viewport;
        let (cx, cy) = (vp.width / 2.0, vp.height / 2.0);
        let circle: Vec<[f32; 2]> = (0..=64).map(|i| {
            let a = std::f32::consts::TAU * i as f32 / 64.0;
            [cx + 110.0 * a.cos(), cy + 110.0 * a.sin()]
        })
        .collect();
        r(ed, "guide.draw", json!({"points": circle}));
        r(ed, "camera.set", json!({"yaw": 35, "pitch": 18, "orthographic": false}));
        let kinds = ["pen", "ink", "oil", "gouache", "marker", "chalk"];
        let colours = ["#23222b", "#111111", "#ff8a1f", "#e8344e", "#2f5bff", "#2ec27e"];
        for (i, (k, c)) in kinds.iter().zip(colours).enumerate() {
            r(ed, "brush.set", json!({"kind": k, "color": c, "size": 14 + i * 4}));
            let x0 = cx - 120.0 + i as f32 * 48.0;
            let pts: Vec<[f32; 3]> = (0..40).map(|j| {
                let t = j as f32 / 39.0;
                [x0 + 18.0 * (t * 6.0).sin(), cy - 150.0 + t * 300.0, 0.4 + 0.6 * (t * 3.1).sin().abs()]
            })
            .collect();
            r(ed, "stroke.draw", json!({"points": pts}));
        }
        if std::env::var("WOBBLE3D_RENDER").is_ok() {
            r(ed, "env.set", json!({"render": true, "lighting": {"groundShadow": true}}));
        }
        r(ed, "tool.set", json!({"tool": "select"}));
        r(ed, "select.set", json!({"ids": [ed.scene.strokes[2].id]}));
    }
    h.run_steps(8);
    h.render().unwrap().save(&out).unwrap();
}
