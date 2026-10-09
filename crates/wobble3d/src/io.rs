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
            for p in &pos {
                let _ = writeln!(obj, "v {:.5} {:.5} {:.5}", p.x, p.y, p.z);
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
        assert!(parse_obj("v 0 0 0\nf 1 2 3\n").is_err(), "faces pointing nowhere are dropped");
        let (_, t) = parse_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf -3/1/1 -2 -1\n").expect("relative indices");
        assert_eq!(t, vec![[0, 1, 2]]);
    }
}
