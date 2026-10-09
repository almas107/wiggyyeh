//! Files: `.wob3d` notes (gzipped JSON), OBJ export of the curves as tube meshes (with an MTL of
//! their colours), and OBJ import for 3D model resources. Loading never trusts the file: sizes
//! are capped and the note is repaired before use.

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

use crate::camera::Camera;
use crate::math::{Vec3, v3};
use crate::model::{Rgba, Scene, Stroke};

pub const FORMAT: &str = "wobbleworks-3d";
pub const VERSION: u32 = 1;
/// Biggest note accepted (decompressed bytes).
pub const MAX_FILE: u64 = 1 << 30;
/// Most OBJ vertices / faces accepted.
pub const OBJ_MAX: usize = 4_000_000;

#[derive(Serialize)]
struct FileOut<'a> {
    format: &'a str,
    version: u32,
    scene: &'a Scene,
    camera: &'a Camera,
}

#[derive(Deserialize)]
struct FileIn {
    format: String,
    version: u32,
    scene: Scene,
    #[serde(default)]
    camera: Camera,
}

/// A note as `.wob3d` bytes.
pub fn save(scene: &Scene, camera: &Camera) -> Result<Vec<u8>, String> {
    let json = serde_json::to_vec(&FileOut { format: FORMAT, version: VERSION, scene, camera }).map_err(|e| format!("could not write the note: {e}"))?;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&json).map_err(|e| format!("could not compress the note: {e}"))?;
    gz.finish().map_err(|e| format!("could not compress the note: {e}"))
}

/// Read `.wob3d` bytes (gzipped or plain JSON).
pub fn load(bytes: &[u8]) -> Result<(Scene, Camera), String> {
    let json: Vec<u8> = if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(bytes).take(MAX_FILE + 1).read_to_end(&mut out).map_err(|e| format!("the note is damaged: {e}"))?;
        if out.len() as u64 > MAX_FILE {
            return Err("the note is too big".into());
        }
        out
    } else {
        bytes.to_vec()
    };
    let f: FileIn = serde_json::from_slice(&json).map_err(|e| format!("not a WobbleWorks 3D note: {e}"))?;
    if f.format != FORMAT {
        return Err(format!("not a WobbleWorks 3D note (format {:?})", f.format));
    }
    if f.version > VERSION {
        return Err(format!("this note was made by a newer WobbleWorks (version {})", f.version));
    }
    let mut scene = f.scene;
    scene.repair();
    let mut camera = f.camera;
    camera.sanitize();
    Ok((scene, camera))
}

/// Tube rings around a curve: (positions, faces as quads of position indices).
fn tube(s: &Stroke, sides: usize) -> (Vec<Vec3>, Vec<[usize; 4]>) {
    let pts: Vec<_> = s.points.iter().filter(|p| p.p.is_finite()).collect();
    let n = pts.len();
    let mut pos = Vec::new();
    let mut faces = Vec::new();
    if n < 2 {
        return (pos, faces);
    }
    let r = s.brush.radius();
    let mut normal = (pts[1].p - pts[0].p).normalized().any_perpendicular();
    for i in 0..n {
        let a = pts[i.saturating_sub(1)].p;
        let b = pts[(i + 1).min(n - 1)].p;
        let t = (b - a).normalized();
        if t != Vec3::ZERO {
            // Parallel transport keeps the rings from twisting.
            normal = (normal - t * normal.dot(t)).normalized();
            if normal == Vec3::ZERO {
                normal = t.any_perpendicular();
            }
        }
        let bin = t.cross(normal).normalized();
        let pr = if s.brush.pressure { 0.15 + 0.85 * pts[i].pressure } else { 1.0 };
        for k in 0..sides {
            let ang = std::f32::consts::TAU * k as f32 / sides as f32;
            pos.push(pts[i].p + (normal * ang.cos() + bin * ang.sin()) * (r * pr));
        }
    }
    for i in 0..n - 1 {
        for k in 0..sides {
            let (a, b) = (i * sides + k, i * sides + (k + 1) % sides);
            faces.push([a, b, b + sides, a + sides]);
        }
    }
    (pos, faces)
}

