//! Render timings on a big note: `cargo run --release -p wobbleworks-3d --example bench [curves]`.

use std::time::Instant;

use serde_json::json;
use wobbleworks_3d::editor::Editor;

fn main() -> Result<(), String> {
    let n: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3000);
    let mut e = Editor::new();
    e.set_viewport(1600.0, 1000.0);
    let kinds = ["pen", "marker", "oil", "ink"];
    let t0 = Instant::now();
    for i in 0..n {
        let k = kinds[i % kinds.len()];
        e.run("brush.set", &json!({"kind": k, "size": 6 + (i % 5) * 3}))?;
        let a = i as f32 * 0.37;
        let pts: Vec<[f32; 4]> = (0..150).map(|j| {
            let t = j as f32 / 149.0;
            [(a.cos() * (1.0 + t)) * 1.5, t * 2.0 - 1.0 + (i % 17) as f32 * 0.05, (a.sin() * (1.0 + t)) * 1.5, 0.5 + 0.5 * t]
        }).collect();
        e.run("stroke.add", &json!({"points": pts}))?;
    }
    println!("{n} curves × 150 points added in {:?}", t0.elapsed());
    for frame in 0..4 {
        let t = Instant::now();
        let f = e.render(frame, true);
        println!("frame {frame}: {} triangles, {} batches in {:?}", f.triangles, f.batches.len(), t.elapsed());
    }
    let t = Instant::now();
    e.run("camera.orbit", &json!({"dx": 10, "dy": 0}))?;
    let f = e.render(0, true);
    println!("after orbit: {} triangles in {:?}", f.triangles, t.elapsed());
    let t = Instant::now();
    let bytes = wobbleworks_3d::io::save(&e.scene, &e.camera)?;
    println!("save: {} KB in {:?}", bytes.len() / 1024, t.elapsed());
    Ok(())
}
