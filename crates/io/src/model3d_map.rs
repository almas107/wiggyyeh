//! 3D layers in PSD. Photoshop has nothing like them, so a 3D layer is written as its last
//! render (ordinary pixels, so every reader shows the picture) plus a private tagged block,
//! `wb3D`, holding the scene and the camera it is seen through; reading the block back turns
//! the layer into a 3D layer again, so our own round trip is lossless.
//!
//! Block layout (big-endian): `WB3D`, u32 version (1), u32 header length, the header as JSON
//! (`{scene_id, main_camera_id, follow_view, fit}`), then the scene bytes to the end.

use std::sync::Arc;

use photocraft_doc::{Model3dFit, Model3dLayer};
use photocraft_raster::Surface;
use serde_json::{Value, json};

/// The tagged block key.
pub const KEY: [u8; 4] = *b"wb3D";
const MAGIC: &[u8; 4] = b"WB3D";
const VERSION: u32 = 1;
/// Largest header accepted (it is a handful of numbers).
const HEADER_MAX: usize = 64 * 1024;

fn fit_name(f: Model3dFit) -> &'static str {
    match f {
        Model3dFit::Height => "height",
        Model3dFit::Width => "width",
        Model3dFit::Contain => "contain",
    }
}

/// The block for a 3D layer.
pub fn write(m: &Model3dLayer) -> Vec<u8> {
    let header = json!({
        "scene_id": m.scene_id,
        "main_camera_id": m.main_camera_id,
        "follow_view": m.follow_view,
        "fit": fit_name(m.fit),
    })
    .to_string()
    .into_bytes();
    let mut out = Vec::with_capacity(12 + header.len() + m.scene.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_be_bytes());
    out.extend_from_slice(&(header.len() as u32).to_be_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(&m.scene);
    out
}

/// A 3D layer from its block and the pixels the layer record carried (its last render). None
/// when the block is not one of ours or is damaged: the layer then stays plain pixels.
pub fn read(data: &[u8], cache: Surface) -> Option<Model3dLayer> {
    let word = |at: usize| -> Option<u32> { Some(u32::from_be_bytes(data.get(at..at.checked_add(4)?)?.try_into().ok()?)) };
    if data.get(..4)? != MAGIC || word(4)? != VERSION {
        return None;
    }
    let len = word(8)? as usize;
    if len > HEADER_MAX {
        return None;
    }
    let end = 12usize.checked_add(len)?;
    let header: Value = serde_json::from_slice(data.get(12..end)?).ok()?;
    let scene = data.get(end..)?.to_vec();
    let fit = match header.get("fit").and_then(Value::as_str) {
        Some("width") => Model3dFit::Width,
        Some("contain") => Model3dFit::Contain,
        _ => Model3dFit::Height,
    };
    Some(Model3dLayer {
        scene: Arc::new(scene),
        scene_id: header.get("scene_id").and_then(Value::as_u64)?,
        main_camera_id: header.get("main_camera_id").and_then(Value::as_u64).unwrap_or(0),
        follow_view: header.get("follow_view").and_then(Value::as_bool).unwrap_or(false),
        fit,
        cache,
        // The scene's app re-renders it when it next sees the layer.
        dirty: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::PixelFormat;

    fn layer() -> Model3dLayer {
        Model3dLayer {
            scene: Arc::new(b"{\"strokes\":[]}".to_vec()),
            scene_id: 77,
            main_camera_id: 12,
            follow_view: true,
            fit: Model3dFit::Contain,
            cache: Surface::new(PixelFormat::RGBA8),
            dirty: false,
        }
    }

    #[test]
    fn the_block_round_trips() {
        let m = layer();
        let back = read(&write(&m), m.cache.clone()).expect("ours");
        assert_eq!(back, m);
    }

    #[test]
    fn damaged_blocks_leave_pixels() {
        let good = write(&layer());
        let empty = Surface::new(PixelFormat::RGBA8);
        for bad in [
            Vec::new(),
            b"WB3D".to_vec(),
            good[..10].to_vec(),
            {
                let mut b = good.clone();
                b[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
                b
            },
            {
                let mut b = good.clone();
                b[0] = b'X';
                b
            },
        ] {
            assert!(read(&bad, empty.clone()).is_none());
        }
    }
}
