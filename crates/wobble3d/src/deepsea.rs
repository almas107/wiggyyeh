//! "Deep sea": Feather's advanced tutorial (a submarine meets a glowing creature) redone with
//! the editor's commands, step by step as the tutorial does it, to show the techniques work
//! here: a hull painted with wide shaded strokes along a stretched sphere guide, a glowing
//! window, lamps with additive light beams stacked for volume, a creature painted with
//! low-opacity glow strokes on a bent guide, dark terrain from dense short strokes, mirrored
//! particles, a deep-blue environment light, and Render mode with bloom, depth of field and
//! pixelation. File › Open the deep-sea scene (`file.showcase {name: "deepsea"}`).

use serde_json::{Value, json};

use crate::editor::Editor;
use crate::math::{Vec3, v3};
use crate::noise::{hash2, unit};

fn r(ed: &mut Editor, cmd: &str, p: Value) -> Result<Value, String> {
    ed.run(cmd, &p).map_err(|e| format!("{cmd}: {e}"))
}

fn pts(v: &[Vec3]) -> Value {
    json!(v.iter().map(|p| [p.x, p.y, p.z]).collect::<Vec<_>>())
}

fn screen(ed: &Editor, p: Vec3) -> Result<[f32; 2], String> {
    ed.view().project(p).map(|q| [q.x, q.y]).ok_or_else(|| "a point is off screen".into())
}

/// A pseudo-random number in 0..1 (stable: the scene is the same every time).
fn rnd(i: u32, k: u32) -> f32 {
    unit(hash2(i.wrapping_mul(0x9e37_79b9), k))
}

/// Draw a curve on the active guide through world points (cast from the current view).
fn paint_on_guide(ed: &mut Editor, world: &[Vec3], pressure: impl Fn(f32) -> f32) -> Result<(), String> {
    let n = world.len().max(2);
    let mut sp = Vec::with_capacity(n);
    for (i, p) in world.iter().enumerate() {
        let s = screen(ed, *p)?;
        sp.push([s[0], s[1], pressure(i as f32 / (n - 1) as f32)]);
    }
    r(ed, "stroke.draw", json!({"points": sp}))?;
    Ok(())
}

