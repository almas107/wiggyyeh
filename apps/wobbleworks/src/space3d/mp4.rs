//! MP4 video export: AV1 (rav1e, pure Rust) in an ISO BMFF file written here.
//!
//! The muxer follows ISO/IEC 14496-12 (the box layout) and the AV1 ISOBMFF binding
//! (aomediacodec.github.io/av1-isobmff): one `av01` track, one sample per temporal unit, temporal
//! delimiters dropped, the sequence header copied into `av1C`. `moov` comes before `mdat` so the
//! file plays while it downloads. Colours: BT.709 matrix, full range, sRGB transfer (signalled in
//! the AV1 sequence header).

use rav1e::prelude::*;
use wobbleworks_3d::raster::Picture;

/// Biggest frame side we encode, and most frames in one video.
pub const SIDE_MAX: u32 = 4096;
pub const FRAMES_MAX: usize = 1200;

/// The movie timescale: ticks per second (whole ticks per frame at 8, 10, 12, 15, 24, 25, 30
/// and 60 fps).
const TIMESCALE: u32 = 30_000;

/// Encode frames (straight RGBA8, all the same size; alpha is dropped over white) as an MP4.
pub fn encode(frames: &[Picture], fps: f32) -> Result<Vec<u8>, String> {
    let first = frames.first().ok_or("no frames to encode")?;
    if frames.len() > FRAMES_MAX {
        return Err(format!("at most {FRAMES_MAX} frames"));
    }
    // 4:2:0 needs even sides: drop the last row / column when odd.
    let (w, h) = (first.width & !1, first.height & !1);
    if w < 16 || h < 16 || w > SIDE_MAX || h > SIDE_MAX {
        return Err(format!("{} × {} can't be a video (16–{SIDE_MAX} pixels a side)", first.width, first.height));
    }
    if frames.iter().any(|f| f.width != first.width || f.height != first.height || f.rgba.len() as u64 != u64::from(f.width) * u64::from(f.height) * 4) {
        return Err("the frames differ in size".into());
    }
    let fps = if fps.is_finite() { fps.clamp(1.0, 60.0) } else { 12.0 };
    let delta = ((TIMESCALE as f32 / fps).round() as u32).max(1);
    // rav1e is careful, but a panic inside it must not take the app down (never crash).
    let encoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| av1(frames, w, h, fps))).map_err(|_| "the video encoder failed".to_string())??;
    mux(&encoded, w, h, delta)
}

struct Encoded {
    /// `av1C` contents (with the sequence header OBU as its config OBU).
    config: Vec<u8>,
    /// Temporal units, temporal delimiters removed, and whether each is a key frame.
    samples: Vec<(Vec<u8>, bool)>,
}

