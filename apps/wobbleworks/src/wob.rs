//! Opening `.wob` projects from the original Wobbleworks (and the earlier standalone app): each
//! layer becomes a wiggle layer whose strokes are drawn again through `wiggle.apply`, so they
//! boil as before; raster frames are copied onto the matching boil frames. The document is
//! built in a private PhotoCraft session, so the result is an ordinary PhotoCraft document.
//!
//! The marks are drawn by PhotoCraft's round brush, so the look is close to, not identical with,
//! the original stamps; stroke seeds are kept, so each stroke's boil is stable.

use base64::Engine as _;
use photocraft_doc::{Document, LayerContent};
use photocraft_engine::{Session, wiggle_cmds};
use serde_json::{Value, json};

/// Refuse project files bigger than this.
pub const MAX_FILE: usize = 256 * 1024 * 1024;
/// Most layers, strokes per layer and points per stroke read.
const MAX_LAYERS: usize = 64;
const MAX_STROKES: usize = 20_000;
const MAX_POINTS: usize = 20_000;
/// Largest canvas side accepted.
const MAX_SIDE: u64 = 16_384;
/// The original's wiggle at 100 %, in pixels.
const BASE_WIGGLE: f64 = 3.0;

/// Is `name` a `.wob` file?
pub fn is_wob(name: &str) -> bool {
    std::path::Path::new(name).extension().is_some_and(|e| e.eq_ignore_ascii_case("wob"))
}

fn blend(name: &str) -> &'static str {
    match name {
        "multiply" => "Multiply",
        "screen" => "Screen",
        "add" => "Linear Dodge (Add)",
        "darken" => "Darken",
        "lighten" => "Lighten",
        _ => "Normal",
    }
}

/// How much a brush wobbles, relative to the original's base wiggle.
fn brush_wiggle(brush: &str) -> f64 {
    match brush {
        "steady" => 0.0,
        "shaky" => 1.4,
        "rowdy" => 2.4,
        "sketch" => 1.2,
        _ => 1.0,
    }
}

