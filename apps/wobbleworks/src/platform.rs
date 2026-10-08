//! The few things that differ between desktop and browser: clocks, file pickers, saving files
//! and dropped files. Opened files arrive through an [`Inbox`] because the browser reads them
//! asynchronously.

use std::sync::{Arc, Mutex, PoisonError};

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64() * 1000.0)
    }
}

static PEN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The latest pen pressure (0..=1) if a pen is in use. Browsers report it through Pointer
/// Events, which eframe doesn't forward, so the web build listens for them itself.
pub fn pen_pressure() -> Option<f32> {
    let v = f32::from_bits(PEN.load(std::sync::atomic::Ordering::Relaxed));
    (v > 0.0 && v.is_finite()).then_some(v.min(1.0))
}

#[cfg(target_arch = "wasm32")]
pub fn listen_pen(target: &web_sys::HtmlCanvasElement) {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::closure::Closure;
    for kind in ["pointerdown", "pointermove", "pointerup", "pointercancel", "pointerleave"] {
        let cb = Closure::<dyn FnMut(web_sys::PointerEvent)>::new(move |e: web_sys::PointerEvent| {
            let p = if e.pointer_type() == "pen" && kind != "pointerleave" { e.pressure() } else { 0.0 };
            PEN.store(p.to_bits(), std::sync::atomic::Ordering::Relaxed);
        });
        let _ = target.add_event_listener_with_callback(kind, cb.as_ref().unchecked_ref());
        // The canvas lives as long as the page.
        cb.forget();
    }
}

/// What an opened file is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// A picture to float over the canvas.
    Image,
    /// A `.wob` project.
    Project,
    /// Decide by the file name (drag and drop).
    Any,
}

pub struct Incoming {
    pub name: String,
    pub bytes: Vec<u8>,
    pub purpose: Purpose,
}

/// Files that have finished loading, waiting for the next frame.
#[derive(Clone, Default)]
pub struct Inbox(Arc<Mutex<Vec<Result<Incoming, String>>>>);

impl Inbox {
    pub fn push(&self, item: Result<Incoming, String>) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).push(item);
    }

    pub fn take(&self) -> Vec<Result<Incoming, String>> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// Largest file we read from a picker or a drop.
const MAX_READ: usize = 256 * 1024 * 1024;

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];
const PROJECT_EXTS: &[&str] = &["wob", "json"];

fn filter(purpose: Purpose) -> (&'static str, &'static [&'static str]) {
    match purpose {
        Purpose::Image => ("Pictures", IMAGE_EXTS),
        Purpose::Project | Purpose::Any => ("WobbleWorks project", PROJECT_EXTS),
    }
}

/// Ask for a file to open; it arrives in `inbox`.
pub fn pick_file(inbox: &Inbox, ctx: &egui::Context, purpose: Purpose) {
    let (desc, exts) = filter(purpose);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = ctx;
        if let Some(path) = rfd::FileDialog::new().add_filter(desc, exts).pick_file() {
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let res = match std::fs::metadata(&path) {
                Ok(m) if m.len() > MAX_READ as u64 => Err(format!("{name} is too big to open")),
                _ => std::fs::read(&path).map(|bytes| Incoming { name: name.clone(), bytes, purpose }).map_err(|e| format!("couldn't read {name}: {e}")),
            };
            inbox.push(res);
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let inbox = inbox.clone();
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(h) = rfd::AsyncFileDialog::new().add_filter(desc, exts).pick_file().await {
                let name = h.file_name();
                let bytes = h.read().await;
                inbox.push(if bytes.len() > MAX_READ { Err(format!("{name} is too big to open")) } else { Ok(Incoming { name, bytes, purpose }) });
                ctx.request_repaint();
            }
        });
    }
}

/// Read files dropped on the window into `inbox`.
pub fn take_dropped(ctx: &egui::Context, inbox: &Inbox) {
    let files = ctx.input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
    for f in files {
        let name = f.path().file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "dropped file".into());
        #[cfg(not(target_arch = "wasm32"))]
        {
            inbox.push(f.bytes().map(|bytes| Incoming { name: name.clone(), bytes, purpose: Purpose::Any }).map_err(|e| format!("couldn't read {name}: {e}")));
        }
        #[cfg(target_arch = "wasm32")]
        {
            let inbox = inbox.clone();
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let res = f
                    .bytes_async()
                    .await
                    .map(|bytes| Incoming { name: name.clone(), bytes, purpose: Purpose::Any })
                    .map_err(|e| format!("couldn't read {name}: {e}"));
                inbox.push(res);
                ctx.request_repaint();
            });
        }
    }
}

/// Save `bytes` as a file the user picks (desktop) or as a download (browser). `Ok(None)` when
/// the user cancelled.
pub fn save_file(name: &str, bytes: &[u8]) -> Result<Option<String>, String> {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(path) = rfd::FileDialog::new().set_file_name(name).add_filter(ext.to_uppercase(), &[ext.as_str()]).save_file() else {
            return Ok(None);
        };
        std::fs::write(&path, bytes).map_err(|e| format!("couldn't save {}: {e}", path.display()))?;
        Ok(Some(path.display().to_string()))
    }
    #[cfg(target_arch = "wasm32")]
    {
        download(name, bytes, mime(&ext))?;
        Ok(Some(name.to_string()))
    }
}

#[cfg(target_arch = "wasm32")]
fn mime(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "gif" => "image/gif",
        "wob" | "json" => "application/json",
        _ => "application/octet-stream",
    }
}

#[cfg(target_arch = "wasm32")]
fn download(name: &str, bytes: &[u8], mime: &str) -> Result<(), String> {
    use wasm_bindgen::JsCast;
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(mime);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;
    let a: web_sys::HtmlAnchorElement = document.create_element("a").map_err(js)?.dyn_into().map_err(|_| "not an anchor")?;
    a.set_href(&url);
    a.set_download(name);
    a.style().set_property("display", "none").map_err(js)?;
    let body = document.body().ok_or("no body")?;
    body.append_child(&a).map_err(js)?;
    a.click();
    a.remove();
    let revoke = wasm_bindgen::closure::Closure::once_into_js(move || {
        let _ = web_sys::Url::revoke_object_url(&url);
    });
    window.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 10_000).map_err(js)?;
    Ok(())
}

/// Guess what a dropped file is from its name and first bytes.
pub fn classify(name: &str, bytes: &[u8]) -> Purpose {
    let lower = name.to_ascii_lowercase();
    if PROJECT_EXTS.iter().any(|e| lower.ends_with(&format!(".{e}"))) || bytes.first() == Some(&b'{') { Purpose::Project } else { Purpose::Image }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_by_name_and_content() {
        assert_eq!(classify("doodle.WOB", b""), Purpose::Project);
        assert_eq!(classify("x.bin", b"{\"format\""), Purpose::Project);
        assert_eq!(classify("cat.png", b"\x89PNG"), Purpose::Image);
    }

    #[test]
    fn inbox_hands_items_over_once() {
        let i = Inbox::default();
        i.push(Err("x".into()));
        assert_eq!(i.take().len(), 1);
        assert!(i.take().is_empty());
        assert!(now_ms() > 1.0e12);
    }
}
