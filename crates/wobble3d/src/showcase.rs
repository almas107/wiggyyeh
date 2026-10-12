//! The showcase note: a little garden scene made with every 3D feature, built through the
//! editor's own commands (so it always shows what the current version does). File › Showcase
//! opens it; `cargo run -p wobbleworks-3d --example render -- out showcase` renders it.
//!
//! What is where (each in its own named group):
//! - Ground: a plane primitive made solid; a pond (Lasso Fill) with flat-tape ripples on it.
//! - House: a solid cube and pyramid; windows and a door as filled Shapes on the wall guide; a
//!   path laid with Fill Selection; a QuickShape-snapped sign.
//! - Tree: a solid tube trunk and sphere crown; painterly leaf strokes in a group clipped to the
//!   crown (they only show on it).
//! - Flower: petals drawn as closed curves, coloured with the paint bucket (under the lines).
//! - Brush rack: one stroke of every brush type on a board, plus a pattern and an echo.
//! - Sky: a glowing sun, a mirrored fence, shots for the sequence player, fog and shadows.

use serde_json::{Value, json};

use crate::editor::{Editor, Mods};
use crate::math::{Vec3, v3};

fn r(ed: &mut Editor, cmd: &str, p: Value) -> Result<Value, String> {
    ed.run(cmd, &p).map_err(|e| format!("{cmd}: {e}"))
}

fn screen(ed: &Editor, p: Vec3) -> Result<[f32; 2], String> {
    ed.view().project(p).map(|q| [q.x, q.y]).ok_or_else(|| "a showcase point is off screen".into())
}

/// A primitive guide scaled and moved into place, then a solid copy in `colour`.
fn solid(ed: &mut Editor, kind: &str, at: [f32; 3], scale: [f32; 3], colour: &str) -> Result<(), String> {
    r(ed, "guide.primitive", json!({"kind": kind}))?;
    for (axis, k) in ["x", "y", "z"].iter().zip(scale) {
        if (k - 1.0).abs() > 1e-6 {
            r(ed, "transform.apply", json!({"mode": "scale", "axis": axis, "amount": k}))?;
        }
    }
    for (axis, d) in ["x", "y", "z"].iter().zip(at) {
        if d.abs() > 1e-6 {
            r(ed, "transform.apply", json!({"mode": "grab", "axis": axis, "amount": d}))?;
        }
    }
    r(ed, "guide.done", Value::Null)?;
    r(ed, "brush.set", json!({"color": colour, "material": "shaded"}))?;
    r(ed, "guide.solid", Value::Null)?;
    Ok(())
}

fn ring(c: Vec3, rx: f32, rz: f32, n: usize) -> Vec<Vec3> {
    (0..n)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            c + v3(rx * a.cos(), 0.0, rz * a.sin())
        })
        .collect()
}

fn pts(v: &[Vec3]) -> Value {
    json!(v.iter().map(|p| [p.x, p.y, p.z]).collect::<Vec<_>>())
}