/// Read a `.wob` file into a document. Problems that don't stop the import come back as warnings.
pub fn import(name: &str, bytes: &[u8]) -> Result<(Document, Vec<String>), String> {
    if bytes.len() > MAX_FILE {
        return Err(format!("{name} is too big ({} MB)", bytes.len() / 1_000_000));
    }
    let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("{name} is not a Wobbleworks project: {e}"))?;
    if v.get("format").and_then(Value::as_str).is_some_and(|f| f != "wobbleworks") {
        return Err(format!("{name} is not a Wobbleworks project"));
    }
    let side = |k: &str| v.get(k).and_then(Value::as_u64).filter(|n| (1..=MAX_SIDE).contains(n));
    let (w, h) = (side("w").ok_or("the project has no width")?, side("h").ok_or("the project has no height")?);
    let settings = v.get("settings").cloned().unwrap_or(Value::Null);
    let frames = settings.get("frames").and_then(Value::as_u64).map_or(wiggle_cmds::DEFAULT_FRAMES, |n| (n as usize).clamp(2, wiggle_cmds::MAX_FRAMES));
    let wiggle = settings.get("wiggle").and_then(Value::as_f64).filter(|x| x.is_finite()).unwrap_or(1.0).clamp(0.0, 10.0);
    let transparent = settings.get("transparent").and_then(Value::as_bool).unwrap_or(false);
    let background = if transparent {
        "transparent".to_string()
    } else {
        settings.get("bg").and_then(Value::as_str).filter(|s| s.starts_with('#')).unwrap_or("#ffffff").to_string()
    };
    let title = v.get("name").and_then(Value::as_str).map_or_else(|| name.trim_end_matches(".wob").to_string(), str::to_string);

    let mut s = Session::new();
    let e = |e: photocraft_engine::EngineError| e.to_string();
    s.execute("file.new", json!({"width": w, "height": h, "background": background, "name": title})).map_err(e)?;
    let mut warnings = Vec::new();
    let layers = v.get("layers").and_then(Value::as_array).cloned().unwrap_or_default();
    if layers.len() > MAX_LAYERS {
        warnings.push(format!("only the first {MAX_LAYERS} of {} layers were read", layers.len()));
    }
    for (li, layer) in layers.iter().take(MAX_LAYERS).enumerate() {
        let lname = layer.get("name").and_then(Value::as_str).unwrap_or("Layer").to_string();
        let r = s.execute("wiggle.new", json!({"frames": frames, "name": lname})).map_err(e)?;
        let group = r.get("layer").cloned().unwrap_or(Value::Null);
        let frame_ids: Vec<u64> = r.get("frames").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
        let opacity = layer.get("opacity").and_then(Value::as_f64).filter(|x| x.is_finite()).unwrap_or(1.0).clamp(0.0, 1.0);
        let visible = layer.get("visible").and_then(Value::as_bool).unwrap_or(true);
        let b = blend(layer.get("blend").and_then(Value::as_str).unwrap_or("normal"));
        if let Err(err) = s.execute("layer.setProps", json!({"layer": group, "opacity": opacity, "visible": visible, "blend": b})) {
            warnings.push(format!("layer {}: {err}", li + 1));
        }
        // Raster frames: PNG data URLs, or "=<index>" for a repeat of an earlier frame.
        let raster = layer.get("raster").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut decoded: Vec<Option<photocraft_raster::Surface>> = Vec::new();
        for (fi, f) in raster.iter().enumerate().take(frames) {
            let surface = match f.as_str() {
                Some(url) if url.starts_with('=') => url[1..].parse::<usize>().ok().and_then(|j| decoded.get(j).cloned().flatten()),
                Some(url) => match frame_surface(url) {
                    Ok(s) => Some(s),
                    Err(err) => {
                        warnings.push(format!("layer {} frame {}: {err}", li + 1, fi + 1));
                        None
                    }
                },
                None => None,
            };
            decoded.push(surface);
        }
        if decoded.iter().any(Option::is_some) {
            let ids = frame_ids.clone();
            s.edit("Import Frames", |doc, _| {
                for (id, surf) in ids.iter().zip(&decoded) {
                    if let (Some(surf), Some(l)) = (surf, doc.layer_mut(photocraft_doc::LayerId(*id)))
                        && surf.format() == doc_format(l)
                    {
                        l.content = LayerContent::Raster(surf.clone());
                    }
                }
                Ok(())
            })
            .map_err(e)?;
        }
        // Strokes, drawn on every frame.
        let strokes = layer.get("strokes").and_then(Value::as_array).cloned().unwrap_or_default();
        if strokes.len() > MAX_STROKES {
            warnings.push(format!("layer {}: only the first {MAX_STROKES} strokes were read", li + 1));
        }
        let mut failed = 0usize;
        for st in strokes.iter().take(MAX_STROKES) {
            if let Some((params, factor)) = stroke_params(st)
                && s.execute(
                    "wiggle.apply",
                    json!({"command": "paint.stroke", "params": params, "amount": (BASE_WIGGLE * wiggle * factor).min(wiggle_cmds::MAX_AMOUNT)}),
                )
                .is_err()
            {
                failed += 1;
            }
        }
        if failed > 0 {
            warnings.push(format!("layer {}: {failed} strokes couldn't be drawn", li + 1));
        }
    }
    let st = s.active().ok_or("the project came out empty")?;
    Ok(((*st.doc).clone(), warnings))
}

fn doc_format(l: &photocraft_doc::Layer) -> photocraft_doc::PixelFormat {
    l.surface().map(photocraft_raster::Surface::format).unwrap_or(photocraft_doc::PixelFormat::RGBA8)
}

/// A raster frame (a PNG data URL) as the surface of an imported image.
fn frame_surface(url: &str) -> Result<photocraft_raster::Surface, String> {
    let b64 = url.split_once("base64,").map(|(_, b)| b).ok_or("not a data URL")?;
    let png = base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|e| e.to_string())?;
    let (doc, _) = crate::io::import("frame.png", &png)?;
    let layer = doc.layers.iter().find(|l| matches!(l.content, LayerContent::Raster(_))).ok_or("the frame has no pixels")?;
    layer.surface().cloned().ok_or_else(|| "the frame has no pixels".into())
}

