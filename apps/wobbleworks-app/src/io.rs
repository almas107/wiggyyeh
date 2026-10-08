//! File services shared by the native and web shells: PhotoCraft's importers and exporters.

use photocraft_codecs::{ChannelLayout, EncodeOptions, Image};
use photocraft_doc::Document;
use photocraft_ui_egui::{ExportSettings, Services};

/// Everything Open reads: PhotoCraft and Photoshop documents and flat images.
pub const OPEN_EXTS: &[&str] = &[
    "pcraft", "psd", "psb", "psdt", "png", "jpg", "jpeg", "tif", "tiff", "webp", "gif", "bmp", "tga", "ico", "qoi", "exr", "hdr", "pbm", "pgm", "ppm", "pam",
    "pfm", "dng", "cr2", "cr3", "nef", "nrw", "arw", "pef", "orf", "rw2", "raf", "abr", "grd",
];

/// Decode a file into a document (PSD, PNG, JPEG, …) through `photocraft-io`.
pub fn import(name: &str, bytes: &[u8]) -> Result<(Document, Vec<String>), String> {
    photocraft_io::import(name, bytes).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
}

/// Encode a document for a file name (the format follows the extension) through `photocraft-io`.
pub fn export(doc: &Document, path: &str, settings: &ExportSettings) -> Result<(Vec<u8>, Vec<String>), String> {
    let mut opts = photocraft_io::ExportOptions::default();
    if let Some(q) = settings.jpeg_quality {
        opts.encode.jpeg_quality = q;
    }
    opts.encode.webp_lossless = settings.webp_lossless;
    if let Some(q) = settings.webp_quality {
        opts.encode.webp_quality = q;
    }
    opts.tiff_layers = settings.tiff_layers;
    opts.xmp = if settings.xmp_all { photocraft_io::XmpEmbed::All } else { photocraft_io::XmpEmbed::None };
    photocraft_io::export(doc, path, &opts).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
}

/// Encode RGBA8 pixels as PNG (screenshots, `ui.render`).
pub fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let img = Image::from_u8(w, h, ChannelLayout::Rgba, rgba.to_vec()).map_err(|e| e.to_string())?;
    photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &EncodeOptions::default()).map_err(|e| e.to_string())
}

/// The codec services every shell has; each shell adds its own pickers and writer.
pub fn codec_services() -> Services {
    Services { import: Some(Box::new(import)), export: Some(Box::new(export)), encode_png: Some(Box::new(encode_png)), ..Default::default() }
}

/// `name` with its extension replaced by `.psd` (`cat.png` → `cat.psd`, `Wobble` → `Wobble.psd`).
pub fn psd_name(name: &str) -> String {
    with_extension(name, "psd")
}

/// `name` with its extension replaced by `ext` (or `ext` added when it has none).
pub fn with_extension(name: &str, ext: &str) -> String {
    let path = std::path::Path::new(name);
    let has_ext = path.extension().is_some() && path.file_stem().is_some_and(|s| !s.is_empty());
    if has_ext { path.with_extension(ext).to_string_lossy().into_owned() } else { format!("{name}.{ext}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn psd_names() {
        assert_eq!(psd_name("cat.png"), "cat.psd");
        assert_eq!(psd_name("Wobble"), "Wobble.psd");
        assert_eq!(psd_name("dir/pic.v2.jpg"), "dir/pic.v2.psd");
        assert_eq!(psd_name(".hidden"), ".hidden.psd");
        assert_eq!(psd_name(""), ".psd");
    }

    #[test]
    fn bad_bytes_are_an_error_not_a_panic() {
        assert!(import("x.psd", b"8BPS garbage").is_err());
        assert!(import("x.png", &[]).is_err());
        assert!(encode_png(4, 4, &[0; 3]).is_err());
    }
}
