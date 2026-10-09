//! The 3D mode over the control channel (and so over MCP's `control_call`): `w3d.*` methods.
//! Everything the UI does is an editor command, so `w3d.execute` reaches all of it; the rest
//! report state and render pictures (as base64, so nothing touches the file system).
//!
//! | Method | Params | Result |
//! |---|---|---|
//! | `w3d.commands` | | `[{id, params}]`: every editor command |
//! | `w3d.execute` | `{command, params?}` | the command's result |
//! | `w3d.state` | | revision, tool, counts, selection, camera, environment flags |
//! | `w3d.render` | `{kind?: png / boilGif / turntableGif / shotsGif / boilMp4 / turntableMp4 / shotsMp4, scale?, frame?, transparent?, width?, height?}` | `{mime, base64, bytes}` |
//!
//! `w3d.show` (switch the 3D mode on or off) lives in the shell, which owns the mode.

use base64::Engine as _;
use serde_json::{Value, json};
use wobbleworks_3d::camera::Viewport;
use wobbleworks_3d::editor::Editor;

use super::Space3d;

/// Biggest picture side `w3d.render` takes for `width` / `height`.
const RENDER_SIDE_MAX: f64 = 4096.0;

/// Whether a control method is one of the 3D mode's.
pub fn handles(method: &str) -> bool {
    method.starts_with("w3d.")
}

/// Run a `w3d.*` method (not `w3d.show`: see the shell).
pub fn call(s: &mut Space3d, method: &str, p: &Value) -> Result<Value, String> {
    match method {
        "w3d.commands" => Ok(Value::Array(Editor::commands().into_iter().map(|(id, params)| json!({"id": id, "params": params})).collect())),
        "w3d.execute" => {
            let id = p.get("command").or_else(|| p.get("id")).and_then(Value::as_str).ok_or("missing `command`")?;
            let params = p.get("params").cloned().unwrap_or(Value::Null);
            s.ed.run(id, &params)
        }
        "w3d.state" => Ok(state(&s.ed)),
        "w3d.render" => render(&mut s.ed, p),
        _ => Err(format!("unknown 3D method `{method}` (see `w3d.commands` for editor commands)")),
    }
}

fn state(ed: &Editor) -> Value {
    let sc = &ed.scene;
    let env = &sc.environment;
    json!({
        "revision": ed.revision,
        "tool": ed.tool.name(),
        "strokes": sc.strokes.len(),
        "guides": sc.guides.len(),
        "groups": sc.groups.len(),
        "images": sc.images.len(),
        "models": sc.models.len(),
        "selection": ed.selection.iter().copied().collect::<Vec<_>>(),
        "camera": serde_json::to_value(ed.camera).unwrap_or(Value::Null),
        "renderMode": env.render_mode,
        "boil": sc.boil.enabled,
        "backgroundImage": env.background_image.is_some(),
        "status": ed.status,
        "dirty": ed.dirty,
    })
}

