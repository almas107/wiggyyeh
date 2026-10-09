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
    if std::env::args().nth(2).as_deref() == Some("metaphor") {
        return metaphor(&out);
    }
    if std::env::args().nth(2).as_deref() == Some("panel") {
        return panel(&out);
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

/// A Metaphor-style painted text box: a white gouache body with ragged edges built up in layers,
/// a black echo behind it, a red accent stroke, seen straight on, two boil frames.
fn metaphor(out: &str) -> Result<(), String> {
    let mut e = Editor::new();
    e.set_viewport(1000.0, 420.0);
    e.run("env.set", &json!({"grid": false, "background": "#2a1f3d"}))?;
    e.run("camera.view", &json!({"view": "front"}))?;
    e.run("camera.set", &json!({"distance": 3.0}))?;
    e.run("boil.set", &json!({"amount": 2.5}))?;
    // The red accent first (behind), then the body rows.
    e.run("brush.set", &json!({"kind": "drybrush", "color": "#e8243c", "size": 120, "pressure": false}))?;
    let accent: Vec<[f32; 3]> = (0..40).map(|j| [90.0 + j as f32 * 21.0, 120.0 - (j as f32 * 0.15).sin() * 10.0, 1.0]).collect();
    e.run("stroke.draw", &json!({"points": accent}))?;
    e.run("brush.set", &json!({"kind": "gouache", "color": "#f6f1e7", "size": 220, "pressure": false,
        "paint": {"roughness": 0.8, "bristles": 0.5, "dryness": 0.1, "layers": 3, "taper": 0.0, "boil": 1.5,
                  "echo": {"color": "#0d0d10", "offset": [10, 8], "width": 1.12}}}))?;
    for row in 0..4 {
        let y = 190.0 + row as f32 * 50.0;
        let pts: Vec<[f32; 3]> = (0..40).map(|j| [110.0 + j as f32 * 20.0 + row as f32 * 8.0, y + (j as f32 * 0.3 + row as f32).sin() * 4.0, 1.0]).collect();
        e.run("stroke.draw", &json!({"points": pts}))?;
    }
    // Geometry-nodes style brushstrokes: scattered oil dabs.
    e.run("brush.set", &json!({"kind": "oil", "color": "#ffb000", "size": 90, "paint": {"roughness": 0.5, "bristles": 0.6, "dryness": 0.2, "layers": 1, "taper": 0.3, "scatter": 0.75, "dabSize": 1.3, "jitter": 0.5, "colorJitter": 0.35, "echo": null}}))?;
    let swirl: Vec<[f32; 3]> = (0..60).map(|j| {
        let t = j as f32 / 59.0;
        [120.0 + t * 760.0, 375.0 + (t * 9.0).sin() * 18.0, 1.0]
    }).collect();
    e.run("stroke.draw", &json!({"points": swirl}))?;
    for frame in 0..2u32 {
        let f = e.render(frame, false);
        let tex = Textures { atlas: &e.atlas, images: HashMap::new() };
        let pic = rasterize(&f, &tex, 1000, 420, Some(e.scene.environment.background), None)?;
        let path = format!("{out}/metaphor_{frame}.png");
        image::save_buffer(&path, &pic.rgba, pic.width, pic.height, image::ColorType::Rgba8).map_err(|e| e.to_string())?;
        println!("{path}");
    }
    Ok(())
}

/// A painted text-box panel: a rough loop filled with ragged black gouache (white echo), a red
/// dry-brush slash behind, slightly turned in 3D.
fn panel(out: &str) -> Result<(), String> {
    let mut e = Editor::new();
    e.set_viewport(1000.0, 500.0);
    e.run("env.set", &json!({"grid": false, "background": "#e9e2d4"}))?;
    e.run("camera.view", &json!({"view": "front"}))?;
    e.run("camera.set", &json!({"distance": 3.0}))?;
    e.run("brush.set", &json!({"kind": "drybrush", "color": "#d81e3c", "size": 260, "pressure": false, "paint": {"taper": 0.4}}))?;
    let slash: Vec<[f32; 3]> = (0..30).map(|j| [160.0 + j as f32 * 24.0, 380.0 - j as f32 * 9.0, 1.0]).collect();
    e.run("stroke.draw", &json!({"points": slash}))?;
    // The loop (any brush; it is filled, then deleted).
    let lp: Vec<[f32; 3]> = (0..=48).map(|i| {
        let t = i as f32 / 48.0 * std::f32::consts::TAU;
        let (c, s) = (t.cos(), t.sin());
        // A squarish blob.
        let r = 1.0 / (c.abs().powf(4.0) + s.abs().powf(4.0)).powf(0.25);
        [500.0 + 330.0 * r * c + (i % 5) as f32 * 3.0, 250.0 + 120.0 * r * s, 1.0]
    }).collect();
    e.run("stroke.draw", &json!({"points": lp}))?;
    let loop_id = e.scene.strokes.last().map(|s| s.id).ok_or("no loop")?;
    e.run("select.set", &json!({"ids": [loop_id]}))?;
    e.run("brush.set", &json!({"kind": "gouache", "color": "#141218", "size": 90, "pressure": false, "applyToSelection": false,
        "paint": {"roughness": 0.85, "bristles": 0.45, "dryness": 0.1, "layers": 2, "taper": 0.0, "boil": 1.5,
                  "echo": {"color": "#fbf8f1", "offset": [-7, -6], "width": 1.08}}}))?;
    e.run("edit.fill", &json!({"angle": -8, "jitter": 0.4}))?;
    e.run("select.set", &json!({"ids": [loop_id]}))?;
    e.run("edit.delete", &json!(null))?;
    e.run("camera.set", &json!({"yaw": 12, "pitch": 6, "orthographic": false}))?;
    for frame in 0..2u32 {
        let f = e.render(frame, false);
        let tex = Textures { atlas: &e.atlas, images: HashMap::new() };
        let pic = rasterize(&f, &tex, 1000, 500, Some(e.scene.environment.background), None)?;
        let path = format!("{out}/panel_{frame}.png");
        image::save_buffer(&path, &pic.rgba, pic.width, pic.height, image::ColorType::Rgba8).map_err(|e| e.to_string())?;
        println!("{path}");
    }
    Ok(())
}
