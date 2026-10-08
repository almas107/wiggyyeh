//! The pixel font as a TrueType font, built in memory at startup (no font file is shipped), so
//! egui can lay out every label, menu and panel of the editor in it.
//!
//! Each run of ink pixels in a glyph row becomes one rectangle contour. One font pixel is
//! [`UNIT`] font units; the em is [`EM_PIXELS`] pixels tall: 8 above the baseline (7-row capitals
//! plus a row of air) and 2 below for descenders.

use crate::pixfont;

/// Font units per font pixel.
pub const UNIT: i16 = 100;
/// Pixel rows per em.
pub const EM_PIXELS: i16 = 10;
const ASCENT_PIXELS: i16 = 8;
const DESCENT_PIXELS: i16 = 2;
/// Blank columns after each glyph.
const GAP: i16 = 1;

struct Outline {
    advance: u16,
    /// Rectangles (x0, y0, x1, y1) in font units, y up from the baseline.
    rects: Vec<(i16, i16, i16, i16)>,
}

fn outline(g: pixfont::Glyph) -> Outline {
    let width = g.iter().map(|r| r.len()).max().unwrap_or(0) as i16;
    let mut rects = Vec::new();
    for (row, line) in g.iter().enumerate() {
        // Row 0 is the top of the capitals, 7 pixels above the baseline.
        let top = (7 - row as i16) * UNIT;
        let mut start: Option<i16> = None;
        for (col, ch) in line.chars().chain(std::iter::once('.')).enumerate() {
            let col = col as i16;
            match (ch == '#', start) {
                (true, None) => start = Some(col),
                (false, Some(s)) => {
                    rects.push((s * UNIT, top - UNIT, col * UNIT, top));
                    start = None;
                }
                _ => {}
            }
        }
    }
    Outline { advance: ((width + GAP) * UNIT) as u16, rects }
}

fn be16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}
fn bei16(v: &mut Vec<u8>, x: i16) {
    v.extend_from_slice(&x.to_be_bytes());
}
fn be32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}

/// The `glyf` record of one glyph (empty for blank glyphs).
fn glyf_record(o: &Outline) -> Vec<u8> {
    let mut v = Vec::new();
    if o.rects.is_empty() {
        return v;
    }
    let x_min = o.rects.iter().map(|r| r.0).min().unwrap_or(0);
    let y_min = o.rects.iter().map(|r| r.1).min().unwrap_or(0);
    let x_max = o.rects.iter().map(|r| r.2).max().unwrap_or(0);
    let y_max = o.rects.iter().map(|r| r.3).max().unwrap_or(0);
    bei16(&mut v, o.rects.len() as i16);
    for b in [x_min, y_min, x_max, y_max] {
        bei16(&mut v, b);
    }
    for i in 0..o.rects.len() {
        be16(&mut v, (i * 4 + 3) as u16);
    }
    be16(&mut v, 0); // no instructions
    // Clockwise outer contours (TrueType): bottom-left, top-left, top-right, bottom-right.
    let pts: Vec<(i16, i16)> = o.rects.iter().flat_map(|&(x0, y0, x1, y1)| [(x0, y0), (x0, y1), (x1, y1), (x1, y0)]).collect();
    v.extend(std::iter::repeat_n(0x01u8, pts.len())); // on-curve, 16-bit deltas
    let (mut px, mut py) = (0i16, 0i16);
    for &(x, _) in &pts {
        bei16(&mut v, x - px);
        px = x;
    }
    for &(_, y) in &pts {
        bei16(&mut v, y - py);
        py = y;
    }
    if v.len() % 2 == 1 {
        v.push(0);
    }
    v
}

fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, c| {
        let mut w = [0u8; 4];
        w[..c.len()].copy_from_slice(c);
        sum.wrapping_add(u32::from_be_bytes(w))
    })
}

