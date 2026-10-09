//! The keymap: Blender's shortcuts by default (most PC users know them), every binding
//! rebindable. A binding names a command and its params, so any UI dispatches keys the same
//! way, and tooltips show the shortcut next to each action. `ui.*` commands are handled by the
//! shell (menus, panels, search), the rest by [`crate::editor::Editor::run`].
//!
//! Mouse navigation is Blender's too: middle drag orbits, Shift+middle pans, Ctrl+middle or the
//! wheel zooms, and Alt+middle click snaps to the nearest view (Feather's double tap).

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    /// Unique action name.
    pub action: String,
    pub label: String,
    /// "Ctrl+Shift+Alt+Key"; empty = unbound.
    pub chord: String,
    pub command: String,
    #[serde(default)]
    pub params: Value,
}

/// (action, label, chord, command, params JSON).
const DEFAULTS: &[(&str, &str, &str, &str, &str)] = &[
    // Views (numpad, as in Blender).
    ("view.front", "Front view", "Numpad1", "camera.view", r#"{"view":"front"}"#),
    ("view.back", "Back view", "Ctrl+Numpad1", "camera.view", r#"{"view":"back"}"#),
    ("view.right", "Right view", "Numpad3", "camera.view", r#"{"view":"right"}"#),
    ("view.left", "Left view", "Ctrl+Numpad3", "camera.view", r#"{"view":"left"}"#),
    ("view.top", "Top view", "Numpad7", "camera.view", r#"{"view":"top"}"#),
    ("view.bottom", "Bottom view", "Ctrl+Numpad7", "camera.view", r#"{"view":"bottom"}"#),
    ("view.nearest", "Snap to nearest view", "Alt+Numpad5", "camera.view", r#"{"view":"nearest"}"#),
    ("view.projection", "Perspective / orthographic", "Numpad5", "camera.toggleProjection", "null"),
    ("view.orbitLeft", "Orbit left", "Numpad4", "camera.orbit", r#"{"dx":-43,"dy":0}"#),
    ("view.orbitRight", "Orbit right", "Numpad6", "camera.orbit", r#"{"dx":43,"dy":0}"#),
    ("view.orbitUp", "Orbit up", "Numpad8", "camera.orbit", r#"{"dx":0,"dy":43}"#),
    ("view.orbitDown", "Orbit down", "Numpad2", "camera.orbit", r#"{"dx":0,"dy":-43}"#),
    ("view.zoomIn", "Zoom in", "NumpadPlus", "camera.zoom", r#"{"factor":1.2}"#),
    ("view.zoomOut", "Zoom out", "NumpadMinus", "camera.zoom", r#"{"factor":0.8333}"#),
    ("view.frameSelected", "Frame selected", "NumpadPeriod", "camera.frameSelected", "null"),
    ("view.frameAll", "Frame all", "Home", "camera.frameAll", "null"),
    ("view.reset", "Reset view", "Shift+Home", "camera.reset", "null"),
    ("view.pie", "View menu", "`", "ui.viewPie", "null"),
    // Modes and tools.
    ("mode.toggle", "Draw / Select mode", "Tab", "tool.toggleMode", "null"),
    ("tool.draw", "Draw (again: Draw Shape)", "D", "tool.set", r#"{"tool":"draw","cycle":true}"#),
    ("tool.erase", "Erase (again: Vacuum)", "E", "tool.set", r#"{"tool":"erase","cycle":true}"#),
    ("tool.select", "Select (again: next select tool)", "W", "tool.set", r#"{"tool":"select","cycle":true}"#),
    ("tool.box", "Box select", "B", "tool.set", r#"{"tool":"select","mode":"box"}"#),
    ("tool.circle", "Circle select", "C", "tool.set", r#"{"tool":"select","mode":"circle"}"#),
    ("tool.guide", "Draw 3D Guide", "Q", "tool.set", r#"{"tool":"guide"}"#),
    ("tool.liquify", "Liquify", "K", "tool.set", r#"{"tool":"liquify"}"#),
    ("tool.injector", "Injector (sample a whole brush)", "I", "tool.set", r#"{"tool":"injector"}"#),
    ("tool.eyedropper", "Eyedropper (sample a colour)", "Shift+I", "tool.set", r#"{"tool":"eyedropper"}"#),
    // Selection.
    ("select.all", "Select all", "A", "select.all", "null"),
    ("select.none", "Select none", "Alt+A", "select.none", "null"),
    ("select.invert", "Invert selection", "Ctrl+I", "select.invert", "null"),
    ("select.linked", "Select the group under the mouse", "L", "select.linkedUnderMouse", "null"),
    // Transform (Blender's G / R / S).
    ("transform.grab", "Move", "G", "transform.start", r#"{"mode":"grab"}"#),
    ("transform.rotate", "Rotate", "R", "transform.start", r#"{"mode":"rotate"}"#),
    ("transform.scale", "Scale", "S", "transform.start", r#"{"mode":"scale"}"#),
    ("edit.flip", "Mirror selection", "Ctrl+M", "ui.flipMenu", "null"),
    ("edit.delete", "Delete", "X", "ui.deleteMenu", "null"),
    ("edit.deleteNow", "Delete", "Delete", "edit.delete", "null"),
    ("edit.duplicate", "Duplicate and move", "Shift+D", "edit.duplicate", r#"{"mode":"move"}"#),
    ("edit.duplicateView", "Duplicate symmetrically by view", "Ctrl+Shift+D", "edit.duplicate", r#"{"mode":"view"}"#),
    ("edit.duplicateMirror", "Duplicate by mirror", "Shift+Alt+D", "edit.duplicate", r#"{"mode":"mirror"}"#),
    ("edit.undo", "Undo", "Ctrl+Z", "edit.undo", "null"),
    ("edit.redo", "Redo", "Ctrl+Shift+Z", "edit.redo", "null"),
    ("edit.redo2", "Redo", "Ctrl+Y", "edit.redo", "null"),
    // Groups (Blender's collections).
    ("group.hide", "Hide the active group", "H", "group.hideActive", "null"),
    ("group.unhide", "Show every group", "Alt+H", "group.showAll", "null"),
    ("group.isolate", "Show only the active group", "Shift+H", "group.isolateActive", "null"),
    ("group.moveTo", "Move to group", "M", "ui.moveToGroup", "null"),
    ("group.newFromSelection", "New group from selection", "Ctrl+G", "group.fromSelection", "null"),
    // Guides.
    ("guide.close", "Close 3D Guide", "Escape", "guide.close", "null"),
    ("guide.bend", "Bend 3D Guide", "Ctrl+B", "tool.set", r#"{"tool":"bend"}"#),
    ("guide.save", "Save 3D Guide", "Ctrl+Shift+G", "guide.save", "null"),
    ("guide.recall", "Recall recent guide", "Ctrl+Alt+G", "guide.recall", "null"),
    ("add.menu", "Add (guides, images, models)", "Shift+A", "ui.addMenu", "null"),
    // Brush.
    ("brush.sizeDown", "Smaller brush", "[", "brush.nudge", r#"{"size":-1}"#),
    ("brush.sizeUp", "Bigger brush", "]", "brush.nudge", r#"{"size":1}"#),
    ("brush.sizeRadial", "Brush size (drag)", "F", "ui.radialSize", "null"),
    ("brush.opacityRadial", "Brush opacity (drag)", "Shift+F", "ui.radialOpacity", "null"),
    ("brush.opacityDown", "Less opaque", "-", "brush.nudge", r#"{"opacity":-1}"#),
    ("brush.opacityUp", "More opaque", "=", "brush.nudge", r#"{"opacity":1}"#),
    ("brush.next", "Next brush type", "Shift+B", "brush.cycle", "null"),
    ("brush.opacity10", "Opacity 10%", "1", "brush.set", r#"{"opacity":0.1}"#),
    ("brush.opacity20", "Opacity 20%", "2", "brush.set", r#"{"opacity":0.2}"#),
    ("brush.opacity30", "Opacity 30%", "3", "brush.set", r#"{"opacity":0.3}"#),
    ("brush.opacity40", "Opacity 40%", "4", "brush.set", r#"{"opacity":0.4}"#),
    ("brush.opacity50", "Opacity 50%", "5", "brush.set", r#"{"opacity":0.5}"#),
    ("brush.opacity60", "Opacity 60%", "6", "brush.set", r#"{"opacity":0.6}"#),
    ("brush.opacity70", "Opacity 70%", "7", "brush.set", r#"{"opacity":0.7}"#),
    ("brush.opacity80", "Opacity 80%", "8", "brush.set", r#"{"opacity":0.8}"#),
    ("brush.opacity90", "Opacity 90%", "9", "brush.set", r#"{"opacity":0.9}"#),
    ("brush.opacity100", "Opacity 100%", "0", "brush.set", r#"{"opacity":1.0}"#),
    ("mirror.toggle", "Mirror on / off", "Shift+X", "mirror.toggle", "null"),
    // Boil and playback.
    ("boil.play", "Boil on / off", "Space", "boil.toggle", "null"),
    // Files and the shell.
    ("file.save", "Save", "Ctrl+S", "ui.save", "null"),
    ("file.saveAs", "Save as", "Ctrl+Shift+S", "ui.saveAs", "null"),
    ("file.open", "Open", "Ctrl+O", "ui.open", "null"),
    ("file.render", "Render image", "F12", "ui.renderImage", "null"),
    ("file.renderAnim", "Render boil animation", "Ctrl+F12", "ui.renderAnimation", "null"),
    ("ui.search", "Search commands", "F3", "ui.search", "null"),
    ("ui.rename", "Rename", "F2", "ui.rename", "null"),
    ("ui.sidebar", "Sidebar", "N", "ui.sidebar", "null"),
    ("ui.toolbar", "Toolbar", "T", "ui.toolbar", "null"),
    ("ui.hide", "Hide the UI", "Ctrl+Space", "ui.hideUi", "null"),
    ("ui.render", "Render mode", "Z", "env.toggleRender", "null"),
];

pub fn defaults() -> Vec<Binding> {
    DEFAULTS
        .iter()
        .map(|(action, label, chord, command, params)| Binding {
            action: (*action).into(),
            label: (*label).into(),
            chord: (*chord).into(),
            command: (*command).into(),
            params: serde_json::from_str(params).unwrap_or(Value::Null),
        })
        .collect()
}

/// Normalise a chord: modifiers in the order Ctrl, Shift, Alt; the key last.
pub fn normalize(chord: &str) -> String {
    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    let mut key = String::new();
    for part in chord.split('+') {
        let p = part.trim();
        match p.to_ascii_lowercase().as_str() {
            "ctrl" | "cmd" | "control" | "command" => ctrl = true,
            "shift" => shift = true,
            "alt" | "opt" | "option" => alt = true,
            "" => {}
            _ => key = p.to_string(),
        }
    }
    // A bare "+" key ("Ctrl++") splits to an empty last part.
    if key.is_empty() && chord.ends_with('+') && chord.len() > 1 {
        key = "+".into();
    }
    if key.chars().count() == 1 {
        key = key.to_uppercase();
    }
    let mut out = String::new();
    if ctrl {
        out.push_str("Ctrl+");
    }
    if shift {
        out.push_str("Shift+");
    }
    if alt {
        out.push_str("Alt+");
    }
    out.push_str(&key);
    out
}

/// A keymap with lookup both ways.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keymap {
    pub bindings: Vec<Binding>,
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap { bindings: defaults() }
    }
}

impl Keymap {
    /// The binding for a pressed chord.
    pub fn lookup(&self, chord: &str) -> Option<&Binding> {
        let c = normalize(chord);
        if c.is_empty() {
            return None;
        }
        self.bindings.iter().find(|b| !b.chord.is_empty() && normalize(&b.chord) == c)
    }

    /// The chord bound to an action (for tooltips: "Move (G)").
    pub fn chord(&self, action: &str) -> Option<&str> {
        self.bindings.iter().find(|b| b.action == action && !b.chord.is_empty()).map(|b| b.chord.as_str())
    }

    /// The chord of the first binding running a command (any params).
    pub fn chord_for_command(&self, command: &str) -> Option<&str> {
        self.bindings.iter().find(|b| b.command == command && !b.chord.is_empty()).map(|b| b.chord.as_str())
    }

    /// Rebind an action; any other action on the same chord is unbound (and returned).
    pub fn rebind(&mut self, action: &str, chord: &str) -> Result<Option<String>, String> {
        let c = normalize(chord);
        if !self.bindings.iter().any(|b| b.action == action) {
            return Err(format!("no action {action:?}"));
        }
        let mut freed = None;
        if !c.is_empty() {
            for b in &mut self.bindings {
                if b.action != action && normalize(&b.chord) == c {
                    b.chord.clear();
                    freed = Some(b.action.clone());
                }
            }
        }
        for b in &mut self.bindings {
            if b.action == action {
                b.chord = c.clone();
            }
        }
        Ok(freed)
    }

    /// Add any default actions a saved keymap lacks (new versions add actions).
    pub fn merge_defaults(&mut self) {
        for d in defaults() {
            if !self.bindings.iter().any(|b| b.action == d.action) {
                let taken = self.bindings.iter().any(|b| normalize(&b.chord) == normalize(&d.chord));
                self.bindings.push(Binding { chord: if taken { String::new() } else { d.chord }, ..d });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_unique_and_parse() {
        let k = Keymap::default();
        let mut seen = std::collections::HashSet::new();
        for b in &k.bindings {
            assert!(seen.insert(normalize(&b.chord)), "duplicate chord {}", b.chord);
            assert!(!b.command.is_empty());
        }
        assert_eq!(k.lookup("g").map(|b| b.action.as_str()), Some("transform.grab"));
        assert_eq!(k.lookup("shift+ctrl+z").map(|b| b.action.as_str()), Some("edit.redo"));
        assert_eq!(k.chord("view.front"), Some("Numpad1"));
        assert!(k.lookup("").is_none());
        assert_eq!(normalize("alt+shift+cmd+d"), "Ctrl+Shift+Alt+D");
    }

    #[test]
    fn rebinding_moves_a_chord() {
        let mut k = Keymap::default();
        let freed = k.rebind("transform.grab", "R").expect("rebind");
        assert_eq!(freed.as_deref(), Some("transform.rotate"));
        assert_eq!(k.lookup("R").map(|b| b.action.as_str()), Some("transform.grab"));
        assert!(k.rebind("nope", "Q").is_err());
        k.bindings.retain(|b| b.action != "view.top");
        k.merge_defaults();
        assert_eq!(k.chord("view.top"), Some("Numpad7"));
    }
}
