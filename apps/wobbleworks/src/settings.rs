//! Everything the user can customise, saved between sessions: the look, the layout, palettes,
//! keyboard shortcuts and the last-used tools.

use egui::{Color32, Key};
use serde::{Deserialize, Serialize};

use crate::fill::FillOpts;
use crate::geom::Mirror;
use crate::model::{Brush, Tip};
use crate::theme::Theme;

/// Canvas tools (brushes are chosen separately and used by Brush and the shape tools).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Tool {
    #[default]
    Brush,
    Line,
    Rect,
    Ellipse,
    Fill,
    Lasso,
    Move,
    Pick,
    Hand,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::Brush => "Brush",
            Tool::Line => "Line",
            Tool::Rect => "Box",
            Tool::Ellipse => "Oval",
            Tool::Fill => "Fill",
            Tool::Lasso => "Lasso",
            Tool::Move => "Move",
            Tool::Pick => "Pick",
            Tool::Hand => "Hand",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Tool::Brush => "Draw with the chosen brush.",
            Tool::Line => "Drag a wobbly straight line (Shift snaps to 45°).",
            Tool::Rect => "Drag a wobbly box (Shift makes a square). Use Blob fill for a solid one.",
            Tool::Ellipse => "Drag a wobbly oval (Shift makes a circle). Use Blob fill for a solid one.",
            Tool::Fill => "Fill an area; computed per frame so it boils with the outline.",
            Tool::Lasso => "Loop around something to pick it up, then drag it.",
            Tool::Move => "Drag the selection, or the whole layer when nothing is selected.",
            Tool::Pick => "Pick a colour from the canvas.",
            Tool::Hand => "Drag to pan (or hold Space, or use the middle button).",
        }
    }
}

/// Anything a shortcut can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    Undo,
    Redo,
    Brush,
    Eraser,
    Line,
    Rect,
    Ellipse,
    Fill,
    Lasso,
    Move,
    Pick,
    Hand,
    SizeDown,
    SizeUp,
    Pause,
    Focus,
    Mirror,
    ZoomIn,
    ZoomOut,
    ZoomFit,
    Zoom100,
    Apply,
    Delete,
    NewLayer,
    Save,
    ExportPng,
    ExportGif,
    Import,
    NextFrame,
    PrevBrush,
    NextBrush,
}

impl Action {
    pub const ALL: [Action; 31] = [
        Action::Undo,
        Action::Redo,
        Action::Brush,
        Action::Eraser,
        Action::Line,
        Action::Rect,
        Action::Ellipse,
        Action::Fill,
        Action::Lasso,
        Action::Move,
        Action::Pick,
        Action::Hand,
        Action::SizeDown,
        Action::SizeUp,
        Action::PrevBrush,
        Action::NextBrush,
        Action::Pause,
        Action::NextFrame,
        Action::Focus,
        Action::Mirror,
        Action::ZoomIn,
        Action::ZoomOut,
        Action::ZoomFit,
        Action::Zoom100,
        Action::Apply,
        Action::Delete,
        Action::NewLayer,
        Action::Save,
        Action::ExportPng,
        Action::ExportGif,
        Action::Import,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Action::Undo => "Undo",
            Action::Redo => "Redo",
            Action::Brush => "Brush",
            Action::Eraser => "Eraser",
            Action::Line => "Line",
            Action::Rect => "Box",
            Action::Ellipse => "Oval",
            Action::Fill => "Fill",
            Action::Lasso => "Lasso",
            Action::Move => "Move",
            Action::Pick => "Pick colour",
            Action::Hand => "Hand",
            Action::SizeDown => "Smaller brush",
            Action::SizeUp => "Bigger brush",
            Action::PrevBrush => "Previous brush",
            Action::NextBrush => "Next brush",
            Action::Pause => "Play / pause",
            Action::NextFrame => "Step one frame",
            Action::Focus => "Hide panels",
            Action::Mirror => "Cycle symmetry",
            Action::ZoomIn => "Zoom in",
            Action::ZoomOut => "Zoom out",
            Action::ZoomFit => "Fit to window",
            Action::Zoom100 => "Actual pixels",
            Action::Apply => "Apply selection",
            Action::Delete => "Delete selection",
            Action::NewLayer => "New layer",
            Action::Save => "Save now",
            Action::ExportPng => "Save PNG",
            Action::ExportGif => "Save GIF",
            Action::Import => "Import picture",
        }
    }
}

