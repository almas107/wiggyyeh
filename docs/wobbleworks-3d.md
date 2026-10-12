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
| Brush: type, colour, size 1–300 mm, opacity, pressure, injector, eyedropper, presets | done | 12 types incl. Feather's flat ("highlighter"), ribbon (stands off the guide), square and diamond (real swept prisms), tapered first brush (pen + taper) |
| Materials: Shadeless, Shaded, Glow, Cutout; patterns Dot, Line, Cross, Terrazzo, Stippled (intensity, angle, contrast) | done | Shown in Render mode; Cutout shows the background image (or colour) |
| Assistance: Mirror X/Y/Z any combination, Draw Shape (line / curve / circle, hold to adjust, press-hold-drag circle), Stable Stroke | done | + Grease Pencil's pressure curve and post-processing; Shift toggles Stable Stroke |
| Erase (centre-line points) / Vacuum (whole curves); guides isolate what they cover | done | |
| Selection: drag-select, deselect, resources; duplicate in place / by view / by mirror; delete | done | + click, box, circle, lasso, invert (Blender) |
| Transform | done (Blender) | No joystick, by request: G / R / S modal, axis & plane constraints, typed numbers, snapping, gizmo (drags confirm on release); drag selected things or primitives directly; the joystick's wider / taller as `transform.stretch` (screen axes, thickness kept) |
| Focus mode (the colour icon on the group panel) | done | Active group a black silhouette, others grey, flat (as the group tutorial shows) |
| Liquify: push, pinch, comb; size (screen), range, strength; undo all, compare, apply | done | |
| Stage: groups (add above active, rename, visibility, isolate, select, duplicate, merge, reorder, move curves in), resources (three-state cube, rename, delete, opacity), environment (axes, grid, background colour and image, fog, lighting dir/colour/strength/from view, ground shadow, toon; glow, DOF, grain, pixelation, bloom) | done | DOF, grain, pixelation and bloom show live (GPU post pass) and in exports |
| Sequence: shots, play (0.5/1/2×; once / loop / swing), thirds grid, camera info | done | |
| Stamp, Find Group, Lighten, import a note as groups | done | |
| Export: PNG (1–4×, transparent), GIF and MP4 (boil, 360° turntable, shots fly-through), OBJ, glTF (.glb, vertex colours) | done | MP4 is AV1 (rav1e, pure Rust; own ISO BMFF muxer), encoded off the UI thread on desktop |
| Clipboard (reference board) | done (WobbleWorks) | The colour card's Reference tab: load an image, pick colours from it |
| AR, Publish to Gallery | open | Platform services, out of scope for a desktop/web app |
| Keyboard shortcuts | done (Blender) | Rebindable in the Keys tab |
| (WobbleWorks) Agents | done | `w3d.*` control methods, MCP via `control_call` |

## What the tutorials taught (2026-10-12)

Sources: Feather's own videos, supplied by the user as three zips (basic tutorials 1–3, advanced
tutorials 1–5, ten "How to" clips): transcribed with Whisper and lined up with frames taken at each
line's timestamp (captioned contact sheets), so what is said and what the hand does are read together.

| Behaviour seen | WobbleWorks |
|---|---|
| A guide is a translucent sheet extruded along the view from the drawn line; strokes land on it; closing it leaves the strokes | same (`guide.draw`) |
| Draw Shape: near-straight → straight at once; hold → the end follows the pen, the start fixed; circle hold → radius, centre fixed; press-hold-slide → circle from its centre; oval → oval, hold resizes; curve hold → curvature | same in Draw Shape, and now in plain Draw on a hold (QuickShape), plus rectangles and polygons |
| Painting = big flat strokes laid as straight lines over each other on one guide (cassette, 15:52) | works without the colour fights Feather users report: the renderer layers paint on a surface in drawing order, also on curved guides |
| 2D joystick: taller / longer / wider handles along the screen, 15° rotation about the screen centre, lock mode; scaled strokes keep their thickness | `transform.stretch` keeps thickness; a uniform scale now scales it (`scaleThickness`), the advanced tutorial calls the kept thickness "inconvenient" |
| 3D joystick: axis cones, arcs, a centre sphere that rotates like a trackball, about the selection's centre | Blender gizmo (move / rotate / scale / all), R R trackball |
| Group panel: long-press selects a group's strokes (green), arrow icon moves the selected strokes in, eye hides, colour icon = focus | click / Ctrl+click, "Move the selected curves here", eye, Focus button |
| Brushes named in the videos: first (tapered), second (wide), third, highlighter (flat), ribbon, square / cube, diamond | ribbon and diamond added; square and diamond are real prisms (the orange button is a lit box) |
| One-finger double tap → nearest perfect view; three-finger double tap → ortho | Alt+middle click, double right-click, `5`, the navigator |
| Clipboard: floating reference window, opacity, lock, colour picker, "import to stage" | the colour card's Reference tab; Add › image resource |
| Shadows drawn by hand (duplicate group, darker, offset, low opacity) | Render mode ground shadows, or the same trick |

