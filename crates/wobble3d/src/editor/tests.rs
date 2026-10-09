use super::*;
use crate::camera::PerfectView;
use serde_json::json;

fn ed() -> Editor {
    let mut e = Editor::new();
    e.set_viewport(1280.0, 800.0);
    e.stable = 0.0;
    e
}

fn circle(cx: f32, cy: f32, r: f32, n: usize) -> Vec<[f32; 2]> {
    (0..=n).map(|i| {
        let a = std::f32::consts::TAU * i as f32 / n as f32;
        [cx + r * a.cos(), cy + r * a.sin()]
    })
    .collect()
}

fn pts(p: &[[f32; 2]]) -> Value {
    json!(p.iter().map(|q| [q[0], q[1]]).collect::<Vec<_>>())
}

#[test]
fn draw_in_the_air_then_undo_and_redo() {
    let mut e = ed();
    let line: Vec<[f32; 2]> = (0..40).map(|i| [400.0 + i as f32 * 10.0, 400.0]).collect();
    let made = e.run("stroke.draw", &json!({"points": pts(&line)})).expect("draw");
    assert_eq!(made.as_array().map(Vec::len), Some(1));
    assert_eq!(e.scene.strokes.len(), 1);
    assert!(e.scene.strokes[0].points.len() > 30);
    assert!(e.dirty);
    e.run("edit.undo", &Value::Null).expect("undo");
    assert!(e.scene.strokes.is_empty());
    e.run("edit.redo", &Value::Null).expect("redo");
    assert_eq!(e.scene.strokes.len(), 1);
}

#[test]
fn a_guide_drawn_from_the_front_takes_strokes_drawn_from_the_side() {
    let mut e = ed();
    e.run("camera.view", &json!({"view": "front"})).expect("front");
    let gid = e.run("guide.draw", &json!({"points": pts(&circle(640.0, 400.0, 120.0, 64))})).expect("guide");
    assert!(gid.as_u64().is_some());
    assert!(e.scene.active_guide().is_some());
    e.run("camera.view", &json!({"view": "right"})).expect("right");
    // A vertical line across the tube from the side lands on its near wall.
    let line: Vec<[f32; 2]> = (0..30).map(|i| [640.0, 300.0 + i as f32 * 7.0]).collect();
    e.run("tool.set", &json!({"tool": "draw"})).expect("draw tool");
    e.run("stroke.draw", &json!({"points": pts(&line)})).expect("draw");
    let st = e.scene.strokes.last().expect("stroke");
    let view_front = {
        let mut c = e.camera;
        c.snap(PerfectView::Front);
        c.view()
    };
    let radius = view_front.world_per_px(e.camera.distance) * 120.0;
    for p in &st.points {
        let r = (p.p.x * p.p.x + p.p.y * p.p.y).sqrt();
        assert!((r - radius).abs() < radius * 0.08, "on the tube: r={r} want {radius}");
        assert!(p.n != Vec3::ZERO, "carries the surface normal");
    }
    // Bend it from the top into a doughnut.
    e.run("camera.view", &json!({"view": "top"})).expect("top");
    e.run("guide.bend", &json!({"points": pts(&circle(640.0, 400.0, 300.0, 96))})).expect("bend");
    let (lo, hi) = e.scene.active_guide().and_then(|g| g.bounds()).expect("bounds");
    assert!(hi.x - lo.x > radius * 4.0);
    // Close, recall, save.
    assert!(e.close_guide());
    assert!(e.scene.active_guide().is_none());
    e.run("guide.recall", &Value::Null).expect("recall");
    assert!(e.scene.active_guide().is_some());
    e.run("guide.save", &Value::Null).expect("save");
    assert!(e.scene.active_guide().is_none());
    assert!(e.scene.guides.iter().any(|g| g.saved));
    e.run("edit.undo", &Value::Null).expect("undo save");
    assert!(e.scene.active_guide().is_some());
}

#[test]
fn without_a_guide_or_air_drawing_there_is_a_clear_error() {
    let mut e = ed();
    e.draw_in_air = false;
    e.pointer_down(10.0, 10.0, 1.0, 0.0, Mods::default());
    assert!(e.scene.strokes.is_empty());
    assert!(e.status.contains("3D Guide"));
}

