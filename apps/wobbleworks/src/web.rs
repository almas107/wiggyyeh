//! The browser shell: file picker and drops through an inbox, saving as downloads.

use std::sync::{Arc, Mutex};

use photocraft_ui_egui::{PhotocraftApp, Services};
use wasm_bindgen::JsCast as _;
use wobbleworks::WobbleApp;
use wobbleworks::io::{OPEN_EXTS, codec_services};

type Inbox = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

const CANVAS_ID: &str = "wobbleworks_canvas";

pub fn start() {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            log::error!("no document");
            return;
        };
        let Some(canvas) = document.get_element_by_id(CANVAS_ID).and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok()) else {
            log::error!("missing <canvas id=\"{CANVAS_ID}\">");
            return;
        };
        let mut options = eframe::WebOptions::default();
        photocraft_ui_egui::gpu_canvas::use_adapter_limits(&mut options.wgpu_options.wgpu_setup);
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(move |cc| {
                    PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                    let inbox: Inbox = Arc::default();
                    let mut w = WobbleApp::new(services(inbox.clone(), cc.egui_ctx.clone()));
                    w.restore(cc.storage);
                    w.audio.out = Some(Box::new(WebAudioOut::new()));
                    let repaint = cc.egui_ctx.clone();
                    w.picker.pick_reference = Some(Box::new(move |inbox| {
                        let ctx = repaint.clone();
                        wasm_bindgen_futures::spawn_local(async move {
                            let Some(file) = rfd::AsyncFileDialog::new().add_filter("Images", OPEN_EXTS).pick_file().await else { return };
                            let bytes = file.read().await;
                            *inbox.lock().unwrap_or_else(|e| e.into_inner()) = Some((file.file_name(), bytes));
                            ctx.request_repaint();
                        });
                    }));
                    let repaint = cc.egui_ctx.clone();
                    w.space.files = wobbleworks::space3d::Files {
                        pick: Some(Box::new(move |inbox, purpose, exts| {
                            let ctx = repaint.clone();
                            wasm_bindgen_futures::spawn_local(async move {
                                let Some(file) = rfd::AsyncFileDialog::new().add_filter("Files", exts).pick_file().await else { return };
                                let bytes = file.read().await;
                                inbox.lock().unwrap_or_else(|e| e.into_inner()).push((purpose.to_string(), file.file_name(), bytes));
                                ctx.request_repaint();
                            });
                        })),
                        // No save dialog on the web: the file downloads under the suggested name.
                        save: Some(Box::new(|suggested: &str, bytes: &[u8]| download(suggested, bytes).map(|()| Some(suggested.to_string())))),
                    };
                    if let Some(rs) = cc.wgpu_render_state.clone() {
                        w.space.set_gpu(&rs);
                        w.app.set_wgpu(rs);
                    }
                    Ok(Box::new(WebShell { w, inbox }))
                }),
            )
            .await;
        if let Some(el) = document.get_element_by_id("wobbleworks_loading") {
            match result {
                Ok(()) => el.remove(),
                Err(e) => el.set_inner_html(&format!("<p>WobbleWorks failed to start: {e:?}</p><p>A browser with WebGPU or WebGL2 is required.</p>")),
            }
        }
    });
}

/// Reads dropped files asynchronously (browsers can't read them synchronously) into the inbox,
/// which the app drains every frame.
struct WebShell {
    w: WobbleApp,
    inbox: Inbox,
}

impl eframe::App for WebShell {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let dropped = ctx.input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
        for f in dropped {
            let inbox = self.inbox.clone();
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let name = f.path().file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "dropped".into());
                match f.bytes_async().await {
                    Ok(bytes) => {
                        inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes));
                        ctx.request_repaint();
                    }
                    Err(e) => log::error!("couldn't read dropped file {name}: {e}"),
                }
            });
        }
        eframe::App::logic(&mut self.w, ctx, frame);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        eframe::App::raw_input_hook(&mut self.w, ctx, raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        eframe::App::ui(&mut self.w, ui, frame);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::App::save(&mut self.w, storage);
    }
}

fn services(inbox: Inbox, ctx: egui::Context) -> Services {
    let open_inbox = inbox.clone();
    Services {
        pick_open: Some(Box::new(move || {
            let inbox = open_inbox.clone();
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let Some(file) = rfd::AsyncFileDialog::new().add_filter("Pictures", OPEN_EXTS).pick_file().await else {
                    return;
                };
                let bytes = file.read().await;
                inbox.lock().unwrap_or_else(|e| e.into_inner()).push((file.file_name(), bytes));
                ctx.request_repaint();
            });
            None
        })),
        // No save dialog on the web: the suggested name becomes the download name.
        pick_save: Some(Box::new(|suggested: &str| {
            Some(std::path::Path::new(suggested).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| suggested.to_string()))
        })),
        write: Some(Box::new(|path: &str, bytes: &[u8]| download(path, bytes))),
        inbox: Some(inbox),
        ..codec_services()
    }
}

/// Trigger a browser download of `bytes` named after the last component of `path`.
fn download(path: &str, bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "wobble.psd".into());
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(mime_for(&name));
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;
    let a: web_sys::HtmlAnchorElement = document.create_element("a").map_err(js)?.dyn_into().map_err(|_| "not an anchor")?;
    a.set_href(&url);
    a.set_download(&name);
    a.style().set_property("display", "none").map_err(js)?;
    let body = document.body().ok_or("no body")?;
    body.append_child(&a).map_err(js)?;
    a.click();
    a.remove();
    // Revoke after the click has been dispatched; the download keeps its own reference.
    let revoke = wasm_bindgen::closure::Closure::once_into_js(move || {
        web_sys::Url::revoke_object_url(&url).ok();
    });
    window.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 10_000).map_err(js)?;
    Ok(())
}

fn mime_for(name: &str) -> &'static str {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("psd" | "psb") => "image/vnd.adobe.photoshop",
        Some("glb") => "model/gltf-binary",
        Some("obj") => "model/obj",
        _ => "application/octet-stream",
    }
}

/// Sound through WebAudio: each sound becomes a one-shot buffer source. The context is made on
/// the first sound (after a click, as browsers require); errors leave the app silent.
pub struct WebAudioOut {
    ctx: Option<web_sys::AudioContext>,
}

impl WebAudioOut {
    pub fn new() -> Self {
        Self { ctx: None }
    }

    fn context(&mut self) -> Option<&web_sys::AudioContext> {
        if self.ctx.is_none() {
            self.ctx = web_sys::AudioContext::new().ok();
        }
        self.ctx.as_ref()
    }
}

impl wobbleworks::audio::AudioOut for WebAudioOut {
    fn rate(&self) -> u32 {
        self.ctx.as_ref().map_or(44_100, |c| c.sample_rate() as u32)
    }

    fn play(&mut self, samples: Vec<f32>) {
        let rate = self.rate() as f32;
        let Some(ctx) = self.context() else { return };
        if samples.is_empty() {
            return;
        }
        let _ = ctx.resume();
        let Ok(buffer) = ctx.create_buffer(1, samples.len() as u32, rate) else { return };
        if buffer.copy_to_channel(&samples, 0).is_err() {
            return;
        }
        let Ok(source) = ctx.create_buffer_source() else { return };
        source.set_buffer(Some(&buffer));
        if source.connect_with_audio_node(&ctx.destination()).is_ok() {
            let _ = source.start();
        }
    }
}
