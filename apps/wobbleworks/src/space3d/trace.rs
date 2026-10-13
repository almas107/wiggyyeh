//! Tracing the 3D note in 2D (Clip Studio Paint's 3D layer as a reference rig; design in
//! `docs/wobbleworks-3d-trace.md`).
//!
//! "Shoot" renders the note through a camera at the picture's size into a pixel layer of the
//! PhotoCraft document, and adds an empty layer above it to trace on. The layer remembers its
//! camera and how it is linked to the 3D view, so it can be shot again after the note changes:
//!
//! - **Pinned**: its own camera; orbiting in 3D never touches it (several angles of one note).
//! - **Live**: follows the 3D view's camera; refreshed when you go back to 2D.
//! - **Shot**: follows a saved camera shot of the note's Sequence.
//!
//! The camera travels as plain numbers in a private block of the layer (`wb3T`, JSON), so
//! PhotoCraft's document model learns nothing about 3D, `.pcraft` and PSD keep it (unknown
//! blocks round-trip verbatim) and other apps just see the picture. Tracing only reads the
//! note and puts the 3D camera back: the 3D canvas is never changed by it.

use std::sync::Arc;

use photocraft_doc::{Document, Layer, LayerContent, LayerId, PixelFormat, Rect};
use photocraft_engine::Session;
use photocraft_raster::Surface;
use serde_json::{Value, json};
use wobbleworks_3d::camera::{Camera, Viewport};
use wobbleworks_3d::editor::Editor;
use wobbleworks_3d::math::v3;
use wobbleworks_3d::raster::{MAX_SIDE, Picture};

/// The layer block that marks a 3D trace layer and holds its camera.
pub const TRACE_KEY: [u8; 4] = *b"wb3T";
/// A trace layer's opacity when it is made: dim enough to draw over.
pub const TRACE_OPACITY: f32 = 0.5;

/// How a trace layer's camera is bound to the 3D view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraLink {
    /// Follows the 3D view's camera.
    Live,
    /// Frozen: the layer's own camera.
    Pinned,
    /// Follows the saved shot with this id.
    Shot(u64),
}

/// How the 3D view's shape is fitted to the picture's shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fit {
    /// Keep the vertical field of view: the picture's width decides how much is seen sideways.
    #[default]
    Height,
    /// Keep the horizontal field of view.
    Width,
    /// Keep everything the 3D view showed (whichever of the two shows more).
    Contain,
}

impl Fit {
    fn name(self) -> &'static str {
        match self {
            Fit::Height => "height",
            Fit::Width => "width",
            Fit::Contain => "contain",
        }
    }

    pub fn parse(s: &str) -> Option<Fit> {
        match s {
            "height" => Some(Fit::Height),
            "width" => Some(Fit::Width),
            "contain" => Some(Fit::Contain),
            _ => None,
        }
    }
}

/// A camera in plain numbers, as a trace layer keeps it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceCamera {
    pub target: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub focal_mm: f32,
    pub orthographic: bool,
    /// Width / height of the 3D view it was shot from (for [`Fit`]).
    pub aspect: f32,
    pub link: CameraLink,
    pub fit: Fit,
}

impl TraceCamera {
    pub fn from_camera(c: &Camera, link: CameraLink, fit: Fit) -> TraceCamera {
        let vp = c.viewport.sane();
        TraceCamera {
            target: [c.target.x, c.target.y, c.target.z],
            yaw: c.yaw,
            pitch: c.pitch,
            distance: c.distance,
            focal_mm: c.focal_mm,
            orthographic: c.orthographic,
            aspect: vp.width / vp.height,
            link,
            fit,
        }
    }

    /// The camera for a `width` × `height` picture, with the lens fitted (see [`Fit`]).
    pub fn camera(&self, width: u32, height: u32) -> Camera {
        let mut c = Camera {
            target: v3(self.target[0], self.target[1], self.target[2]),
            yaw: self.yaw,
            pitch: self.pitch,
            distance: self.distance,
            focal_mm: self.focal_mm,
            orthographic: self.orthographic,
            snapped_from_perspective: false,
            viewport: Viewport { width: width.max(1) as f32, height: height.max(1) as f32 },
        };
        c.sanitize();
        let picture = c.viewport.width / c.viewport.height;
        let shot = if self.aspect.is_finite() && self.aspect > 0.0 { self.aspect } else { picture };
        // tan(fov_y / 2) is proportional to 1 / focal, so keeping the horizontal field of view
        // on a narrower picture means a shorter lens by picture / shot.
        let keep_width = match self.fit {
            Fit::Height => false,
            Fit::Width => true,
            Fit::Contain => picture < shot,
        };
        if keep_width {
            c.focal_mm *= picture / shot;
            c.sanitize();
        }
        c
    }