fn render(ed: &mut Editor, p: &Value) -> Result<Value, String> {
    let kind = p.get("kind").and_then(Value::as_str).unwrap_or("png");
    let scale = p.get("scale").and_then(Value::as_u64).unwrap_or(1).clamp(1, 4) as u32;
    let frame = p.get("frame").and_then(Value::as_u64).unwrap_or(0).min(u64::from(u32::MAX)) as u32;
    let transparent = p.get("transparent").and_then(Value::as_bool).unwrap_or(false);
    let side = |k: &str| -> Result<Option<f32>, String> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => match v.as_f64() {
                Some(n) if (1.0..=RENDER_SIDE_MAX).contains(&n) => Ok(Some(n as f32)),
                _ => Err(format!("`{k}` must be 1–{RENDER_SIDE_MAX} pixels")),
            },
        }
    };
    let (w, h) = (side("width")?, side("height")?);
    // A size for this picture only: the view's camera keeps its own.
    let saved = ed.camera.viewport;
    if w.is_some() || h.is_some() {
        ed.camera.viewport = Viewport { width: w.unwrap_or(saved.width), height: h.unwrap_or(saved.height) };
    }
    use super::export::{Anim, Video, animation};
    let anim = match kind {
        "png" => None,
        "boilGif" => Some((Anim::Boil, Video::Gif)),
        "turntableGif" => Some((Anim::Turntable, Video::Gif)),
        "shotsGif" => Some((Anim::Shots, Video::Gif)),
        "boilMp4" => Some((Anim::Boil, Video::Mp4)),
        "turntableMp4" => Some((Anim::Turntable, Video::Mp4)),
        "shotsMp4" => Some((Anim::Shots, Video::Mp4)),
        _ => {
            ed.camera.viewport = saved;
            return Err(format!("unknown render kind `{kind}` (png, boilGif, turntableGif, shotsGif, boilMp4, turntableMp4, shotsMp4)"));
        }
    };
    let out = match anim {
        None => super::export::png(ed, frame, scale, transparent).map(|b| ("image/png", b)),
        Some((a, v)) => animation(ed, a, v, scale).map(|b| (if v == Video::Gif { "image/gif" } else { "video/mp4" }, b)),
    };
    ed.camera.viewport = saved;
    let (mime, bytes) = out?;
    Ok(json!({"mime": mime, "bytes": bytes.len(), "base64": base64::engine::general_purpose::STANDARD.encode(&bytes)}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn space() -> Space3d {
        let mut s = Space3d::new();
        s.ed.set_viewport(320.0, 200.0);
        s
    }

    #[test]
    fn agents_draw_inspect_and_render_through_w3d_methods() {
        let mut s = space();
        assert!(handles("w3d.execute") && !handles("engine.execute"));
        let cmds = call(&mut s, "w3d.commands", &Value::Null).expect("commands");
        assert!(cmds.as_array().is_some_and(|a| a.iter().any(|c| c["id"] == "stroke.add")));
        call(&mut s, "w3d.execute", &json!({"command": "stroke.add", "params": {"points": [[-1, 0, 0], [1, 0.5, 0]]}})).expect("add");
        let st = call(&mut s, "w3d.state", &Value::Null).expect("state");
        assert_eq!(st["strokes"], 1);
        assert_eq!(st["tool"], "draw");
        let r = call(&mut s, "w3d.render", &json!({"width": 64, "height": 48})).expect("render");
        assert_eq!(r["mime"], "image/png");
        let png = base64::engine::general_purpose::STANDARD.decode(r["base64"].as_str().expect("b64")).expect("decode");
        let img = image::load_from_memory(&png).expect("png");
        assert_eq!((img.width(), img.height()), (64, 48));
        assert_eq!(s.ed.camera.viewport.width, 320.0, "the view keeps its size");
        // Save and open again through editor commands.
        let hex = call(&mut s, "w3d.execute", &json!({"command": "file.serialize"})).expect("save");
        call(&mut s, "w3d.execute", &json!({"command": "file.new"})).expect("new");
        assert_eq!(s.ed.scene.strokes.len(), 0);
        call(&mut s, "w3d.execute", &json!({"command": "file.deserialize", "params": hex})).expect("open");
        assert_eq!(s.ed.scene.strokes.len(), 1);
    }

    #[test]
    fn bad_requests_are_errors() {
        let mut s = space();
        for (m, p) in [
            ("w3d.execute", json!({})),
            ("w3d.execute", json!({"command": "no.such"})),
            ("w3d.execute", json!({"command": 5})),
            ("w3d.render", json!({"kind": "mp5"})),
            ("w3d.render", json!({"width": 0})),
            ("w3d.render", json!({"width": 1e9})),
            ("w3d.render", json!({"height": "big"})),
            ("w3d.nope", json!({})),
        ] {
            assert!(call(&mut s, m, &p).is_err(), "{m} {p}");
        }
        assert_eq!(s.ed.camera.viewport.width, 320.0);
    }
}