#[test]
fn primitives_and_lofts_are_sessions() {
    let mut e = ed();
    e.run("guide.primitive", &json!({"kind": "tube", "segments": 6})).expect("primitive");
    assert_eq!(e.tool, Tool::Primitive);
    assert_eq!(e.scene.guides.len(), 1);
    e.run("guide.segments", &json!({"value": 3})).expect("segments");
    e.run("transform.apply", &json!({"mode": "scale", "amount": 2.0})).expect("scale the primitive");
    e.run("guide.done", &Value::Null).expect("done");
    assert_eq!(e.tool, Tool::Draw);
    assert!(e.scene.active_guide().is_some());
    e.run("edit.undo", &Value::Null).expect("undo");
    assert!(e.scene.guides.is_empty(), "the whole primitive session is one undo step");

    // Loft between two curves.
    let mut e = ed();
    e.run("stroke.add", &json!({"points": [[-1,0,0],[0,0.2,0],[1,0,0]]})).expect("a");
    e.run("stroke.add", &json!({"points": [[-1,0,2],[0,0.4,2],[1,0,2]]})).expect("b");
    let ids: Vec<u64> = e.scene.strokes.iter().map(|s| s.id).collect();
    e.run("guide.loft", &json!({"ids": ids, "tension": 0.8})).expect("loft");
    assert!(e.scene.active_guide().is_some());
    assert!(e.run("guide.loft", &json!({"ids": [ids[0]]})).is_err());
}

#[test]
fn blender_transforms_follow_axes_and_numbers_and_cancel() {
    let mut e = ed();
    e.run("stroke.add", &json!({"points": [[0,0,0],[1,0,0]]})).expect("add");
    e.run("select.all", &Value::Null).expect("select");
    e.run("transform.start", &json!({"mode": "grab", "x": 600, "y": 400})).expect("G");
    e.run("transform.axis", &json!({"axis": "z"})).expect("Z");
    e.run("transform.type", &json!({"text": "2"})).expect("2");
    e.run("transform.confirm", &Value::Null).expect("enter");
    let p = e.scene.strokes[0].points[0].p;
    assert!((p.z - 2.0).abs() < 1e-5 && p.x.abs() < 1e-5, "{p:?}");
    // Cancel restores.
    e.run("transform.start", &json!({"mode": "rotate", "x": 700, "y": 400})).expect("R");
    e.run("transform.mouse", &json!({"x": 640, "y": 200})).expect("move");
    assert_ne!(e.scene.strokes[0].points[0].p, p);
    e.cancel();
    assert_eq!(e.scene.strokes[0].points[0].p, p);
    // Exact rotation.
    e.run("transform.apply", &json!({"mode": "rotate", "axis": "y", "amount": 90})).expect("rotate");
    e.run("edit.undo", &Value::Null).expect("undo");
    assert_eq!(e.scene.strokes[0].points[0].p, p);
    assert!(e.run("transform.start", &json!({"mode": "spin"})).is_err());
    e.run("select.none", &Value::Null).expect("none");
    assert!(e.run("transform.start", &json!({"mode": "grab"})).is_err());
}

#[test]
fn the_gizmo_starts_constrained_moves() {
    let mut e = ed();
    e.run("stroke.add", &json!({"points": [[0,0,0],[0.5,0.5,0]]})).expect("add");
    e.run("tool.set", &json!({"tool": "select"})).expect("select tool");
    e.run("select.all", &Value::Null).expect("all");
    let parts = e.gizmo_parts().expect("gizmo shows");
    let tip = parts.iter().find(|p| p.handle == Handle::Move(0) && !p.filled).and_then(|p| p.line.last().copied()).expect("x arrow");
    let before = e.scene.strokes[0].points[0].p;
    e.pointer_down(tip[0], tip[1], 1.0, 0.0, Mods::default());
    assert!(e.in_modal());
    e.pointer_move(tip[0] + 60.0, tip[1] - 40.0, 1.0, 0.1, Mods::default());
    e.pointer_down(tip[0] + 60.0, tip[1] - 40.0, 1.0, 0.2, Mods::default());
    let after = e.scene.strokes[0].points[0].p;
    assert!(after.x != before.x && (after.y - before.y).abs() < 1e-5 && (after.z - before.z).abs() < 1e-5, "{before:?} {after:?}");
}

