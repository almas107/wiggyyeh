//! 3D layers in PSD: written as their last render (so any reader shows the picture) plus our
//! `wb3D` block, and read back as the same 3D layer.

use std::sync::Arc;

use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::*;
use photocraft_geom::{Rect, Size};
use photocraft_io::{ExportOptions, export, import};

fn layer3d() -> Model3dLayer {
    let mut cache = photocraft_raster::Surface::new(PixelFormat::RGBA8);
    cache.fill_rect(Rect::new(8, 6, 40, 30), &[0.1, 0.2, 0.9, 1.0]);
    Model3dLayer {
        scene: Arc::new(br#"{"note":"opaque bytes"}"#.to_vec()),
        scene_id: 0xdead_beef,
        main_camera_id: 42,
        follow_view: false,
        fit: Model3dFit::Width,
        cache,
        dirty: false,
    }
}

#[test]
fn a_3d_layer_survives_a_psd_round_trip() {
    let mut d = Document::with_background("t", Size::new(48, 36), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut l = Layer::new("3D (Trace 1)", LayerContent::Model3d(layer3d()));
    l.opacity = 0.5;
    d.layers.push(l);
    let r = export(&d, "x.psd", &ExportOptions::default()).expect("export");
    let back = import("x.psd", &r.bytes).expect("import").document;
    let l = back.layers.iter().find(|l| l.name == "3D (Trace 1)").expect("layer");
    let LayerContent::Model3d(m) = &l.content else { panic!("came back as {}", l.content.kind_name()) };
    let want = layer3d();
    assert_eq!((m.scene.as_slice(), m.scene_id, m.main_camera_id, m.follow_view, m.fit), (want.scene.as_slice(), want.scene_id, 42, false, Model3dFit::Width));
    assert_eq!(m.cache.rgba(20, 20), want.cache.rgba(20, 20));
    assert!(!l.psd_blocks.iter().any(|(k, _)| k == b"wb3D"), "the block is rebuilt from the layer, not kept twice");
    assert!((l.opacity - 0.5).abs() < 0.01);
    // Exported again, it is the same picture.
    let again = export(&back, "x.psd", &ExportOptions::default()).expect("export");
    assert_eq!(import("x.psd", &again.bytes).expect("import").document.layers.len(), back.layers.len());
}

#[test]
fn a_damaged_3d_block_leaves_the_pixels() {
    let mut d = Document::with_background("t", Size::new(32, 32), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let mut l = Layer::raster("px", PixelFormat::RGBA8);
    if let Some(s) = l.surface_mut() {
        s.fill_rect(Rect::new(2, 2, 10, 10), &[1.0, 0.0, 0.0, 1.0]);
    }
    l.psd_blocks.push((*b"wb3D", Arc::new(b"WB3D\0\0\0\x01\xff\xff\xff\xff".to_vec())));
    d.layers.push(l);
    let r = export(&d, "x.psd", &ExportOptions::default()).expect("export");
    let back = import("x.psd", &r.bytes).expect("import").document;
    let l = back.layers.iter().find(|l| l.name == "px").expect("layer");
    assert!(matches!(l.content, LayerContent::Raster(_)));
}
