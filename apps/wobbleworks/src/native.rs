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
    let args = match parse_args(std::env::args().skip(1), std::env::var("WOBBLEWORKS_CONTROL_PORT").ok()) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("wobbleworks: {e}");
            std::process::exit(2);
        }
    };
    let control = match args.control.map(|c| c.configure()).transpose() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("wobbleworks: {e}");
            std::process::exit(2);
        }
    };
    let open = args.open;
    eframe::run_native(
        "WobbleWorks",
        options,
        Box::new(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let automation = control.as_ref().map(|(_, _, workspace)| workspace.clone());
            let mut w = WobbleApp::new(services(automation));
            if let Some((port, token, _)) = &control {
                w.set_control(crate::control_server::start(*port, token.clone(), cc.egui_ctx.clone()));
            }
            w.restore(cc.storage);
            w.audio.out = CpalOut::open().map(|o| Box::new(o) as Box<dyn wobbleworks::audio::AudioOut>);
            w.picker.pick_reference = Some(Box::new(|inbox| {
                let Some(path) = rfd::FileDialog::new().add_filter("Images", OPEN_EXTS).pick_file() else { return };
                match photocraft_format::read_file(&path) {
                    Ok(bytes) => *inbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((path.to_string_lossy().into_owned(), bytes)),
                    Err(e) => log::warn!("couldn't read {}: {e}", path.display()),
                }
            }));
            w.space.files = space_files();
            w.app.background_jobs = true;
            if let Some(rs) = cc.wgpu_render_state.clone() {
                w.space.set_gpu(&rs);
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

/// File dialogs for the 3D mode: pick into its inbox, save with a dialog and a crash-safe write.
fn space_files() -> wobbleworks::space3d::Files {
    wobbleworks::space3d::Files {
        pick: Some(Box::new(|inbox, purpose, exts| {
            let Some(path) = rfd::FileDialog::new().add_filter("Files", exts).pick_file() else { return };
            match photocraft_format::read_file(&path) {
                Ok(bytes) => {
                    inbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((purpose.to_string(), path.to_string_lossy().into_owned(), bytes))
                }
                Err(e) => log::warn!("couldn't read {}: {e}", path.display()),
            }
        })),
        save: Some(Box::new(|suggested: &str, bytes: &[u8]| {
            let ext = Path::new(suggested).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
            let mut d = rfd::FileDialog::new();
            if !ext.is_empty() {
                d = d.add_filter(ext.to_uppercase(), &[ext.as_str()]);
            }
            if let Some(name) = Path::new(suggested).file_name() {
                d = d.set_file_name(name.to_string_lossy());
            }
            let Some(mut path) = d.save_file() else { return Ok(None) };
            if !ext.is_empty() && path.extension().is_none() {
                path.set_extension(&ext);
            }
            photocraft_format::atomic_write(&path, bytes).map_err(|e| e.to_string())?;
            Ok(Some(path.to_string_lossy().into_owned()))
        })),
    }
}

/// What the command line asked for: a file to open, and the control server.
#[derive(Debug, Default, PartialEq)]
struct Args {
    open: Option<String>,
    control: Option<ControlArgs>,
}

#[derive(Debug, Default, PartialEq)]
struct ControlArgs {
    port: u16,
    token: Option<String>,
    token_file: Option<std::path::PathBuf>,
    read_root: Option<std::path::PathBuf>,
    write_root: Option<std::path::PathBuf>,
}

impl ControlArgs {
    /// The port, the token (made and printed when none is given, as PhotoCraft does) and the
    /// folders automation may read and write.
    fn configure(self) -> Result<(u16, String, photocraft_automation::AuthorizedWorkspace), String> {
        use photocraft_automation::security::{server_token, token_inputs};
        let (supplied, token_file) = token_inputs(self.token, self.token_file);
        let token = server_token(supplied.as_deref(), token_file.as_deref()).map_err(|e| format!("cannot configure control authentication: {e}"))?;
        if let Some(path) = &token_file {
            eprintln!("wobbleworks: control token file: {}", path.display());
        } else if supplied.is_none() {
            eprintln!("wobbleworks: control token: {token}");
        }
        let workspace = photocraft_automation::AuthorizedWorkspace::new(self.read_root.as_deref(), self.write_root.as_deref())
            .map_err(|e| format!("cannot configure automation workspace: {e}"))?;
        Ok((self.port, token, workspace))
    }
}

fn parse_port(value: &str, source: &str) -> Result<u16, String> {
    value.trim().parse::<u16>().map_err(|_| format!("{source}: `{value}` is not a port number (0–65535)"))
}

/// `wobbleworks [file] [--control <port>] [--control-token <hex> | --control-token-file <path>]
/// [--automation-read-root <dir>] [--automation-write-root <dir>]` (the port may also come from
/// `WOBBLEWORKS_CONTROL_PORT`). A bad port is an error, never a silently missing server.
fn parse_args(args: impl IntoIterator<Item = String>, env_port: Option<String>) -> Result<Args, String> {
    let mut out = Args::default();
    let mut c = ControlArgs::default();
    let mut port = match env_port.filter(|v| !v.trim().is_empty()) {
        Some(v) => Some(parse_port(&v, "WOBBLEWORKS_CONTROL_PORT")?),
        None => None,
    };
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        let mut value = |flag: &str| args.next().ok_or_else(|| format!("{flag}: missing value"));
        match a.as_str() {
            "--control" => port = Some(parse_port(&value("--control")?, "--control")?),
            "--control-token" => c.token = Some(value("--control-token")?),
            "--control-token-file" => c.token_file = Some(value("--control-token-file")?.into()),
            "--automation-read-root" => c.read_root = Some(value("--automation-read-root")?.into()),
            "--automation-write-root" => c.write_root = Some(value("--automation-write-root")?.into()),
            _ if a.starts_with("-psn_") => {}
            _ if a.starts_with("--") => return Err(format!("unknown option `{a}`")),
            _ => out.open = Some(a),
        }
    }
    out.control = port.map(|port| ControlArgs { port, ..c });
    Ok(out)
}

fn services(automation: Option<photocraft_automation::AuthorizedWorkspace>) -> Services {
    // Control requests may only read and write inside the authorized folders (PhotoCraft's rules).
    let automation_read = automation.clone().map(|workspace| {
        Box::new(move |path: &str| {
            let bytes = workspace.read(path).map_err(|error| error.to_string())?;
            let name = Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or(path).to_string();
            Ok((name, bytes))
        }) as photocraft_ui_egui::AutomationReadFn
    });
    let automation_write = automation.clone().map(|workspace| {
        Box::new(move |path: &str, bytes: &[u8]| workspace.write(path, bytes).map_err(|error| error.to_string())) as photocraft_ui_egui::AutomationWriteFn
    });
    let step: fn(&str, &serde_json::Value) -> photocraft_engine::Result<()> = photocraft_automation::workspace::authorize_desktop_engine_step;
    let automation_authorize = automation.is_some().then_some(step);
    let automation_command = automation.map(|_| {
        Box::new(|id: &str, params: &serde_json::Value| {
            photocraft_automation::workspace::authorize_desktop_engine_command(id, params).map_err(|error| error.to_string())
        }) as photocraft_ui_egui::AutomationCommandFn
    });
    Services {
        automation_read,
        automation_write,
        automation_command,
        automation_authorize,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_command_line_names_a_file_and_the_control_server() {
        assert_eq!(parse_args(args(&["pic.png"]), None), Ok(Args { open: Some("pic.png".into()), control: None }));
        let a = parse_args(args(&["--control", "7878", "--control-token-file", "/tmp/t", "x.psd"]), None).expect("ok");
        assert_eq!(a.open.as_deref(), Some("x.psd"));
        let c = a.control.expect("control");
        assert_eq!((c.port, c.token_file), (7878, Some("/tmp/t".into())));
        assert_eq!(parse_args(args(&[]), Some("9000".into())).expect("env").control.map(|c| c.port), Some(9000));
        for bad in [&["--control"][..], &["--control", "nope"], &["--control", "70000"], &["--frobnicate"]] {
            assert!(parse_args(args(bad), None).is_err(), "{bad:?}");
        }
        assert!(parse_args(args(&[]), Some("x".into())).is_err());
    }
}