    pub fn to_json(&self) -> Value {
        let link = match self.link {
            CameraLink::Live => json!("live"),
            CameraLink::Pinned => json!("pinned"),
            CameraLink::Shot(id) => json!({"shot": id}),
        };
        json!({
            "v": 1,
            "target": self.target,
            "yaw": self.yaw,
            "pitch": self.pitch,
            "distance": self.distance,
            "focal": self.focal_mm,
            "orthographic": self.orthographic,
            "aspect": self.aspect,
            "link": link,
            "fit": self.fit.name(),
        })
    }

    /// Read [`Self::to_json`]; None when it isn't a trace camera. Numbers are sanitised when
    /// the camera is made, so hostile values can't wreck a render.
    pub fn from_json(v: &Value) -> Option<TraceCamera> {
        let num = |k: &str| v.get(k).and_then(Value::as_f64).map(|x| x as f32).filter(|x| x.is_finite());
        let t = v.get("target").and_then(Value::as_array)?;
        let tn = |i: usize| t.get(i).and_then(Value::as_f64).map(|x| x as f32).filter(|x| x.is_finite());
        let link = match v.get("link") {
            Some(Value::String(s)) if s == "live" => CameraLink::Live,
            Some(Value::Object(o)) => CameraLink::Shot(o.get("shot").and_then(Value::as_u64)?),
            _ => CameraLink::Pinned,
        };
        Some(TraceCamera {
            target: [tn(0)?, tn(1)?, tn(2)?],
            yaw: num("yaw")?,
            pitch: num("pitch")?,
            distance: num("distance")?,
            focal_mm: num("focal")?,
            orthographic: v.get("orthographic").and_then(Value::as_bool).unwrap_or(false),
            aspect: num("aspect").unwrap_or(0.0),
            link,
            fit: v.get("fit").and_then(Value::as_str).and_then(Fit::parse).unwrap_or_default(),
        })
    }

    /// The camera numbers to render with now: a live layer takes the 3D view's camera, a shot
    /// layer its shot's (None when the shot was deleted: the layer keeps its last picture).
    pub fn resolve(&self, ed: &Editor) -> Option<TraceCamera> {
        match self.link {
            CameraLink::Pinned => Some(*self),
            CameraLink::Live => Some(TraceCamera::from_camera(&ed.camera, self.link, self.fit)),
            CameraLink::Shot(id) => {
                let shot = ed.scene.sequence.shots.iter().find(|s| s.id == id)?;
                let mut cam = shot.camera;
                // A shot keeps the viewport it was taken in; its framing is the 3D view's.
                cam.viewport = ed.camera.viewport;
                Some(TraceCamera::from_camera(&cam, self.link, self.fit))
            }
        }
    }
}

/// The trace camera a layer carries, if it is a trace layer.
pub fn layer_camera(layer: &Layer) -> Option<TraceCamera> {
    let (_, data) = layer.psd_blocks.iter().find(|(k, _)| *k == TRACE_KEY)?;
    let v: Value = serde_json::from_slice(data).ok()?;
    TraceCamera::from_json(&v)
}

fn set_layer_camera(layer: &mut Layer, tc: &TraceCamera) {
    let data = Arc::new(tc.to_json().to_string().into_bytes());
    match layer.psd_blocks.iter_mut().find(|(k, _)| *k == TRACE_KEY) {
        Some(slot) => slot.1 = data,
        None => layer.psd_blocks.push((TRACE_KEY, data)),
    }
}

