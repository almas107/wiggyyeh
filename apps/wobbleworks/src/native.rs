//! The desktop shell: file dialogs (rfd), crash-safe writes, and the eframe window.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use photocraft_ui_egui::{PhotocraftApp, Services};
use wobbleworks::WobbleApp;
use wobbleworks::io::{OPEN_EXTS, codec_services};

pub fn run() -> eframe::Result {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("WobbleWorks internal error: {info}");
        default_hook(info);
    }));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("WobbleWorks")
            .with_app_id("ai.storyteller.wobbleworks")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([360.0, 420.0])
            .with_drag_and_drop(true),
        centered: true,
        ..Default::default()
    };
    let open = std::env::args().nth(1);
    eframe::run_native(
        "WobbleWorks",
        options,
        Box::new(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut w = WobbleApp::new(services());
            w.restore(cc.storage);
            w.audio.out = CpalOut::open().map(|o| Box::new(o) as Box<dyn wobbleworks::audio::AudioOut>);
            w.picker.pick_reference = Some(Box::new(|inbox| {
                let Some(path) = rfd::FileDialog::new().add_filter("Images", OPEN_EXTS).pick_file() else { return };
                match photocraft_format::read_file(&path) {
                    Ok(bytes) => *inbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((path.to_string_lossy().into_owned(), bytes)),
                    Err(e) => log::warn!("couldn't read {}: {e}", path.display()),
                }
            }));
            w.app.background_jobs = true;
            if let Some(rs) = cc.wgpu_render_state.clone() {
                w.app.set_wgpu(rs);
            }
            if let Some(path) = open {
                // A panic while decoding a file becomes an error; the blank picture stays.
                let opened = catch_unwind(AssertUnwindSafe(|| w.app.open_path(&path))).unwrap_or_else(|_| Err("internal error while opening (logged)".into()));
                if let Err(e) = opened {
                    w.app.ui.status = format!("Couldn't open {path}: {e}");
                    w.app.ui.status_error = true;
                }
            }
            Ok(Box::new(w))
        }),
    )
}

fn services() -> Services {
    Services {
        pick_open: Some(Box::new(|| {
            let path = rfd::FileDialog::new().add_filter("Pictures", OPEN_EXTS).pick_file()?;
            let bytes = photocraft_format::read_file(&path).map_err(|e| e.to_string());
            Some((path.to_string_lossy().to_string(), bytes))
        })),
        pick_open_paths: Some(Box::new(|| {
            rfd::FileDialog::new()
                .add_filter("Pictures", OPEN_EXTS)
                .pick_files()
                .map(|paths| paths.into_iter().map(|p| p.to_string_lossy().into_owned()).collect())
        })),
        pick_save: Some(Box::new(|suggested: &str| {
            let mut d = rfd::FileDialog::new().add_filter("Photoshop", &["psd", "psb"]).add_filter("PNG", &["png"]).add_filter("PhotoCraft", &["pcraft"]);
            if let Some(name) = Path::new(suggested).file_name() {
                d = d.set_file_name(name.to_string_lossy());
            }
            Some(d.save_file()?.to_string_lossy().to_string())
        })),
        // Crash-safe: temp file, fsync, rename, so a failed save never destroys the old file.
        write: Some(Box::new(|path: &str, bytes: &[u8]| photocraft_format::atomic_write(Path::new(path), bytes).map_err(|e| e.to_string()))),
        ..codec_services()
    }
}

/// Sound through the default output device (cpal). Sounds are mixed in the device callback;
/// without a device (or on any error) the app stays silent.
/// Sounds playing: their samples and how far through each one the device is.
type Voices = std::sync::Arc<std::sync::Mutex<Vec<(Vec<f32>, usize)>>>;

pub struct CpalOut {
    rate: u32,
    voices: Voices,
    _stream: cpal::Stream,
}

impl CpalOut {
    pub fn open() -> Option<Self> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let device = cpal::default_host().default_output_device()?;
        let config = device.default_output_config().ok()?;
        let channels = usize::from(config.channels()).max(1);
        let rate = config.sample_rate().0;
        let voices: Voices = std::sync::Arc::default();
        let mix = voices.clone();
        let stream = device
            .build_output_stream(
                &config.into(),
                move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let mut voices = mix.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    for frame in out.chunks_mut(channels) {
                        let mut v = 0.0;
                        for (samples, pos) in voices.iter_mut() {
                            if let Some(s) = samples.get(*pos) {
                                v += s;
                                *pos += 1;
                            }
                        }
                        let v = v.clamp(-1.0, 1.0);
                        frame.iter_mut().for_each(|c| *c = v);
                    }
                    voices.retain(|(s, pos)| *pos < s.len());
                },
                |e| log::warn!("sound: {e}"),
                None,
            )
            .map_err(|e| log::warn!("no sound: {e}"))
            .ok()?;
        stream.play().map_err(|e| log::warn!("no sound: {e}")).ok()?;
        Some(Self { rate, voices, _stream: stream })
    }
}

impl wobbleworks::audio::AudioOut for CpalOut {
    fn rate(&self) -> u32 {
        self.rate
    }

    fn play(&mut self, samples: Vec<f32>) {
        let mut v = self.voices.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // A busy moment can't pile up unbounded sound.
        if v.len() < 32 {
            v.push((samples, 0));
        }
    }
}
