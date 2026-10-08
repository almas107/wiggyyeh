//! WobbleWorks: a cute, wiggly drawing app built on PhotoCraft.
//!
//! The app wraps PhotoCraft's own editor ([`photocraft_ui_egui::PhotocraftApp`], which owns the
//! engine [`photocraft_engine::Session`]): every action is a PhotoCraft command dispatched by id,
//! and every PhotoCraft feature stays reachable through the "Advanced editor" mode, which shows
//! PhotoCraft's full menus and panels. The simple mode shows only the canvas and WobbleWorks' own
//! bar. See `docs/wobbleworks-spec.md` for the plan and progress.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod anim;
pub mod audio;
pub mod colour;
pub mod handdrawn;
pub mod io;
pub mod juice;
pub mod mascot;
pub mod pixfont;
pub mod rough;
pub mod shell;
pub mod svgicon;
pub mod theme;
pub mod ttf;
pub mod widgets;
pub mod wob;

pub use shell::WobbleApp;