/// A key plus modifiers. `cmd` is Ctrl (Cmd on macOS).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub key: Key,
    #[serde(default)]
    pub cmd: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
}

impl Binding {
    const fn k(key: Key) -> Self {
        Binding { key, cmd: false, shift: false, alt: false }
    }
    const fn cmd(key: Key) -> Self {
        Binding { key, cmd: true, shift: false, alt: false }
    }
    const fn cmd_shift(key: Key) -> Self {
        Binding { key, cmd: true, shift: true, alt: false }
    }

    pub fn label(&self) -> String {
        let mut s = String::new();
        if self.cmd {
            s.push_str(if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" });
        }
        if self.alt {
            s.push_str("Alt+");
        }
        if self.shift {
            s.push_str("Shift+");
        }
        s.push_str(self.key.symbol_or_name());
        s
    }

    pub fn matches(&self, key: Key, m: egui::Modifiers) -> bool {
        self.key == key && self.cmd == m.command && self.shift == m.shift && self.alt == m.alt
    }
}

pub fn default_keys() -> Vec<(Action, Binding)> {
    use Action as A;
    vec![
        (A::Undo, Binding::cmd(Key::Z)),
        (A::Redo, Binding::cmd_shift(Key::Z)),
        (A::Redo, Binding::cmd(Key::Y)),
        (A::Brush, Binding::k(Key::B)),
        (A::Eraser, Binding::k(Key::E)),
        (A::Line, Binding::k(Key::L)),
        (A::Rect, Binding::k(Key::R)),
        (A::Ellipse, Binding::k(Key::O)),
        (A::Fill, Binding::k(Key::G)),
        (A::Lasso, Binding::k(Key::S)),
        (A::Move, Binding::k(Key::V)),
        (A::Pick, Binding::k(Key::I)),
        (A::Hand, Binding::k(Key::H)),
        (A::SizeDown, Binding::k(Key::OpenBracket)),
        (A::SizeUp, Binding::k(Key::CloseBracket)),
        (A::PrevBrush, Binding::k(Key::Comma)),
        (A::NextBrush, Binding::k(Key::Period)),
        (A::Pause, Binding::k(Key::P)),
        (A::NextFrame, Binding::k(Key::Slash)),
        (A::Focus, Binding::k(Key::Tab)),
        (A::Mirror, Binding::k(Key::M)),
        (A::ZoomIn, Binding::cmd(Key::Equals)),
        (A::ZoomIn, Binding::cmd(Key::Plus)),
        (A::ZoomOut, Binding::cmd(Key::Minus)),
        (A::ZoomFit, Binding::cmd(Key::Num9)),
        (A::Zoom100, Binding::cmd(Key::Num0)),
        (A::Apply, Binding::k(Key::Enter)),
        (A::Delete, Binding::k(Key::Delete)),
        (A::Delete, Binding::k(Key::Backspace)),
        (A::NewLayer, Binding::cmd_shift(Key::N)),
        (A::Save, Binding::cmd(Key::S)),
        (A::ExportPng, Binding::cmd(Key::E)),
        (A::ExportGif, Binding::cmd_shift(Key::E)),
        (A::Import, Binding::cmd(Key::I)),
    ]
}

/// The tool setup, remembered between sessions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tools {
    pub tool: Tool,
    pub brush: Brush,
    /// The brush to go back to after the eraser.
    pub last_brush: Brush,
    pub tip: Tip,
    pub color: Color32,
    pub size: f64,
    pub mirror: Mirror,
    /// Steady-hand smoothing, 0 (off) to 10.
    pub stabilizer: u8,
    pub fill: FillOpts,
    /// Use pen pressure for size when the device reports it.
    pub pressure: bool,
}

impl Default for Tools {
    fn default() -> Self {
        Tools {
            tool: Tool::Brush,
            brush: Brush::Marker,
            last_brush: Brush::Marker,
            tip: Tip::Round,
            color: Color32::from_rgb(0x17, 0x16, 0x1c),
            size: 6.0,
            mirror: Mirror::Off,
            stabilizer: 0,
            fill: FillOpts::default(),
            pressure: true,
        }
    }
}

