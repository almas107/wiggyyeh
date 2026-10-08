//! Wiggle layers: WigglyPaint-style boiling lines on PhotoCraft documents.
//!
//! A wiggle layer is an ordinary layer group whose children are the boil frames: pixel layers
//! named `Boil 1`, `Boil 2`, … (bottom to top). Every mark is made on each frame with its points
//! nudged by deterministic noise (the original Wobbleworks generator, so frames are stable and
//! the loop repeats forever); playing the frames in turn makes the lines boil. Being a plain
//! group, a wiggle layer keeps its blend mode, opacity, mask and effects, and survives a PSD round
//! trip as per-frame layers.
//!
//! - `wiggle.new` adds one (frame 1 showing, frame 1 active);
//! - `wiggle.apply` runs a painting command on every frame of the active wiggle layer as one
//!   history step (`wiggle.stroke` is the brush);
//! - `wiggle.showFrame` shows one frame of every wiggle layer: playback, so no history step and
//!   the document stays saved;
//! - `wiggle.info` lists them.

use std::sync::Arc;

use photocraft_doc::{Document, Layer, LayerContent, LayerId};
use serde_json::{Map, Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// Frame layer name prefix.
pub const FRAME_PREFIX: &str = "Boil ";
/// Frames a new wiggle layer gets.
pub const DEFAULT_FRAMES: usize = 3;
pub const MAX_FRAMES: usize = 12;
/// How far points wander (pixels) by default.
pub const DEFAULT_AMOUNT: f64 = 3.0;
pub const MAX_AMOUNT: f64 = 50.0;

/// Commands `wiggle.apply` runs on every frame.
pub const APPLY: &[&str] = &["paint.stroke", "paint.pencil", "paint.bucket", "paint.gradient", "paint.mixerBrush", "edit.fill"];

/// Deterministic noise in `[0, 1)`: bit-for-bit the original Wobbleworks generator.
pub fn rnd(seed: u32) -> f64 {
    let mut t = seed.wrapping_add(0x6D2B_79F5);
    t = (t ^ (t >> 15)).wrapping_mul(t | 1);
    t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
    f64::from(t ^ (t >> 14)) / 4_294_967_296.0
}

/// Noise for three integer coordinates (JavaScript `ToInt32` wrapping, as the original).
pub fn jr(a: i64, b: i64, c: i64) -> f64 {
    let a = (a as u32).wrapping_mul(73_856_093);
    let b = (b as u32).wrapping_mul(19_349_663);
    let c = (c as u32).wrapping_mul(83_492_791);
    rnd(a ^ b ^ c)
}

/// `Math.round`: halves round up, negatives too.
fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// Point `i` of a stroke with seed `seed`, nudged for `frame` by up to `amount` pixels (whole
/// pixels, as the original).
pub fn wobble(x: f64, y: f64, seed: i64, i: i64, frame: i64, amount: f64) -> (f64, f64) {
    if amount < 0.25 || !amount.is_finite() {
        return (x, y);
    }
    (x + js_round((jr(seed, i, frame) * 2.0 - 1.0) * amount), y + js_round((jr(seed + 4177, i, frame) * 2.0 - 1.0) * amount))
}

/// Is `l` a wiggle layer (a group of `Boil N` pixel layers)?
pub fn is_wiggle(l: &Layer) -> bool {
    match l.children() {
        Some(c) => c.len() >= 2 && c.iter().all(|f| matches!(f.content, LayerContent::Raster(_)) && frame_number(&f.name).is_some()),
        None => false,
    }
}

fn frame_number(name: &str) -> Option<usize> {
    name.strip_prefix(FRAME_PREFIX)?.trim().parse().ok()
}

/// A wiggle layer's frames in play order.
pub fn frames(l: &Layer) -> Vec<LayerId> {
    let mut f: Vec<(usize, LayerId)> = l.children().unwrap_or(&[]).iter().filter_map(|c| Some((frame_number(&c.name)?, c.id))).collect();
    f.sort_by_key(|(n, _)| *n);
    f.into_iter().map(|(_, id)| id).collect()
}

/// The wiggle layer holding layer `id` (or `id` itself when it is one).
pub fn wiggle_of(doc: &Document, id: LayerId) -> Option<&Layer> {
    if let Some(l) = doc.layer(id)
        && is_wiggle(l)
    {
        return Some(l);
    }
    let path = doc.path_of(id)?;
    let parent = doc.layer_at(path.get(..path.len().checked_sub(1)?)?)?;
    is_wiggle(parent).then_some(parent)
}

/// Every wiggle layer in the document (outermost first).
pub fn all(doc: &Document) -> Vec<&Layer> {
    doc.walk().into_iter().map(|(_, _, l)| l).filter(|l| is_wiggle(l)).collect()
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document".into())
}

fn has_wiggle(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document")?;
    let id = d.active_layer.ok_or("no active layer")?;
    wiggle_of(&d.doc, id).map(|_| ()).ok_or_else(|| "the active layer is not a wiggle layer (Layer › New › Wiggle Layer)".into())
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn new_wiggle(s: &mut Session, p: &Value) -> Result<Value> {
    let n = match p.get("frames") {
        None | Some(Value::Null) => DEFAULT_FRAMES,
        Some(v) => {
            v.as_u64().filter(|n| (2..=MAX_FRAMES as u64).contains(n)).ok_or_else(|| bad("wiggle.new", format!("`frames` must be 2..{MAX_FRAMES}")))? as usize
        }
    };
    let (group, ids) = s.edit("New Wiggle Layer", |doc, active| {
        let name = p.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| doc.next_layer_name("Wiggle"));
        let format = doc.pixel_format();
        let children: Vec<Layer> = (1..=n)
            .map(|i| {
                let mut f = Layer::raster(format!("{FRAME_PREFIX}{i}"), format);
                f.visible = i == 1;
                f
            })
            .collect();
        let ids: Vec<LayerId> = children.iter().map(|c| c.id).collect();
        let group = doc.insert_above(*active, Layer::group(name, children));
        *active = ids.first().copied();
        Ok((group, ids))
    })?;
    Ok(json!({"layer": group.0, "frames": ids.iter().map(|i| i.0).collect::<Vec<_>>()}))
}

/// The stroke seed of painting params: their `seed`, else a hash of their points.
fn stroke_seed(params: &Value) -> i64 {
    if let Some(seed) = params.get("seed").and_then(Value::as_u64) {
        return (seed & 0x7fff_ffff) as i64;
    }
    let text = params.get("points").map(Value::to_string).unwrap_or_default();
    let h = text.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
    i64::from(h & 0x7fff_ffff)
}

/// Points along a stroke are resampled this far apart (pixels) before wobbling, so long straight
/// runs boil along their length rather than only at their ends.
pub const SPACING: f64 = 8.0;
/// Most points a resampled stroke may have (hostile coordinates can't blow it up).
const MAX_POINTS: usize = 50_000;

fn xy(v: &Value) -> Option<(f64, f64)> {
    match v {
        Value::Array(a) => Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?)),
        Value::Object(o) => Some((o.get("x")?.as_f64()?, o.get("y")?.as_f64()?)),
        _ => None,
    }
}