fn av1(frames: &[Picture], w: u32, h: u32, fps: f32) -> Result<Encoded, String> {
    let enc = EncoderConfig {
        width: w as usize,
        height: h as usize,
        time_base: Rational::new(1, (fps.round() as u64).max(1)),
        bit_depth: 8,
        chroma_sampling: ChromaSampling::Cs420,
        pixel_range: PixelRange::Full,
        color_description: Some(ColorDescription {
            color_primaries: ColorPrimaries::BT709,
            transfer_characteristics: TransferCharacteristics::SRGB,
            matrix_coefficients: MatrixCoefficients::BT709,
        }),
        // Line art: a fairly low quantizer keeps edges crisp.
        quantizer: 70,
        min_key_frame_interval: 1,
        max_key_frame_interval: 120,
        // No hidden frames: every packet is one shown frame, in order.
        low_latency: true,
        speed_settings: SpeedSettings::from_preset(10),
        // Tiles are encoded in parallel (on native, where rav1e has threads).
        tiles: std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 16),
        ..Default::default()
    };
    let cfg = Config::new().with_encoder_config(enc);
    let mut ctx: Context<u8> = cfg.new_context().map_err(|e| format!("video encoder: {e}"))?;
    let (cw, ch) = ((w / 2) as usize, (h / 2) as usize);
    let mut y = vec![0u8; (w * h) as usize];
    let mut u = vec![0u8; cw * ch];
    let mut v = vec![0u8; cw * ch];
    let mut samples = Vec::with_capacity(frames.len());
    let take = |ctx: &mut Context<u8>, samples: &mut Vec<(Vec<u8>, bool)>| -> Result<bool, String> {
        loop {
            match ctx.receive_packet() {
                Ok(p) => samples.push((strip_delimiters(&p.data), p.frame_type == FrameType::KEY)),
                Err(EncoderStatus::Encoded) => {}
                Err(EncoderStatus::NeedMoreData) => return Ok(false),
                Err(EncoderStatus::LimitReached) => return Ok(true),
                Err(e) => return Err(format!("video encoder: {e}")),
            }
        }
    };
    for f in frames {
        to_yuv420(f, w, h, &mut y, &mut u, &mut v);
        let mut frame = ctx.new_frame();
        frame.planes[0].copy_from_raw_u8(&y, w as usize, 1);
        frame.planes[1].copy_from_raw_u8(&u, cw, 1);
        frame.planes[2].copy_from_raw_u8(&v, cw, 1);
        ctx.send_frame(frame).map_err(|e| format!("video encoder: {e}"))?;
        take(&mut ctx, &mut samples)?;
    }
    ctx.send_frame(None).map_err(|e| format!("video encoder: {e}"))?;
    while !take(&mut ctx, &mut samples)? {}
    if samples.len() != frames.len() {
        return Err(format!("the encoder gave {} frames for {}", samples.len(), frames.len()));
    }
    let mut config = ctx.container_sequence_header();
    if let Some(seq) = samples.first().and_then(|(s, _)| obus(s).into_iter().find(|o| o.0 == OBU_SEQUENCE_HEADER)) {
        config.extend_from_slice(seq.1);
    }
    Ok(Encoded { config, samples })
}

/// Straight RGBA → full-range BT.709 Y′CbCr 4:2:0 (over white where see-through).
fn to_yuv420(f: &Picture, w: u32, h: u32, y: &mut [u8], u: &mut [u8], v: &mut [u8]) {
    let (w, h, stride) = (w as usize, h as usize, f.width as usize);
    let rgb = |x: usize, yy: usize| -> [f32; 3] {
        let i = (yy * stride + x) * 4;
        match f.rgba.get(i..i + 4) {
            Some(p) => {
                let a = f32::from(p[3]) / 255.0;
                [0, 1, 2].map(|k| f32::from(p[k]) * a + 255.0 * (1.0 - a))
            }
            None => [255.0; 3],
        }
    };
    let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let q = |x: f32| x.round().clamp(0.0, 255.0) as u8;
    for yy in 0..h {
        for x in 0..w {
            if let Some(o) = y.get_mut(yy * w + x) {
                *o = q(luma(rgb(x, yy)));
            }
        }
    }
    let cw = w / 2;
    for cy in 0..h / 2 {
        for cx in 0..cw {
            let mut s = [0.0f32; 3];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let c = rgb(cx * 2 + dx, cy * 2 + dy);
                for k in 0..3 {
                    s[k] += c[k] / 4.0;
                }
            }
            let l = luma(s);
            if let (Some(uo), Some(vo)) = (u.get_mut(cy * cw + cx), v.get_mut(cy * cw + cx)) {
                *uo = q((s[2] - l) / 1.8556 + 128.0);
                *vo = q((s[0] - l) / 1.5748 + 128.0);
            }
        }
    }
}

const OBU_SEQUENCE_HEADER: u8 = 1;
const OBU_TEMPORAL_DELIMITER: u8 = 2;

