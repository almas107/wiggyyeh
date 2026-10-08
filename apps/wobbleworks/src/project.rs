//! The `.wob` project format and image import/export.
//!
//! `.wob` is JSON. Version 2 is a superset of the original Wobbleworks version 1, so files saved
//! by the old web page open here unchanged (same stroke encoding, same noise, same look). Raster
//! frames are PNG data URLs; a frame identical to an earlier one is stored as `"=<index>"`.

use std::io::{Cursor, Read, Write};
use std::sync::Arc;

use egui::Color32;
use serde::{Deserialize, Serialize};

use crate::geom::clamp_coord;
use crate::model::{Brush, Doc, Layer, MAX_LAYERS, MAX_POINTS, MAX_SIZE, Playback, Pt, Stroke, Tip};
use crate::pixels::{Blend, MAX_SIDE, Pixmap, parse_hex, to_hex};

pub const FORMAT: &str = "wobbleworks";
pub const VERSION: u32 = 2;
/// Refuse project files bigger than this.
pub const MAX_FILE: usize = 256 * 1024 * 1024;
/// Largest picture we decode on import (either side).
pub const MAX_IMPORT_SIDE: u32 = 16_384;

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FileSettings {
    pub bg: Option<String>,
    pub transparent: bool,
    pub speed: Option<f64>,
    pub wiggle: Option<f64>,
    pub zoom: Option<f64>,
    pub frames: Option<usize>,
    pub playback: Option<Playback>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FileStroke {
    pub t: String,
    pub sh: String,
    pub c: String,
    pub z: f64,
    pub s: f64,
    pub k: u8,
    /// Flat x, y pairs.
    pub p: Vec<f64>,
    /// Optional pressure per point, in percent.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub w: Vec<u16>,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct FileLayer {
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    pub clip: bool,
    #[serde(rename = "alphaLock")]
    pub alpha_lock: bool,
    pub blend: Blend,
    pub raster: Vec<Option<String>>,
    pub strokes: Vec<FileStroke>,
}

impl Default for FileLayer {
    fn default() -> Self {
        FileLayer {
            name: "Layer".into(),
            visible: true,
            opacity: 1.0,
            clip: false,
            alpha_lock: false,
            blend: Blend::Normal,
            raster: Vec::new(),
            strokes: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WobFile {
    pub format: String,
    pub version: u32,
    pub id: Option<String>,
    pub name: Option<String>,
    pub w: usize,
    pub h: usize,
    pub modified: f64,
    pub settings: FileSettings,
    pub current: usize,
    pub layers: Vec<FileLayer>,
}

/// A loaded project: the document plus what's stored around it.
pub struct Loaded {
    pub doc: Doc,
    pub name: String,
    pub zoom: Option<f32>,
    /// Problems that didn't stop the load (e.g. an unreadable frame).
    pub warnings: Vec<String>,
}

fn stroke_to_file(s: &Stroke) -> FileStroke {
    let mut p = Vec::with_capacity(s.pts.len() * 2);
    let mut w = Vec::new();
    let pressured = s.pts.iter().any(|q| (q.p - 1.0).abs() > 0.005);
    for q in &s.pts {
        p.push((q.x * 10.0).round() / 10.0);
        p.push((q.y * 10.0).round() / 10.0);
        if pressured {
            w.push((q.p.clamp(0.0, 6.0) * 100.0).round() as u16);
        }
    }
    FileStroke { t: s.brush.id().into(), sh: s.tip.id().into(), c: to_hex(s.color), z: s.size, s: f64::from(s.seed), k: u8::from(s.lock), p, w }
}

fn stroke_from_file(f: &FileStroke) -> Option<Stroke> {
    let mut pts: Vec<Pt> =
        f.p.chunks_exact(2)
            .enumerate()
            .take(MAX_POINTS)
            .map(|(i, c)| {
                let p = f.w.get(i).map_or(1.0, |v| f32::from(*v) / 100.0);
                Pt {
                    x: clamp_coord(c.first().copied().unwrap_or(0.0)),
                    y: clamp_coord(c.get(1).copied().unwrap_or(0.0)),
                    p: if p.is_finite() { p.clamp(0.05, 6.0) } else { 1.0 },
                }
            })
            .collect();
    if pts.is_empty() {
        return None;
    }
    pts.shrink_to_fit();
    let size = if f.z.is_finite() { f.z.clamp(1.0, MAX_SIZE) } else { 6.0 };
    // Seeds were 32-bit integers in the original; keep the low 32 bits of anything else.
    let seed = if f.s.is_finite() { (f.s as i64) as u32 } else { 0 };
    Some(Stroke { brush: Brush::from_id(&f.t), tip: Tip::from_id(&f.sh), color: parse_hex(&f.c).unwrap_or(Color32::BLACK), size, seed, lock: f.k != 0, pts })
}

/// Serialize a document. Raster frames are PNG-encoded with `png_cache` reused per layer
/// version (autosave calls this often; unchanged rasters aren't re-encoded).
pub fn to_file(doc: &Doc, name: &str, id: Option<&str>, modified: f64, zoom: f32, png_cache: &mut PngCache) -> WobFile {
    let layers = doc
        .layers
        .iter()
        .map(|l| {
            let urls = png_cache.urls(l);
            let raster = urls
                .iter()
                .enumerate()
                .map(|(i, u)| {
                    let u = u.as_ref()?;
                    match urls.iter().position(|v| v.as_ref() == Some(u)) {
                        Some(j) if j < i => Some(format!("={j}")),
                        _ => Some(u.clone()),
                    }
                })
                .collect();
            FileLayer {
                name: l.name.clone(),
                visible: l.visible,
                opacity: l.opacity,
                clip: l.clip,
                alpha_lock: l.alpha_lock,
                blend: l.blend,
                raster,
                strokes: l.strokes.iter().map(stroke_to_file).collect(),
            }
        })
        .collect();
    WobFile {
        format: FORMAT.into(),
        version: VERSION,
        id: id.map(Into::into),
        name: Some(name.into()),
        w: doc.w,
        h: doc.h,
        modified,
        settings: FileSettings {
            bg: Some(to_hex(doc.bg)),
            transparent: doc.transparent,
            speed: Some(f64::from(doc.speed_ms)),
            wiggle: Some(doc.wiggle),
            zoom: Some(f64::from(zoom)),
            frames: Some(doc.frames),
            playback: Some(doc.playback),
        },
        current: doc.current,
        layers,
    }
}

/// Encoded PNG data URLs of raster frames, remembered per (layer id, version).
#[derive(Default)]
pub struct PngCache {
    map: std::collections::HashMap<u64, (u64, Vec<Option<String>>)>,
}

impl PngCache {
    fn urls(&mut self, l: &Layer) -> Vec<Option<String>> {
        if let Some((ver, v)) = self.map.get(&l.id)
            && *ver == l.ver
        {
            return v.clone();
        }
        let v: Vec<Option<String>> = l
            .raster
            .iter()
            .map(|r| r.as_ref().filter(|p| !p.is_blank()).and_then(|p| png_bytes(p).ok()).map(|b| format!("data:image/png;base64,{}", base64_encode(&b))))
            .collect();
        self.map.insert(l.id, (l.ver, v.clone()));
        v
    }

    pub fn retain(&mut self, doc: &Doc) {
        self.map.retain(|id, _| doc.layers.iter().any(|l| l.id == *id));
    }
}

pub fn to_json(f: &WobFile) -> Result<String, String> {
    serde_json::to_string(f).map_err(|e| format!("couldn't write the project: {e}"))
}

/// Parse a `.wob` (version 1 or 2).
pub fn from_json(text: &str) -> Result<Loaded, String> {
    if text.len() > MAX_FILE {
        return Err("that project file is too big".into());
    }
    let f: WobFile = serde_json::from_str(text).map_err(|e| format!("not a readable WobbleWorks project ({e})"))?;
    if f.format != FORMAT {
        return Err("not a WobbleWorks project".into());
    }
    if f.version > VERSION + 50 {
        return Err(format!("this project was made by a much newer WobbleWorks (format {})", f.version));
    }
    if f.w == 0 || f.h == 0 || f.w > MAX_SIDE || f.h > MAX_SIDE {
        return Err(format!("unsupported canvas size {}×{}", f.w, f.h));
    }
    let mut warnings = Vec::new();
    let frames = f.settings.frames.unwrap_or(3).clamp(crate::model::MIN_FRAMES, crate::model::MAX_FRAMES);
    let mut doc = Doc::new(f.w, f.h);
    doc.frames = frames;
    doc.layers.clear();
    if f.layers.len() > MAX_LAYERS {
        warnings.push(format!("only the first {MAX_LAYERS} layers were opened"));
    }
    for fl in f.layers.iter().take(MAX_LAYERS) {
        let mut l = Layer::new(fl.name.chars().take(80).collect::<String>(), frames);
        l.visible = fl.visible;
        l.opacity = if fl.opacity.is_finite() { fl.opacity.clamp(0.0, 1.0) } else { 1.0 };
        l.clip = fl.clip;
        l.alpha_lock = fl.alpha_lock;
        l.blend = fl.blend;
        l.strokes = Arc::new(fl.strokes.iter().filter_map(stroke_from_file).collect());
        for fr in 0..frames {
            let Some(Some(src)) = fl.raster.get(fr) else { continue };
            // "=j" points at an earlier frame (one level only).
            let src = match src.strip_prefix('=').and_then(|j| j.parse::<usize>().ok()) {
                Some(j) => match fl.raster.get(j) {
                    Some(Some(s)) if !s.starts_with('=') => s,
                    _ => continue,
                },
                None => src,
            };
            match decode_data_url(src, doc.w, doc.h) {
                Ok(p) => {
                    if let Some(slot) = l.raster.get_mut(fr) {
                        *slot = Some(Arc::new(p));
                    }
                }
                Err(e) => warnings.push(format!("layer \"{}\" frame {}: {e}", l.name, fr + 1)),
            }
        }
        doc.layers.push(l);
    }
    doc.current = f.current;
    if let Some(bg) = f.settings.bg.as_deref().and_then(parse_hex) {
        doc.bg = bg;
    }
    doc.transparent = f.settings.transparent;
    if let Some(s) = f.settings.speed.filter(|s| s.is_finite()) {
        doc.speed_ms = s.clamp(20.0, 2000.0) as u32;
    }
    if let Some(w) = f.settings.wiggle {
        doc.wiggle = w;
    }
    doc.playback = f.settings.playback.unwrap_or_default();
    doc.normalize();
    let zoom = f.settings.zoom.filter(|z| z.is_finite() && *z > 0.0).map(|z| z.clamp(0.05, 32.0) as f32);
    Ok(Loaded { doc, name: f.name.unwrap_or_else(|| "Untitled".into()).chars().take(80).collect(), zoom, warnings })
}

fn decode_data_url(s: &str, w: usize, h: usize) -> Result<Pixmap, String> {
    let b64 = s.split_once(',').map_or(s, |(_, b)| b);
    let bytes = base64_decode(b64).ok_or("bad image data")?;
    let p = decode_image(&bytes)?;
    Ok(if p.w == w && p.h == h { p } else { p.resized(w, h) })
}

/// Decode any supported picture (PNG, JPEG, GIF, WebP, BMP) with size limits.
pub fn decode_image(bytes: &[u8]) -> Result<Pixmap, String> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| format!("can't read that picture: {e}"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMPORT_SIDE);
    limits.max_image_height = Some(MAX_IMPORT_SIDE);
    limits.max_alloc = Some(1024 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| format!("can't read that picture: {e}"))?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    if w > MAX_SIDE || h > MAX_SIDE {
        // Shrink very large photos before they become a floating selection.
        let fit = (MAX_SIDE as f64 / w as f64).min(MAX_SIDE as f64 / h as f64);
        let (nw, nh) = (((w as f64 * fit) as u32).max(1), ((h as f64 * fit) as u32).max(1));
        let small = image::imageops::resize(&rgba, nw, nh, image::imageops::FilterType::Triangle);
        return Pixmap::from_rgba(nw as usize, nh as usize, small.as_raw()).ok_or_else(|| "picture size mismatch".to_string());
    }
    Pixmap::from_rgba(w, h, rgba.as_raw()).ok_or_else(|| "picture size mismatch".to_string())
}

pub fn png_bytes(p: &Pixmap) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let enc = image::codecs::png::PngEncoder::new(&mut out);
    image::ImageEncoder::write_image(enc, &p.to_rgba(), p.w as u32, p.h as u32, image::ExtendedColorType::Rgba8)
        .map_err(|e| format!("PNG export failed: {e}"))?;
    Ok(out)
}

/// An animated GIF that loops forever.
pub fn gif_bytes(frames: &[Pixmap], delay_ms: u32) -> Result<Vec<u8>, String> {
    use image::codecs::gif::{GifEncoder, Repeat};
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new_with_speed(&mut out, 10);
        enc.set_repeat(Repeat::Infinite).map_err(|e| format!("GIF export failed: {e}"))?;
        for p in frames {
            let buf = image::RgbaImage::from_raw(p.w as u32, p.h as u32, p.to_rgba()).ok_or("GIF export failed: frame size")?;
            let frame = image::Frame::from_parts(buf, 0, 0, image::Delay::from_numer_denom_ms(delay_ms.max(20), 1));
            enc.encode_frame(frame).map_err(|e| format!("GIF export failed: {e}"))?;
        }
    }
    Ok(out)
}

/// All frames side by side in one PNG (for game engines and sticker packs).
pub fn sprite_sheet(frames: &[Pixmap]) -> Option<Pixmap> {
    let first = frames.first()?;
    let w = first.w.checked_mul(frames.len())?;
    if w > MAX_SIDE * 8 {
        return None;
    }
    let mut out = Pixmap { w, h: first.h, px: vec![Color32::TRANSPARENT; w * first.h] };
    for (i, f) in frames.iter().enumerate() {
        let ox = i * first.w;
        for y in 0..f.h.min(first.h) {
            for x in 0..f.w.min(first.w) {
                if let (Some(d), Some(s)) = (out.px.get_mut(y * w + ox + x), f.px.get(y * f.w + x)) {
                    *d = *s;
                }
            }
        }
    }
    Some(out)
}

/// The order frames play in, for a given playback mode (ping-pong repeats the middle backwards).
pub fn play_order(frames: usize, mode: Playback) -> Vec<usize> {
    let mut v: Vec<usize> = (0..frames).collect();
    if mode == Playback::PingPong && frames > 2 {
        v.extend((1..frames - 1).rev());
    }
    v
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let b = [c.first().copied().unwrap_or(0), c.get(1).copied().unwrap_or(0), c.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let ch = |shift: u32| char::from(B64.get(((n >> shift) & 63) as usize).copied().unwrap_or(b'A'));
        s.push(ch(18));
        s.push(ch(12));
        s.push(if c.len() > 1 { ch(6) } else { '=' });
        s.push(if c.len() > 2 { ch(0) } else { '=' });
    }
    s
}

pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

/// Compact form for browser storage: deflate, then base64.
pub fn pack(text: &str) -> Result<String, String> {
    let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    let bytes = enc.finish().map_err(|e| e.to_string())?;
    Ok(format!("z:{}", base64_encode(&bytes)))
}

pub fn unpack(s: &str) -> Result<String, String> {
    let Some(b64) = s.strip_prefix("z:") else { return Ok(s.to_string()) };
    let bytes = base64_decode(b64).ok_or("damaged save data")?;
    let mut out = String::new();
    flate2::read::DeflateDecoder::new(&bytes[..]).take(MAX_FILE as u64).read_to_string(&mut out).map_err(|e| format!("damaged save data: {e}"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file in the exact shape the original web page wrote.
    const V1: &str = r##"{"format":"wobbleworks","version":1,"id":"abc","name":"Old doodle","w":64,"h":48,"modified":1,
      "settings":{"bg":"#ffd23f","transparent":false,"speed":90,"wiggle":2,"zoom":1.5},"current":1,
      "layers":[{"name":"Layer 1","visible":true,"opacity":1,"raster":[null,null,null],"clip":false,"alphaLock":false,
        "strokes":[{"t":"shaky","sh":"diamond","c":"#ff2e88","z":6,"s":123456789,"k":0,"p":[1,2,30.5,40]}]},
        {"name":"Ink","visible":false,"opacity":0.5,"raster":[null,null,null],"clip":true,"alphaLock":true,"strokes":[]}]}"##;

    #[test]
    fn opens_original_v1_files() {
        let l = from_json(V1).unwrap();
        let d = &l.doc;
        assert_eq!((d.w, d.h, d.frames, d.current), (64, 48, 3, 1));
        assert_eq!(d.bg, Color32::from_rgb(0xff, 0xd2, 0x3f));
        assert_eq!(d.speed_ms, 90);
        assert_eq!(d.wiggle, 2.0);
        assert_eq!(l.name, "Old doodle");
        assert_eq!(l.zoom, Some(1.5));
        let s = &d.layers[0].strokes[0];
        assert_eq!((s.brush, s.tip, s.seed), (Brush::Shaky, Tip::Diamond, 123_456_789));
        assert_eq!(s.pts[1].x, 30.5);
        assert!(d.layers[1].clip && d.layers[1].alpha_lock && !d.layers[1].visible);
    }

    #[test]
    fn v2_round_trip_keeps_everything() {
        let mut d = Doc::new(40, 30);
        d.set_frames(4);
        d.layers[0].raster_mut(2, 40, 30).unwrap().set(5, 6, Color32::RED);
        d.layers[0].blend = Blend::Multiply;
        Arc::make_mut(&mut d.layers[0].strokes).push(Stroke {
            brush: Brush::Nib,
            tip: Tip::Heart,
            color: Color32::BLUE,
            size: 9.0,
            seed: 7,
            lock: true,
            pts: vec![Pt { x: 1.0, y: 2.0, p: 0.5 }, Pt::new(3.0, 4.0)],
        });
        d.playback = Playback::PingPong;
        let mut cache = PngCache::default();
        let json = to_json(&to_file(&d, "Pic", Some("id1"), 5.0, 2.0, &mut cache)).unwrap();
        let back = from_json(&json).unwrap();
        let b = &back.doc;
        assert_eq!(b.frames, 4);
        assert_eq!(b.playback, Playback::PingPong);
        assert_eq!(b.layers[0].blend, Blend::Multiply);
        assert_eq!(b.layers[0].raster[2].as_ref().unwrap().get(5, 6), Color32::RED);
        assert!(b.layers[0].raster[0].is_none());
        assert_eq!(b.layers[0].strokes[0], d.layers[0].strokes[0]);
    }

    #[test]
    fn duplicate_frames_are_stored_once() {
        let mut d = Doc::new(20, 20);
        for f in 0..3 {
            d.layers[0].raster_mut(f, 20, 20).unwrap().set(1, 1, Color32::RED);
        }
        let file = to_file(&d, "x", None, 0.0, 1.0, &mut PngCache::default());
        assert_eq!(file.layers[0].raster[1].as_deref(), Some("=0"));
        let back = from_json(&to_json(&file).unwrap()).unwrap();
        assert_eq!(back.doc.layers[0].raster[2].as_ref().unwrap().get(1, 1), Color32::RED);
    }

    #[test]
    fn hostile_files_are_errors_not_crashes() {
        for bad in [
            "",
            "null",
            "[]",
            r#"{"format":"nope"}"#,
            r#"{"format":"wobbleworks","w":0,"h":10}"#,
            r#"{"format":"wobbleworks","w":99999999,"h":10}"#,
            r#"{"format":"wobbleworks","w":10,"h":10,"version":99999}"#,
        ] {
            assert!(from_json(bad).is_err(), "{bad}");
        }
        let weird = r#"{"format":"wobbleworks","w":10,"h":10,"current":99,"settings":{"speed":-1,"wiggle":1e300,"frames":0},
          "layers":[{"opacity":7,"raster":["data:image/png;base64,!!!!","=0","=5",null,"=2"],
          "strokes":[{"t":"x","c":"zz","z":-4,"s":1e300,"p":[1e308,-1e308,3]},{"p":[]}]}]}"#;
        let l = from_json(weird).unwrap();
        assert_eq!(l.doc.current, 0);
        assert_eq!(l.doc.frames, crate::model::MIN_FRAMES);
        assert_eq!(l.doc.layers[0].opacity, 1.0);
        assert_eq!(l.doc.layers[0].strokes.len(), 1);
        assert!(!l.warnings.is_empty());
    }

    #[test]
    fn base64_and_pack_round_trip() {
        for data in [&b""[..], b"a", b"ab", b"abc", b"hello wobbly world \x00\xff"] {
            assert_eq!(base64_decode(&base64_encode(data)).unwrap(), data);
        }
        assert!(base64_decode("@@").is_none());
        let s = "{\"a\":1}".repeat(500);
        let p = pack(&s).unwrap();
        assert!(p.len() < s.len());
        assert_eq!(unpack(&p).unwrap(), s);
        assert_eq!(unpack("plain").unwrap(), "plain");
        assert!(unpack("z:AAAA").is_err());
    }

    #[test]
    fn exports_encode() {
        let frames = vec![Pixmap::filled(8, 8, Color32::RED), Pixmap::filled(8, 8, Color32::BLUE)];
        let gif = gif_bytes(&frames, 100).unwrap();
        assert!(gif.starts_with(b"GIF89a"));
        let png = png_bytes(&frames[0]).unwrap();
        let back = decode_image(&png).unwrap();
        assert_eq!(back.get(3, 3), Color32::RED);
        let sheet = sprite_sheet(&frames).unwrap();
        assert_eq!((sheet.w, sheet.h), (16, 8));
        assert_eq!(sheet.get(12, 1), Color32::BLUE);
        assert!(decode_image(b"not a picture").is_err());
        assert_eq!(play_order(4, Playback::PingPong), vec![0, 1, 2, 3, 2, 1]);
        assert_eq!(play_order(2, Playback::PingPong), vec![0, 1]);
    }
}