/// The visible curves as an OBJ (tubes) plus its MTL. Groups become OBJ objects.
pub fn export_obj(scene: &Scene, mtl_name: &str) -> (String, String) {
    use std::fmt::Write as _;
    let mut obj = String::new();
    let mut mtl = String::new();
    let _ = writeln!(obj, "# WobbleWorks 3D\nmtllib {mtl_name}");
    let mut colours: Vec<Rgba> = Vec::new();
    let mut base = 1usize;
    for g in &scene.groups {
        if !scene.group_shown(g.id) {
            continue;
        }
        let name: String = g.name.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
        let _ = writeln!(obj, "o {name}");
        for s in scene.strokes.iter().filter(|s| s.group == g.id) {
            let (pos, faces) = tube(s, 8);
            if faces.is_empty() {
                continue;
            }
            let ci = colours.iter().position(|c| *c == s.brush.color).unwrap_or_else(|| {
                colours.push(s.brush.color);
                colours.len() - 1
            });
            let _ = writeln!(obj, "usemtl c{ci}");
            let c = s.brush.color.to_f32();
            for p in &pos {
                let _ = writeln!(obj, "v {:.5} {:.5} {:.5} {:.4} {:.4} {:.4}", p.x, p.y, p.z, c[0], c[1], c[2]);
            }
            for f in &faces {
                let _ = writeln!(obj, "f {} {} {} {}", f[0] + base, f[1] + base, f[2] + base, f[3] + base);
            }
            base += pos.len();
        }
    }
    for (i, c) in colours.iter().enumerate() {
        let f = c.to_f32();
        let _ = writeln!(mtl, "newmtl c{i}\nKd {:.4} {:.4} {:.4}\nd {:.4}\n", f[0], f[1], f[2], f[3]);
    }
    (obj, mtl)
}