/// `paint.stroke` params for a stored stroke, and its brush's wiggle factor.
fn stroke_params(st: &Value) -> Option<(Value, f64)> {
    let flat: Vec<f64> = st.get("p")?.as_array()?.iter().filter_map(Value::as_f64).filter(|x| x.is_finite()).collect();
    let pressure: Vec<f64> = st.get("w").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
    let points: Vec<Value> = flat
        .chunks_exact(2)
        .take(MAX_POINTS)
        .enumerate()
        .map(|(i, c)| {
            let p = pressure.get(i).map_or(1.0, |w| (w / 100.0).clamp(0.05, 6.0));
            json!([c[0].clamp(-1e5, 1e5), c[1].clamp(-1e5, 1e5), p])
        })
        .collect();
    if points.is_empty() {
        return None;
    }
    let brush = st.get("t").and_then(Value::as_str).unwrap_or("marker");
    let size = st.get("z").and_then(Value::as_f64).filter(|x| x.is_finite()).unwrap_or(6.0).clamp(1.0, 500.0);
    let colour = st.get("c").and_then(Value::as_str).filter(|c| c.starts_with('#') && c.len() == 7).unwrap_or("#000000");
    let seed = st.get("s").and_then(Value::as_f64).filter(|x| x.is_finite()).map_or(0, |x| (x as i64) as u32);
    let mut params = json!({"points": points, "size": size, "color": colour, "hardness": 1.0, "seed": seed});
    if brush == "eraser"
        && let Some(o) = params.as_object_mut()
    {
        o.insert("erase".into(), json!(true));
    }
    Some((params, brush_wiggle(brush)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_url(w: u32, h: u32, rgba: [u8; 4]) -> String {
        let img = photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgba, rgba.repeat((w * h) as usize)).unwrap();
        let png = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png))
    }

    #[test]
    fn a_project_becomes_wiggle_layers_with_its_strokes_and_frames() {
        let file = json!({
            "format": "wobbleworks", "version": 1, "w": 64, "h": 48, "name": "Doodle",
            "settings": {"bg": "#ffeedd", "frames": 3, "wiggle": 1.0},
            "layers": [
                {"name": "Ink", "opacity": 0.5, "blend": "multiply",
                 "strokes": [{"t": "shaky", "sh": "round", "c": "#ff0000", "z": 4, "s": 12345, "k": 0, "p": [5, 24, 60, 24], "w": [100, 150]}],
                 "raster": []},
                {"name": "Paint", "strokes": [], "raster": [png_url(64, 48, [0, 0, 255, 255]), "=0", null]}
            ]
        });
        let (doc, warnings) = import("doodle.wob", file.to_string().as_bytes()).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!((doc.size.width, doc.size.height), (64, 48));
        assert_eq!(doc.name, "Doodle");
        let wigs = wiggle_cmds::all(&doc);
        assert_eq!(wigs.len(), 2);
        let ink = wigs.iter().find(|w| w.name == "Ink").unwrap();
        assert!((ink.opacity - 0.5).abs() < 1e-6);
        assert_eq!(ink.blend, photocraft_color::BlendMode::Multiply);
        let ink_on = |id| {
            let s = doc.layer(id).unwrap().surface().unwrap();
            s.read_region(photocraft_geom::Rect::from_xywh(0, 0, 64, 48)).chunks(4).map(|p| p[3]).sum::<f32>()
        };
        assert!(wiggle_cmds::frames(ink).into_iter().all(|f| ink_on(f) > 0.0), "the stroke is on every frame");
        let paint = wigs.iter().find(|w| w.name == "Paint").unwrap();
        let pf = wiggle_cmds::frames(paint);
        assert!(ink_on(pf[0]) > 1000.0 && ink_on(pf[1]) > 1000.0, "frame 2 repeats frame 1");
        assert_eq!(ink_on(pf[2]), 0.0, "an empty frame stays empty");
    }

    #[test]
    fn bad_files_are_errors_or_warnings_never_panics() {
        assert!(import("x.wob", b"not json").is_err());
        assert!(import("x.wob", br#"{"format":"other","w":10,"h":10}"#).is_err());
        assert!(import("x.wob", br#"{"w":0,"h":10}"#).is_err());
        assert!(import("x.wob", br#"{"w":1e12,"h":10}"#).is_err());
        let weird = json!({"w": 20, "h": 20, "settings": {"frames": 99, "wiggle": "x", "bg": 5},
            "layers": [{"name": 7, "blend": "nope", "opacity": "x",
                "strokes": [{"p": [1e300, 2, "a", 4]}, {"p": []}, {}, 5, {"p": [1, 1, 2, 2], "z": -9, "c": "#zz"}],
                "raster": ["data:image/png;base64,!!!", "=9", "=x", 42]}]});
        let (doc, warnings) = import("weird.wob", weird.to_string().as_bytes()).unwrap();
        assert_eq!(wiggle_cmds::frame_count(&doc), wiggle_cmds::MAX_FRAMES);
        assert!(!warnings.is_empty());
        assert!(is_wob("A.WOB") && !is_wob("a.psd"));
    }
}
