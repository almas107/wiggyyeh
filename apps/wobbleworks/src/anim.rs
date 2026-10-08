//! Animation export: the boil frames of every wiggle layer as an animated GIF or a PNG sequence.
//! Each frame is the document with that frame of every wiggle layer showing
//! (`photocraft_engine::wiggle_cmds::at_frame`), flattened through PhotoCraft's exporter so colour
//! management matches File › Export. (A layered PSD keeps the frames as layers: plain Save.)

use photocraft_doc::Document;
use photocraft_engine::wiggle_cmds;
use photocraft_ui_egui::ExportSettings;

/// Largest picture exported as an animation (pixels): GIF tops out at 65 535 a side, and every
/// frame is held in memory while encoding.
pub const MAX_PIXELS: u64 = 4096 * 4096;

/// The document once per boil frame (once when it has no wiggle layers).
pub fn frames(doc: &Document) -> Vec<Document> {
    let n = wiggle_cmds::frame_count(doc).max(1);
    (0..n).map(|i| wiggle_cmds::at_frame(doc, i)).collect()
}

fn check_size(doc: &Document) -> Result<(u32, u32), String> {
    let (w, h) = (doc.size.width, doc.size.height);
    if w == 0 || h == 0 {
        return Err("the picture is empty".into());
    }
    if u64::from(w) * u64::from(h) > MAX_PIXELS || w > 65_535 || h > 65_535 {
        return Err(format!("{w} × {h} is too big for an animation (at most {} pixels)", MAX_PIXELS));
    }
    Ok((w, h))
}

/// Each frame as PNG bytes.
pub fn png_frames(doc: &Document) -> Result<Vec<Vec<u8>>, String> {
    check_size(doc)?;
    frames(doc).iter().map(|f| crate::io::export(f, "frame.png", &ExportSettings::default()).map(|(b, _)| b)).collect()
}

/// An animated GIF of the boil frames, looping forever at `fps` frames per second.
pub fn gif(doc: &Document, fps: f32) -> Result<Vec<u8>, String> {
    use image::codecs::gif::{GifEncoder, Repeat};
    let (w, h) = check_size(doc)?;
    let fps = if fps.is_finite() { fps.clamp(1.0, 50.0) } else { 8.0 };
    let delay = image::Delay::from_numer_denom_ms((1000.0 / fps).round() as u32, 1);
    let mut frames = Vec::new();
    for png in png_frames(doc)? {
        let img = photocraft_codecs::decode(&png).map_err(|e| e.to_string())?;
        let rgba = image::RgbaImage::from_raw(w, h, img.to_rgba8()).ok_or("a frame came out the wrong size")?;
        frames.push(image::Frame::from_parts(rgba, 0, 0, delay));
    }
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new_with_speed(&mut out, 10);
        enc.set_repeat(Repeat::Infinite).map_err(|e| e.to_string())?;
        enc.encode_frames(frames).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_engine::Session;
    use serde_json::json;

    fn wiggly() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
        s.execute("wiggle.new", json!({})).unwrap();
        s.execute("wiggle.stroke", json!({"points": [[4, 15], [36, 15]], "size": 4, "color": "#ff0000", "amount": 3})).unwrap();
        s
    }

    #[test]
    fn a_gif_has_one_frame_per_boil_frame_and_loops() {
        use image::AnimationDecoder;
        let s = wiggly();
        let doc = s.active().unwrap().doc.clone();
        let bytes = gif(&doc, 8.0).unwrap();
        assert_eq!(&bytes[..6], b"GIF89a");
        let dec = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(&bytes)).unwrap();
        let frames = dec.into_frames().collect_frames().unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].buffer().dimensions(), (40, 30));
        assert!(frames.windows(2).any(|f| f[0].buffer() != f[1].buffer()), "the lines boil");
        let pngs = png_frames(&doc).unwrap();
        assert_eq!(pngs.len(), 3);
        assert!(pngs.iter().all(|p| p.starts_with(b"\x89PNG")));
    }

    #[test]
    fn plain_pictures_export_one_frame_and_huge_ones_are_refused() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        let doc = s.active().unwrap().doc.clone();
        assert_eq!(frames(&doc).len(), 1);
        assert!(gif(&doc, f32::NAN).is_ok());
        let mut big = (*doc).clone();
        big.size.width = 100_000;
        assert!(gif(&big, 8.0).is_err());
        assert!(png_frames(&big).is_err());
    }
}