/// Build the showcase into a fresh note in `ed` (its viewport is kept).
pub fn build(ed: &mut Editor) -> Result<(), String> {
    r(ed, "file.new", Value::Null)?;
    r(ed, "env.set", json!({"grid": false, "background": "#cfe6ff", "fog": true, "lighting": {"groundShadow": true, "azimuth": 40, "altitude": 55}}))?;
    r(ed, "boil.set", json!({"enabled": true, "amount": 1.6, "frames": 3, "fps": 8}))?;
    r(ed, "assist.set", json!({"drawInAir": true}))?;
    r(ed, "camera.set", json!({"yaw": 28, "pitch": 24, "distance": 15, "target": [0.0, 1.2, 0.0], "orthographic": false}))?;
    let first = ed.scene.active_group;
    r(ed, "group.rename", json!({"id": first, "name": "Ground: solid plane"}))?;
    r(ed, "brush.set", json!({"kind": "pen", "size": 30, "opacity": 1.0, "pressure": false, "paint": {"boil": 1.0}}))?;

    // ---- Ground: a plane primitive made solid (a 12 m lawn).
    solid(ed, "plane", [0.0, 0.0, 0.0], [3.0, 1.0, 3.0], "#8fcf6a")?;
    // The pond: Lasso Fill on the ground guide, then flat tape ripples lying on it.
    r(ed, "group.new", json!({"name": "Pond: Lasso Fill + flat tape"}))?;
    r(ed, "brush.set", json!({"color": "#4aa3e8"}))?;
    let pond: Vec<[f32; 2]> = ring(v3(1.2, 0.0, 3.0), 1.6, 0.9, 64).into_iter().map(|p| screen(ed, p)).collect::<Result<_, _>>()?;
    r(ed, "fill.lasso", json!({"points": pond}))?;
    r(ed, "brush.set", json!({"kind": "flat", "color": "#e8f6ff", "size": 40}))?;
    for (k, rad) in [0.5f32, 0.95].iter().enumerate() {
        let arc: Vec<Vec3> = (0..24)
            .map(|i| {
                v3(1.2, 0.0, 3.0) + v3(rad * (0.4 + i as f32 * 0.09).cos(), 0.0, rad * 0.55 * (0.4 + i as f32 * 0.09).sin()) + v3(0.0, 0.0, k as f32 * 0.05)
            })
            .collect();
        r(ed, "stroke.add", json!({"points": pts(&arc), "normal": [0, 1, 0]}))?;
    }
    r(ed, "guide.close", Value::Null)?;

    // ---- House: solid cube walls and a pyramid roof; windows and a door are filled Shapes on
    // the wall guide (later paint layers over the wall, like 2D painting).
    r(ed, "group.new", json!({"name": "House: solid primitives + Shapes"}))?;
    r(ed, "brush.set", json!({"kind": "pen", "size": 30}))?;
    solid(ed, "cube", [-3.0, 1.0, -0.5], [1.2, 1.0, 1.0], "#f3d9a4")?;
    r(ed, "camera.view", json!({"view": "front"}))?;
    r(ed, "camera.set", json!({"target": [-3.0, 1.0, 0.5], "distance": 6}))?;
    r(ed, "brush.set", json!({"color": "#5b8fd6", "material": "shadeless"}))?;
    for wx in [-3.7f32, -2.3] {
        let (a, b) = (screen(ed, v3(wx - 0.3, 1.5, 0.5))?, screen(ed, v3(wx + 0.3, 1.0, 0.5))?);
        r(ed, "fill.shape", json!({"kind": "rect", "filled": true, "x0": a[0], "y0": a[1], "x1": b[0], "y1": b[1]}))?;
    }
    r(ed, "brush.set", json!({"color": "#7a4a2e"}))?;
    let (a, b) = (screen(ed, v3(-3.25, 1.05, 0.5))?, screen(ed, v3(-2.75, 0.02, 0.5))?);
    r(ed, "fill.shape", json!({"kind": "rect", "filled": true, "x0": a[0], "y0": a[1], "x1": b[0], "y1": b[1]}))?;
    // A round window above the door: an outline ellipse in ink.
    r(ed, "brush.set", json!({"color": "#2a2633", "size": 18, "material": "shaded"}))?;
    let (a, b) = (screen(ed, v3(-3.2, 1.75, 0.5))?, screen(ed, v3(-2.8, 1.35, 0.5))?);
    r(ed, "fill.shape", json!({"kind": "ellipse", "filled": false, "x0": a[0], "y0": a[1], "x1": b[0], "y1": b[1]}))?;
    // A sign beside the door, drawn roughly and snapped clean by holding the pen (QuickShape).
    let corners = [v3(-2.5, 0.8, 0.5), v3(-2.05, 0.8, 0.5), v3(-2.05, 0.45, 0.5), v3(-2.5, 0.45, 0.5), v3(-2.5, 0.8, 0.5)];
    let sc: Vec<[f32; 2]> = corners.iter().map(|p| screen(ed, *p)).collect::<Result<_, _>>()?;
    r(ed, "tool.set", json!({"tool": "draw"}))?;
    let mut t = ed.time + 100.0;
    let mut k = 0u32;
    ed.pointer_down(sc[0][0], sc[0][1], 1.0, t, Mods::default());
    for w in sc.windows(2) {
        for i in 1..=16 {
            let f = i as f32 / 16.0;
            k += 1;
            let jitter = ((k * 7) % 5) as f32 * 0.6 - 1.2;
            t += 0.01;
            ed.pointer_move(w[0][0] + (w[1][0] - w[0][0]) * f + jitter, w[0][1] + (w[1][1] - w[0][1]) * f - jitter, 1.0, t, Mods::default());
        }
    }
    ed.tick(t + 0.7);
    if let Some(l) = sc.last() {
        ed.pointer_up(l[0], l[1], t + 0.8, Mods::default());
    }
    r(ed, "guide.close", Value::Null)?;
    r(ed, "camera.set", json!({"yaw": 28, "pitch": 24, "distance": 15, "target": [0.0, 1.2, 0.0], "orthographic": false}))?;
    solid(ed, "pyramid", [-3.0, 2.55, -0.5], [1.45, 0.55, 1.25], "#d65a4f")?;
    r(ed, "guide.close", Value::Null)?;
    // A stone path: a closed curve on the ground, filled with Fill Selection.
    r(ed, "brush.set", json!({"kind": "marker", "color": "#9a8f86", "size": 26, "material": "shaded"}))?;
    let path = [v3(-3.35, 0.01, 0.6), v3(-2.65, 0.01, 0.6), v3(-2.2, 0.01, 2.4), v3(-1.0, 0.01, 4.2), v3(-1.9, 0.01, 4.4), v3(-3.0, 0.01, 2.5)];
    let ids = r(ed, "stroke.add", json!({"points": pts(&path), "normal": [0, 1, 0]}))?;
    r(ed, "select.set", json!({"ids": ids}))?;
    r(ed, "brush.set", json!({"color": "#c9bfb4", "applyToSelection": false}))?;
    r(ed, "fill.selection", Value::Null)?;
    r(ed, "select.none", Value::Null)?;

    // ---- Tree: a solid trunk and crown, with leaf strokes clipped to the crown.
    r(ed, "group.new", json!({"name": "Tree: trunk"}))?;
    solid(ed, "tube", [3.0, 1.0, -1.5], [0.22, 1.0, 0.22], "#7a5232")?;
    r(ed, "guide.close", Value::Null)?;
    let crown = r(ed, "group.new", json!({"name": "Tree: crown (clipping base)"}))?;
    solid(ed, "sphere", [3.0, 2.9, -1.5], [1.25, 1.1, 1.25], "#4f9a45")?;
    r(ed, "guide.close", Value::Null)?;
    let leaves = r(ed, "group.new", json!({"name": "Tree: leaves (clipped to the crown)"}))?;
    r(ed, "brush.set", json!({"kind": "oil", "color": "#2f6b34", "size": 120, "material": "shaded"}))?;
    for i in 0..9 {
        let y = 2.0 + i as f32 * 0.25;
        let swing: Vec<Vec3> = (0..20).map(|j| v3(3.0 - 1.9 + j as f32 * 0.2, y + (j as f32 * 0.5).sin() * 0.08, -1.5 + 1.35)).collect();
        r(ed, "stroke.add", json!({"points": pts(&swing)}))?;
    }
    r(ed, "brush.set", json!({"kind": "gouache", "color": "#b7e07a", "size": 70}))?;
    let light: Vec<Vec3> = (0..16).map(|j| v3(2.2 + j as f32 * 0.12, 3.6 - (j as f32 * 0.3).sin() * 0.15, -1.5 + 1.3)).collect();
    r(ed, "stroke.add", json!({"points": pts(&light)}))?;
    if let Some(id) = leaves.as_u64() {
        r(ed, "group.clip", json!({"id": id, "clip": true}))?;
    }
    let _ = crown;

    // ---- Flower: closed petals drawn as curves, coloured with the paint bucket.
    r(ed, "group.new", json!({"name": "Flower: paint bucket"}))?;
    r(ed, "camera.view", json!({"view": "front"}))?;
    r(ed, "camera.set", json!({"target": [0.3, 1.0, 1.0], "distance": 5}))?;
    r(ed, "brush.set", json!({"kind": "pen", "color": "#2a2633", "size": 12, "material": "shaded"}))?;
    let centre = v3(0.3, 1.3, 1.0);
    let flower: Vec<Vec3> = (0..=120)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / 120.0;
            let rad = 0.32 + 0.16 * (5.0 * a).cos();
            centre + v3(rad * a.cos(), rad * a.sin(), 0.0)
        })
        .collect();
    r(ed, "stroke.add", json!({"points": pts(&flower), "normal": [0, 0, 1]}))?;
    let heart: Vec<Vec3> = (0..=40)
        .map(|i| centre + v3(0.11 * (std::f32::consts::TAU * i as f32 / 40.0).cos(), 0.11 * (std::f32::consts::TAU * i as f32 / 40.0).sin(), 0.0))
        .collect();
    r(ed, "stroke.add", json!({"points": pts(&heart), "normal": [0, 0, 1]}))?;
    let stem: Vec<Vec3> = (0..20).map(|i| v3(0.3 + (i as f32 * 0.3).sin() * 0.03, 0.0 + i as f32 * 0.044, 1.0)).collect();
    r(ed, "brush.set", json!({"kind": "ink", "color": "#3c7a3a", "size": 30}))?;
    r(ed, "stroke.add", json!({"points": pts(&stem), "normal": [0, 0, 1]}))?;
    r(ed, "brush.set", json!({"color": "#ff8fc0"}))?;
    let petal = screen(ed, centre + v3(0.33, 0.0, 0.0))?;
    r(ed, "fill.at", json!({"x": petal[0], "y": petal[1]}))?;
    r(ed, "brush.set", json!({"color": "#ffd23f"}))?;
    let mid = screen(ed, centre)?;
    r(ed, "fill.at", json!({"x": mid[0], "y": mid[1]}))?;

    // ---- Brush rack: every brush type on a board, plus a pattern and an echo (Metaphor look).
    r(ed, "group.new", json!({"name": "Brush rack: every brush"}))?;
    r(ed, "camera.set", json!({"yaw": 28, "pitch": 24, "distance": 15, "target": [0.0, 1.2, 0.0], "orthographic": false}))?;
    solid(ed, "plane", [0.4, 1.9, -3.2], [0.8, 1.0, 0.45], "#fffaf0")?;
    r(ed, "guide.close", Value::Null)?;
    // The board is a plane lying flat: stand it up facing the viewer.
    if let Some(id) = ed.scene.strokes.last().map(|s| s.id) {
        r(ed, "select.set", json!({"ids": [id]}))?;
        r(ed, "transform.apply", json!({"mode": "rotate", "axis": "x", "amount": 90}))?;
        r(ed, "select.none", Value::Null)?;
    }
    let kinds = ["pen", "marker", "flat", "square", "nib", "diamond", "oil", "gouache", "drybrush", "chalk", "ink", "ribbon"];
    let colours = ["#23222b", "#ff2e88", "#2f5bff", "#2ec27e", "#8a4dff", "#00a6a6", "#ff8a1f", "#e8344e", "#7a4a2e", "#4fb3ff", "#17161c", "#f0a020"];
    for (i, (kind, colour)) in kinds.iter().zip(colours).enumerate() {
        let x0 = -1.0 + (i % 6) as f32 * 0.48;
        let y0 = if i < 6 { 2.45 } else { 1.55 };
        let wave: Vec<Vec3> = (0..24).map(|j| v3(x0 + j as f32 * 0.018, y0 - j as f32 * 0.03 + (j as f32 * 0.4).sin() * 0.06, -3.15)).collect();
        r(ed, "brush.set", json!({"kind": kind, "color": colour, "size": 34, "pattern": null}))?;
        r(ed, "stroke.add", json!({"points": pts(&wave), "normal": [0, 0, 1]}))?;
    }
    r(ed, "brush.set", json!({"kind": "pen", "color": "#ff8a1f", "size": 60, "pattern": {"kind": "dot", "intensity": 0.8, "angle": 30, "contrast": 0.7}}))?;
    let dotted: Vec<Vec3> = (0..20).map(|j| v3(-1.2 + j as f32 * 0.12, 3.15, -3.1)).collect();
    r(ed, "stroke.add", json!({"points": pts(&dotted), "normal": [0, 0, 1]}))?;
    r(
        ed,
        "brush.set",
        json!({"kind": "ink", "color": "#17161c", "size": 50, "pattern": null, "paint": {"echo": {"color": "#ff2e60", "offset": [6, 5], "width": 1.1}}}),
    )?;
    let swoosh: Vec<Vec3> = (0..30).map(|j| v3(-1.3 + j as f32 * 0.1, 0.65 + (j as f32 * 0.21).sin() * 0.12, -3.1)).collect();
    r(ed, "stroke.add", json!({"points": pts(&swoosh), "normal": [0, 0, 1]}))?;
    r(ed, "brush.set", json!({"paint": {"echo": null}}))?;

    // ---- Sky: a glowing sun and a mirrored fence.
    r(ed, "group.new", json!({"name": "Sky: Glow sun"}))?;
    let sun_at = v3(0.4, 3.9, -7.5);
    r(ed, "brush.set", json!({"kind": "pen", "color": "#ffcf40", "size": 120, "material": "glow", "glow": 0.9}))?;
    for i in 0..12 {
        let a = std::f32::consts::TAU * i as f32 / 12.0;
        let ray = [sun_at + v3(0.75 * a.cos(), 0.75 * a.sin(), 0.0), sun_at + v3(1.15 * a.cos(), 1.15 * a.sin(), 0.0)];
        r(ed, "stroke.add", json!({"points": pts(&ray)}))?;
    }
    r(ed, "camera.set", json!({"target": [sun_at.x, sun_at.y, sun_at.z]}))?;
    let sun: Vec<[f32; 2]> = (0..48)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / 48.0;
            screen(ed, sun_at + v3(0.6 * a.cos(), 0.6 * a.sin(), 0.0))
        })
        .collect::<Result<_, _>>()?;
    r(ed, "fill.lasso", json!({"points": sun}))?;
    r(ed, "group.new", json!({"name": "Fence: Mirror X"}))?;
    r(ed, "mirror.set", json!({"on": true, "x": true, "y": false, "z": false}))?;
    r(ed, "brush.set", json!({"kind": "square", "color": "#a0714f", "size": 60, "material": "shaded"}))?;
    for i in 0..4 {
        let x = 1.6 + i as f32 * 0.9;
        r(ed, "stroke.add", json!({"points": pts(&[v3(x, 0.0, 5.0), v3(x, 0.9, 5.0)])}))?;
    }
    r(ed, "stroke.add", json!({"points": pts(&[v3(1.6, 0.6, 5.0), v3(4.3, 0.6, 5.0)])}))?;
    r(ed, "mirror.set", json!({"on": false}))?;

    // ---- Shots for the sequence player, and the starting view.
    for (yaw, pitch, dist, target) in
        [(28.0, 24.0, 15.0, [0.0, 1.2, 0.0]), (-35.0, 12.0, 9.0, [-2.5, 1.2, 0.5]), (60.0, 30.0, 8.0, [3.0, 2.0, -1.5]), (5.0, 8.0, 6.0, [0.3, 1.2, 1.0])]
    {
        r(ed, "camera.set", json!({"yaw": yaw, "pitch": pitch, "distance": dist, "target": target, "orthographic": false}))?;
        r(ed, "shot.add", Value::Null)?;
    }
    r(ed, "camera.set", json!({"yaw": 28, "pitch": 24, "distance": 15, "target": [0.0, 1.2, 0.0], "orthographic": false}))?;
    r(ed, "brush.set", json!({"kind": "pen", "color": "#23222b", "size": 12, "material": "shaded", "pattern": null, "glow": 0.6}))?;
    r(ed, "tool.set", json!({"tool": "draw"}))?;
    ed.status = "The showcase: every group is one feature (see the Stage panel)".into();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The showcase builds with the current commands and has every kind of thing in it.
    #[test]
    fn the_showcase_builds_and_shows_every_feature() {
        let mut e = Editor::new();
        e.set_viewport(1280.0, 800.0);
        build(&mut e).expect("showcase");
        // The sign was drawn roughly (with Stable Stroke on) and snapped to a clean rectangle.
        assert!(e.scene.strokes.iter().any(|s| s.group == e.scene.groups[2].id && !s.is_fill() && s.points.len() > 40), "the snapped sign");
        let sc = &e.scene;
        let fills = sc.strokes.iter().filter(|s| s.is_fill()).count();
        assert!(fills >= 12, "solids, lasso, shapes, bucket and path fills: {fills}");
        assert!(sc.groups.iter().any(|g| g.clip), "a clipped group");
        assert!(sc.groups.len() >= 9, "{}", sc.groups.len());
        assert!(sc.sequence.shots.len() >= 4);
        for kind in crate::model::BrushKind::ALL {
            assert!(sc.strokes.iter().any(|s| s.brush.kind == kind && !s.is_fill()), "{kind:?} on the rack");
        }
        assert!(sc.strokes.iter().any(|s| s.brush.material == crate::model::Material::Glow));
        assert!(sc.strokes.iter().any(|s| s.brush.paint.echo.is_some()));
        assert!(sc.strokes.iter().any(|s| s.brush.pattern.is_some()));
        // Renders on every boil frame.
        for f in 0..3 {
            assert!(e.render(f, false).triangles > 1000);
        }
        // Round-trips through the file format.
        let bytes = crate::io::save(&e.scene, &e.camera).expect("save");
        let (back, _) = crate::io::load(&bytes).expect("load");
        assert_eq!(back.strokes.len(), e.scene.strokes.len());
    }
}