/// The font file bytes.
pub fn build() -> Vec<u8> {
    let chars: Vec<char> = pixfont::chars().collect();
    // Glyph 0 is .notdef (the hollow box).
    let mut outlines = vec![outline(&["####", "#..#", "#..#", "#..#", "#..#", "#..#", "####"])];
    outlines.extend(chars.iter().map(|&c| outline(pixfont::glyph(c).unwrap_or(&[".."]))));
    let n = outlines.len() as u16;
    let em = (EM_PIXELS * UNIT) as u16;
    let ascent = ASCENT_PIXELS * UNIT;
    let descent = -(DESCENT_PIXELS * UNIT);
    let max_adv = outlines.iter().map(|o| o.advance).max().unwrap_or(0);
    let max_points = outlines.iter().map(|o| o.rects.len() * 4).max().unwrap_or(0) as u16;
    let max_contours = outlines.iter().map(|o| o.rects.len()).max().unwrap_or(0) as u16;

    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for o in &outlines {
        be32(&mut loca, glyf.len() as u32);
        glyf.extend(glyf_record(o));
    }
    be32(&mut loca, glyf.len() as u32);

    let mut head = Vec::new();
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0); // checkSumAdjustment, patched below
    be32(&mut head, 0x5F0F_3CF5);
    be16(&mut head, 0x000B);
    be16(&mut head, em);
    head.extend([0u8; 16]); // created, modified
    for b in [0, descent, (max_adv as i16), ascent] {
        bei16(&mut head, b);
    }
    be16(&mut head, 0); // macStyle
    be16(&mut head, 8); // lowestRecPPEM
    bei16(&mut head, 2);
    bei16(&mut head, 1); // long loca
    bei16(&mut head, 0);

    let mut hhea = Vec::new();
    be32(&mut hhea, 0x0001_0000);
    bei16(&mut hhea, ascent);
    bei16(&mut hhea, descent);
    bei16(&mut hhea, 0);
    be16(&mut hhea, max_adv);
    bei16(&mut hhea, 0);
    bei16(&mut hhea, 0);
    bei16(&mut hhea, max_adv as i16);
    bei16(&mut hhea, 1);
    bei16(&mut hhea, 0);
    bei16(&mut hhea, 0);
    hhea.extend([0u8; 8]);
    bei16(&mut hhea, 0);
    be16(&mut hhea, n);

    let mut maxp = Vec::new();
    be32(&mut maxp, 0x0001_0000);
    be16(&mut maxp, n);
    be16(&mut maxp, max_points);
    be16(&mut maxp, max_contours);
    be16(&mut maxp, 0);
    be16(&mut maxp, 0);
    be16(&mut maxp, 2);
    for _ in 0..7 {
        be16(&mut maxp, 0);
    }
    be16(&mut maxp, 0);

    let mut hmtx = Vec::new();
    for o in &outlines {
        be16(&mut hmtx, o.advance);
        bei16(&mut hmtx, o.rects.iter().map(|r| r.0).min().unwrap_or(0));
    }

    // cmap: one format 4 subtable (Windows, Unicode BMP), one segment per character.
    let mut segs: Vec<(u16, u16)> = chars.iter().enumerate().filter_map(|(i, &c)| u16::try_from(u32::from(c)).ok().map(|cp| (cp, (i + 1) as u16))).collect();
    segs.sort_unstable();
    segs.push((0xFFFF, 0));
    let segx2 = (segs.len() * 2) as u16;
    let mut pow = 1u16;
    while pow * 2 <= segs.len() as u16 {
        pow *= 2;
    }
    let mut sub = Vec::new();
    be16(&mut sub, 4);
    be16(&mut sub, (16 + segs.len() * 8) as u16);
    be16(&mut sub, 0);
    be16(&mut sub, segx2);
    be16(&mut sub, pow * 2);
    be16(&mut sub, pow.trailing_zeros() as u16);
    be16(&mut sub, segx2.saturating_sub(pow * 2));
    for &(c, _) in &segs {
        be16(&mut sub, c);
    }
    be16(&mut sub, 0);
    for &(c, _) in &segs {
        be16(&mut sub, c);
    }
    for &(c, g) in &segs {
        be16(&mut sub, if c == 0xFFFF { 1 } else { g.wrapping_sub(c) });
    }
    for _ in &segs {
        be16(&mut sub, 0);
    }
    let mut cmap = Vec::new();
    be16(&mut cmap, 0);
    be16(&mut cmap, 1);
    be16(&mut cmap, 3);
    be16(&mut cmap, 1);
    be32(&mut cmap, 12);
    cmap.extend(sub);

    let mut post = Vec::new();
    be32(&mut post, 0x0003_0000);
    be32(&mut post, 0);
    bei16(&mut post, -UNIT);
    bei16(&mut post, UNIT);
    post.extend([0u8; 20]);

    let mut os2 = Vec::new();
    be16(&mut os2, 4);
    bei16(&mut os2, (max_adv / 2) as i16); // xAvgCharWidth
    be16(&mut os2, 400);
    be16(&mut os2, 5);
    be16(&mut os2, 0);
    for v in [650i16, 600, 0, 75, 650, 600, 0, 350, 50, 250] {
        bei16(&mut os2, v);
    }
    bei16(&mut os2, 0); // sFamilyClass
    os2.extend([0u8; 10]); // panose
    os2.extend([0u8; 16]); // unicode ranges
    os2.extend(*b"WOBL");
    be16(&mut os2, 0x0040); // REGULAR
    be16(&mut os2, segs.first().map_or(32, |s| s.0));
    be16(&mut os2, 0xFFFF);
    bei16(&mut os2, ascent);
    bei16(&mut os2, descent);
    bei16(&mut os2, 0);
    be16(&mut os2, ascent as u16);
    be16(&mut os2, (-descent) as u16);
    be32(&mut os2, 1);
    be32(&mut os2, 0);
    bei16(&mut os2, 7 * UNIT / 2);
    bei16(&mut os2, 7 * UNIT);
    be16(&mut os2, 0);
    be16(&mut os2, 32);
    be16(&mut os2, 0);

    let mut tables: Vec<([u8; 4], Vec<u8>)> = vec![
        (*b"OS/2", os2),
        (*b"cmap", cmap),
        (*b"glyf", glyf),
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"hmtx", hmtx),
        (*b"loca", loca),
        (*b"maxp", maxp),
        (*b"post", post),
    ];
    tables.sort_by_key(|t| t.0);

    let num = tables.len() as u16;
    let mut pow = 1u16;
    while pow * 2 <= num {
        pow *= 2;
    }
    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000);
    be16(&mut out, num);
    be16(&mut out, pow * 16);
    be16(&mut out, pow.trailing_zeros() as u16);
    be16(&mut out, num * 16 - pow * 16);
    let mut offset = 12 + 16 * tables.len();
    let mut head_at = 0;
    let mut body = Vec::new();
    for (tag, data) in &tables {
        out.extend_from_slice(tag);
        be32(&mut out, checksum(data));
        be32(&mut out, offset as u32);
        be32(&mut out, data.len() as u32);
        if tag == b"head" {
            head_at = offset;
        }
        let mut padded = data.clone();
        while padded.len() % 4 != 0 {
            padded.push(0);
        }
        offset += padded.len();
        body.extend(padded);
    }
    out.extend(body);
    let adjust = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
    if let Some(slot) = out.get_mut(head_at + 8..head_at + 12) {
        slot.copy_from_slice(&adjust.to_be_bytes());
    }
    out
}

