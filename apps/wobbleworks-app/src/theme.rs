//! The cartoon look: colour presets, and how they restyle PhotoCraft's editor.
//!
//! WobbleWorks paints its own widgets from [`Theme`]. PhotoCraft's editor (canvas surround,
//! dialogs, and every panel in the advanced editor) reads `photocraft_ui_egui::theme::Tokens`
//! from egui's context; [`apply`] publishes tokens made from the same colours, over PhotoCraft's
//! light Studio layout, so both halves of the app match.

use egui::{Color32, CornerRadius, Stroke};
use photocraft_ui_egui::theme::{ThemeKind, Tokens};

/// Colour tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    /// Text and outlines.
    pub ink: Color32,
    /// The backdrop behind everything.
    pub paper: Color32,
    /// Panels and buttons.
    pub card: Color32,
    /// Accent: the current tool, focus, sliders.
    pub hot: Color32,
    /// Secondary accent: selected rows, the logo.
    pub cool: Color32,
    /// Hover and toggled-on buttons.
    pub sun: Color32,
    /// Quiet text.
    pub dim: Color32,
    /// Hard drop shadows.
    pub shadow: Color32,
    pub dark: bool,
}

impl Default for Theme {
    fn default() -> Self {
        BUBBLEGUM
    }
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub const BUBBLEGUM: Theme = Theme {
    ink: hex(0x17161c),
    paper: hex(0xe4e1ee),
    card: hex(0xfbfaff),
    hot: hex(0xff2e88),
    cool: hex(0x2ee6c5),
    sun: hex(0xffd23f),
    dim: hex(0x6b6878),
    shadow: hex(0x17161c),
    dark: false,
};

/// Built-in looks (from the earlier WobbleWorks; more arrive with the settings in a later stage).
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
];

/// Mix two colours (`t` = 0 gives `a`).
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// `#rrggbb` of a colour.
pub fn to_hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

/// The PhotoCraft layout WobbleWorks restyles: rounded cards, no Photoshop tab strips.
pub fn base_kind(t: &Theme) -> ThemeKind {
    if t.dark { ThemeKind::Studio } else { ThemeKind::StudioLight }
}

/// PhotoCraft tokens in WobbleWorks colours.
pub fn tokens(t: &Theme) -> Tokens {
    let mut k = Tokens::for_kind(base_kind(t));
    k.chrome = mix(t.card, t.paper, 0.45);
    k.canvas = t.paper;
    k.canvas_dot = mix(t.ink, t.paper, 0.75);
    k.dock = t.paper;
    k.card = t.card;
    k.card_border = mix(t.ink, t.card, 0.25);
    k.field = mix(t.card, t.paper, 0.35);
    k.field_border = mix(t.ink, t.card, 0.45);
    k.hover = mix(t.card, t.sun, 0.4);
    k.pressed = t.sun;
    k.text = t.ink;
    k.text_dim = t.dim;
    k.text_faint = mix(t.dim, t.card, 0.35);
    k.icon = t.ink;
    k.accent = t.hot;
    k.accent_soft = mix(t.hot, t.card, 0.72);
    k.accent_border = t.hot;
    k.accent_text = t.ink;
    k.separator = mix(t.ink, t.card, 0.7);
    k.shadow = t.shadow.gamma_multiply(0.35);
    k.primary_bg = t.hot;
    k.primary_text = Color32::WHITE;
    k.radius_sm = 7.0;
    k.radius = 11.0;
    k.radius_lg = 16.0;
    k.bevel = false;
    k.pro = false;
    k.tab_strip = t.paper;
    k.row_selected = mix(t.cool, t.card, 0.45);
    k
}

const TOKENS_ID: &str = "photocraft-theme";

/// Are WobbleWorks' tokens the ones PhotoCraft will read this frame?
pub fn is_applied(ctx: &egui::Context, t: &Theme) -> bool {
    Tokens::get(ctx) == tokens(t)
}

/// Restyle egui and PhotoCraft with `t`: PhotoCraft's own theme setup first (fonts, sizes,
/// spacing), then WobbleWorks' colours and chunky outlines on top.
pub fn apply(ctx: &egui::Context, t: &Theme) {
    photocraft_ui_egui::theme::apply(ctx, base_kind(t));
    let k = tokens(t);
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(TOKENS_ID), k));
    let ink = Stroke::new(2.0, t.ink);
    let hard = |o: i8| egui::Shadow { offset: [o, o], blur: 0, spread: 0, color: t.shadow };
    ctx.all_styles_mut(|s| {
        let v = &mut s.visuals;
        v.dark_mode = t.dark;
        v.panel_fill = k.chrome;
        v.window_fill = t.card;
        v.window_stroke = ink;
        v.window_shadow = hard(5);
        v.popup_shadow = hard(4);
        v.window_corner_radius = CornerRadius::same(14);
        v.menu_corner_radius = CornerRadius::same(10);
        v.extreme_bg_color = k.field;
        v.faint_bg_color = mix(t.card, t.paper, 0.5);
        v.code_bg_color = k.field;
        v.override_text_color = Some(t.ink);
        v.hyperlink_color = t.hot;
        v.selection.bg_fill = mix(t.hot, t.card, 0.55);
        v.selection.stroke = Stroke::new(1.5, t.ink);
        v.text_cursor.stroke = Stroke::new(2.0, t.hot);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = t.card;
        w.noninteractive.weak_bg_fill = t.card;
        w.noninteractive.bg_stroke = Stroke::new(1.0, k.separator);
        w.noninteractive.fg_stroke = Stroke::new(1.0, t.ink);
        for (st, fill) in [(&mut w.inactive, k.field), (&mut w.hovered, k.hover), (&mut w.active, t.sun), (&mut w.open, k.hover)] {
            st.bg_fill = fill;
            st.weak_bg_fill = fill;
            st.bg_stroke = Stroke::new(1.5, mix(t.ink, t.card, 0.3));
            st.fg_stroke = Stroke::new(1.5, t.ink);
            st.corner_radius = CornerRadius::same(8);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_have_readable_text() {
        let lum = |c: Color32| 0.299 * f32::from(c.r()) + 0.587 * f32::from(c.g()) + 0.114 * f32::from(c.b());
        for (name, t) in PRESETS {
            assert!((lum(t.ink) - lum(t.card)).abs() > 120.0, "{name}: ink on card");
            assert!((lum(t.ink) - lum(t.paper)).abs() > 100.0, "{name}: ink on paper");
            assert_eq!(t.dark, lum(t.card) < 128.0, "{name}: dark flag");
        }
    }

    #[test]
    fn tokens_restyle_photocraft_and_stick() {
        let ctx = egui::Context::default();
        assert!(!is_applied(&ctx, &BUBBLEGUM));
        apply(&ctx, &BUBBLEGUM);
        assert!(is_applied(&ctx, &BUBBLEGUM));
        let k = Tokens::get(&ctx);
        assert_eq!(k.canvas, BUBBLEGUM.paper);
        assert_eq!(k.accent, BUBBLEGUM.hot);
        assert!(!k.pro, "no Photoshop tab strips");
        assert_eq!(to_hex(BUBBLEGUM.hot), "#ff2e88");
        assert_eq!(mix(Color32::BLACK, Color32::WHITE, f32::NAN), Color32::BLACK);
    }
}
