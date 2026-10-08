//! The cartoon look: colour tokens, presets, and how they map onto egui's style.
//!
//! Everything visual comes from [`Theme`] plus a few shape settings (roundness, outline width,
//! shadow depth), all user-editable in Settings → Look.

use egui::{Color32, CornerRadius, FontId, Shadow, Stroke, TextStyle, Visuals};
use serde::{Deserialize, Serialize};

/// Colour tokens.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    /// Text and outlines.
    pub ink: Color32,
    /// The backdrop behind everything.
    pub paper: Color32,
    /// Panels and buttons.
    pub card: Color32,
    /// Accent: focus rings, sliders, selection outlines.
    pub hot: Color32,
    /// Selected rows (current layer, current project).
    pub cool: Color32,
    /// Toggled-on buttons.
    pub sun: Color32,
    /// Quiet text.
    pub dim: Color32,
    /// Hard drop shadows.
    pub shadow: Color32,
    pub dark: bool,
}

impl Default for Theme {
    fn default() -> Self {
        PRESETS.first().map_or(BUBBLEGUM, |p| p.1)
    }
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

const BUBBLEGUM: Theme = Theme {
    ink: hex(0x17161c),
    paper: hex(0xe4e1ee),
    card: hex(0xfbfaff),
    hot: hex(0xff2e88),
    cool: hex(0x2ee6c5),
    sun: hex(0xffd23f),
    dim: hex(0x8b8798),
    shadow: hex(0x17161c),
    dark: false,
};

/// Built-in looks.
pub const PRESETS: &[(&str, Theme)] = &[
    ("Bubblegum", BUBBLEGUM),
    (
        "Mint Choc",
        Theme {
            ink: hex(0x3b2418),
            paper: hex(0xcfeee0),
            card: hex(0xf4fff9),
            hot: hex(0xe0577a),
            cool: hex(0x9fe3c4),
            sun: hex(0xffc96b),
            dim: hex(0x7a6a5f),
            shadow: hex(0x3b2418),
            dark: false,
        },
    ),
    (
        "Lemonade",
        Theme {
            ink: hex(0x2b2b12),
            paper: hex(0xfff3a8),
            card: hex(0xfffde8),
            hot: hex(0xff7a1a),
            cool: hex(0xb8f06a),
            sun: hex(0xff9ecb),
            dim: hex(0x8a8250),
            shadow: hex(0x2b2b12),
            dark: false,
        },
    ),
    (
        "Grape Soda",
        Theme {
            ink: hex(0x2a1240),
            paper: hex(0xd9c8f5),
            card: hex(0xf8f2ff),
            hot: hex(0x8b3dff),
            cool: hex(0xffb3e6),
            sun: hex(0x7ee8fa),
            dim: hex(0x806c99),
            shadow: hex(0x2a1240),
            dark: false,
        },
    ),
    (
        "Peach Fuzz",
        Theme {
            ink: hex(0x4a2121),
            paper: hex(0xffd8c2),
            card: hex(0xfff6f0),
            hot: hex(0xff5a5f),
            cool: hex(0xffe08a),
            sun: hex(0x9be7d8),
            dim: hex(0x9a7470),
            shadow: hex(0x4a2121),
            dark: false,
        },
    ),
    (
        "Midnight Snack",
        Theme {
            ink: hex(0xf3efff),
            paper: hex(0x1d1a2b),
            card: hex(0x2c2840),
            hot: hex(0xff4fa3),
            cool: hex(0x22977f),
            sun: hex(0x8a6d1c),
            dim: hex(0x9a93b5),
            shadow: hex(0x07060c),
            dark: true,
        },
    ),
    (
        "Arcade",
        Theme {
            ink: hex(0xe8fff4),
            paper: hex(0x0f2027),
            card: hex(0x183842),
            hot: hex(0x39ff14),
            cool: hex(0x1f7a8c),
            sun: hex(0xb3367a),
            dim: hex(0x7fa8a8),
            shadow: hex(0x000000),
            dark: true,
        },
    ),
];

/// Shape settings that go with the colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub t: Theme,
    pub round: f32,
    pub line: f32,
    pub shadow: f32,
    /// UI boil frame (0–2), or `None` when the UI holds still.
    pub boil: Option<u8>,
    pub labels: bool,
}

impl Look {
    /// A small whole-pixel jitter for widget `id` on the current boil frame.
    pub fn jitter(&self, id: egui::Id) -> egui::Vec2 {
        let Some(f) = self.boil else { return egui::Vec2::ZERO };
        let h = id.value();
        let n = |k: u64| crate::brush::rnd((h ^ (h >> 32)) as u32 ^ (u32::from(f).wrapping_add(k as u32)).wrapping_mul(0x9E37_79B9));
        egui::vec2(((n(1) * 3.0).floor() - 1.0) as f32 * 0.5, ((n(7) * 3.0).floor() - 1.0) as f32 * 0.5)
    }