/// The name WobbleWorks registers the font under.
pub const FONT_NAME: &str = "WobblePixel";

/// Put the pixel font first in every family PhotoCraft uses (its own fonts stay as fallbacks for
/// characters the pixel font lacks). `scale` sizes it against the fonts it replaces.
pub fn install(defs: &mut egui::FontDefinitions, scale: f32) {
    let mut data = egui::FontData::from_owned(build());
    data.tweak.scale = scale;
    defs.font_data.insert(FONT_NAME.into(), std::sync::Arc::new(data));
    for (family, stack) in defs.families.iter_mut() {
        if !matches!(family, egui::FontFamily::Monospace) && !stack.iter().any(|n| n == FONT_NAME) {
            stack.insert(0, FONT_NAME.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_font_egui_lays_out_with_pixel_advances() {
        let bytes = build();
        assert_eq!(&bytes[..4], &[0, 1, 0, 0]);
        let ctx = egui::Context::default();
        let mut defs = egui::FontDefinitions::empty();
        defs.families.insert(egui::FontFamily::Proportional, vec![]);
        defs.families.insert(egui::FontFamily::Monospace, vec![]);
        install(&mut defs, 1.0);
        ctx.set_fonts(defs);
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        let width = |text: &str| {
            let mut w = 0.0;
            ctx.run_ui(egui::RawInput::default(), |ui| {
                // 10 points per em: one font pixel per point.
                w = ui.painter().layout_no_wrap(text.into(), egui::FontId::proportional(10.0), egui::Color32::WHITE).size().x;
            })
            .textures_delta
            .clear();
            w
        };
        // "A" is 5 pixels plus 1 of spacing; "i" is 3 plus 1.
        assert!((width("A") - 6.0).abs() < 0.3, "{}", width("A"));
        assert!((width("AAA") - 18.0).abs() < 0.5, "{}", width("AAA"));
        assert!((width("i") - 4.0).abs() < 0.3, "{}", width("i"));
    }

    #[test]
    fn every_character_gets_a_glyph() {
        let n = pixfont::chars().count();
        assert!(n >= 95);
        assert!(pixfont::chars().all(|c| pixfont::glyph(c).is_some()));
    }
}
