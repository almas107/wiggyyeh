//! WobbleWorks: a cute drawing app where every line boils.
//!
//! Native (eframe on wgpu) and web (the same Rust compiled to WebAssembly; build with
//! `trunk build --release` in this directory). See `README.md`.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
// Windows: no console window behind the app in release builds.
#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod app;
mod brush;
mod canvas;
mod fill;
mod geom;
mod history;
mod icons;
mod model;
mod panels;
mod pixels;
mod platform;
mod project;
mod render;
mod select;
mod settings;
mod store;
mod theme;
mod widgets;

use egui::{Frame, Margin, ScrollArea, Stroke};

/// The eframe shell around [`app::App`].
struct Wobble {
    app: app::App,
    tiles: canvas::Tiles,
}

impl Wobble {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Wobble { app: app::App::new(cc), tiles: canvas::Tiles::default() }
    }
}

impl eframe::App for Wobble {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Last-resort guard: an unexpected panic in a frame is reported, not fatal (native only;
        // wasm aborts on panic). Autosave keeps the drawing either way.
        #[cfg(not(target_arch = "wasm32"))]
        {
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.frame(ui, &ctx)));
            if res.is_err() {
                self.app.cancel_stroke();
                self.app.say("Oops, something went wrong in that frame. Your drawing is safe.");
            }
        }
        #[cfg(target_arch = "wasm32")]
        self.frame(ui, &ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.app.persist(storage);
    }

    fn on_exit(&mut self) {
        self.app.shutdown();
    }
}

impl Wobble {
    fn frame(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let a = &mut self.app;
        a.frame_logic(ctx);
        let look = a.look;
        let t = look.t;

        egui::Panel::top("wob-top").show_separator_line(false).frame(Frame::new().fill(t.card).inner_margin(Margin::symmetric(10, 7))).show(ui, |ui| {
            a.top_bar(ui);
            let r = ui.max_rect();
            ui.painter().hline(r.x_range(), r.bottom() + 7.0, Stroke::new(look.line + 1.0, t.ink));
        });
        egui::Panel::bottom("wob-status").show_separator_line(false).frame(Frame::new().fill(t.card).inner_margin(Margin::symmetric(10, 5))).show(ui, |ui| {
            let r = ui.max_rect();
            ui.painter().hline(r.x_range(), r.top() - 5.0, Stroke::new(look.line, t.ink));
            a.status_bar(ui);
        });
        if !a.focus {
            let side_frame = Frame::new().fill(t.paper).inner_margin(Margin { left: 10, right: 10, top: 10, bottom: 4 });
            let tools_w = if look.labels { 236.0 } else { 214.0 };
            let (tools, panels) = if a.s.swap_sides {
                (egui::Panel::right("wob-tools"), egui::Panel::left("wob-panels"))
            } else {
                (egui::Panel::left("wob-tools"), egui::Panel::right("wob-panels"))
            };
            tools.exact_size(tools_w).resizable(false).show_separator_line(false).frame(side_frame).show(ui, |ui| {
                ScrollArea::vertical().id_salt("tools-scroll").show(ui, |ui| a.tools_panel(ui));
            });
            panels.exact_size(a.s.panel_width).resizable(false).show_separator_line(false).frame(side_frame).show(ui, |ui| {
                ScrollArea::vertical().id_salt("panels-scroll").show(ui, |ui| a.side_panel(ui));
            });
        }
        egui::CentralPanel::default().frame(Frame::new().fill(t.paper)).show(ui, |ui| a.canvas_ui(ui, &mut self.tiles));
        a.dialogs(ctx);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("WobbleWorks")
            .with_app_id("ai.storyteller.wobbleworks")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([640.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native("WobbleWorks", options, Box::new(|cc| Ok(Box::new(Wobble::new(cc)))))
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast as _;
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else { return };
        let Some(canvas) = document.get_element_by_id("wobbleworks_canvas").and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok()) else {
            log::error!("missing <canvas id=\"wobbleworks_canvas\">");
            return;
        };
        platform::listen_pen(&canvas);
        // Browsers don't run `on_exit`: mark the session closed when the page goes away, so a
        // normal reload isn't reported as a crash.
        if let Some(w) = web_sys::window() {
            let cb = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(|| {
                if let Some(s) = store::Store::open() {
                    s.end_session();
                }
            });
            let _ = w.add_event_listener_with_callback("pagehide", cb.as_ref().unchecked_ref());
            cb.forget();
        }
        let result = eframe::WebRunner::new().start(canvas, eframe::WebOptions::default(), Box::new(|cc| Ok(Box::new(Wobble::new(cc))))).await;
        if let Some(el) = document.get_element_by_id("wobbleworks_loading") {
            match result {
                Ok(()) => el.remove(),
                Err(e) => el.set_inner_html(&format!("<p>WobbleWorks couldn't start: {e:?}</p><p>It needs a browser with WebGPU or WebGL2.</p>")),
            }
        }
    });
}