/// The OBUs of a temporal unit: (type, whole OBU bytes). Stops at anything malformed.
fn obus(data: &[u8]) -> Vec<(u8, &[u8])> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while let Some(&head) = data.get(at) {
        let kind = (head >> 3) & 0x0f;
        let has_ext = head & 0x04 != 0;
        let has_size = head & 0x02 != 0;
        let mut p = at + 1 + usize::from(has_ext);
        let end = if has_size {
            let Some((size, n)) = leb128(data.get(p..).unwrap_or(&[])) else { break };
            p += n;
            match p.checked_add(size) {
                Some(e) if e <= data.len() => e,
                _ => break,
            }
        } else {
            data.len()
        };
        let Some(obu) = data.get(at..end) else { break };
        out.push((kind, obu));
        at = end;
    }
    out
}

fn leb128(b: &[u8]) -> Option<(usize, usize)> {
    let mut v: u64 = 0;
    for (i, byte) in b.iter().take(8).enumerate() {
        v |= u64::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            return usize::try_from(v).ok().map(|v| (v, i + 1));
        }
    }
    None
}

/// A temporal unit without its temporal delimiter OBUs (the ISOBMFF binding drops them).
fn strip_delimiters(data: &[u8]) -> Vec<u8> {
    let parts = obus(data);
    let kept: usize = parts.iter().map(|o| o.1.len()).sum();
    if parts.is_empty() || kept != data.len() {
        // Not parseable as size-delimited OBUs: keep it as it came.
        return data.to_vec();
    }
    parts.into_iter().filter(|o| o.0 != OBU_TEMPORAL_DELIMITER).flat_map(|o| o.1.iter().copied()).collect()
}

/// A box: size, four-character type, contents.
fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

/// A full box: version and flags, then contents.
fn full(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(body.len() + 4);
    b.extend_from_slice(&((u32::from(version) << 24) | (flags & 0x00ff_ffff)).to_be_bytes());
    b.extend_from_slice(body);
    bx(kind, &b)
}

struct W(Vec<u8>);

impl W {
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn zeros(&mut self, n: usize) -> &mut Self {
        self.0.extend(std::iter::repeat_n(0, n));
        self
    }
    fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.0.extend_from_slice(b);
        self
    }
    /// The identity transformation matrix (mvhd, tkhd).
    fn matrix(&mut self) -> &mut Self {
        for v in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
            self.u32(v);
        }
        self
    }
}

fn mux(e: &Encoded, w: u32, h: u32, delta: u32) -> Result<Vec<u8>, String> {
    let n = u32::try_from(e.samples.len()).map_err(|_| "too many frames")?;
    let duration = n.checked_mul(delta).ok_or("the video is too long")?;
    let data_len: usize = e.samples.iter().map(|s| s.0.len()).sum();
    let data_len = u32::try_from(data_len).ok().filter(|l| *l < u32::MAX - 8).ok_or("the video is too big")?;
    let ftyp = bx(b"ftyp", &W(Vec::new()).bytes(b"isom").u32(0x200).bytes(b"isomiso6av01mp41").0);
    // moov's size doesn't depend on the chunk offset, so build it once to measure, then again.
    let moov_at = |offset: u32| moov(e, w, h, delta, n, duration, offset);
    let offset = (ftyp.len() + moov_at(0).len() + 8) as u32;
    let moov = moov_at(offset);
    let mut out = Vec::with_capacity(offset as usize + data_len as usize);
    out.extend_from_slice(&ftyp);
    out.extend_from_slice(&moov);
    out.extend_from_slice(&(data_len + 8).to_be_bytes());
    out.extend_from_slice(b"mdat");
    for (s, _) in &e.samples {
        out.extend_from_slice(s);
    }
    Ok(out)
}