## From the YouTube comments (436 comments on the tutorial playlists)

| Asked for | Done |
|---|---|
| Paint colours glitch through each other / "clipping" when layering (the most repeated complaint) | depth prepass + drawing order on the nearest surface (`SURFACE_TOL`), flat tape takes the surface depth where the boil moves it; tested on planes and spheres |
| Paint bucket / fill closed loops / fill layer | Fill tool (bucket, fills under its lines), Fill Selection, solid primitives |
| A lasso tool to colour | Lasso Fill |
| Straight lines by default, hold to straighten, clean shapes | QuickShape in Draw (line, rectangle, polygon, circle, ellipse, arc); Shapes tool (rect, ellipse, polygon, star, line; filled or outline) |
| Snap to the ends of existing lines | `snap.set {ends}` (on by default) |
| Pick the axis to draw on instead of drawing a guide | `guide.plane` floor / wall / side / view |
| Auto-generate guides from existing strokes, extrude lines | `guide.fromCurve` |
| Grid opacity slider | `env.set {gridOpacity}` |
| Two-finger tap undo | two-finger tap undo, three-finger redo |
| Snap to the six views, ortho / perspective buttons on Windows | navigator, View menu, numpad keys |
| Clipping masks (this request) | `group.clip`, per-group link toggle |
| Not done: Android/Windows ports (n/a: native + web already), nested groups, mechanism simulation, measurements, frame animation beyond boil + shots, a lineart post effect | |

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
cargo test -p wobbleworks-3d                     # core: 78 tests incl. a hostile-params hunt
cargo test -p wobbleworks --test three_d         # real egui input: draw, G X 2 Enter, orbit, guides
cargo run -p wobbleworks-3d --example render -- out_dir [render|swatches]   # PNGs, no window
WOBBLE3D_SNAPSHOT=shot.png cargo test -p wobbleworks --test three_d snapshot -- --ignored
# also: WOBBLE3D_RENDER=1, WOBBLE3D_CUTOUT=1, WOBBLE3D_EFFECTS='{"dof": 1.4, "bloom": 0.6, "grain": 0.4}' 
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
- **2026-10-09, stage D:** Render mode effects in the live view (a GPU post pass reading the
  depth buffer: DOF, bloom, block-averaged pixelation, grain), sized like the exports; Cutout
  paint shows the background image (`Tex::Cutout`, CPU and GPU). The WGSL is validated with
  naga, including its WebGL2 GLSL translation (it caught a depth-texture read WebGL2 can't do).
- **2026-10-09, control:** `wobbleworks --control <port>` (PhotoCraft's protocol, token and
  workspace rules) with the 3D mode's `w3d.show / commands / execute / state / render`
  (`space3d/control.rs`); other methods go to PhotoCraft unchanged. MCP reaches them through
  `control_call` in bridge mode (`docs/control-protocol.md`).
- **2026-10-09, video:** MP4 export (AV1 via rav1e, muxer in `space3d/mp4.rs`, moov first,
  full-range BT.709): boil (looped to 4 s), 120-step turntable at 30 fps, and the shots
  fly-through Feather plays (also as GIF). Checked with ffprobe/ffmpeg: 480 × 320, 120 frames,
  exactly 30 fps, decodes clean. Tiles cut encoding from 9.1 s to 4.6 s on 4 cores (dev build).
- **2026-10-09, stroke feel (Grease Pencil):** `feel.rs` with Blender's documented draw-brush
  settings: pressure curve (power + lightest touch), post-processing when the pen lifts
  (Smooth + iterations, Smooth Thickness + iterations, Subdivide, adaptive Simplify, Trim
  Strokes End), Shift while drawing toggles Stable Stroke (GP's Shift-LMB). Runs on screen
  samples before projection, so curves stay on their guide. `feel.set` / `feel.get`; Stroke
  feel card in the brush panel; saved with the settings.
- **2026-10-12, Feather behaviours and user requests:** see the two sections above. Renderer:
  a depth prepass of solid paint pushed back by `SURFACE_TOL` (3e-3 of the depth), then paint
  in drawing order on the nearest surface, then guides only where clearly in front (CPU raster
  and wgpu alike); clipping masks (mask pass, up to four bases per view); fills are strokes with
  triangles (`fill.rs`). Editor: Fill, Lasso Fill, Shapes tools, QuickShape, drag-to-move,
  orbit about the pointer, right-drag orbit, axis planes, guide from curve, end snapping, focus,
  stretch, ribbon / diamond / prism brushes, the showcase (`showcase.rs`, File › Open the
  showcase). 94 core tests, 54 app tests; screenshot of the showcase through lavapipe matches
  the CPU raster. The dark-mode button in PhotoCraft's title bar now switches WobbleWorks'
  dark and light looks.
- **2026-10-12, the advanced tutorial (frames + transcript, not transcript alone):** glow is
  additive light now (overlapping low-opacity glow strokes build up to white, as in Feather's
  bioluminescent creature) and soft (halo-row strips, faded caps); the background colour lights
  the scene (ambient 0.18 + 0.3 × background); selections are a flat mint, as Feather draws
  them; depth of field blurs to round-ish bokeh; `edit.adjustColor` shifts hue / saturation /
  brightness / opacity of the selection by the same amount; OBJ / glTF export writes a fill's
  own triangles instead of a tube round its outline. `deepsea.rs` rebuilds the tutorial's scene
  one group per technique (`file.showcase {name: "deepsea"}`). 96 core tests.
- **2026-10-12, fills, guides and Sequence (checked against the videos' frames):**
  - The eraser cuts a round hole in a fill (conforming splits along its rim, no cracks).
  - Fills cast ground shadows in Render mode (clipped where they enter the floor; paint lying
    on the floor casts none).
  - Painterly fills (oil, gouache, dry brush, chalk, ink) are brushed: the surface is sliced
    into strokes a brush-width apart, lit like the patch.
  - A fill stops at a curved guide's edge instead of growing fins off it.
  - Drawn and bent guides are covered in fine lines along the sweep with a few rings, as in
    Feather's bend tutorial; the guide / bend stroke previews as a thin dashed line.
  - Sequence bar (View › Sequence): thumbnails of every shot (click to go, drag to reorder,
    rename, update, delete, + for this view), and a Play tab with previous / play / next and a
    timeline that scrubs the camera (`sequence.at`, `shot.rename`, `shot.thumbnail`).
  - 103 core tests, 55 app tests.
- **Still open:** see "Gaps for any project" below; the egui-mesh fallback (no GPU) draws
  paint in drawing order without depth; more than four clipping bases in one view leave the
  rest unclipped; Feather's start line is green, ours orange; side-by-side feel tuning with
  real users (defaults: smooth 0.35 × 1, thickness 0.3 × 1).

## Gaps for any project (audit of every tutorial, frame sheet and comment, 2026-10-12)

What a user would miss on most projects, most important first. S/M/L is the size of the work.

**Workflow-blocking**
1. New / Open / Showcase replace the note without asking and clear undo; one autosave slot.
   Feather has a notes home with many notes. (S for a confirm + backup, M for a library.)
2. Mirrored halves are not linked: selecting one side does not select its twin, so one move
   can't push both halves apart symmetrically (Basic 3 3:36). (M)
3. OBJ / glTF export makes every curve a round tube: flat, ribbon, square and diamond lose
   their shape; opacity and glow are lost (no alphaMode / emissive). (M)
4. No floating clipboard reference window to trace over (Basic 2 0:31, Clipboard video);
   at most two images in the scene, so no front / side / top blueprint set. (M)
5. Touch screens: one finger always draws; no finger orbit, three-finger lens slide or
   pen-vs-finger split (Navigate, Draw with Finger). (M)

**Features**
- Export to JPG; record the drawing live to video; choose the output size. (S, M, S)
- Pick several groups in the panel to select all their curves; duplicates land above their
  source, selected (the duplicate-recolour-nudge shadow trick). (S)
- Stable Stroke preview pad; copy / paste between notes; depth of field amount per shot. (S)

**Feel**
- No brush-size preview on the canvas while sliding the size. (S)
- Web: large notes may silently fail to autosave (browser storage quota). (S to detect)

**Asked for in the comments, not done**
- Lock the view; snap strokes to angles; move the mirror plane (S each).
- Edit a finished stroke's end points; record the drawing process (M).
- Volumetric fog / airbrush fill, frame animation, nested groups, line-art effect, text,
  measurements, colour smudging (L each).