fn with_xy(v: &Value, x: f64, y: f64) -> Value {
    let mut out = v.clone();
    match &mut out {
        Value::Array(a) => {
            if let (Some(nx), Some(ny)) = (serde_json::Number::from_f64(x), serde_json::Number::from_f64(y))
                && a.len() >= 2
            {
                a[0] = Value::Number(nx);
                a[1] = Value::Number(ny);
            }
        }
        Value::Object(o) => {
            o.insert("x".into(), json!(x));
            o.insert("y".into(), json!(y));
        }
        _ => {}
    }
    out
}

/// `points` with extra points every [`SPACING`] pixels along each segment (pressure and other
/// fields come from the segment's start).
fn resample(points: &[Value]) -> Vec<Value> {
    let mut out = Vec::with_capacity(points.len());
    for pair in points.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        out.push(a.clone());
        if let (Some((x0, y0)), Some((x1, y1))) = (xy(a), xy(b)) {
            let len = (x1 - x0).hypot(y1 - y0);
            if len.is_finite() && len > SPACING {
                let n = ((len / SPACING) as usize).min(MAX_POINTS.saturating_sub(out.len()));
                for k in 1..n {
                    let t = k as f64 / n as f64;
                    out.push(with_xy(a, x0 + (x1 - x0) * t, y0 + (y1 - y0) * t));
                }
            }
        }
        if out.len() >= MAX_POINTS {
            break;
        }
    }
    if let Some(last) = points.last() {
        out.push(last.clone());
    }
    out
}

