//! WobbleWorks 3D: Feather-style 3D curve drawing where every line can boil.
//!
//! This crate is the whole 3D feature without any UI: the note (curves, groups, guides,
//! resources, environment), the turntable camera, 3D guides (drawn, bent, lofted, primitives),
//! drawing aids (stabiliser, shape correction, mirror), selection and transforms, liquify, the
//! boil, tessellation into screen triangles, a CPU rasteriser for exports, files and undo.
//! Every action is a command with an id ([`editor::Editor::run`]), so any UI (the egui one in
//! `apps/wobbleworks`, a future one, tests, agents) drives it the same way.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod assist;
pub mod camera;
pub mod guide;
pub mod math;
pub mod model;
pub mod noise;
pub mod raster;
pub mod render;
pub mod texture;
pub mod transform;
