//! Undo and redo. Snapshots share stroke lists and raster frames with the live document through
//! `Arc`s, so a snapshot costs a few pointers per layer, not a copy of the pixels.

use egui::Color32;

use crate::model::{Doc, Layer};

pub const MAX_UNDO: usize = 100;

#[derive(Clone)]
pub struct Snapshot {
    layers: Vec<Layer>,
    current: usize,
    w: usize,
    h: usize,
    frames: usize,
    bg: Color32,
    transparent: bool,
}

impl Snapshot {
    pub fn of(d: &Doc) -> Self {
        Snapshot { layers: d.layers.clone(), current: d.current, w: d.w, h: d.h, frames: d.frames, bg: d.bg, transparent: d.transparent }
    }

    fn restore(self, d: &mut Doc) {
        d.layers = self.layers;
        d.current = self.current;
        d.w = self.w;
        d.h = self.h;
        d.frames = self.frames;
        d.bg = self.bg;
        d.transparent = self.transparent;
        d.normalize();
    }
}

#[derive(Default)]
pub struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

impl History {
    /// Record the state before a change.
    pub fn push(&mut self, d: &Doc) {
        self.undo.push(Snapshot::of(d));
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Forget the last recorded state (the change it was taken for turned out to do nothing).
    pub fn discard_last(&mut self) {
        self.undo.pop();
    }

    pub fn undo(&mut self, d: &mut Doc) -> bool {
        let Some(s) = self.undo.pop() else { return false };
        self.redo.push(Snapshot::of(d));
        s.restore(d);
        true
    }

    pub fn redo(&mut self, d: &mut Doc) -> bool {
        let Some(s) = self.redo.pop() else { return false };
        self.undo.push(Snapshot::of(d));
        s.restore(d);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_redo_walks_both_ways() {
        let mut d = Doc::new(32, 32);
        let mut h = History::default();
        assert!(!h.undo(&mut d));
        h.push(&d);
        d.layers.push(Layer::new("two", 3));
        d.current = 1;
        h.push(&d);
        d.resize(64, 48);
        assert!(h.undo(&mut d));
        assert_eq!((d.w, d.h, d.layers.len()), (32, 32, 2));
        assert!(h.undo(&mut d));
        assert_eq!((d.layers.len(), d.current), (1, 0));
        assert!(h.redo(&mut d));
        assert!(h.redo(&mut d));
        assert_eq!(d.w, 64);
        assert!(!h.redo(&mut d));
    }

    #[test]
    fn history_is_capped() {
        let mut d = Doc::new(16, 16);
        let mut h = History::default();
        for _ in 0..(MAX_UNDO + 20) {
            h.push(&d);
        }
        let mut n = 0;
        while h.undo(&mut d) {
            n += 1;
        }
        assert_eq!(n, MAX_UNDO);
    }
}
