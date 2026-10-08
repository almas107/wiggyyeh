//! Stage 4: juice, sound and Wob, driven through real frames of the app.

use egui_kittest::Harness;
use photocraft_ui_egui::Services;
use serde_json::json;
use wobbleworks::WobbleApp;
use wobbleworks::audio::Sound;
use wobbleworks::io::codec_services;
use wobbleworks::mascot::Mood;

fn harness() -> Harness<'static, WobbleApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 820.0)).with_max_steps(16).build_eframe(|cc| {
        photocraft_ui_egui::PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let services =
            Services { write: Some(Box::new(|_: &str, _: &[u8]| Ok(()))), pick_save: Some(Box::new(|s: &str| Some(s.to_string()))), ..codec_services() };
        WobbleApp::new(services)
    });
    h.run_steps(3);
    h
}

fn now(h: &Harness<'_, WobbleApp>) -> f64 {
    h.ctx.input(|i| i.time)
}

#[test]
fn undo_blips_and_surprises_wob() {
    let mut h = harness();
    h.state_mut().stroke(&[(100.0, 100.0), (300.0, 120.0)], "#ff0000", 10.0).unwrap();
    h.run_steps(2);
    assert!(h.state().audio.recent.contains(&Sound::Chime), "the first stroke is celebrated");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().audio.recent.back(), Some(&Sound::Undo));
    let t = now(&h);
    assert_eq!(h.state().mascot.mood(t), Mood::Surprised);
    assert!(h.state().mascot.saying(t).is_some());
}

#[test]
fn saving_celebrates_and_big_actions_shake() {
    let mut h = harness();
    h.state_mut().save_psd(Some("x.psd".into())).unwrap();
    h.run_steps(2);
    assert!(h.state().audio.recent.contains(&Sound::Chime));
    assert!(h.state().juice.particle_count() > 0, "confetti");
    assert_eq!(h.state().mascot.mood(now(&h)), Mood::Cheer);
    h.state_mut().run("layer.delete", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().audio.recent.back(), Some(&Sound::Thud));
    assert!(h.state().juice.busy(now(&h)));
}

#[test]
fn reduce_motion_and_settings_round_trip() {
    let mut h = harness();
    h.state_mut().set_reduce_motion(true);
    h.state_mut().save_psd(Some("x.psd".into())).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().juice.particle_count(), 0, "no confetti with reduce motion");
    assert!(!h.state().boiling);
    h.state_mut().audio.volume = 0.25;
    h.state_mut().audio.muted = true;
    h.state_mut().mascot.enabled = false;
    let saved = h.state().settings_json();
    let mut h2 = harness();
    h2.state_mut().restore_settings(&saved);
    let s = h2.state();
    assert!(s.reduce_motion && s.audio.muted && !s.mascot.enabled && (s.audio.volume - 0.25).abs() < 1e-6);
    h2.state_mut().restore_settings("{\"volume\": 99, \"muted\": \"yes\"}");
    assert_eq!(h2.state().audio.volume, 1.0, "clamped");
    h2.state_mut().restore_settings("garbage");
    // The settings card and a sleepy Wob render.
    h2.state_mut().show_settings = true;
    h2.state_mut().mascot.enabled = true;
    h2.run_steps(3);
}