fn moov(e: &Encoded, w: u32, h: u32, delta: u32, n: u32, duration: u32, offset: u32) -> Vec<u8> {
    let mvhd =
        full(b"mvhd", 0, 0, &W(Vec::new()).u32(0).u32(0).u32(TIMESCALE).u32(duration).u32(0x0001_0000).u16(0x0100).zeros(10).matrix().zeros(24).u32(2).0);
    let tkhd = full(
        b"tkhd",
        0,
        0x3,
        &W(Vec::new()).u32(0).u32(0).u32(1).u32(0).u32(duration).zeros(8).u16(0).u16(0).u16(0).u16(0).matrix().u32(w << 16).u32(h << 16).0,
    );
    let mdhd = full(b"mdhd", 0, 0, &W(Vec::new()).u32(0).u32(0).u32(TIMESCALE).u32(duration).u16(0x55c4).u16(0).0);
    let hdlr = full(b"hdlr", 0, 0, &W(Vec::new()).u32(0).bytes(b"vide").zeros(12).bytes(b"WobbleWorks video\0").0);
    let vmhd = full(b"vmhd", 0, 1, &[0; 8]);
    let dref = full(b"dref", 0, 0, &W(Vec::new()).u32(1).bytes(&full(b"url ", 0, 1, &[])).0);
    let dinf = bx(b"dinf", &dref);
    let mut name = [0u8; 32];
    let label = b"AV1 (WobbleWorks)";
    name[0] = label.len() as u8;
    name[1..=label.len()].copy_from_slice(label);
    let av01 = bx(
        b"av01",
        &W(Vec::new())
            .zeros(6)
            .u16(1)
            .zeros(16)
            .u16(w as u16)
            .u16(h as u16)
            .u32(0x0048_0000)
            .u32(0x0048_0000)
            .u32(0)
            .u16(1)
            .bytes(&name)
            .u16(0x0018)
            .u16(0xffff)
            .bytes(&bx(b"av1C", &e.config))
            .0,
    );
    let stsd = full(b"stsd", 0, 0, &W(Vec::new()).u32(1).bytes(&av01).0);
    let stts = full(b"stts", 0, 0, &W(Vec::new()).u32(1).u32(n).u32(delta).0);
    let keys: Vec<u32> = e.samples.iter().enumerate().filter(|(_, s)| s.1).map(|(i, _)| i as u32 + 1).collect();
    let mut stss = W(Vec::new());
    stss.u32(keys.len() as u32);
    for k in &keys {
        stss.u32(*k);
    }
    let stss = full(b"stss", 0, 0, &stss.0);
    let stsc = full(b"stsc", 0, 0, &W(Vec::new()).u32(1).u32(1).u32(n).u32(1).0);
    let mut stsz = W(Vec::new());
    stsz.u32(0).u32(n);
    for (s, _) in &e.samples {
        stsz.u32(s.len() as u32);
    }
    let stsz = full(b"stsz", 0, 0, &stsz.0);
    let stco = full(b"stco", 0, 0, &W(Vec::new()).u32(1).u32(offset).0);
    let stbl = bx(b"stbl", &[stsd, stts, stss, stsc, stsz, stco].concat());
    let minf = bx(b"minf", &[vmhd, dinf, stbl].concat());
    let mdia = bx(b"mdia", &[mdhd, hdlr, minf].concat());
    let trak = bx(b"trak", &[tkhd, mdia].concat());
    bx(b"moov", &[mvhd, trak].concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32, t: u8) -> Picture {
        let rgba = (0..w * h).flat_map(|i| [((i % w) * 255 / w) as u8, t.wrapping_mul(40), 200, 255]).collect();
        Picture { width: w, height: h, rgba }
    }

    /// Walk the boxes at one level: (type, contents).
    fn boxes(b: &[u8]) -> Vec<([u8; 4], &[u8])> {
        let mut out = Vec::new();
        let mut at = 0;
        while at + 8 <= b.len() {
            let size = u32::from_be_bytes(b[at..at + 4].try_into().unwrap()) as usize;
            let kind: [u8; 4] = b[at + 4..at + 8].try_into().unwrap();
            assert!(size >= 8 && at + size <= b.len(), "box {:?} overruns", std::str::from_utf8(&kind));
            out.push((kind, &b[at + 8..at + size]));
            at += size;
        }
        assert_eq!(at, b.len(), "boxes fill their parent exactly");
        out
    }

    fn find<'a>(b: &'a [u8], path: &[&[u8; 4]]) -> &'a [u8] {
        let mut cur = b;
        for k in path {
            cur = boxes(cur).into_iter().find(|(t, _)| t == *k).unwrap_or_else(|| panic!("no {:?}", std::str::from_utf8(*k))).1;
        }
        cur
    }

    #[test]
    fn frames_become_a_well_formed_av1_mp4() {
        let frames: Vec<Picture> = (0..6).map(|t| frame(66, 41, t)).collect();
        let mp4 = encode(&frames, 12.0).expect("mp4");
        let top = boxes(&mp4);
        assert_eq!(top.iter().map(|b| b.0).collect::<Vec<_>>(), vec![*b"ftyp", *b"moov", *b"mdat"]);
        let stbl = find(&mp4, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]);
        // Six samples, 1/12 s apart, the first a key frame.
        let stsz = find(stbl, &[b"stsz"]);
        assert_eq!(u32::from_be_bytes(stsz[8..12].try_into().unwrap()), 6);
        let stts = find(stbl, &[b"stts"]);
        assert_eq!(u32::from_be_bytes(stts[12..16].try_into().unwrap()), 2500);
        let stss = find(stbl, &[b"stss"]);
        assert_eq!(u32::from_be_bytes(stss[8..12].try_into().unwrap()), 1);
        // The sample sizes add up to mdat, which the chunk offset points at.
        let sizes: u32 = stsz[12..].chunks(4).map(|c| u32::from_be_bytes(c.try_into().unwrap())).sum();
        let mdat = top[2].1;
        assert_eq!(sizes as usize, mdat.len());
        let stco = find(stbl, &[b"stco"]);
        let offset = u32::from_be_bytes(stco[8..12].try_into().unwrap()) as usize;
        assert_eq!(&mp4[offset - 4..offset], b"mdat");
        // 66 × 41 is encoded as 66 × 40 (even sides); av1C starts with its marker and version
        // and carries the sequence header OBU.
        let stsd = find(stbl, &[b"stsd"]);
        let av01 = &stsd[8 + 8..];
        assert_eq!(&stsd[12..16], b"av01");
        assert_eq!(u16::from_be_bytes(av01[24..26].try_into().unwrap()), 66);
        assert_eq!(u16::from_be_bytes(av01[26..28].try_into().unwrap()), 40);
        let av1c = find(&av01[78..], &[b"av1C"]);
        assert_eq!(av1c[0], 0x81);
        assert_eq!((av1c[4] >> 3) & 0x0f, OBU_SEQUENCE_HEADER);
        // Samples hold no temporal delimiters; the first starts with a sequence header.
        assert_eq!((mdat[0] >> 3) & 0x0f, OBU_SEQUENCE_HEADER);
    }

    #[test]
    fn bad_input_is_an_error() {
        assert!(encode(&[], 12.0).is_err());
        assert!(encode(&[frame(8, 8, 0)], 12.0).is_err(), "too small");
        assert!(encode(&[frame(32, 32, 0), frame(34, 32, 0)], 12.0).is_err(), "sizes differ");
        let broken = Picture { width: 32, height: 32, rgba: vec![0; 10] };
        assert!(encode(&[broken], 12.0).is_err());
        assert!(encode(&[frame(32, 32, 0)], f32::NAN).is_ok(), "a bad rate falls back");
    }

    #[test]
    fn obu_parsing_survives_garbage() {
        for junk in [&[][..], &[0xff], &[0x12, 0x05, 1], &[0x0a, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]] {
            let _ = obus(junk);
            let _ = strip_delimiters(junk);
        }
        // A temporal delimiter (type 2, size 0) then a padding OBU (type 15, 1 byte).
        assert_eq!(strip_delimiters(&[0x12, 0x00, 0x7a, 0x01, 0xaa]), vec![0x7a, 0x01, 0xaa]);
    }
}
