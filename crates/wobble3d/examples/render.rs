//! Render a demo note to PNGs (one per boil frame), to look at the renderer without a window:
//! `cargo run -p wobbleworks-3d --example render -- out_dir [render]`

use std::collections::HashMap;

use serde_json::json;
use wobbleworks_3d::editor::Editor;
use wobbleworks_3d::raster::{Textures, rasterize};

fn main() -> Result<(), String> {
    let out = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let render_mode = std::env::args().nth(2).as_deref() == Some("render");
    if std::env::args().nth(2).as_deref() == Some("swatches") {
        return swatches(&out);
    }
    let mut e = Editor::new();
    e.set_viewport(960.0, 640.0);
    e.run("env.set", &json!({"render": render_mode, "lighting": {"groundShadow": true}}))?;
    // A tube guide from the front, curves drawn on it from the side and the top.
    e.run("camera.view", &json!({"view": "front"}))?;
    let circle: Vec<[f32; 2]> = (0..=64).map(|i| {
        let a = std::f32::consts::TAU * i as f32 / 64.0;
        [480.0 + 90.0 * a.cos(), 320.0 + 90.0 * a.sin()]
    }).collect();
    e.run("guide.draw", &json!({"points": circle}))?;
    e.run("camera.set", &json!({"yaw": 35, "pitch": 20, "orthographic": false}))?;
    let kinds = ["pen", "marker", "flat", "square", "nib", "oil", "gouache", "drybrush", "chalk", "ink"];
    let colours = ["#23222b", "#ff2e88", "#2f5bff", "#2ec27e", "#8a4dff", "#ff8a1f", "#e8344e", "#7a4a2e", "#4fb3ff", "#17161c"];
    for (i, (k, c)) in kinds.iter().zip(colours).enumerate() {
        e.run("brush.set", &json!({"kind": k, "color": c, "size": 14 + (i % 3) * 8}))?;
        if *k == "ink" {
            e.run("brush.set", &json!({"size": 40, "paint": {"echo": {"color": "#ff2e60", "offset": [7, 6], "width": 1.1}}}))?;
        }
        let x0 = 300.0 + (i as f32) * 38.0;
        let line: Vec<[f32; 3]> = (0..60).map(|j| {
            let t = j as f32 / 59.0;
            [x0 + 20.0 * (t * 6.0).sin(), 160.0 + t * 330.0, 0.3 + 0.7 * (t * 3.1).sin().abs()]
        }).collect();
        e.run("stroke.draw", &json!({"points": line}))?;
    }
    // Metaphor-style painted banner in the air: gouache body, ink echo.
    e.run("guide.close", &json!(null))?;
    e.run("brush.set", &json!({"kind": "gouache", "color": "#f4efe6", "size": 120, "paint": {"echo": {"color": "#111111", "offset": [10, 8], "width": 1.15}, "layers": 3}}))?;
    let banner: Vec<[f32; 3]> = (0..50).map(|j| [120.0 + j as f32 * 15.0, 560.0 + (j as f32 * 0.4).sin() * 6.0, 1.0]).collect();
    e.run("stroke.draw", &json!({"points": banner}))?;
    for frame in 0..3u32 {
        let f = e.render(frame, true);
        let tex = Textures { atlas: &e.atlas, images: HashMap::new() };
        let pic = rasterize(&f, &tex, 960, 640, Some(e.scene.environment.background), Some(&e.scene.environment.effects))?;
        let path = format!("{out}/wobble3d_{}{frame}.png", if render_mode { "render_" } else { "" });
        image::save_buffer(&path, &pic.rgba, pic.width, pic.height, image::ColorType::Rgba8).map_err(|e| e.to_string())?;
        println!("{path}: {} triangles", f.triangles);
    }
    Ok(())
}

/// Every brush kind as a big S-curve, seen straight on.
fn swatches(out: &str) -> Result<(), String> {
    let mut e = Editor::new();
    e.set_viewport(1200.0, 700.0);
    e.run("env.set", &json!({"grid": false}))?;
    e.run("camera.view", &json!({"view": "front"}))?;
    e.run("camera.set", &json!({"distance": 3.0}))?;
    let kinds = ["pen", "marker", "flat", "square", "nib", "oil", "gouache", "drybrush", "chalk", "ink"];
    let colours = ["#23222b", "#ff2e88", "#2f5bff", "#2ec27e", "#8a4dff", "#ff8a1f", "#e8344e", "#7a4a2e", "#4fb3ff", "#17161c"];
    for (i, (k, c)) in kinds.iter().zip(colours).enumerate() {
        e.run("brush.set", &json!({"kind": k, "color": c, "size": 60, "pressure": true}))?;
        let x0 = 80.0 + (i as f32) * 112.0;
        let line: Vec<[f32; 3]> = (0..80).map(|j| {
            let t = j as f32 / 79.0;
            [x0 + 35.0 * (t * 6.0).sin(), 60.0 + t * 580.0, 0.35 + 0.65 * (t * 3.1).sin().abs()]
        }).collect();
        e.run("stroke.draw", &json!({"points": line}))?;
    }
    for frame in 0..2u32 {
        let f = e.render(frame, false);
        let tex = Textures { atlas: &e.atlas, images: HashMap::new() };
        let pic = rasterize(&f, &tex, 1200, 700, Some(e.scene.environment.background), None)?;
        let path = format!("{out}/swatches_{frame}.png");
        image::save_buffer(&path, &pic.rgba, pic.width, pic.height, image::ColorType::Rgba8).map_err(|e| e.to_string())?;
        println!("{path}");
    }
    Ok(())
}