#[test]
fn erase_select_delete_duplicate_and_mirror() {
    let mut e = ed();
    e.run("camera.view", &json!({"view": "front"})).expect("front");
    e.run("mirror.set", &json!({"on": true, "x": true})).expect("mirror");
    let line: Vec<[f32; 2]> = (0..30).map(|i| [700.0 + i as f32 * 5.0, 400.0]).collect();
    e.run("stroke.draw", &json!({"points": pts(&line)})).expect("draw");
    assert_eq!(e.scene.strokes.len(), 2, "mirror copy");
    e.run("mirror.set", &json!({"on": false})).expect("mirror off");
    // Vacuum the copy on the left.
    e.run("erase.at", &json!({"x": 640.0 - 70.0, "y": 400, "vacuum": true})).expect("vacuum");
    assert_eq!(e.scene.strokes.len(), 1);
    // Erase splits.
    e.run("erase.at", &json!({"x": 770, "y": 400, "radius": 4})).expect("erase");
    assert_eq!(e.scene.strokes.len(), 2);
    e.run("select.all", &Value::Null).expect("all");
    let made = e.run("edit.duplicate", &json!({"mode": "view"})).expect("dup by view");
    assert_eq!(made.as_array().map(Vec::len), Some(2));
    assert!(e.run("edit.duplicate", &json!({"mode": "mirror"})).is_err(), "mirror is off");
    e.run("select.box", &json!({"x0": 0, "y0": 0, "x1": 640, "y1": 800})).expect("box");
    assert_eq!(e.selection.len(), 2, "the view-mirrored copies are on the left");
    e.run("edit.delete", &Value::Null).expect("delete");
    assert_eq!(e.scene.strokes.len(), 2);
    e.run("select.at", &json!({"x": 5, "y": 5})).expect("click empty");
    assert!(e.selection.is_empty());
}

#[test]
fn draw_shape_straightens_and_holding_adjusts() {
    let mut e = ed();
    e.run("camera.view", &json!({"view": "front"})).expect("front");
    // Draw is already the tool: pressing it again switches to Draw Shape (Feather's B, B).
    e.run("tool.set", &json!({"tool": "draw", "cycle": true})).expect("draw shape");
    assert_eq!(e.tool, Tool::DrawShape);
    let m = Mods::default();
    e.pointer_down(400.0, 400.0, 1.0, 0.0, m);
    for i in 1..30 {
        e.pointer_move(400.0 + i as f32 * 10.0, 400.0 + ((i % 3) as f32 - 1.0) * 2.0, 1.0, i as f64 * 0.01, m);
    }
    // Rest: the line is recognised; then drag the end.
    e.tick(1.0);
    assert!(e.overlay().adjusting);
    e.pointer_move(690.0, 300.0, 1.0, 1.1, m);
    e.pointer_up(690.0, 300.0, 1.2, m);
    let st = e.scene.strokes.last().expect("line");
    let v = e.view();
    let end = st.points.last().and_then(|p| v.project(p.p)).expect("end");
    assert!((end.x - 690.0).abs() < 2.0 && (end.y - 300.0).abs() < 2.0, "{end:?}");
    // Straight: every point on the chord.
    let a = v.project(st.points[0].p).expect("start");
    for p in &st.points {
        let q = v.project(p.p).expect("p");
        let t = ((q.x - a.x) * (end.x - a.x) + (q.y - a.y) * (end.y - a.y)) / ((end.x - a.x).powi(2) + (end.y - a.y).powi(2));
        let (px, py) = (a.x + (end.x - a.x) * t, a.y + (end.y - a.y) * t);
        assert!(((q.x - px).powi(2) + (q.y - py).powi(2)).sqrt() < 1.0);
    }
}

#[test]
fn the_brush_panel_edits_selected_curves() {
    let mut e = ed();
    e.run("stroke.add", &json!({"points": [[0,0,0],[1,0,0]]})).expect("add");
    e.run("select.all", &Value::Null).expect("all");
    e.run("brush.set", &json!({"color": "#ff0000", "kind": "oil", "size": 40})).expect("brush");
    let b2 = e.scene.strokes[0].brush;
    assert_eq!(b2.color, Rgba::rgb(255, 0, 0));
    assert_eq!(b2.kind, BrushKind::Oil);
    assert!(b2.paint.bristles > 0.0);
    assert!(e.run("brush.set", &json!({"color": "red"})).is_err());
    assert!(e.run("brush.set", &json!({"kind": "laser"})).is_err());
    e.run("brush.set", &json!({"size": 1e12})).expect("clamped");
    assert_eq!(e.brush.size_mm, crate::model::SIZE_MAX_MM);
}

#[test]
fn groups_hide_isolate_merge_and_block_drawing_when_hidden() {
    let mut e = ed();
    let g2 = e.run("group.new", &json!({"name": "Wings"})).expect("group").as_u64().expect("id");
    e.run("stroke.add", &json!({"points": [[0,0,0],[1,0,0]]})).expect("add");
    assert_eq!(e.scene.strokes[0].group, g2);
    e.run("group.hideActive", &Value::Null).expect("hide");
    assert!(e.run("stroke.add", &json!({"points": [[0,0,0]]})).is_err());
    e.run("group.showAll", &Value::Null).expect("show");
    let g1 = e.scene.groups[0].id;
    e.run("group.merge", &json!({"ids": [g1, g2]})).expect("merge");
    assert_eq!(e.scene.groups.len(), 1);
    assert!(e.run("group.delete", &json!({"ids": [e.scene.groups[0].id]})).is_err(), "keeps one group");
}