/// `params` with every point nudged for `frame` (points as `[x, y, …]` arrays or `{x, y, …}`).
fn wobbled(params: &Value, frame: usize, amount: f64) -> Value {
    let seed = stroke_seed(params);
    let mut out = params.clone();
    if amount >= 0.25
        && let Some(points) = out.get_mut("points").and_then(Value::as_array_mut)
    {
        *points = resample(points);
    }
    if let Some(points) = out.get_mut("points").and_then(Value::as_array_mut) {
        for (i, pt) in points.iter_mut().enumerate() {
            let i = i as i64;
            match pt {
                Value::Array(a) => {
                    if let (Some(x), Some(y)) = (a.first().and_then(Value::as_f64), a.get(1).and_then(Value::as_f64)) {
                        let (nx, ny) = wobble(x, y, seed, i, frame as i64, amount);
                        if let (Some(vx), Some(vy)) = (serde_json::Number::from_f64(nx), serde_json::Number::from_f64(ny)) {
                            a[0] = Value::Number(vx);
                            a[1] = Value::Number(vy);
                        }
                    }
                }
                Value::Object(o) => {
                    if let (Some(x), Some(y)) = (o.get("x").and_then(Value::as_f64), o.get("y").and_then(Value::as_f64)) {
                        let (nx, ny) = wobble(x, y, seed, i, frame as i64, amount);
                        o.insert("x".into(), json!(nx));
                        o.insert("y".into(), json!(ny));
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Run `command` with `params` on every frame of the active wiggle layer, as one history step.
fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "wiggle.apply";
    let command = p.get("command").and_then(Value::as_str).ok_or_else(|| bad(cmd, "missing `command`"))?;
    if !APPLY.contains(&command) {
        return Err(bad(cmd, format!("`command` must be one of {}", APPLY.join(", "))));
    }
    let params = match p.get("params") {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(v @ Value::Object(_)) => v.clone(),
        Some(_) => return Err(bad(cmd, "`params` must be an object")),
    };
    let amount = match p.get("amount") {
        None | Some(Value::Null) => DEFAULT_AMOUNT,
        Some(v) => {
            v.as_f64().filter(|a| a.is_finite() && (0.0..=MAX_AMOUNT).contains(a)).ok_or_else(|| bad(cmd, format!("`amount` must be 0..{MAX_AMOUNT}")))?
        }
    };
    run_on_frames(s, command, &params, amount)
}

fn run_on_frames(s: &mut Session, command: &str, params: &Value, amount: f64) -> Result<Value> {
    let (frame_ids, active) = {
        let d = s.active().ok_or(EngineError::NoDocument)?;
        let id = d.active_layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
        let w = wiggle_of(&d.doc, id).ok_or_else(|| EngineError::Other("the active layer is not a wiggle layer".into()))?;
        (frames(w), d.active_layer)
    };
    let mut done = 0usize;
    for (i, &f) in frame_ids.iter().enumerate() {
        s.select_layer(f)?;
        let q = if params.get("points").is_some() { wobbled(params, i, amount) } else { params.clone() };
        if let Err(e) = s.execute(command, q) {
            // Take back the frames already painted, leaving nothing to redo.
            for _ in 0..done {
                s.undo();
            }
            if let Some(st) = s.active_mut() {
                st.history.clear_redo();
            }
            if let Some(a) = active {
                let _ = s.select_layer(a);
            }
            return Err(e);
        }
        done += 1;
    }
    // One history step: keep only the state from before the first frame.
    if let Some(st) = s.active_mut() {
        for _ in 1..done {
            st.history.purge_last();
        }
    }
    if let Some(a) = active {
        s.select_layer(a)?;
    }
    Ok(json!({"frames": done}))
}

/// Show frame `frame` (wrapping) of every wiggle layer, without a history step. The active layer
/// follows to the shown frame when it was a frame of that wiggle layer, so painting stays visible.
fn show_frame(s: &mut Session, p: &Value) -> Result<Value> {
    let frame = p.get("frame").and_then(Value::as_u64).ok_or_else(|| bad("wiggle.showFrame", "missing `frame` (a whole number)"))? as usize;
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let mut doc = (*st.doc).clone();
    let mut active = st.active_layer;
    let changed = show_in(&mut doc.layers, frame, &mut active, 0);
    if changed {
        let was_saved = st.saved_revision == st.revision;
        st.doc = Arc::new(doc);
        st.revision += 1;
        st.last_damage = None;
        if was_saved {
            st.saved_revision = st.revision;
        }
    }
    if active != st.active_layer
        && let Some(a) = active
    {
        s.select_layer(a)?;
    }
    Ok(json!({"frame": frame, "changed": changed}))
}

/// Groups nest at most this deep (documents can be hostile).
const MAX_DEPTH: usize = 64;

fn show_in(layers: &mut [Layer], frame: usize, active: &mut Option<LayerId>, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    let mut changed = false;
    for l in layers.iter_mut() {
        if is_wiggle(l) {
            let order = frames(l);
            let n = order.len().max(1);
            let shown = order.get(frame % n).copied();
            let mut moved_from_frame = false;
            if let Some(children) = l.children_mut() {
                for c in children.iter_mut() {
                    let on = Some(c.id) == shown;
                    if c.visible != on {
                        c.visible = on;
                        changed = true;
                    }
                    if Some(c.id) == *active && !on {
                        moved_from_frame = true;
                    }
                }
            }
            if moved_from_frame {
                *active = shown;
            }
        } else if let Some(children) = l.children_mut() {
            changed |= show_in(children, frame, active, depth + 1);
        }
    }
    changed
}

fn info(s: &Session) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let list: Vec<Value> = all(&d.doc)
        .into_iter()
        .map(|w| {
            let f = frames(w);
            let shown = f.iter().position(|id| d.doc.layer(*id).is_some_and(|l| l.visible));
            json!({"layer": w.id.0, "name": w.name, "frames": f.iter().map(|i| i.0).collect::<Vec<_>>(), "shown": shown})
        })
        .collect();
    let active = d.active_layer.and_then(|id| wiggle_of(&d.doc, id)).map(|w| w.id.0);
    Ok(json!({"wiggles": list, "active": active}))
}

/// The most frames any wiggle layer in `doc` has (0 without wiggle layers).
pub fn frame_count(doc: &Document) -> usize {
    all(doc).iter().map(|w| frames(w).len()).max().unwrap_or(0)
}

/// `doc` showing frame `frame` of every wiggle layer (for rendering animation frames).
pub fn at_frame(doc: &Document, frame: usize) -> Document {
    let mut d = doc.clone();
    let mut none = None;
    show_in(&mut d.layers, frame, &mut none, 0);
    d
}

macro_rules! spec {
    ($id:expr, $label:expr, $params:expr, $en:expr, $journal:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[], shortcut: None, params: $params, enabled: $en, journal: $journal, run: $run }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!("wiggle.new", "New Wiggle Layer", r#"{"frames":2..12=3,"name":str?} → {layer, frames:[id]}"#, has_doc, true, new_wiggle),
        spec!(
            "wiggle.apply",
            "Paint on Every Boil Frame",
            r#"{"command":"paint.stroke|paint.pencil|paint.bucket|paint.gradient|paint.mixerBrush|edit.fill","params":{…that command's params}?,"amount":0..50=3 (pixels points wander)} → {frames}"#,
            has_wiggle,
            true,
            apply
        ),
        spec!("wiggle.stroke", "Wiggle Stroke", r#"{…paint.stroke params, "amount":0..50=3} → {frames}"#, has_wiggle, true, |s, p| {
            let mut params = p.as_object().cloned().unwrap_or_default();
            let amount = match params.remove("amount") {
                None | Some(Value::Null) => DEFAULT_AMOUNT,
                Some(v) => v
                    .as_f64()
                    .filter(|a| a.is_finite() && (0.0..=MAX_AMOUNT).contains(a))
                    .ok_or_else(|| bad("wiggle.stroke", format!("`amount` must be 0..{MAX_AMOUNT}")))?,
            };
            run_on_frames(s, "paint.stroke", &Value::Object(params), amount)
        }),
        spec!("wiggle.showFrame", "Show Boil Frame", r#"{"frame":n} (wraps per wiggle layer; no history step) → {frame, changed}"#, has_doc, false, show_frame),
        spec!("wiggle.info", "Wiggle Layers", "{} → {wiggles:[{layer,name,frames,shown}], active}", has_doc, false, |s, _| info(s)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands;

    fn doc() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        s
    }

    fn pixel(s: &mut Session, layer: u64, x: i64, y: i64) -> f64 {
        let d = s.active().unwrap();
        let l = d.doc.layer(LayerId(layer)).unwrap();
        let surf = l.surface().unwrap();
        f64::from(surf.read_region(photocraft_geom::Rect::from_xywh(x as i32, y as i32, 1, 1)).last().copied().unwrap_or(0.0))
    }

    #[test]
    fn noise_matches_the_original_wobbleworks() {
        assert_eq!(jr(0, 0, 0), 0.26642920868471265);
        assert_eq!(jr(123_456_789, 7, 2), 0.7799772853031754);
        assert_eq!(jr(999_999_999 + 4177, 300, 1), 0.5306690616998821);
        assert_eq!(rnd(-5i32 as u32), 0.48384718922898173);
        assert_eq!(js_round(-1.5), -1.0);
        assert_eq!(wobble(10.0, 10.0, 1, 2, 3, 0.0), (10.0, 10.0));
        let (x, y) = wobble(10.0, 10.0, 1, 2, 3, 4.0);
        assert!((x - 10.0).abs() <= 4.0 && (y - 10.0).abs() <= 4.0 && x.fract() == 0.0);
    }

    #[test]
    fn a_wiggle_layer_is_a_group_of_boil_frames() {
        let mut s = doc();
        let r = s.execute("wiggle.new", json!({})).unwrap();
        let frames: Vec<u64> = r["frames"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
        assert_eq!(frames.len(), 3);
        let d = s.active().unwrap();
        assert_eq!(d.active_layer, Some(LayerId(frames[0])));
        let w = wiggle_of(&d.doc, LayerId(frames[1])).unwrap();
        assert_eq!(w.id.0, r["layer"].as_u64().unwrap());
        assert!(d.doc.layer(LayerId(frames[0])).unwrap().visible && !d.doc.layer(LayerId(frames[1])).unwrap().visible);
        let info = s.execute("wiggle.info", json!({})).unwrap();
        assert_eq!(info["wiggles"][0]["shown"], 0);
        assert_eq!(info["active"], r["layer"]);
        assert!(s.execute("wiggle.new", json!({"frames": 1})).is_err());
        assert!(s.execute("wiggle.new", json!({"frames": 13})).is_err());
        assert!(s.execute("wiggle.new", json!({"frames": 5})).is_ok());
    }

    #[test]
    fn strokes_land_on_every_frame_differently_as_one_step() {
        let mut s = doc();
        let r = s.execute("wiggle.new", json!({})).unwrap();
        let frames: Vec<u64> = r["frames"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
        let undo_before = s.active().unwrap().history.past_len();
        s.execute("wiggle.stroke", json!({"points": [[5, 30], [20, 32], [40, 28], [60, 31]], "size": 3, "amount": 4})).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), undo_before + 1, "one history step");
        assert_eq!(s.active().unwrap().active_layer, Some(LayerId(frames[0])), "the active layer is kept");
        let coverage = |s: &mut Session, f: u64| -> Vec<f64> { (0..64).map(|y| pixel(s, f, 30, y)).collect() };
        let a = coverage(&mut s, frames[0]);
        let b = coverage(&mut s, frames[1]);
        let c = coverage(&mut s, frames[2]);
        assert!(a.iter().sum::<f64>() > 0.0 && b.iter().sum::<f64>() > 0.0 && c.iter().sum::<f64>() > 0.0, "every frame painted");
        assert!(a != b || b != c, "frames differ");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(coverage(&mut s, frames[1]).iter().sum::<f64>(), 0.0, "one undo takes the whole stroke back");
        // Fills spread too.
        s.execute("wiggle.apply", json!({"command": "edit.fill", "params": {"color": "#ff0000"}})).unwrap();
        assert!(pixel(&mut s, frames[2], 1, 1) > 0.0);
    }

    #[test]
    fn playback_shows_one_frame_without_history_and_keeps_the_document_saved() {
        let mut s = doc();
        let r = s.execute("wiggle.new", json!({})).unwrap();
        let frames: Vec<u64> = r["frames"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
        let st = s.active_mut().unwrap();
        st.saved_revision = st.revision;
        let undo = s.active().unwrap().history.past_len();
        let out = s.execute("wiggle.showFrame", json!({"frame": 4})).unwrap();
        assert_eq!(out["changed"], true);
        let d = s.active().unwrap();
        assert!(d.doc.layer(LayerId(frames[1])).unwrap().visible, "4 wraps to frame 2");
        assert!(!d.doc.layer(LayerId(frames[0])).unwrap().visible);
        assert_eq!(d.active_layer, Some(LayerId(frames[1])), "the active frame follows");
        assert_eq!(d.history.past_len(), undo);
        assert!(!d.is_dirty());
        let doc2 = at_frame(&d.doc, 2);
        assert!(doc2.layer(LayerId(frames[2])).unwrap().visible);
        assert_eq!(frame_count(&d.doc), 3);
    }

    #[test]
    fn bad_params_and_states_are_errors() {
        let mut s = Session::new();
        assert!(s.execute("wiggle.showFrame", json!({"frame": 1})).is_err());
        let mut s = doc();
        assert!(s.execute("wiggle.stroke", json!({"points": [[1, 1]]})).is_err(), "no wiggle layer");
        s.execute("wiggle.new", json!({})).unwrap();
        for p in [
            json!({"command": "wiggle.apply"}),
            json!({"command": "file.close"}),
            json!({}),
            json!({"command": "paint.stroke", "params": 5}),
            json!({"command": "paint.stroke", "amount": -1}),
            json!({"command": "paint.stroke", "amount": "x"}),
        ] {
            assert!(s.execute("wiggle.apply", p.clone()).is_err(), "{p}");
        }
        assert!(s.execute("wiggle.stroke", json!({"points": []})).is_err());
        assert!(s.execute("wiggle.stroke", json!({"points": [[1, 1]], "amount": 1e9})).is_err());
        assert!(s.execute("wiggle.showFrame", json!({"frame": -2})).is_err());
        assert!(s.execute("wiggle.showFrame", json!({})).is_err());
        // A failed frame leaves no half-painted frames and nothing to redo.
        let before = s.active().unwrap().history.past_len();
        assert!(s.execute("wiggle.apply", json!({"command": "paint.stroke", "params": {"points": [[1e12, 1], [2, 2]]}})).is_err());
        assert_eq!(s.active().unwrap().history.past_len(), before);
        for id in APPLY {
            assert!(commands::find(id).is_some(), "{id} is a command");
        }
        // Hostile coordinates resample to a bounded number of points.
        assert!(resample(&[json!([0, 0]), json!([1e12, 0])]).len() <= MAX_POINTS + 1);
    }

    #[test]
    fn long_runs_are_resampled_so_they_boil_along_their_length() {
        let pts = resample(&[json!([0, 0, 0.5]), json!([80, 0, 1.0]), json!({"x": 80, "y": 4})]);
        assert_eq!(pts.len(), 12, "9 added along the 80 px run");
        assert_eq!(pts[1], json!([8.0, 0.0, 0.5]), "pressure carried along");
        let w = wobbled(&json!({"points": [[0, 0], [80, 0]]}), 1, 3.0);
        let ys: Vec<f64> = w["points"].as_array().unwrap().iter().map(|p| p[1].as_f64().unwrap()).collect();
        assert!(ys.iter().any(|y| *y != 0.0), "the middle wanders");
        assert_eq!(wobbled(&json!({"points": [[0, 0], [80, 0]]}), 1, 0.0)["points"].as_array().unwrap().len(), 2, "no wiggle, no resampling");
    }
}
