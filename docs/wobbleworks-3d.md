# WobbleWorks 3D: Feather-style drawing in 3D

The second mode of WobbleWorks (the **3D** button in the dock). The goal: **1:1 with Feather
(Draw in 3D)**, driven like **Blender** on a PC, with WobbleWorks' **boiling lines** and
**painterly brushes** (the Metaphor: ReFantazio painted look) on top.

Sources: the Feather docs (support.feather.art, fetched 2026-10-09, every page), the paper
*Feather: 3D sketchbook light as a feather* (Kim, Hong, Yang, SIGGRAPH '23 Appy Hour,
doi:10.1145/3588427.3595355), and App Store reviews of Feather (iPad) for what to do better.
Clean-room: behaviour from the docs, paper and observation only.

## How it is built (and why the UI can change)

| Part | Where | What |
|---|---|---|
| Core | `crates/wobble3d` (`wobbleworks-3d`) | No UI and no workspace deps. Note model, camera, guides, drawing aids, tools, Blender-style transforms, renderer (screen triangles), texture atlas, CPU rasteriser, files, keymap, undo. **Every action is a command** (`Editor::run(id, params)`, ~100 ids; `Editor::commands()`). |
| UI | `apps/wobbleworks/src/space3d/` | Presentation only: panels, input mapping, egui meshes. Swapping the UI means rewriting this folder; nothing else changes. |
| Shell | `apps/wobbleworks/src/shell.rs` | The 3D toggle, colour strip → 3D brush, pen pressure (PhotoCraft's stylus), sounds / Wob / juice from editor events, saved settings, file drops. |

Rendering: the core tessellates the note for a camera and boil frame into screen triangles
(`render.rs`, curves built on every core). Each vertex carries the depth of the surface it was
drawn on at its own screen position (reverse-Z, `near / depth`) nudged by drawing order, and a
"solid" flag. The view (`space3d/gpu.rs`, wgpu: native and WebGL2) draws solid paint with a
depth buffer, then soft edges and see-through paint blended back to front, so **curves on a
shared surface layer in drawing order** (Feather's reviews: colours bled through each other).
`raster.rs` does the same two passes on the CPU for exports and tests; without wgpu the view
falls back to depth-sorted egui meshes.

## Feather feature map

Status: **done**, **partial** (works, details differ), **open**.

| Feather | Status | Notes |
|---|---|---|
| Turntable orbit, pan, zoom, perfect views (6), persp/ortho, FOV 10–500 mm, orbit point pin/unpin/reset | done | Blender mouse + keys; axis-ball navigator; Alt+middle click = Feather's double tap |
| 3D Guide: draw (extruded along the view), bend (sweep along a stroke from another view, repeatable), orange start edge, section lines, opacity (never fully opaque), close / save / recall | done | Paper's tube → doughnut example is a test |
| Loft (curves in order, tension), primitives (cube, pyramid, sphere, tube; segments) | done | + plane primitive |
| Draw on guide, on image resources (bounded flat guide), on OBJ models | done | + drawing in the air (toggle) |
| Brush: type, colour, size 1–300 mm, opacity, pressure, injector, eyedropper, presets | done | 10 brush types (Feather's names are undocumented) |
| Materials: Shadeless, Shaded, Glow, Cutout; patterns Dot, Line, Cross, Terrazzo, Stippled (intensity, angle, contrast) | done | Shown in Render mode |
| Assistance: Mirror X/Y/Z any combination, Draw Shape (line / curve / circle, hold to adjust, press-hold-drag circle), Stable Stroke | done | |
| Erase (centre-line points) / Vacuum (whole curves); guides isolate what they cover | done | |
| Selection: drag-select, deselect, resources; duplicate in place / by view / by mirror; delete | done | + click, box, circle, lasso, invert (Blender) |
| Transform | done (Blender) | No joystick, by request: G / R / S modal, axis & plane constraints, typed numbers, snapping, gizmo |
| Liquify: push, pinch, comb; size (screen), range, strength; undo all, compare, apply | done | |
| Stage: groups (add above active, rename, visibility, isolate, select, duplicate, merge, reorder, move curves in), resources (three-state cube, rename, delete, opacity), environment (axes, grid, background colour and image, fog, lighting dir/colour/strength/from view, ground shadow, toon; glow, DOF, grain, pixelation, bloom) | done | DOF, grain, pixelation and bloom show in exports (the live view shows the rest) |
| Sequence: shots, play (0.5/1/2×; once / loop / swing), thirds grid, camera info | done | |
| Stamp, Find Group, Lighten, import a note as groups | done | |
| Export: PNG (1–4×, transparent), GIF (boil, 360° turntable), OBJ, glTF (.glb, vertex colours) | done | MP4 open (needs an encoder) |
| Clipboard (reference board) | done (WobbleWorks) | The colour card's Reference tab: load an image, pick colours from it |
| AR, Publish to Gallery | open | Platform services, out of scope for a desktop/web app |
| Keyboard shortcuts | done (Blender) | Rebindable in the Keys tab |

## Better than Feather (from its App Store reviews)

- Tooltips on every control, with the shortcut.
- A clickable axis navigator and view menu instead of unreliable double-tap snapping.
- Blender-style transforms with per-axis scale and exact numbers instead of the joystick.
- Draw Shape corrects only in its own tool: plain Draw never fights a deliberate line.
- Left-handed layout; Ctrl+Z / right-click cancels; undo history panel with jumps.
- Animation: every line boils; painterly brushes shimmer per frame; shots fly the camera.
- Layered painting on a guide works (later strokes cover earlier ones exactly), and a Fill
  tool (missing in Feather) fills closed curves with strokes in any brush.
- Brush types are shown as rendered samples, not icons.
- Never crash: every command is fuzzed with hostile params (`every_command_survives_hostile_params`).

## Painterly brushes (the Metaphor look without geometry nodes)

Oil, Gouache, Dry brush, Chalk and Ink use procedural alpha textures (`texture.rs`) baked on
demand per setting: ragged edges, bristle streaks, dry-brush gaps, grain, taper. **Layers** paint
the stroke several times with small offsets (built-up paint), **Echo** paints it again behind,
offset and recoloured (the cut-paper shadow of Metaphor's text boxes), and every boil frame picks
another texture variant, so the paint itself shimmers. "Metaphor look" in the brush panel sets
all of it at once.

## Tests and looking at it

```sh
cargo test -p wobbleworks-3d                     # core: 63 tests incl. a hostile-params hunt
cargo test -p wobbleworks --test three_d         # real egui input: draw, G X 2 Enter, orbit, guides
cargo run -p wobbleworks-3d --example render -- out_dir [render|swatches]   # PNGs, no window
WOBBLE3D_SNAPSHOT=shot.png cargo test -p wobbleworks --test three_d snapshot -- --ignored
```

The screenshot needs a GPU or `mesa-vulkan-drivers`; Linux builds need `libasound2-dev`.

## Progress log

- **2026-10-09, stage A (core):** `crates/wobble3d` with camera, guides (drawn / bent / lofted /
  primitives), model, renderer, atlas, rasteriser, drawing aids, Blender transforms + gizmo,
  ops (pick, erase, vacuum, isolation, liquify, duplicates), files (.wob3d, OBJ in/out, GLB),
  keymap, editor with ~100 commands. Fixed on the way: atlas UVs went stale when rows were added
  mid-frame (now normalised after the frame), AA fringe folded inward on one side.
- **2026-10-09, stage B (UI):** `space3d` in WobbleWorks: header menus, toolbar, Feather brush
  panel, context bar, sidebar (Stage / Boil / Shots / Item / History / Keys / Help), popups
  (Shift+A, X, Ctrl+M, M, `, F3, F, Shift+F, F2, right-click), navigator, exports, platform
  file dialogs (rfd native, picker + download on web), drops. PhotoCraft's shortcuts are held
  back while 3D shows. Native and wasm build; clippy clean.
- **2026-10-09, stage C:** painterly look (jagged frayed edges, dry-brush ends, scattered dabs
  re-rolled per boil frame, colour jitter; "Metaphor look" and "Brushstrokes" presets), Fill,
  speed (screen-space simplification, parallel build, cached noise, per-frame picture cache:
  3000 curves × 150 points from ~430 ms to ~70 ms a frame on 4 cores; a still boiling view
  costs nothing), depth-buffered GPU view with drawing order on shared surfaces, background
  image, depth of field, Stamp, Find Group, Lighten, import note, brush previews, Shots overlays.
- **2026-10-09, autosave:** the 3D note is autosaved with the app settings (eframe storage, every
  30 s and on exit, only when it changed) and comes back on the next start.
- **Still open:** DOF / grain / pixelation / bloom in the live view (exports have them); MP4;
  Cutout showing the background *image* (it shows the background colour); 3D commands over
  the control channel / MCP (they run in-process through `Editor::run` today); Feather's
  undocumented brush type names.