#[test]
fn files_round_trip_through_commands() {
    let mut e = ed();
    e.run("stroke.add", &json!({"points": [[0,0,0],[1,1,1]]})).expect("add");
    let file = e.run("file.serialize", &Value::Null).expect("save");
    let hex = file.get("hex").and_then(Value::as_str).expect("hex").to_string();
    let mut e2 = ed();
    e2.run("file.deserialize", &json!({"hex": hex})).expect("load");
    assert_eq!(e2.scene.strokes, e.scene.strokes);
    assert!(e2.run("file.deserialize", &json!({"hex": "zz"})).is_err());
    let obj = e.run("export.obj", &Value::Null).expect("obj");
    assert!(obj.get("obj").and_then(Value::as_str).is_some_and(|o| o.contains("\nf ")));
}

#[test]
fn liquify_session_undo_all_and_compare() {
    let mut e = ed();
    e.run("camera.view", &json!({"view": "front"})).expect("front");
    e.run("stroke.add", &json!({"points": (0..21).map(|i| [i as f32 * 0.1 - 1.0, 0.0, 0.0]).collect::<Vec<_>>()})).expect("add");
    e.run("select.all", &Value::Null).expect("all");
    e.run("tool.set", &json!({"tool": "liquify"})).expect("liquify");
    let before = e.scene.strokes.clone();
    e.run("liquify.stroke", &json!({"points": [[640, 400], [640, 360]]})).expect("push");
    assert_ne!(e.scene.strokes, before);
    e.run("liquify.compare", &json!({"on": true})).expect("compare");
    assert_eq!(e.shown_scene().strokes, before);
    e.run("liquify.compare", &json!({"on": false})).expect("compare off");
    e.run("liquify.undoAll", &Value::Null).expect("undo all");
    assert_eq!(e.scene.strokes, before);
    e.run("liquify.apply", &Value::Null).expect("apply");
    assert!(e.run("liquify.undoAll", &Value::Null).is_err());
}

#[test]
fn every_command_survives_hostile_params() {
    let hostile = [
        Value::Null,
        json!({}),
        json!([]),
        json!("x"),
        json!({"x": f64::MAX, "y": -1e308, "dx": 1e308, "dy": f64::MIN, "factor": 0, "mm": -5, "by": 1e30}),
        json!({"view": "sideways", "tool": "nope", "mode": 5, "kind": "?", "axis": "w", "text": "--..9e9e9", "amount": 1e30}),
        json!({"points": [[1], [f64::MAX, 2, 3], "x", []], "ids": [-1, "a", 1e30], "id": 18446744073709551615u64}),
        json!({"points": [[0,0],[1e9,1e9],[-1e9,5]], "ids": [], "id": 0, "index": 1e19, "value": -1e9, "segments": 1e12}),
        json!({"hex": "1f8b", "obj": "v 1 1\nf 1 2 3", "width": 4294967295u64, "height": 2, "rgbaHex": "ff"}),
        json!({"color": "#gg0000", "pattern": {"kind": "dot", "angle": 1e300}, "paint": {"layers": 1e9, "echo": {"offset": [1e300, "a"]}}}),
        json!({"lighting": {"azimuth": 1e300, "color": "#zz"}, "effects": {"pixelate": -4}, "frames": 0, "fps": -1}),
    ];
    for (cmd, _) in COMMANDS {
        for h in &hostile {
            let mut e = ed();
            // Something to work on.
            let _ = e.run("stroke.add", &json!({"points": [[0,0,0],[1,0,0]]}));
            let _ = e.run("select.all", &Value::Null);
            let _ = e.run(cmd, h);
            let _ = e.run(cmd, h);
            let _ = e.render(1, true);
            let _ = e.overlay();
        }
    }
}

#[test]
fn pointer_sequences_with_garbage_do_not_panic() {
    let mut e = ed();
    for tool in Tool::ALL {
        let _ = e.run("tool.set", &json!({"tool": tool.name()}));
        let m = Mods { shift: true, ctrl: false, alt: true };
        e.pointer_down(f32::NAN, 3.0, f32::NAN, 0.0, m);
        e.pointer_down(10.0, 10.0, 7.0, 0.0, m);
        e.pointer_move(f32::INFINITY, 0.0, 1.0, 0.1, m);
        e.pointer_move(1e9, -1e9, 1.0, 0.2, m);
        e.tick(5.0);
        e.pointer_up(f32::NAN, f32::NAN, 0.3, m);
        e.cancel();
        let _ = e.render(0, true);
    }
}