/// The note seen through `tc` at `width` × `height`, at boil frame 0, transparent where
/// nothing is drawn. Sides over [`MAX_SIDE`] are rendered at that size and scaled up.
pub fn render(ed: &mut Editor, tc: &TraceCamera, width: u32, height: u32) -> Result<Picture, String> {
    if width == 0 || height == 0 {
        return Err("the picture is empty".into());
    }
    let longest = width.max(height);
    let (rw, rh) = if longest > MAX_SIDE {
        let k = MAX_SIDE as f64 / longest as f64;
        (((width as f64 * k).round() as u32).clamp(1, MAX_SIDE), ((height as f64 * k).round() as u32).clamp(1, MAX_SIDE))
    } else {
        (width, height)
    };
    let cam = tc.camera(rw, rh);
    let pic = super::export::picture_at(ed, &cam, 0, rw, rh, true)?;
    if (rw, rh) == (width, height) {
        return Ok(pic);
    }
    let img = image::RgbaImage::from_raw(pic.width, pic.height, pic.rgba).ok_or("the 3D picture came out the wrong size")?;
    let big = image::imageops::resize(&img, width, height, image::imageops::FilterType::Triangle);
    Ok(Picture { width, height, rgba: big.into_raw() })
}

/// A picture as a layer surface in the document's pixel format (straight RGBA8 in, as
/// PhotoCraft's surfaces are straight alpha too).
pub fn to_surface(pic: &Picture, format: PixelFormat) -> Result<Surface, String> {
    let need = (pic.width as usize).checked_mul(pic.height as usize).and_then(|n| n.checked_mul(4)).ok_or("the picture is too big")?;
    if pic.rgba.len() != need || pic.width == 0 || pic.height == 0 {
        return Err("the 3D picture came out the wrong size".into());
    }
    let (w, h) = (i32::try_from(pic.width).map_err(|_| "too wide")?, i32::try_from(pic.height).map_err(|_| "too tall")?);
    let s = Surface::from_interleaved(PixelFormat::RGBA8, Rect::new(0, 0, w, h), &pic.rgba);
    Ok(if format == PixelFormat::RGBA8 { s } else { s.convert(format) })
}

fn doc_size(doc: &Document) -> (u32, u32) {
    (doc.size.width, doc.size.height)
}

fn link_name(link: CameraLink) -> &'static str {
    match link {
        CameraLink::Live => "live",
        CameraLink::Pinned => "pinned",
        CameraLink::Shot(_) => "shot",
    }
}

/// Shoot: render the 3D view's camera (or `link`'s) into a new trace layer above the active
/// layer, dimmed, with an empty layer above it to trace on (made active). One undo step.
pub fn shoot(session: &mut Session, ed: &mut Editor, link: CameraLink, fit: Fit) -> Result<Value, String> {
    let st = session.active().ok_or("there is no picture to trace into: make one first")?;
    let (w, h) = doc_size(&st.doc);
    let format = st.doc.pixel_format();
    let tc = TraceCamera::from_camera(&ed.camera, link, fit);
    let tc = tc.resolve(ed).ok_or("that camera shot doesn't exist")?;
    let pic = render(ed, &tc, w, h)?;
    let surface = to_surface(&pic, format)?;
    let name = match link {
        CameraLink::Shot(id) => {
            let n = ed.scene.sequence.shots.iter().find(|s| s.id == id).map_or_else(|| "shot".to_string(), |s| s.name.clone());
            format!("3D trace ({n})")
        }
        _ => format!("3D trace ({})", link_name(link)),
    };
    let (trace, above) = session
        .edit("Shoot 3D", |doc, active| {
            let mut layer = Layer::new(name, LayerContent::Raster(surface));
            layer.opacity = TRACE_OPACITY;
            layer.locks.pixels = true;
            set_layer_camera(&mut layer, &tc);
            let trace = doc.insert_above(*active, layer);
            let above = doc.insert_above(Some(trace), Layer::new("Trace", LayerContent::Raster(Surface::new(format))));
            *active = Some(above);
            Ok((trace, above))
        })
        .map_err(|e| e.to_string())?;
    Ok(json!({"layer": trace.0, "traceOn": above.0, "link": link_name(link), "size": [w, h]}))
}