pub fn build(ed: &mut Editor) -> Result<(), String> {
    r(ed, "file.new", Value::Null)?;
    r(ed, "feel.set", json!({"post": false}))?;
    r(ed, "stable.set", json!({"value": 0.0}))?;
    r(ed, "assist.set", json!({"quickShape": false, "drawInAir": true}))?;
    r(ed, "snap.set", json!({"ends": false}))?;
    // Part 1: set the mood early: a deep-blue sea, no grid.
    r(ed, "env.set", json!({"grid": false, "background": "#0b1a5c", "fog": true}))?;
    r(ed, "boil.set", json!({"enabled": true, "amount": 0.8}))?;
    let g0 = ed.scene.active_group;
    r(ed, "group.rename", json!({"id": g0, "name": "Sub hull (wide shaded strokes)"}))?;

    // Part 2: the hull. A sphere primitive stretched into a capsule as the guide, then wide
    // shaded strokes laid side by side along it (the faceted painted panels).
    r(ed, "guide.primitive", json!({"kind": "sphere", "segments": 32}))?;
    r(ed, "transform.apply", json!({"mode": "scale", "axis": "x", "amount": 1.6}))?;
    r(ed, "transform.apply", json!({"mode": "scale", "axis": "y", "amount": 0.85}))?;
    r(ed, "transform.apply", json!({"mode": "scale", "axis": "z", "amount": 0.85}))?;
    r(ed, "guide.done", Value::Null)?;
    r(ed, "camera.set", json!({"yaw": 0, "pitch": 0, "distance": 7, "target": [0, 0, 0], "orthographic": true}))?;
    r(ed, "brush.set", json!({"kind": "flat", "color": "#dfe6ff", "size": 300, "opacity": 1.0, "pressure": false, "material": "shaded"}))?;
    for k in 0..9 {
        let y = -0.72 + k as f32 * 0.18;
        let half = (1.0 - (y / 0.85).powi(2)).max(0.05).sqrt() * 1.5;
        let line: Vec<Vec3> = (0..24).map(|i| v3(-half + 2.0 * half * i as f32 / 23.0, y, 2.0)).collect();
        paint_on_guide(ed, &line, |_| 1.0)?;
    }
    // From the top too, so the hull is painted all round.
    r(ed, "camera.set", json!({"yaw": 0, "pitch": 89.5}))?;
    for k in 0..7 {
        let z = -0.6 + k as f32 * 0.2;
        let line: Vec<Vec3> = (0..24).map(|i| v3(-1.5 + 3.0 * i as f32 / 23.0, 2.0, z)).collect();
        paint_on_guide(ed, &line, |_| 1.0)?;
    }
    // Panel lines in dark navy with the cube brush.
    r(ed, "camera.set", json!({"yaw": 0, "pitch": 0}))?;
    r(ed, "brush.set", json!({"kind": "square", "color": "#1a2050", "size": 14}))?;
    for x in [0.2f32, 0.75] {
        let ring: Vec<Vec3> = (0..20).map(|i| v3(x, -0.7 + 1.4 * i as f32 / 19.0, 2.0)).collect();
        paint_on_guide(ed, &ring, |_| 1.0)?;
    }
    r(ed, "guide.close", Value::Null)?;

    // The pressure hull's glass: a sphere at the front, glowing softly (Glow, low opacity).
    r(ed, "group.new", json!({"name": "Window (glow)"}))?;
    r(ed, "guide.primitive", json!({"kind": "sphere", "segments": 24}))?;
    r(ed, "transform.apply", json!({"mode": "scale", "amount": 0.62}))?;
    r(ed, "transform.apply", json!({"mode": "grab", "axis": "x", "amount": -1.2}))?;
    r(ed, "guide.done", Value::Null)?;
    r(ed, "brush.set", json!({"color": "#6fb4ff", "material": "glow", "opacity": 0.28, "glow": 0.5}))?;
    r(ed, "guide.solid", Value::Null)?;
    r(ed, "guide.close", Value::Null)?;
    // A dark frame ring around it.
    r(ed, "camera.set", json!({"yaw": -90, "pitch": 0}))?;
    r(ed, "brush.set", json!({"kind": "pen", "color": "#141a3c", "size": 60, "material": "shaded", "opacity": 1.0}))?;
    let frame: Vec<Vec3> = (0..=48)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / 48.0;
            v3(-1.05, 0.6 * a.sin(), 0.6 * a.cos())
        })
        .collect();
    r(ed, "stroke.add", json!({"points": pts(&frame)}))?;

    // Lamps and their light beams: glow dots, then wide pressure-faded glow strokes, duplicated
    // and stacked for a volumetric look.
    r(ed, "group.new", json!({"name": "Lamps and beams (additive glow)"}))?;
    r(ed, "brush.set", json!({"kind": "pen", "color": "#cfe4ff", "size": 120, "material": "glow", "opacity": 1.0, "glow": 1.0}))?;
    let lamps = [v3(-1.1, 0.55, 0.55), v3(-1.1, 0.55, -0.55), v3(-1.1, -0.55, 0.55), v3(-1.1, -0.55, -0.55)];
    for l in lamps {
        r(ed, "stroke.add", json!({"points": pts(&[l])}))?;
    }
    r(ed, "brush.set", json!({"kind": "marker", "color": "#3f74ff", "size": 220, "opacity": 0.14, "pressure": true}))?;
    for l in lamps {
        for layer in 0..3 {
            let off = (layer as f32 - 1.0) * 0.1;
            let beam: Vec<Vec3> = (0..16)
                .map(|i| {
                    let t = i as f32 / 15.0;
                    // Forward and down, towards the creature, spreading a little.
                    l + v3(-t * 2.6, off - t * 0.9 + l.y * t * 0.3, off * 0.5 + l.z * t * 0.3)
                })
                .collect();
            let fade: Vec<[f32; 4]> = beam.iter().enumerate().map(|(i, p)| [p.x, p.y, p.z, 1.0 - i as f32 / 16.0]).collect();
            r(ed, "stroke.add", json!({"points": fade}))?;
        }
    }

    // Part 3: the creature: low-opacity glow strokes in one direction on a curved guide,
    // overlapping into a luminous texture.
    r(ed, "group.new", json!({"name": "Creature (glow on a curved guide)"}))?;
    let c0 = v3(-2.8, -1.5, 2.2);
    // The guide is drawn from the top (a wavy line: an upright, gently curved sheet), then
    // painted from the front, where it faces the pen.
    r(ed, "camera.set", json!({"yaw": 0, "pitch": 89.5, "target": [c0.x, c0.y, c0.z], "distance": 6, "orthographic": true}))?;
    let spine: Vec<[f32; 2]> =
        (0..30).map(|i| screen(ed, c0 + v3(-1.4 + 2.8 * i as f32 / 29.0, 0.0, 0.18 * (i as f32 * 0.22).sin()))).collect::<Result<_, _>>()?;
    r(ed, "guide.draw", json!({"points": spine}))?;
    r(ed, "camera.set", json!({"yaw": 0, "pitch": 0}))?;
    r(ed, "brush.set", json!({"kind": "ink", "color": "#5aa0ff", "size": 70, "opacity": 0.3, "material": "glow", "pressure": true, "glow": 0.7}))?;
    for k in 0..14 {
        let spread = (k as f32 - 6.5) * 0.045;
        let body: Vec<Vec3> = (0..20)
            .map(|i| {
                let t = i as f32 / 19.0;
                // A teardrop: wide at the head, tapering into the tail fins.
                let w = (1.0 - t).powf(0.6) * (t * 4.0).min(1.0);
                c0 + v3(0.9 - 1.9 * t, spread * w * 6.0 + 0.1 * (t * 6.0 + k as f32).sin() * t, 0.0)
            })
            .collect();
        paint_on_guide(ed, &body, |t| 0.3 + 0.7 * (1.0 - t))?;
    }
    // Brighter core lines with the tapered first brush (pen + taper).
    r(ed, "brush.set", json!({"kind": "pen", "color": "#cfe6ff", "size": 22, "opacity": 0.8, "paint": {"taper": 0.8}}))?;
    for k in 0..3 {
        let line: Vec<Vec3> = (0..20).map(|i| c0 + v3(0.85 - 1.9 * i as f32 / 19.0, (k as f32 - 1.0) * 0.12 * (i as f32 / 19.0), 0.0)).collect();
        paint_on_guide(ed, &line, |t| 1.0 - t * 0.6)?;
    }
    r(ed, "guide.close", Value::Null)?;
    // Turn it to swim across the view (as the tutorial re-poses the creature at the end).
    if let Some(cg) = ed.scene.groups.iter().find(|g| g.name.starts_with("Creature")).map(|g| g.id) {
        r(ed, "select.group", json!({"id": cg}))?;
        r(ed, "transform.apply", json!({"mode": "rotate", "axis": "y", "amount": -50}))?;
        r(ed, "select.none", Value::Null)?;
    }

    // Part 4: terrain. Dark navy (not black), short dense strokes on vertical and horizontal
    // guides, a few floating rocks.
    r(ed, "group.new", json!({"name": "Terrain (dense dark strokes)"}))?;
    r(ed, "brush.set", json!({"kind": "marker", "color": "#070b26", "size": 110, "opacity": 1.0, "material": "shaded", "pressure": false}))?;
    let mut i = 0u32;
    // Seabed below, a ridge behind, rocks to the side and floating above.
    for (cx, cy, cz, w, h, n) in [
        (-2.5f32, -3.4f32, -0.8f32, 9.0f32, 0.8f32, 220u32),
        (2.0, -1.8, -4.5, 7.0, 2.0, 200),
        (-8.5, 0.5, -3.0, 1.6, 4.0, 120),
        (2.6, 2.3, -2.6, 2.6, 0.9, 70),
        (-5.0, 2.8, -3.0, 1.6, 0.8, 50),
    ] {
        for _ in 0..n {
            i += 1;
            let p = v3(cx + (rnd(i, 1) - 0.5) * w, cy + (rnd(i, 2) - 0.5) * h * (1.0 - (rnd(i, 1) - 0.5).abs()), cz + (rnd(i, 3) - 0.5) * 0.8);
            let d = v3(rnd(i, 4) - 0.5, (rnd(i, 5) - 0.5) * 0.4, 0.0) * 1.4;
            r(ed, "stroke.add", json!({"points": pts(&[p, p + d * 0.5, p + d])}))?;
        }
    }
    // Bioluminescent dots on the terrain.
    r(ed, "brush.set", json!({"kind": "pen", "color": "#5fffd0", "size": 40, "material": "glow", "glow": 0.9, "opacity": 0.9}))?;
    for k in 0..40 {
        let p = v3(-6.0 + rnd(k, 7) * 10.0, -3.0 + rnd(k, 8) * 0.5, -0.2 + rnd(k, 9) * 0.6);
        r(ed, "stroke.add", json!({"points": pts(&[p])}))?;
    }

    // Part 5: particles. Mirror X and Y on, dots placed around a spiral, then mirror off.
    r(ed, "group.new", json!({"name": "Particles (mirror X + Y)"}))?;
    r(ed, "mirror.set", json!({"on": true, "x": true, "y": true, "z": false}))?;
    r(ed, "brush.set", json!({"kind": "pen", "color": "#a9c8ff", "size": 18, "material": "glow", "opacity": 0.8, "glow": 0.6}))?;
    for k in 0..36 {
        let a = k as f32 * 0.7;
        let rad = 1.2 + k as f32 * 0.12;
        let p = v3(rad * a.cos(), rad * a.sin() * 0.6, -1.0 + rnd(k, 11) * 3.0);
        r(ed, "stroke.add", json!({"points": pts(&[p])}))?;
    }
    r(ed, "mirror.set", json!({"on": false}))?;

    // Finish: Render mode with the tutorial's effects, lit from above, and shots.
    r(
        ed,
        "env.set",
        json!({"render": true, "lighting": {"azimuth": -60, "altitude": 55, "strength": 1.1}, "effects": {"bloom": 0.55, "dof": 5.6, "grain": 0.12, "pixelate": 0, "glow": 0.4}}),
    )?;
    for (yaw, pitch, dist, target) in [(-55.0, 18.0, 9.0, [-1.8, -0.6, 0.3]), (-80.0, 4.0, 5.0, [-1.0, 0.0, 0.0]), (-50.0, 6.0, 5.0, [-2.8, -1.5, 2.2])] {
        r(ed, "camera.set", json!({"yaw": yaw, "pitch": pitch, "distance": dist, "target": target, "orthographic": false}))?;
        r(ed, "shot.add", Value::Null)?;
    }
    r(ed, "camera.set", json!({"yaw": -55, "pitch": 18, "distance": 9, "target": [-1.8, -0.6, 0.3], "orthographic": false}))?;
    r(ed, "tool.set", json!({"tool": "draw"}))?;
    r(ed, "feel.set", json!({"reset": true}))?;
    r(ed, "stable.set", json!({"value": 0.3}))?;
    r(ed, "assist.set", json!({"quickShape": true}))?;
    r(ed, "snap.set", json!({"ends": true}))?;
    ed.status = "Deep sea: Feather's advanced tutorial, one group per technique".into();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_deep_sea_scene_builds_and_glows() {
        let mut e = Editor::new();
        e.set_viewport(960.0, 600.0);
        build(&mut e).expect("deep sea");
        let sc = &e.scene;
        assert!(sc.environment.render_mode);
        assert!(sc.strokes.iter().filter(|s| s.brush.material == crate::model::Material::Glow).count() > 60);
        assert!(sc.strokes.len() > 250, "{}", sc.strokes.len());
        assert!(sc.groups.len() >= 6);
        assert!(e.render(0, false).triangles > 5000);
        let sc = &e.scene;
        // Every creature curve landed whole on its guide.
        let creature = sc.groups.iter().find(|g| g.name.starts_with("Creature")).map(|g| g.id);
        let cs: Vec<_> = sc.strokes.iter().filter(|s| Some(s.group) == creature).collect();
        assert_eq!(cs.len(), 17, "{}", cs.len());
    }
}
