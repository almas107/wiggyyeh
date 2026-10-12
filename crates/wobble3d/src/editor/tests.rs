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
    (0..=n)
        .map(|i| {
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

#[test]
fn stamp_drops_copies_where_you_click() {
    let mut e = ed();
    e.run("camera.view", &json!({"view": "front"})).expect("front");
    e.run("stroke.add", &json!({"points": [[0,0,0],[0.2,0.1,0]]})).expect("add");
    assert!(e.run("tool.set", &json!({"tool": "stamp"})).is_err(), "needs a selection");
    e.run("select.all", &Value::Null).expect("all");
    e.run("tool.set", &json!({"tool": "stamp"})).expect("stamp");
    e.pointer_down(200.0, 200.0, 1.0, 0.0, Mods::default());
    for i in 1..20 {
        e.pointer_move(200.0 + i as f32 * 20.0, 200.0, 1.0, i as f64 * 0.01, Mods::default());
    }
    e.pointer_up(580.0, 200.0, 0.3, Mods::default());
    assert!(e.scene.strokes.len() >= 4, "{}", e.scene.strokes.len());
    assert_eq!(e.selection.len(), 1, "the original stays selected");
    let v = e.view();
    let first = e.scene.strokes[1].centre();
    let s = v.project(first).expect("on screen");
    assert!((s.x - 200.0).abs() < 2.0 && (s.y - 200.0).abs() < 2.0, "{s:?}");
}

#[test]
fn drawing_lands_on_active_images_and_models_and_notes_import_as_groups() {
    let mut e = ed();
    e.draw_in_air = false;
    e.run("camera.view", &json!({"view": "front"})).expect("front");
    // A model: a big square facing the camera at z = 0.5.
    let obj = "v -2 -2 0.5\nv 2 -2 0.5\nv 2 2 0.5\nv -2 2 0.5\nf 1 2 3 4\n";
    let mid = e.run("resource.addModel", &json!({"obj": obj})).expect("model").as_u64().expect("id");
    e.run("resource.state", &json!({"id": mid, "state": "active"})).expect("draw on it");
    let line: Vec<[f32; 2]> = (0..20).map(|i| [500.0 + i as f32 * 10.0, 400.0]).collect();
    e.run("stroke.draw", &json!({"points": line.iter().map(|p| [p[0], p[1]]).collect::<Vec<_>>()})).expect("draw");
    let st = e.scene.strokes.last().expect("stroke");
    assert!(st.points.iter().all(|p| p.p.z.abs() < 1e-3), "the model was centred on the grid, so its face is at z = 0");
    // An image resource limits drawing to its rectangle.
    e.run("resource.state", &json!({"id": mid, "state": "hidden"})).expect("hide model");
    let img = e.add_image("ref", 2, 2, vec![255; 16]).expect("image");
    e.run("resource.state", &json!({"id": img, "state": "active"})).expect("draw on image");
    let n = e.scene.strokes.len();
    let off_image: Vec<[f32; 2]> = (0..10).map(|i| [5.0 + i as f32, 5.0]).collect();
    e.run("stroke.draw", &json!({"points": off_image})).expect("draw outside");
    assert_eq!(e.scene.strokes.len(), n, "nothing outside the image");
    e.run("stroke.draw", &json!({"points": [[630, 390], [650, 410]]})).expect("draw on image");
    assert_eq!(e.scene.strokes.len(), n + 1);
    // Find Group, Lighten, Import.
    let v = e.view();
    let p = v.project(e.scene.strokes[0].points[3].p).expect("on screen");
    assert_eq!(e.run("info.groupAt", &json!({"x": p.x, "y": p.y})).expect("group"), json!("Group 1"));
    assert!(e.run("file.lighten", &json!({"all": true})).expect("lighten").as_u64().is_some());
    let other = e.scene.clone();
    let groups = e.import_note(&other, "Copy").expect("import");
    assert_eq!(groups.len(), 1);
    assert_eq!(e.scene.strokes.len(), (n + 1) * 2);
}

/// Draw a jittery horizontal line with real pointer events; the curve's vertical wiggle in
/// world units.
fn jittery_line(e: &mut Editor, mods: Mods) -> f32 {
    e.pointer_down(300.0, 400.0, 0.5, 0.0, mods);
    for i in 1..60 {
        let y = 400.0 + if i % 2 == 0 { 4.0 } else { -4.0 };
        e.pointer_move(300.0 + i as f32 * 8.0, y, 0.5, i as f64 * 0.01, mods);
    }
    e.pointer_up(780.0, 400.0, 0.6, mods);
    let s = e.scene.strokes.last().expect("a curve");
    s.points.windows(2).map(|w| (w[1].p.y - w[0].p.y).abs()).sum()
}

#[test]
fn post_processing_smooths_what_the_pen_drew() {
    let mut e = ed();
    e.run("camera.view", &json!({"view": "front"})).expect("front");
    e.run("feel.set", &json!({"post": false})).expect("off");
    let raw = jittery_line(&mut e, Mods::default());
    e.run("feel.set", &json!({"post": true, "smooth": 1.0, "smoothIterations": 4})).expect("on");
    let smoothed = jittery_line(&mut e, Mods::default());
    assert!(smoothed < raw * 0.4, "smoothed {smoothed} vs raw {raw}");
    // Subdivide adds points; simplify removes them.
    e.run("feel.set", &json!({"subdivide": 1, "simplify": 0.0})).expect("sub");
    let n_sub = {
        jittery_line(&mut e, Mods::default());
        e.scene.strokes.last().map_or(0, |s| s.points.len())
    };
    e.run("feel.set", &json!({"subdivide": 0, "simplify": 1.0, "smoothIterations": 10, "smooth": 2.0})).expect("simp");
    let n_simple = {
        jittery_line(&mut e, Mods::default());
        e.scene.strokes.last().map_or(0, |s| s.points.len())
    };
    assert!(n_simple < n_sub, "{n_simple} < {n_sub}");
}

#[test]
fn shift_toggles_the_stabilizer_and_the_pressure_curve_applies() {
    let mut e = ed();
    e.run("camera.view", &json!({"view": "front"})).expect("front");
    let shift = Mods { shift: true, ..Mods::default() };
    // Stable Stroke off: Shift turns it on for this stroke.
    e.pointer_down(300.0, 300.0, 1.0, 0.0, shift);
    assert_eq!(e.live.as_ref().map(|l| l.stab.amount), Some(SHIFT_STABLE));
    e.cancel();
    // Stable Stroke on: Shift turns it off.
    e.run("stable.set", &json!({"value": 0.6})).expect("stable");
    e.pointer_down(300.0, 300.0, 1.0, 0.0, shift);
    assert_eq!(e.live.as_ref().map(|l| l.stab.amount), Some(0.0));
    e.cancel();
    e.pointer_down(300.0, 300.0, 1.0, 0.0, Mods::default());
    assert_eq!(e.live.as_ref().map(|l| l.stab.amount), Some(0.6));
    e.cancel();
    // A floor of 0.5: the lightest touch still draws half pressure.
    e.run("stable.set", &json!({"value": 0.0})).expect("stable");
    e.run("feel.set", &json!({"pressureMin": 0.5, "post": false})).expect("curve");
    e.pointer_down(300.0, 300.0, 0.0, 0.0, Mods::default());
    e.pointer_move(400.0, 300.0, 0.0, 0.1, Mods::default());
    e.pointer_up(500.0, 300.0, 0.2, Mods::default());
    let s = e.scene.strokes.last().expect("curve");
    assert!(s.points.iter().all(|p| p.pressure >= 0.5 - 1e-6), "{:?}", s.points.iter().map(|p| p.pressure).collect::<Vec<_>>());
}

#[test]
fn feel_set_rejects_bad_params() {
    let mut e = ed();
    for p in [json!({}), json!({"smooth": "lots"}), json!({"subdivide": -1}), json!({"post": 1}), json!({"smoothIterations": 2.5})] {
        assert!(e.run("feel.set", &p).is_err(), "{p}");
    }
    let v = e.run("feel.set", &json!({"smooth": 99, "subdivide": 50, "pressureGamma": 0})).expect("clamped");
    assert_eq!(v["smooth"], 2.0);
    assert_eq!(v["subdivide"], 3);
    assert_eq!(v["pressureGamma"], 0.25);
    let v = e.run("feel.set", &json!({"reset": true})).expect("reset");
    assert_eq!(v, serde_json::to_value(crate::feel::StrokeFeel::default()).expect("json"));
    assert_eq!(e.run("feel.get", &Value::Null).expect("get"), v);
}

/// The colour of a rendered pixel (CPU raster of the current view).
fn pixel(e: &mut Editor, x: usize, y: usize) -> [u8; 4] {
    let (w, h) = (e.camera.viewport.width as u32, e.camera.viewport.height as u32);
    let f = e.render(0, false);
    let tex = crate::raster::Textures { atlas: &e.atlas, images: std::collections::HashMap::new() };
    let pic = crate::raster::rasterize(&f, &tex, w, h, Some(e.scene.environment.background), None).expect("raster");
    let i = (y * w as usize + x) * 4;
    [pic.rgba[i], pic.rgba[i + 1], pic.rgba[i + 2], pic.rgba[i + 3]]
}

fn front_ed() -> Editor {
    let mut e = Editor::new();
    e.set_viewport(400.0, 300.0);
    e.stable = 0.0;
    e.feel.post = false;
    e.run("env.set", &json!({"grid": false})).expect("env");
    e.run("boil.set", &json!({"enabled": false})).expect("boil");
    e.run("camera.view", &json!({"view": "front"})).expect("front");
    e
}

fn square(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<[f32; 2]> {
    let c = [[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]];
    c.windows(2)
        .flat_map(|w| (0..20).map(move |i| [w[0][0] + (w[1][0] - w[0][0]) * i as f32 / 20.0, w[0][1] + (w[1][1] - w[0][1]) * i as f32 / 20.0]))
        .chain([[x0, y0]])
        .collect()
}

/// The paint bucket fills a closed outline (under it, in the brush colour) and refuses an
/// open one or a click outside.
#[test]
fn the_bucket_fills_closed_shapes_under_their_lines() {
    let mut e = front_ed();
    e.run("brush.set", &json!({"kind": "marker", "color": "#000000", "size": 20, "pressure": false})).expect("ink");
    e.run("stroke.draw", &json!({"points": pts(&square(120.0, 80.0, 280.0, 220.0))})).expect("outline");
    let outline_id = e.scene.strokes[0].id;
    e.run("brush.set", &json!({"color": "#ff0000"})).expect("red");
    let ids = e.run("fill.at", &json!({"x": 200, "y": 150})).expect("fill");
    assert_eq!(ids.as_array().map(Vec::len), Some(1));
    assert_eq!(e.scene.strokes.len(), 2);
    assert!(e.scene.strokes[0].is_fill(), "the fill goes under the outline");
    assert_eq!(e.scene.strokes[1].id, outline_id);
    let p = pixel(&mut e, 200, 150);
    assert!(p[0] > 200 && p[1] < 60 && p[2] < 60, "inside is red: {p:?}");
    let edge = pixel(&mut e, 120, 150);
    assert!(edge[0] < 80 && edge[1] < 80, "the line stays on top: {edge:?}");
    assert!(pixel(&mut e, 60, 150)[1] > 200, "outside stays white");
    // One undo step.
    e.run("edit.undo", &Value::Null).expect("undo");
    assert_eq!(e.scene.strokes.len(), 1);
    // Outside the shape the flood leaks: an error, nothing made.
    assert!(e.run("fill.at", &json!({"x": 30, "y": 30})).is_err());
    assert!(e.run("fill.at", &json!({"x": -5, "y": 30})).is_err());
    assert!(e.run("fill.at", &json!({"x": f64::NAN, "y": 30})).is_err());
    assert_eq!(e.scene.strokes.len(), 1);
}

/// Lasso Fill and filled shapes paint on the guide; later paint covers earlier paint; the
/// eraser leaves fills alone and Vacuum takes them whole; select and move work on them.
#[test]
fn lasso_fill_and_shapes_paint_on_the_guide() {
    let mut e = front_ed();
    e.run("brush.set", &json!({"color": "#0000ff"})).expect("blue");
    e.run("tool.set", &json!({"tool": "lassofill"})).expect("tool");
    let loop_pts = circle(200.0, 150.0, 80.0, 48);
    e.pointer_down(loop_pts[0][0], loop_pts[0][1], 1.0, 0.0, Mods::default());
    for (k, q) in loop_pts.iter().enumerate().skip(1) {
        e.pointer_move(q[0], q[1], 1.0, 0.01 * k as f64, Mods::default());
    }
    e.pointer_up(200.0 + 80.0, 150.0, 1.0, Mods::default());
    assert_eq!(e.scene.strokes.len(), 1, "{}", e.status);
    assert!(e.scene.strokes[0].is_fill());
    let p = pixel(&mut e, 200, 150);
    assert!(p[2] > 200 && p[0] < 60, "{p:?}");
    // A filled rectangle over it, by dragging the Shapes tool.
    e.run("brush.set", &json!({"color": "#00ff00"})).expect("green");
    e.run("tool.set", &json!({"tool": "shapes"})).expect("shapes");
    e.run("shape.set", &json!({"kind": "rect", "filled": true})).expect("rect");
    e.pointer_down(150.0, 120.0, 1.0, 0.0, Mods::default());
    e.pointer_move(250.0, 180.0, 1.0, 0.1, Mods::default());
    assert!(!e.overlay().polyline.is_empty(), "the shape previews while dragging");
    e.pointer_up(250.0, 180.0, 0.2, Mods::default());
    assert_eq!(e.scene.strokes.len(), 2);
    let p = pixel(&mut e, 200, 150);
    assert!(p[1] > 200 && p[0] < 60 && p[2] < 60, "the later rectangle covers the lasso fill: {p:?}");
    // Outline shapes are curves.
    e.run("fill.shape", &json!({"kind": "ellipse", "filled": false, "x0": 20, "y0": 20, "x1": 120, "y1": 80})).expect("outline");
    assert!(!e.scene.strokes[2].is_fill() && e.scene.strokes[2].points.len() > 20);
    assert!(e.run("fill.shape", &json!({"kind": "blob", "x0": 0, "y0": 0, "x1": 10, "y1": 10})).is_err());
    // Erase leaves fills, Vacuum removes the one under it.
    e.run("tool.set", &json!({"tool": "erase"})).expect("erase");
    e.run("erase.at", &json!({"x": 200, "y": 150})).expect("erase");
    assert_eq!(e.scene.strokes.iter().filter(|s| s.is_fill()).count(), 2);
    // Click-select picks the fill under the pointer and moves it with the gizmo-free drag.
    e.run("tool.set", &json!({"tool": "select"})).expect("select");
    e.pointer_down(200.0, 150.0, 1.0, 0.0, Mods::default());
    e.pointer_up(200.0, 150.0, 0.1, Mods::default());
    assert_eq!(e.selection.len(), 1);
    let sel = *e.selection.iter().next().expect("one");
    let before = e.scene.stroke(sel).expect("fill").centre();
    e.pointer_down(200.0, 150.0, 1.0, 0.2, Mods::default());
    e.pointer_move(230.0, 150.0, 1.0, 0.3, Mods::default());
    e.pointer_move(260.0, 150.0, 1.0, 0.4, Mods::default());
    e.pointer_up(260.0, 150.0, 0.5, Mods::default());
    assert!(!e.in_modal(), "letting go puts it down");
    let after = e.scene.stroke(sel).expect("fill").centre();
    assert!(after.x > before.x + 0.1, "dragged right: {before:?} → {after:?}");
    e.run("tool.set", &json!({"tool": "vacuum"})).expect("vacuum");
    e.run("erase.at", &json!({"x": 100, "y": 150, "vacuum": true})).expect("vacuum");
}

/// A primitive placed with the Primitives tool moves by dragging it, and becomes a solid,
/// coloured object with guide.solid.
#[test]
fn primitives_drag_into_place_and_turn_solid() {
    let mut e = front_ed();
    e.run("guide.primitive", &json!({"kind": "sphere"})).expect("sphere");
    let gid = e.scene.active_guide.expect("guide");
    let x0 = e.scene.guide(gid).expect("g").xform.translation.x;
    let view = e.view();
    let c = view.project(Vec3::ZERO).expect("centre");
    e.pointer_down(c.x, c.y, 1.0, 0.0, Mods::default());
    e.pointer_move(c.x + 20.0, c.y, 1.0, 0.1, Mods::default());
    e.pointer_move(c.x + 60.0, c.y, 1.0, 0.2, Mods::default());
    e.pointer_up(c.x + 60.0, c.y, 0.3, Mods::default());
    assert!(!e.in_modal());
    let x1 = e.scene.guide(gid).expect("g").xform.translation.x;
    assert!(x1 > x0 + 0.1, "the sphere moved right with the drag: {x0} → {x1}");
    e.run("guide.done", &Value::Null).expect("done");
    e.run("brush.set", &json!({"color": "#ff8800"})).expect("orange");
    let ids = e.run("guide.solid", &Value::Null).expect("solid");
    assert_eq!(ids.as_array().map(Vec::len), Some(1));
    let solid = e.scene.strokes.last().cloned().expect("solid");
    assert!(solid.is_fill() && solid.tris.len() > 100);
    let at = view.project(crate::math::v3(x1, 0.0, 0.0)).expect("on screen");
    let p = pixel(&mut e, at.x as usize, at.y as usize);
    assert!(p[0] > 200 && p[2] < 80, "the sphere is painted orange: {p:?}");
    // Survives a save and load.
    let hex = e.run("file.serialize", &Value::Null).expect("save");
    let mut f = Editor::new();
    let hex = hex.get("hex").cloned().unwrap_or(hex);
    f.run("file.deserialize", &json!({"hex": hex})).expect("load");
    assert_eq!(f.scene.strokes.last().map(|s| s.tris.len()), Some(solid.tris.len()));
}

/// QuickShape in plain Draw: a wobbly rectangle held at the end snaps to a clean one; a
/// scribble held at the end stays a scribble.
#[test]
fn holding_the_pen_snaps_a_rough_rectangle_clean() {
    let mut e = front_ed();
    e.run("tool.set", &json!({"tool": "draw"})).expect("draw");
    let rough: Vec<[f32; 2]> =
        square(100.0, 80.0, 300.0, 220.0).iter().enumerate().map(|(i, p)| [p[0] + ((i * 7) % 5) as f32 - 2.0, p[1] + ((i * 3) % 5) as f32 - 2.0]).collect();
    e.pointer_down(rough[0][0], rough[0][1], 1.0, 0.0, Mods::default());
    for (k, q) in rough.iter().enumerate().skip(1) {
        e.pointer_move(q[0], q[1], 1.0, 0.005 * k as f64, Mods::default());
    }
    let t = 0.005 * rough.len() as f64;
    e.tick(t + 0.1);
    assert!(!e.overlay().adjusting, "not yet");
    e.tick(t + HOLD_SECONDS + 0.05);
    assert!(e.overlay().adjusting, "held still: snapped ({})", e.status);
    assert!(e.status.contains("rectangle"), "{}", e.status);
    e.pointer_up(rough.last().expect("p")[0], rough.last().expect("p")[1], t + 0.6, Mods::default());
    let st = e.scene.strokes.last().expect("stroke");
    let view = e.view();
    let ys: Vec<f32> = st.points.iter().filter_map(|p| view.project(p.p)).map(|q| q.y).collect();
    // The top edge is a straight level line: many points share y within a pixel.
    let top = ys.iter().copied().fold(f32::MAX, f32::min);
    assert!(ys.iter().filter(|y| (**y - top).abs() < 1.0).count() > 20, "a clean level top edge");

    let scribble: Vec<[f32; 2]> = (0..80).map(|i| [100.0 + i as f32 * 3.0, 150.0 + ((i as f32) * 0.15).sin() * 30.0 + ((i * 13) % 3) as f32]).collect();
    let n = e.scene.strokes.len();
    e.pointer_down(scribble[0][0], scribble[0][1], 1.0, 10.0, Mods::default());
    for (k, q) in scribble.iter().enumerate().skip(1) {
        e.pointer_move(q[0], q[1], 1.0, 10.0 + 0.005 * k as f64, Mods::default());
    }
    e.tick(10.0 + 0.4 + HOLD_SECONDS + 0.05);
    assert!(!e.overlay().adjusting, "a scribble is not forced into a shape");
    e.pointer_up(340.0, 150.0, 11.5, Mods::default());
    assert_eq!(e.scene.strokes.len(), n + 1);
    // Off: no snapping.
    e.run("assist.set", &json!({"quickShape": false})).expect("off");
    assert!(!e.quick_shape);
}

/// Clipping masks: a group clipped to the group below shows its paint only on that group's
/// paint; releasing the mask shows all of it. The bottom group cannot clip.
#[test]
fn clipped_groups_paint_only_on_the_group_below() {
    let mut e = front_ed();
    e.run("brush.set", &json!({"color": "#ff0000"})).expect("red");
    e.run("fill.lasso", &json!({"points": pts(&circle(200.0, 150.0, 60.0, 48))})).expect("base");
    assert!(e.run("group.clip", &json!({})).is_err(), "nothing below the bottom group");
    let g2 = e.run("group.new", &json!({"name": "shading"})).expect("group").as_u64().expect("id");
    e.run("brush.set", &json!({"kind": "marker", "color": "#0000ff", "size": 60, "pressure": false})).expect("blue");
    e.run("stroke.draw", &json!({"points": pts(&(0..40).map(|i| [60.0 + i as f32 * 7.0, 150.0]).collect::<Vec<_>>())})).expect("band");
    let blue = |p: [u8; 4]| p[2] > 200 && p[0] < 60;
    assert!(blue(pixel(&mut e, 80, 150)), "unclipped: the band shows outside the circle");
    assert_eq!(e.run("group.clip", &json!({"id": g2})).expect("clip"), json!(true));
    assert!(blue(pixel(&mut e, 200, 150)), "on the circle: blue");
    let out = pixel(&mut e, 80, 150);
    assert!(out[0] > 200 && out[1] > 200 && out[2] > 200, "off the circle: nothing ({out:?})");
    let red = pixel(&mut e, 200, 110);
    assert!(red[0] > 200 && red[2] < 60, "the base shows where the clipped band isn't ({red:?})");
    // Release: the band shows everywhere again. Undo brings the clip back.
    e.run("group.clip", &json!({"id": g2, "clip": false})).expect("release");
    assert!(blue(pixel(&mut e, 80, 150)));
    e.run("edit.undo", &Value::Null).expect("undo");
    assert!(!blue(pixel(&mut e, 80, 150)));
    // Saved with the note.
    let hex = e.run("file.serialize", &Value::Null).expect("save");
    let mut f = Editor::new();
    f.run("file.deserialize", &json!({"hex": hex.get("hex").cloned().unwrap_or(Value::Null)})).expect("load");
    assert!(f.scene.groups.iter().any(|g| g.clip));
}

/// Feather users' requests: axis-plane guides without drawing one, a guide made from a curve,
/// stroke ends that snap to other ends, scaling that scales thickness, focus mode, grid opacity.
#[test]
fn comment_requests_planes_guides_from_curves_snapping_and_focus() {
    let mut e = front_ed();
    // The floor plane through the orbit point: drawing from the top lands on y = 0.
    let gid = e.run("guide.plane", &json!({"axis": "y"})).expect("plane").as_u64().expect("id");
    assert_eq!(e.scene.active_guide, Some(gid));
    e.run("camera.view", &json!({"view": "top"})).expect("top");
    e.run("stroke.draw", &json!({"points": pts(&[[150.0, 150.0], [200.0, 160.0], [250.0, 150.0]])})).expect("draw");
    assert!(e.scene.strokes[0].points.iter().all(|p| p.p.y.abs() < 1e-3), "on the floor");
    assert!(e.run("guide.plane", &json!({"axis": "w"})).is_err());
    // A second stroke starting 6 px from the first one's end snaps onto it.
    let end = e.scene.strokes[0].points.last().expect("end").p;
    let at = e.view().project(end).expect("on screen");
    e.run("stroke.draw", &json!({"points": pts(&[[at.x + 5.0, at.y + 3.0], [at.x + 40.0, at.y + 30.0], [at.x + 80.0, at.y + 40.0]])})).expect("draw");
    assert!(e.scene.strokes[1].points[0].p.distance(end) < 1e-5, "snapped to the end");
    e.run("snap.set", &json!({"ends": false})).expect("off");
    e.run("stroke.draw", &json!({"points": pts(&[[at.x + 5.0, at.y + 3.0], [at.x + 40.0, at.y + 60.0]])})).expect("draw");
    assert!(e.scene.strokes[2].points[0].p.distance(end) > 1e-3, "not snapped when off");
    // A guide extruded from the first curve.
    e.run("guide.close", &Value::Null).expect("close");
    e.run("select.set", &json!({"ids": [e.scene.strokes[0].id]})).expect("select");
    e.run("camera.view", &json!({"view": "front"})).expect("front");
    let g2 = e.run("guide.fromCurve", &json!({})).expect("from curve");
    assert!(g2.as_u64().is_some() && e.scene.active_guide.is_some());
    assert!(e.run("guide.fromCurve", &json!({"id": 999_999})).is_err());
    // Scaling a curve by half halves its brush size (and can be turned off).
    let id = e.scene.strokes[0].id;
    let before = e.scene.strokes[0].brush.size_mm;
    e.run("select.set", &json!({"ids": [id]})).expect("select");
    e.run("transform.apply", &json!({"mode": "scale", "amount": 0.5})).expect("scale");
    assert!((e.scene.stroke(id).expect("s").brush.size_mm - before * 0.5).abs() < 1e-3);
    // Stretching taller on screen keeps the thickness, and makes it taller on screen.
    let h0 = e.scene.stroke(id).expect("s").bounds().map(|(lo, hi)| hi.y - lo.y).expect("b");
    e.run("transform.stretch", &json!({"y": 2.0})).expect("stretch");
    assert!((e.scene.stroke(id).expect("s").brush.size_mm - before * 0.5).abs() < 1e-3);
    let h1 = e.scene.stroke(id).expect("s").bounds().map(|(lo, hi)| hi.y - lo.y).expect("b");
    assert!(h1 > h0 * 1.9 || h0 < 1e-4, "{h0} → {h1}");
    assert!(e.run("transform.stretch", &json!({"x": 0})).is_err());
    e.run("assist.set", &json!({"scaleThickness": false})).expect("off");
    e.run("transform.apply", &json!({"mode": "scale", "amount": 0.5})).expect("scale");
    assert!((e.scene.stroke(id).expect("s").brush.size_mm - before * 0.5).abs() < 1e-3);
    // Focus mode and grid opacity.
    assert_eq!(e.run("view.focus", &json!({})).expect("focus"), json!(true));
    let _ = e.render(0, true);
    // The active group's curves are a black silhouette, the others light grey.
    let mut f = front_ed();
    f.run("brush.set", &json!({"kind": "marker", "color": "#ff0000", "size": 60, "pressure": false})).expect("red");
    f.run("stroke.draw", &json!({"points": pts(&(0..30).map(|i| [60.0 + i as f32 * 3.0, 100.0]).collect::<Vec<_>>())})).expect("a");
    f.run("group.new", &json!({})).expect("group");
    f.run("stroke.draw", &json!({"points": pts(&(0..30).map(|i| [240.0 + i as f32 * 3.0, 100.0]).collect::<Vec<_>>())})).expect("b");
    f.run("view.focus", &json!({"on": true})).expect("focus");
    let (other, active) = (pixel(&mut f, 100, 100), pixel(&mut f, 280, 100));
    assert!(active[0] < 50 && active[1] < 50, "active group black: {active:?}");
    assert!(other[0] > 170 && (other[0] as i32 - other[2] as i32).abs() < 20, "others grey: {other:?}");
    e.run("env.set", &json!({"gridOpacity": 0.25})).expect("grid");
    assert_eq!(e.scene.environment.grid_opacity, 0.25);
    e.run("env.set", &json!({"gridOpacity": f64::NAN})).ok();
    assert!(e.scene.environment.grid_opacity.is_finite());
}

/// What the advanced tutorial's frames show: glow adds light in Render mode (overlaps get
/// brighter), the background colour tints the shading, selections are flat mint, and a
/// selection's colours shift together.
#[test]
fn advanced_tutorial_render_behaviours() {
    let mut e = front_ed();
    e.run("env.set", &json!({"background": "#0a1a60", "render": true})).expect("deep sea");
    e.run("brush.set", &json!({"kind": "marker", "color": "#3060ff", "size": 60, "opacity": 0.35, "material": "glow", "pressure": false})).expect("glow");
    e.run("stroke.draw", &json!({"points": pts(&(0..30).map(|i| [60.0 + i as f32 * 8.0, 150.0]).collect::<Vec<_>>())})).expect("one");
    let once = pixel(&mut e, 200, 150);
    e.run("stroke.draw", &json!({"points": pts(&(0..30).map(|i| [60.0 + i as f32 * 8.0, 150.0]).collect::<Vec<_>>())})).expect("two");
    let twice = pixel(&mut e, 200, 150);
    let sum = |p: [u8; 4]| p[0] as u32 + p[1] as u32 + p[2] as u32;
    assert!(sum(twice) > sum(once) + 40, "overlapping glow is brighter: {once:?} → {twice:?}");
    assert!(sum(once) > sum(pixel(&mut e, 200, 40)), "glow is brighter than the sea");
    // Environment light: a white shaded curve in shadow takes the background's blue.
    let mut f = front_ed();
    f.run("env.set", &json!({"background": "#0a1a60", "render": true, "lighting": {"azimuth": 180, "altitude": -60}})).expect("env");
    f.run("brush.set", &json!({"kind": "flat", "color": "#ffffff", "size": 60, "material": "shaded", "pressure": false})).expect("white");
    f.run("stroke.draw", &json!({"points": pts(&(0..30).map(|i| [60.0 + i as f32 * 8.0, 150.0]).collect::<Vec<_>>())})).expect("draw");
    let sh = pixel(&mut f, 200, 150);
    assert!(u32::from(sh[2]) > u32::from(sh[0]) + 10, "shadow tinted by the blue background: {sh:?}");
    // Selection: flat mint.
    let id = f.scene.strokes[0].id;
    f.run("select.set", &json!({"ids": [id]})).expect("select");
    let m = pixel(&mut f, 200, 150);
    assert!(m[1] > 200 && m[0] < 160, "mint: {m:?}");
    // Relative colour adjustment keeps differences between colours.
    f.run("brush.set", &json!({"color": "#804040", "applyToSelection": true})).expect("recolour");
    f.run("edit.adjustColor", &json!({"brightness": 0.2})).expect("brighten");
    let c = f.scene.stroke(id).expect("s").brush.color;
    assert!(c.0[0] > 0x80 + 30, "{c:?}");
    f.run("select.none", &Value::Null).expect("none");
    assert!(f.run("edit.adjustColor", &json!({"hue": 10})).is_err(), "needs a selection");
}