/// Shoot the trace layers again: every one (`only = None`) or one layer. Pinned layers keep
/// their camera (only the note may have changed), live ones take the 3D view's, shot ones
/// their shot's. One undo step; returns how many were shot.
pub fn refresh(session: &mut Session, ed: &mut Editor, only: Option<LayerId>) -> Result<usize, String> {
    let st = session.active().ok_or("there is no picture")?;
    let (w, h) = doc_size(&st.doc);
    let format = st.doc.pixel_format();
    let mut todo: Vec<(LayerId, TraceCamera, Surface)> = Vec::new();
    for (_, _, layer) in st.doc.walk() {
        if only.is_some_and(|id| id != layer.id) {
            continue;
        }
        let Some(tc) = layer_camera(layer) else { continue };
        let Some(now) = tc.resolve(ed) else { continue };
        let pic = render(ed, &now, w, h)?;
        todo.push((layer.id, now, to_surface(&pic, format)?));
    }
    if todo.is_empty() {
        return match only {
            Some(_) => Err("that layer isn't a 3D trace layer".into()),
            None => Ok(0),
        };
    }
    let n = todo.len();
    session
        .edit("Update 3D trace", |doc, _| {
            for (id, tc, surface) in todo {
                if let Some(layer) = doc.layer_mut(id) {
                    layer.content = LayerContent::Raster(surface);
                    set_layer_camera(layer, &tc);
                }
            }
            Ok(())
        })
        .map_err(|e| e.to_string())?;
    Ok(n)
}

/// Whether the active picture has trace layers that follow the 3D view or its shots (those
/// go stale when the camera or the note changes; pinned ones only when the note does).
pub fn has_following(session: &Session) -> bool {
    session.active().is_some_and(|st| st.doc.walk().iter().any(|(_, _, l)| layer_camera(l).is_some_and(|tc| tc.link != CameraLink::Pinned)))
}