#[cfg(test)]
mod ui_tests {
    use super::*;
    use crate::model::{Brush, Tip};

    fn wobble(_cc: &eframe::CreationContext<'_>) -> Wobble {
        let s = settings::Settings { wobbly_ui: true, ..Default::default() };
        let mut app = app::App::with_settings(s, Some(store::Store::memory()));
        app.paused = true;
        Wobble { app, tiles: canvas::Tiles::default() }
    }

    /// Draw a little scene through the same calls the pointer handlers use.
    fn doodle(a: &mut app::App) {
        a.doc = model::Doc::new(640, 480);
        a.view.fit_pending = true;
        let stroke = |a: &mut app::App, b: Brush, c: u32, size: f64, pts: &[(f64, f64)]| {
            a.s.tools.brush = b;
            a.s.tools.size = size;
            a.set_color(egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8));
            a.begin_stroke(pts[0]);
            for p in &pts[1..] {
                a.feed_stroke(*p, false);
            }
            a.end_stroke();
        };
        let circle = |cx: f64, cy: f64, r: f64| -> Vec<(f64, f64)> {
            (0..=64).map(|i| f64::from(i) / 64.0 * std::f64::consts::TAU).map(|t| (cx + r * t.cos(), cy + r * t.sin())).collect()
        };
        stroke(a, Brush::Blob, 0xffd23f, 6.0, &circle(320.0, 230.0, 130.0));
        stroke(a, Brush::Marker, 0x17161c, 8.0, &circle(320.0, 230.0, 130.0));
        stroke(a, Brush::Blob, 0x17161c, 6.0, &circle(270.0, 200.0, 16.0));
        stroke(a, Brush::Blob, 0x17161c, 6.0, &circle(370.0, 200.0, 16.0));
        stroke(
            a,
            Brush::Ribbon,
            0xff2e88,
            12.0,
            &(0..=30).map(|i| f64::from(i) / 30.0 * std::f64::consts::PI).map(|t| (320.0 - 70.0 * t.cos(), 270.0 + 40.0 * t.sin())).collect::<Vec<_>>(),
        );
        stroke(a, Brush::Spray, 0x2ec4ff, 14.0, &(0..40).map(|i| (60.0 + f64::from(i) * 13.0, 430.0 + 10.0 * (f64::from(i) * 0.5).sin())).collect::<Vec<_>>());
        a.s.tools.tip = Tip::Star;
        stroke(a, Brush::Beads, 0x7b4dff, 16.0, &(0..20).map(|i| (80.0 + f64::from(i) * 6.0, 80.0 + 20.0 * (f64::from(i) * 0.4).sin())).collect::<Vec<_>>());
        a.s.tools.tip = Tip::Heart;
        stroke(a, Brush::Beads, 0xff2e88, 22.0, &[(540.0, 90.0), (560.0, 120.0)]);
        a.s.tools.tip = Tip::Round;
        a.s.tools.brush = Brush::Shaky;
    }

    /// The whole UI runs headless for a few frames (every panel, the settings and help windows)
    /// without panicking.
    #[test]
    fn ui_runs_headless() {
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_max_steps(8).build_eframe(|cc| wobble(cc));
        h.run_steps(2);
        doodle(&mut h.state_mut().app);
        h.run_steps(2);
        h.state_mut().app.show_settings = true;
        h.state_mut().app.show_help = true;
        h.state_mut().app.show_new = true;
        for tab in 0..5 {
            h.state_mut().app.settings_tab = tab;
            h.run_steps(2);
        }
        h.state_mut().app.finish_lasso(vec![(150.0, 80.0), (480.0, 80.0), (480.0, 400.0), (150.0, 400.0)]);
        h.state_mut().app.s.swap_sides = true;
        h.state_mut().app.s.show_labels = true;
        h.state_mut().app.s.tools.tool = settings::Tool::Fill;
        h.run_steps(3);
        h.state_mut().app.focus = true;
        h.run_steps(2);
        assert!(h.state().app.floating.is_some());
    }

    /// The web page has no script of its own (trunk injects the loader), names the canvas the
    /// app looks for, and offers a reload if the WebAssembly never arrives.
    #[test]
    fn web_page_is_script_free_and_recoverable() {
        let html = include_str!("../index.html");
        assert!(!html.contains("<script"));
        assert!(html.contains("id=\"wobbleworks_canvas\"") && html.contains("id=\"wobbleworks_loading\""));
        assert!(html.contains("<a href=\"\">Reload</a>"));
    }

    fn press(h: &mut egui_kittest::Harness<'_, Wobble>, pos: egui::Pos2, pressed: bool) {
        h.input_mut().events.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
    }

    /// Real pointer events on the canvas draw, fill, lasso and pan.
    #[test]
    fn pointer_input_drives_the_tools() {
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_max_steps(8).build_eframe(|cc| wobble(cc));
        h.run_steps(3);
        let centre = egui::pos2(640.0, 430.0);
        // Brush: press, move, release.
        h.input_mut().events.push(egui::Event::PointerMoved(centre));
        h.run_steps(1);
        press(&mut h, centre, true);
        h.run_steps(1);
        for i in 1..=10 {
            h.input_mut().events.push(egui::Event::PointerMoved(centre + egui::vec2(i as f32 * 6.0, i as f32 * 3.0)));
            h.run_steps(1);
        }
        press(&mut h, centre + egui::vec2(60.0, 30.0), false);
        h.run_steps(2);
        let strokes = h.state().app.doc.layers[0].strokes.clone();
        assert_eq!(strokes.len(), 1, "one stroke drawn");
        assert!(strokes[0].pts.len() > 5);
        assert!(h.state().app.hist.can_undo());
        // Fill tool: a click fills.
        h.state_mut().app.set_tool(settings::Tool::Fill);
        let corner = egui::pos2(500.0, 300.0);
        h.input_mut().events.push(egui::Event::PointerMoved(corner));
        h.run_steps(1);
        press(&mut h, corner, true);
        h.run_steps(1);
        press(&mut h, corner, false);
        h.run_steps(2);
        assert!(h.state().app.doc.layers[0].raster.iter().all(Option::is_some), "fill wrote every frame");
        // Hand tool pans.
        h.state_mut().app.set_tool(settings::Tool::Hand);
        let before = h.state().app.view.offset;
        press(&mut h, centre, true);
        h.run_steps(1);
        h.input_mut().events.push(egui::Event::PointerMoved(centre + egui::vec2(40.0, 0.0)));
        h.run_steps(1);
        press(&mut h, centre + egui::vec2(40.0, 0.0), false);
        h.run_steps(2);
        assert!((h.state().app.view.offset.x - before.x - 40.0).abs() < 1.0, "panned");
    }

    /// Offscreen screenshot through wgpu, for looking at the UI:
    /// `WOBBLE_SNAPSHOT=shot.png cargo test -p wobbleworks snapshot -- --ignored`.
    /// `WOBBLE_THEME` picks a preset; `WOBBLE_SETTINGS=1` opens Settings.
    #[test]
    #[ignore = "needs a GPU or software renderer; run on demand"]
    fn snapshot() {
        let out = std::env::var("WOBBLE_SNAPSHOT").unwrap_or_else(|_| "wobbleworks.png".into());
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1360.0, 860.0))
            .with_pixels_per_point(1.0)
            .with_max_steps(16)
            .wgpu()
            .build_eframe(|cc| wobble(cc));
        if let Ok(name) = std::env::var("WOBBLE_THEME")
            && let Some((n, t)) = theme::PRESETS.iter().find(|p| p.0 == name)
        {
            h.state_mut().app.s.preset = (*n).into();
            h.state_mut().app.s.theme = *t;
        }
        h.run_steps(2);
        doodle(&mut h.state_mut().app);
        if std::env::var("WOBBLE_SETTINGS").is_ok() {
            h.state_mut().app.show_settings = true;
        }
        if std::env::var("WOBBLE_LASSO").is_ok() {
            h.state_mut().app.finish_lasso(vec![(400.0, 60.0), (630.0, 60.0), (630.0, 160.0), (400.0, 160.0)]);
            if let Some(f) = &mut h.state_mut().app.floating {
                f.xf.rot = 20.0;
                f.xf.dx = -40.0;
            }
        }
        h.run_steps(4);
        let img = h.render().unwrap();
        img.save(&out).unwrap();
    }
}
