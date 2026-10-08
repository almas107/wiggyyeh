//! Stage 5: every PhotoCraft feature is reachable in WobbleWorks. The editor keeps PhotoCraft's
//! menu bar, tools and panels, and every live menu item opens or runs inside WobbleWorks (with
//! the hand-drawn pass, pixel font and juice on top) without crashing.

use egui_kittest::Harness;
use photocraft_ui_egui::menu_catalog::CATALOG;
use photocraft_ui_egui::{Services, parity};
use serde_json::json;
use wobbleworks_app::WobbleApp;
use wobbleworks_app::io::codec_services;

fn harness() -> Harness<'static, WobbleApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 820.0)).with_max_steps(8).build_eframe(|cc| {
        photocraft_ui_egui::PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        // No pickers or writers: nothing can block on a dialog or touch the disk.
        WobbleApp::new(Services { ..codec_services() })
    });
    h.run_steps(3);
    h
}

#[test]
fn the_full_editor_is_showing() {
    let h = harness();
    let ui = &h.state().app.ui;
    assert_eq!(ui.view.screen_mode, "standard", "PhotoCraft's menu bar and panels are on screen");
    assert!(ui.panels.toolbar && ui.panels.options_bar);
    let p = parity::compute();
    assert!(p.live >= parity::FLOOR, "{} live menu items", p.live);
}

/// Items that would leave the test (quit the app) or only open web pages.
fn skipped(id: &str) -> bool {
    id == "file.exit" || id.starts_with("help.") || id == "file.close" || id == "file.closeAll" || id == "file.closeOthers"
}

#[test]
#[ignore = "opens every menu item (about a minute); run with --ignored"]
fn every_live_menu_item_opens_inside_wobbleworks() {
    let mut h = harness();
    // A selection and some pixels, so selection- and pixel-dependent items have work to do.
    h.state_mut().stroke(&[(100.0, 100.0), (600.0, 300.0)], "#ff2e88", 30.0).unwrap();
    let _ = h.state_mut().run("select.rect", json!({"x": 50, "y": 50, "width": 400, "height": 300}));
    let mut seen = std::collections::HashSet::new();
    let mut ran = 0;
    for &(_, _, _, id) in CATALOG {
        if id == "---" || skipped(id) || !seen.insert(id) {
            continue;
        }
        if std::env::var("WOBBLE_TRACE").is_ok() {
            eprintln!("invoke {id}");
        }
        let ctx = h.ctx.clone();
        // Errors (disabled items, nothing selected) are fine; panics are not.
        let _ = photocraft_ui_egui::menus::invoke(&mut h.state_mut().app, &ctx, id, json!({}));
        h.run_steps(1);
        ran += 1;
        // Keep a document around for the next item.
        if h.state().app.session.active().is_none() {
            h.state_mut().new_picture().unwrap();
        }
    }
    assert!(ran > 500, "{ran} items");
}

/// Regression: a second window on the document embeds a second canvas (no extra OS windows in
/// tests or on the web); its canvas widgets used to reuse the main canvas's ids and trip egui's
/// "widget changed layer" check.
#[test]
fn a_second_window_on_the_document_renders() {
    let mut h = harness();
    for id in ["window.arrange.floatAllInWindows", "window.arrange.newWindowForDocument", "window.arrange.matchZoom"] {
        let ctx = h.ctx.clone();
        let _ = photocraft_ui_egui::menus::invoke(&mut h.state_mut().app, &ctx, id, json!({}));
        h.run_steps(1);
    }
    h.run_steps(3);
}