/// The visible curves as a binary glTF (`.glb`): tubes with baked vertex colours (Feather's GLTF
/// export), one mesh per group so Blender gets one object per group.
pub fn export_glb(scene: &Scene) -> Result<Vec<u8>, String> {
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut meshes = Vec::new();
    let mut nodes = Vec::new();
    for g in &scene.groups {
        if !scene.group_shown(g.id) {
            continue;
        }
        let mut pos: Vec<Vec3> = Vec::new();
        let mut col: Vec<[f32; 4]> = Vec::new();
        let mut idx: Vec<u32> = Vec::new();
        for s in scene.strokes.iter().filter(|s| s.group == g.id) {
            let (p, faces) = tube(s, 8);
            if faces.is_empty() {
                continue;
            }
            let base = u32::try_from(pos.len()).map_err(|_| "the note is too big for glTF")?;
            let c = s.brush.color.to_f32();
            // glTF vertex colours are linear.
            let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
            let c = [lin(c[0]), lin(c[1]), lin(c[2]), s.brush.opacity];
            col.extend(std::iter::repeat_n(c, p.len()));
            pos.extend(p);
            for f in faces {
                let q = f.map(|i| base + i as u32);
                idx.extend_from_slice(&[q[0], q[1], q[2], q[0], q[2], q[3]]);
            }
            if pos.len() > OBJ_MAX {
                return Err("the note is too big to export".into());
            }
        }
        if idx.is_empty() {
            continue;
        }
        let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for p in &pos {
            for (k, v) in p.to_array().into_iter().enumerate() {
                lo[k] = lo[k].min(v);
                hi[k] = hi[k].max(v);
            }
        }
        let mut push_view = |data: &[u8], target: u32| -> usize {
            while !bin.len().is_multiple_of(4) {
                bin.push(0);
            }
            let off = bin.len();
            bin.extend_from_slice(data);
            views.push(serde_json::json!({"buffer": 0, "byteOffset": off, "byteLength": data.len(), "target": target}));
            views.len() - 1
        };
        let pb: Vec<u8> = pos.iter().flat_map(|p| p.to_array()).flat_map(f32::to_le_bytes).collect();
        let cb: Vec<u8> = col.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
        let ib: Vec<u8> = idx.iter().flat_map(|v| v.to_le_bytes()).collect();
        let (vp, vc, vi) = (push_view(&pb, 34962), push_view(&cb, 34962), push_view(&ib, 34963));
        let a0 = accessors.len();
        accessors.push(serde_json::json!({"bufferView": vp, "componentType": 5126, "count": pos.len(), "type": "VEC3", "min": lo, "max": hi}));
        accessors.push(serde_json::json!({"bufferView": vc, "componentType": 5126, "count": col.len(), "type": "VEC4"}));
        accessors.push(serde_json::json!({"bufferView": vi, "componentType": 5125, "count": idx.len(), "type": "SCALAR"}));
        meshes.push(serde_json::json!({"name": g.name, "primitives": [{"attributes": {"POSITION": a0, "COLOR_0": a0 + 1}, "indices": a0 + 2, "material": 0}]}));
        nodes.push(serde_json::json!({"name": g.name, "mesh": meshes.len() - 1}));
    }
    if nodes.is_empty() {
        return Err("nothing to export: draw some curves first".into());
    }
    let node_ids: Vec<usize> = (0..nodes.len()).collect();
    let gltf = serde_json::json!({
        "asset": {"version": "2.0", "generator": "WobbleWorks 3D"},
        "scene": 0,
        "scenes": [{"nodes": node_ids}],
        "nodes": nodes,
        "meshes": meshes,
        "materials": [{"name": "Curves", "doubleSided": true, "pbrMetallicRoughness": {"baseColorFactor": [1.0, 1.0, 1.0, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.85}}],
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [{"byteLength": bin.len()}],
    });
    let mut json = serde_json::to_vec(&gltf).map_err(|e| e.to_string())?;
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let total = u32::try_from(total).map_err(|_| "the note is too big for glTF")?;
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    Ok(out)
}

/// Positions and triangles from OBJ text (polygons are fanned; normals and UVs ignored).
pub fn parse_obj(text: &str) -> Result<(Vec<Vec3>, Vec<[u32; 3]>), String> {
    let mut pos: Vec<Vec3> = Vec::new();
    let mut tris: Vec<[u32; 3]> = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => {
                let c: Vec<f32> = it.take(3).filter_map(|t| t.parse::<f32>().ok()).collect();
                if c.len() == 3 && c.iter().all(|v| v.is_finite()) {
                    if pos.len() >= OBJ_MAX {
                        return Err("the model has too many vertices".into());
                    }
                    pos.push(v3(c[0], c[1], c[2]));
                } else {
                    return Err(format!("bad vertex line: {line:.60}"));
                }
            }
            Some("f") => {
                let n = pos.len() as i64;
                let idx: Vec<u32> = it
                    .filter_map(|t| t.split('/').next().and_then(|i| i.parse::<i64>().ok()))
                    .filter_map(|i| {
                        let k = if i < 0 { n + i } else { i - 1 };
                        (0..n).contains(&k).then_some(k as u32)
                    })
                    .collect();
                for k in 1..idx.len().saturating_sub(1) {
                    if tris.len() >= OBJ_MAX {
                        return Err("the model has too many faces".into());
                    }
                    tris.push([idx[0], idx[k], idx[k + 1]]);
                }
            }
            _ => {}
        }
    }
    if tris.is_empty() {
        return Err("the OBJ has no faces".into());
    }
    Ok((pos, tris))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Brush, Point};
    use std::sync::Arc;

    #[test]
    fn notes_round_trip() {
        let mut s = Scene::default();
        s.strokes.push(Arc::new(Stroke {
            id: 5,
            group: 1,
            points: vec![Point { p: Vec3::ZERO, pressure: 0.5, n: Vec3::Y }, Point { p: Vec3::X, pressure: 1.0, n: Vec3::Y }],
            brush: Brush::default(),
            seed: 1,
        }));
        let g = crate::guide::Guide::primitive(6, crate::guide::Primitive::Tube, 10).expect("tube");
        s.guides.push(Arc::new(g));
        s.repair();
        let cam = Camera { yaw: 12.0, ..Camera::default() };
        let bytes = save(&s, &cam).expect("save");
        let (back, c2) = load(&bytes).expect("load");
        assert_eq!(back.strokes, s.strokes);
        assert_eq!(back.guides.len(), 1);
        assert!(!back.guides[0].grids.is_empty(), "guide surfaces rebuild on load");
        assert_eq!(c2.yaw, 12.0);
    }

    #[test]
    fn bad_files_are_errors() {
        assert!(load(b"").is_err());
        assert!(load(b"{}").is_err());
        assert!(load(&[0x1f, 0x8b, 0, 0]).is_err());
        assert!(load(br#"{"format":"other","version":1,"scene":{}}"#).is_err());
        let newer = format!(r#"{{"format":"{FORMAT}","version":99,"scene":{{}}}}"#);
        assert!(load(newer.as_bytes()).is_err());
    }

    #[test]
    fn obj_out_and_in() {
        let mut s = Scene::default();
        s.strokes.push(Arc::new(Stroke {
            id: 5,
            group: 1,
            points: (0..5).map(|i| Point { p: v3(i as f32, 0.0, 0.0), pressure: 1.0, n: Vec3::ZERO }).collect(),
            brush: Brush::default(),
            seed: 1,
        }));
        let (obj, mtl) = export_obj(&s, "note.mtl");
        assert!(mtl.contains("newmtl c0"));
        let (pos, tris) = parse_obj(&obj).expect("parse our own OBJ");
        assert_eq!(pos.len(), 5 * 8);
        assert_eq!(tris.len(), 4 * 8 * 2);
        assert!(parse_obj("v 1 2\n").is_err());
        let glb = export_glb(&s).expect("glb");
        assert_eq!(&glb[0..4], b"glTF");
        assert_eq!(u32::from_le_bytes([glb[8], glb[9], glb[10], glb[11]]) as usize, glb.len());
        assert!(export_glb(&Scene::default()).is_err());
        assert!(parse_obj("v 0 0 0\nf 1 2 3\n").is_err(), "faces pointing nowhere are dropped");
        let (_, t) = parse_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf -3/1/1 -2 -1\n").expect("relative indices");
        assert_eq!(t, vec![[0, 1, 2]]);
    }
}