/// The backdrop pattern behind the canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Backdrop {
    #[default]
    Dots,
    Grid,
    Stripes,
    Plain,
}

impl Backdrop {
    pub const ALL: [Backdrop; 4] = [Backdrop::Dots, Backdrop::Grid, Backdrop::Stripes, Backdrop::Plain];
    pub fn label(self) -> &'static str {
        match self {
            Backdrop::Dots => "Polka dots",
            Backdrop::Grid => "Graph paper",
            Backdrop::Stripes => "Candy stripes",
            Backdrop::Plain => "Plain",
        }
    }
}

pub const PALETTES: &[(&str, &[u32])] = &[
    (
        "Wobble",
        &[
            0x17161c, 0xffffff, 0xff2e88, 0xff5a1f, 0xffd23f, 0x3ddc97, 0x2ec4ff, 0x7b4dff, 0x8b5e3c, 0xf6b8c8, 0x9ef01a, 0x00b4a6, 0x004e89, 0x6a0572,
            0xc1121f, 0x8b8798,
        ],
    ),
    (
        "Pastel",
        &[
            0x3d3a4b, 0xffffff, 0xffadad, 0xffd6a5, 0xfdffb6, 0xcaffbf, 0x9bf6ff, 0xa0c4ff, 0xbdb2ff, 0xffc6ff, 0xf1c0e8, 0xcfbaf0, 0xa3c4f3, 0x90dbf4,
            0x8eecf5, 0x98f5e1,
        ],
    ),
    (
        "Pico-8",
        &[
            0x000000, 0x1d2b53, 0x7e2553, 0x008751, 0xab5236, 0x5f574f, 0xc2c3c7, 0xfff1e8, 0xff004d, 0xffa300, 0xffec27, 0x00e436, 0x29adff, 0x83769c,
            0xff77a8, 0xffccaa,
        ],
    ),
    ("Game Boy", &[0x0f380f, 0x306230, 0x8bac0f, 0x9bbc0f]),
    ("Sunset", &[0x2d1e2f, 0xffffff, 0x5c2a4d, 0x9e3d64, 0xd65a6f, 0xf08a5d, 0xf9c74f, 0xfff3b0]),
    ("Ink & paper", &[0x000000, 0x404040, 0x808080, 0xbfbfbf, 0xffffff]),
];

pub fn palette(name: &str) -> Vec<Color32> {
    PALETTES
        .iter()
        .find(|p| p.0 == name)
        .or(PALETTES.first())
        .map(|p| p.1.iter().map(|&v| Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)).collect())
        .unwrap_or_default()
}

pub const MAX_PALETTE: usize = 48;
pub const MAX_RECENT: usize = 10;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub preset: String,
    pub theme: Theme,
    pub roundness: f32,
    pub outline: f32,
    pub shadow: f32,
    pub ui_scale: f32,
    pub text_size: f32,
    /// UI outlines boil along with the canvas.
    pub wobbly_ui: bool,
    pub reduce_motion: bool,
    pub show_labels: bool,
    /// Tools on the right, panels on the left.
    pub swap_sides: bool,
    pub panel_width: f32,
    pub backdrop: Backdrop,
    pub checker_a: Color32,
    pub checker_b: Color32,
    pub pixel_grid: bool,
    pub brush_cursor: bool,
    pub palette: Vec<Color32>,
    pub recent: Vec<Color32>,
    pub keys: Vec<(Action, Binding)>,
    pub tools: Tools,
    pub export_scale: u8,
    pub new_w: usize,
    pub new_h: usize,
    pub confirm_delete: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            preset: "Bubblegum".into(),
            theme: Theme::default(),
            roundness: 10.0,
            outline: 2.0,
            shadow: 4.0,
            ui_scale: 1.0,
            text_size: 13.0,
            wobbly_ui: true,
            reduce_motion: false,
            show_labels: false,
            swap_sides: false,
            panel_width: 270.0,
            backdrop: Backdrop::Dots,
            checker_a: Color32::WHITE,
            checker_b: Color32::from_rgb(0xdc, 0xd9, 0xe6),
            pixel_grid: true,
            brush_cursor: true,
            palette: palette("Wobble"),
            recent: Vec::new(),
            keys: default_keys(),
            tools: Tools::default(),
            export_scale: 1,
            new_w: 640,
            new_h: 480,
            confirm_delete: true,
        }
    }
}