/// Whether the active picture has any trace layers.
pub fn has_traces(session: &Session) -> bool {
    session.active().is_some_and(|st| st.doc.walk().iter().any(|(_, _, l)| layer_camera(l).is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{ColorMode, SampleType, Size};

    fn note() -> Editor {
        let mut ed = Editor::new();
        ed.set_viewport(640.0, 400.0);
        ed.stable = 0.0;
        let line: Vec<[f32; 2]> = (0..40).map(|i| [200.0 + i as f32 * 6.0, 200.0]).collect();
        ed.run("stroke.draw", &json!({"points": line})).expect("draw");
        ed
    }

    fn session(w: u32, h: u32) -> Session {
        let mut s = Session::new();
        s.add_document(Document::new("t", Size::new(w, h), ColorMode::Rgb, SampleType::U8), None);
        s
    }

    #[test]
    fn tracing_never_touches_the_note() {
        let mut ed = note();
        let before = (ed.scene.clone(), ed.camera, ed.scene_revision);
        let tc = TraceCamera::from_camera(&ed.camera, CameraLink::Pinned, Fit::Height);
        let pic = render(&mut ed, &tc, 300, 420).expect("render");
        assert_eq!((pic.width, pic.height), (300, 420));
        assert!(pic.rgba.chunks_exact(4).any(|p| p[3] > 0), "the stroke shows");
        assert!(ed.scene == before.0, "the note is unchanged");
        assert_eq!(ed.camera, before.1, "the 3D view's camera is put back");
        assert_eq!(ed.scene_revision, before.2);
    }

    #[test]
    fn a_known_camera_at_two_picture_sizes() {
        let mut ed = note();
        let tc = TraceCamera::from_camera(&ed.camera, CameraLink::Pinned, Fit::Height);
        for (w, h) in [(320, 200), (200, 320)] {
            let pic = render(&mut ed, &tc, w, h).expect("render");
            assert_eq!((pic.width, pic.height, pic.rgba.len()), (w, h, (w * h * 4) as usize));
        }
        assert!(render(&mut ed, &tc, 0, 10).is_err());
    }

    #[test]
    fn fit_width_keeps_the_horizontal_view_on_a_tall_picture() {
        let c = Camera { viewport: Viewport { width: 1600.0, height: 800.0 }, ..Camera::default() };
        let tc = TraceCamera::from_camera(&c, CameraLink::Pinned, Fit::Width);
        let tall = tc.camera(800, 1600);
        // Horizontal half-extent ∝ aspect × tan(fov_y / 2) ∝ aspect / focal.
        let wide = 2.0 / c.focal_mm;
        let fitted = 0.5 / tall.focal_mm;
        assert!((wide - fitted).abs() / wide < 1e-4, "{wide} vs {fitted}");
        assert_eq!(TraceCamera { fit: Fit::Height, ..tc }.camera(800, 1600).focal_mm, c.focal_mm);
        // Contain on a wider picture keeps the height.
        assert_eq!(TraceCamera { fit: Fit::Contain, ..tc }.camera(4000, 1000).focal_mm, c.focal_mm);
    }

    #[test]
    fn the_camera_round_trips_and_hostile_blocks_are_ignored() {
        let c = Camera { yaw: 91.0, orthographic: true, ..Camera::default() };
        for link in [CameraLink::Live, CameraLink::Pinned, CameraLink::Shot(7)] {
            let tc = TraceCamera::from_camera(&c, link, Fit::Contain);
            assert_eq!(TraceCamera::from_json(&tc.to_json()), Some(tc));
        }
        for bad in [json!(null), json!({"target": [1, 2]}), json!({"target": [0, 0, 0], "yaw": "x"}), json!([1, 2, 3])] {
            assert!(TraceCamera::from_json(&bad).is_none(), "{bad}");
        }
        let mut layer = Layer::new("x", LayerContent::Raster(Surface::new(PixelFormat::RGBA8)));
        layer.psd_blocks.push((TRACE_KEY, Arc::new(b"{not json".to_vec())));
        assert!(layer_camera(&layer).is_none());
        // Absurd numbers still give a usable camera.
        let wild = TraceCamera { distance: 1e30, focal_mm: -4.0, aspect: f32::NAN, ..TraceCamera::from_camera(&c, CameraLink::Pinned, Fit::Width) };
        let cam = wild.camera(100, 100);
        assert!(cam.distance.is_finite() && cam.focal_mm >= 10.0);
    }

    #[test]
    fn shoot_adds_a_dimmed_trace_layer_and_a_layer_to_trace_on() {
        let mut ed = note();
        let mut s = session(256, 160);
        let r = shoot(&mut s, &mut ed, CameraLink::Pinned, Fit::Height).expect("shoot");
        let st = s.active().expect("doc");
        let trace = st.doc.layer(LayerId(r["layer"].as_u64().expect("id"))).expect("trace layer");
        assert_eq!(trace.opacity, TRACE_OPACITY);
        assert!(trace.locks.pixels);
        assert_eq!(layer_camera(trace).map(|t| t.link), Some(CameraLink::Pinned));
        let LayerContent::Raster(px) = &trace.content else { panic!("pixels") };
        let opaque = (0..160).flat_map(|y| (0..256).map(move |x| (x, y))).filter(|&(x, y)| px.rgba(x, y)[3] > 0.0).count();
        assert!(opaque > 20, "the stroke is in the layer: {opaque}");
        assert_eq!(st.active_layer.map(|l| l.0), r["traceOn"].as_u64());
        // One undo step takes both layers away.
        let n = st.doc.walk().len();
        assert!(s.undo());
        assert_eq!(s.active().expect("doc").doc.walk().len(), n - 2);
    }

    #[test]
    fn pinned_layers_hold_their_angle_and_live_ones_follow() {
        let mut ed = note();
        let mut s = session(200, 120);
        let pinned = shoot(&mut s, &mut ed, CameraLink::Pinned, Fit::Height).expect("pinned")["layer"].as_u64().expect("id");
        let live = shoot(&mut s, &mut ed, CameraLink::Live, Fit::Height).expect("live")["layer"].as_u64().expect("id");
        assert!(has_following(&s));
        let yaw0 = ed.camera.yaw;
        ed.run("camera.orbit", &json!({"dx": -100.0})).expect("orbit");
        assert_eq!(refresh(&mut s, &mut ed, None).expect("refresh"), 2);
        let doc = &s.active().expect("doc").doc;
        let cam = |id: u64| doc.layer(LayerId(id)).and_then(layer_camera).expect("camera");
        assert_eq!(cam(pinned).yaw, yaw0);
        assert_eq!(cam(live).yaw, ed.camera.yaw);
        assert_ne!(cam(live).yaw, yaw0);
        // A shot that was deleted leaves its layer alone; a plain layer can't be refreshed.
        assert!(shoot(&mut s, &mut ed, CameraLink::Shot(999), Fit::Height).is_err());
        let plain = s.active().expect("doc").active_layer.expect("trace-on layer");
        assert!(refresh(&mut s, &mut ed, Some(plain)).is_err());
    }
}