    pub fn radius(&self) -> CornerRadius {
        CornerRadius::same(self.round.clamp(0.0, 30.0) as u8)
    }

    pub fn hard_shadow(&self) -> Shadow {
        let s = self.shadow.clamp(0.0, 12.0) as i8;
        Shadow { offset: [s, s], blur: 0, spread: 0, color: self.t.shadow }
    }

    pub fn outline(&self) -> Stroke {
        Stroke::new(self.line, self.t.ink)
    }
}

/// Mix two colours (`t` = 0 gives `a`).
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Apply the look to egui's global style.
pub fn apply(ctx: &egui::Context, look: &Look, text_size: f32) {
    let t = look.t;
    let mut v = if t.dark { Visuals::dark() } else { Visuals::light() };
    let ink = Stroke::new(look.line, t.ink);
    let r = look.radius();
    v.override_text_color = Some(t.ink);
    v.panel_fill = t.paper;
    v.window_fill = t.card;
    v.faint_bg_color = mix(t.card, t.paper, 0.5);
    v.extreme_bg_color = mix(t.card, t.paper, 0.35);
    v.code_bg_color = t.paper;
    v.window_stroke = ink;
    v.window_corner_radius = r;
    v.menu_corner_radius = r;
    v.window_shadow = look.hard_shadow();
    v.popup_shadow = look.hard_shadow();
    v.hyperlink_color = t.hot;
    v.selection.bg_fill = mix(t.hot, t.card, 0.35);
    v.selection.stroke = Stroke::new(look.line, t.ink);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.striped = false;
    v.text_cursor.stroke = Stroke::new(2.0, t.hot);
    let w = &mut v.widgets;
    for (st, fill) in [
        (&mut w.noninteractive, t.card),
        (&mut w.inactive, t.card),
        (&mut w.hovered, mix(t.card, t.sun, 0.35)),
        (&mut w.active, t.sun),
        (&mut w.open, mix(t.card, t.cool, 0.4)),
    ] {
        st.bg_fill = fill;
        st.weak_bg_fill = fill;
        st.bg_stroke = ink;
        st.fg_stroke = Stroke::new(look.line.max(1.0), t.ink);
        st.corner_radius = r;
        st.expansion = 0.0;
    }
    w.noninteractive.bg_stroke = Stroke::new(look.line * 0.75, mix(t.ink, t.card, 0.55));
    w.inactive.bg_fill = mix(t.paper, t.card, 0.3);
    w.hovered.expansion = 1.0;
    let s = text_size.clamp(9.0, 24.0);
    // Same look whatever the system's light/dark preference is.
    ctx.all_styles_mut(|st| {
        st.visuals = v.clone();
        st.text_styles = [
            (TextStyle::Small, FontId::proportional(s * 0.82)),
            (TextStyle::Body, FontId::proportional(s)),
            (TextStyle::Button, FontId::proportional(s)),
            (TextStyle::Heading, FontId::proportional(s * 1.55)),
            (TextStyle::Monospace, FontId::monospace(s * 0.95)),
        ]
        .into();
        st.spacing.item_spacing = egui::vec2(7.0, 7.0);
        st.spacing.button_padding = egui::vec2(9.0, 5.0);
        st.spacing.slider_width = 120.0;
        st.spacing.interact_size.y = s + 10.0;
        st.spacing.window_margin = egui::Margin::same(12);
        st.spacing.menu_margin = egui::Margin::same(8);
        st.interaction.tooltip_delay = 0.35;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_have_readable_text() {
        for (name, t) in PRESETS {
            let lum = |c: Color32| 0.299 * f32::from(c.r()) + 0.587 * f32::from(c.g()) + 0.114 * f32::from(c.b());
            assert!((lum(t.ink) - lum(t.card)).abs() > 120.0, "{name}: ink on card");
            assert!((lum(t.ink) - lum(t.paper)).abs() > 100.0, "{name}: ink on paper");
            assert_eq!(t.dark, lum(t.card) < 128.0, "{name}: dark flag");
        }
    }

    #[test]
    fn jitter_is_whole_half_pixels_and_off_when_still() {
        let mut l = Look { t: Theme::default(), round: 9.0, line: 2.0, shadow: 3.0, boil: None, labels: false };
        assert_eq!(l.jitter(egui::Id::new(1)), egui::Vec2::ZERO);
        l.boil = Some(2);
        let j = l.jitter(egui::Id::new("x"));
        assert!(j.x.abs() <= 0.5 && j.y.abs() <= 0.5);
        assert_eq!(mix(Color32::BLACK, Color32::WHITE, 0.0), Color32::BLACK);
    }
}