impl Settings {
    /// Repair anything out of range (settings are read from disk and may be hand-edited).
    pub fn sanitize(&mut self) {
        let f = |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        self.roundness = f(self.roundness, 0.0, 24.0, 10.0);
        self.outline = f(self.outline, 0.5, 5.0, 2.0);
        self.shadow = f(self.shadow, 0.0, 10.0, 4.0);
        self.ui_scale = f(self.ui_scale, 0.6, 2.5, 1.0);
        self.text_size = f(self.text_size, 9.0, 22.0, 13.0);
        self.panel_width = f(self.panel_width, 200.0, 480.0, 270.0);
        self.palette.truncate(MAX_PALETTE);
        if self.palette.is_empty() {
            self.palette = palette("Wobble");
        }
        self.recent.truncate(MAX_RECENT);
        self.export_scale = self.export_scale.clamp(1, 8);
        self.new_w = self.new_w.clamp(crate::pixels::MIN_SIDE, crate::pixels::MAX_SIDE);
        self.new_h = self.new_h.clamp(crate::pixels::MIN_SIDE, crate::pixels::MAX_SIDE);
        let t = &mut self.tools;
        t.size = if t.size.is_finite() { t.size.clamp(1.0, crate::model::MAX_SIZE) } else { 6.0 };
        t.stabilizer = t.stabilizer.min(10);
        t.fill.grow = t.fill.grow.min(crate::fill::MAX_GROW);
        if t.brush == Brush::Eraser && t.last_brush == Brush::Eraser {
            t.last_brush = Brush::Marker;
        }
    }

    pub fn remember_color(&mut self, c: Color32) {
        self.recent.retain(|x| *x != c);
        self.recent.insert(0, c);
        self.recent.truncate(MAX_RECENT);
    }

    pub fn action_for(&self, key: Key, m: egui::Modifiers) -> Option<Action> {
        self.keys.iter().find(|(_, b)| b.matches(key, m)).map(|(a, _)| *a)
    }

    pub fn bindings_for(&self, a: Action) -> Vec<Binding> {
        self.keys.iter().filter(|(x, _)| *x == a).map(|(_, b)| *b).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_survive_a_round_trip_and_garbage() {
        let s = Settings::default();
        let j = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&j).unwrap();
        assert_eq!(back, s);
        let mut odd: Settings = serde_json::from_str(r#"{"ui_scale":1e9,"palette":[],"tools":{"size":-3,"stabilizer":250}}"#).unwrap();
        odd.sanitize();
        assert_eq!(odd.ui_scale, 2.5);
        assert!(!odd.palette.is_empty());
        assert_eq!(odd.tools.size, 1.0);
        assert_eq!(odd.tools.stabilizer, 10);
    }

    #[test]
    fn every_action_has_a_default_key_and_no_duplicates() {
        let keys = default_keys();
        for a in Action::ALL {
            assert!(keys.iter().any(|(x, _)| *x == a), "{a:?}");
        }
        for (i, (_, b)) in keys.iter().enumerate() {
            assert!(!keys.iter().skip(i + 1).any(|(_, c)| c == b), "{b:?} bound twice");
        }
        let s = Settings::default();
        assert_eq!(s.action_for(Key::Z, egui::Modifiers::COMMAND), Some(Action::Undo));
        assert_eq!(s.action_for(Key::B, egui::Modifiers::NONE), Some(Action::Brush));
    }

    #[test]
    fn recent_colours_dedupe_and_cap() {
        let mut s = Settings::default();
        for i in 0..20u8 {
            s.remember_color(Color32::from_gray(i));
        }
        s.remember_color(Color32::from_gray(15));
        assert_eq!(s.recent.len(), MAX_RECENT);
        assert_eq!(s.recent[0], Color32::from_gray(15));
        assert_eq!(s.recent.iter().filter(|c| **c == Color32::from_gray(15)).count(), 1);
    }
}
