# Tracing 3D in 2D: Clip Studio Paint's 3D layer, mapped onto PhotoCraft

What Clip Studio Paint (CSP) does with 3D models on a 2D canvas, what we already have, and the
design for bridging WobbleWorks 3D to the PhotoCraft canvas.

The one-line version: **the bridge is small, because the camera and the renderer already do the
hard part.** `wobbleworks_3d::camera` is a complete projective camera, and
`space3d::export::picture` already renders the note through *an arbitrary camera* while putting
the live one back. What is missing is a place in the document to keep a 3D scene, and a surface
for it to render into.

## Status (2026-10-13)

Steps 1, 2 and 4 of section 5 have landed, and step 3 in a lighter form. In the app:
**View › Trace in 2D › Shoot (pinned)**, **Shoot live**, **Shoot from a camera shot**, with a
Fit choice (Height / Width / All). Shoot renders the view at the picture's size into a dimmed
(50%), pixel-locked layer named "3D trace (…)", adds an empty "Trace" layer above it (made
active), switches to 2D, and is one undo step. Leaving the 3D mode shoots the trace layers again
when the note changed (all of them) or the camera moved (live and shot layers only). Agents:
`w3d.shoot {link, shot?, fit?}` and `w3d.retrace {layer?}` (`docs/control-protocol.md`).

Where it differs from the design below, and why:

- **No `crates/trace3d`.** The bridge is `apps/wobbleworks/src/space3d/trace.rs`: the app is its
  only user and already depends on both sides, so a crate would add a layering entry and nothing
  else. It can move into a crate unchanged if PhotoCraft itself ever traces 3D.
- **No `LayerContent::Model3d` yet.** A trace layer is a pixel layer that carries its
  `TraceCamera` (link and fit included) as JSON in a private layer block, `wb3T`. `.pcraft`
  and PSD keep unknown blocks verbatim, so the camera survives saves; other apps see the picture
  (which is section 5's step 9 for free). The note itself is not embedded: the layer refers to
  the one 3D note that WobbleWorks keeps (and autosaves). This avoids touching `doc`, `format`,
  `compose` and `gpu` for a variant whose only extra power today would be re-rendering, which
  the refresh-on-leave already does.
- **Alpha** (open question 1): PhotoCraft surfaces are straight alpha, like `Picture`, so the
  RGBA8 bytes go in unchanged (`Surface::from_interleaved`, then `convert` to the picture's
  format).
- **Undo** (open question 2): the histories stay separate. Shooting and refreshing are steps in
  PhotoCraft's history; the 3D note's history is untouched (tracing never writes to it:
  `tracing_never_touches_the_note`).
- **Size:** `export::picture_at` renders at any size up to `MAX_SIDE` (8192); larger pictures
  are rendered at 8192 on the long side and scaled up.

Still open: the interactive (capped-size) render while a live layer follows the camera (4.4:
today live layers refresh when you leave 3D, not while orbiting), LT conversion (4.5), Paint on
3D (4.6), snapping to projected geometry and the derived perspective ruler (4.7), and a real
`Model3d` layer if trace layers ever need to re-render inside PhotoCraft without WobbleWorks.

## 1. What CSP actually does

CSP's 3D is not a modeller. It is a *reference rig*: you park a posed model under your drawing
and draw on top of it. Four distinct features, often confused with each other:

| CSP feature | What it is | Where the work lives |
|---|---|---|
| **3D layer** | A layer whose content is a live 3D scene (models, primitives, drawing figures, camera). Rasterised for display, re-rendered when you move anything. Never flattened until you ask. | Layer model + renderer |
| **Camera / object manipulation** | Two separate gizmos: move the *camera* (orbit / pan / dolly / FOV) or move the *object*. Snapping to canvas centre, grids, guides, rulers. | Camera |
| **Draw on the 3D model** | Strokes land on the model's UV surface (3D texture paint), so they stay stuck to it as the camera turns. | Unproject (2D to 3D) |
| **LT conversion** (EX only) | Converts a 3D layer (or photo) into **L**ine and **T**one layers: outlines, creases, surfaces, shadows and textures become line art plus halftone. Presets: *3D Lines*, *Texture Lines*, *Tone*. | Geometry to vector/raster |

The tracing workflow users actually perform:

1. Drop a 3D model on the canvas, so it becomes a 3D layer.
2. Pose it, then move the **camera** until the angle is right.
3. Drop the 3D layer's opacity, add a raster layer above it, and **trace by hand**; or
4. run **LT conversion** to get the lines mechanically, then clean up.

Both (3) and (4) depend on the same primitive: *the scene, seen through one chosen camera,
rendered at the document's pixel size, without disturbing the scene.*

## 2. What we already have

Three pieces exist, and they are the expensive ones.

**A complete camera** — `crates/wobble3d/src/camera.rs`. `Camera` (orbit target, yaw, pitch,
distance, 10–500 mm lens, ortho/perspective, viewport) and `View` with everything projection
needs:

- `View::project(Vec3) -> Option<Projected>` — world point to screen pixels, with depth and
  pixels-per-world-unit at that depth.
- `View::ray(sx, sy) -> (Vec3, Vec3)` — the ray through a screen point. This is the 2D to 3D
  direction, i.e. CSP's "draw on the model".
- `View::unproject_at(sx, sy, anchor)` — screen point onto the camera-facing plane through an
  anchor.
- `View::world_per_px(depth)` — the scale factor a 2D brush needs to keep a constant world size.

So **no new camera is needed.** The user-visible "add a camera to the 3D mode" is really *"let the
2D side name and pin a camera"* — see section 4.2.

**A renderer that already takes a camera by value** — `apps/wobbleworks/src/space3d/export.rs`:

```rust
pub fn picture(ed: &mut Editor, camera: &Camera, frame: u32, scale: u32, transparent: bool)
    -> Result<Picture, String>
```

Its body is the whole non-destructive contract, already written down:

```rust
let saved = ed.camera;
ed.camera = cam;
let f = ed.render(frame, false);
ed.camera = saved;      // the 3D viewport is put back, untouched
```

`ed.scene` is only read. This is exactly the "without changing whatever is on the 3D canvas"
requirement, and it is already satisfied.

**A CPU rasteriser with a matching pixel format** — `crates/wobble3d/src/raster.rs` yields
`Picture { width, height, rgba: Vec<u8> }` in *straight* RGBA8, and
`photocraft_raster::Surface::from_interleaved(PixelFormat::RGBA8, rect, bytes)` consumes exactly
that. The 3D output drops into a PhotoCraft layer in one call.

**And one process.** WobbleWorks *is* PhotoCraft (`apps/wobbleworks/README.md`): the same
`Session`, command registry, document model and file formats, with the 3D mode as a dock button.
No IPC, no second document — the bridge is a function call.

## 3. The gap

`LayerContent` (`crates/doc/src/lib.rs:431`) has seven variants — Raster, Group, Adjustment, Fill,
Text, Shape, Smart — and **none of them can hold a 3D scene**. `crates/doc` depends only on
`geom`, `color` and `raster`; nothing in PhotoCraft's crates depends on `wobbleworks-3d` (only
`apps/wobbleworks` does).

So there is nowhere to park a live scene in a document, and nothing re-renders when a camera
moves. That is the entire gap. Everything below is about filling it without dragging the 3D crate
into PhotoCraft's core.

This also closes a gap the 3D mode already knows it has — `docs/wobbleworks-3d.md`, gap #4: *"No
floating clipboard reference window to trace over."* A 3D layer under a raster layer is a better
answer than a floating window.

## 4. Design

### 4.1 A 3D layer that keeps `doc` dependency-free

Add an eighth variant. The scene travels as **opaque bytes**, so `crates/doc` never learns about
`wobbleworks-3d`:

```rust
// crates/doc/src/lib.rs
pub enum LayerContent {
    // ...
    Model3d(Model3dLayer),
}

pub struct Model3dLayer {
    /// The live note, serialised (`.wob3d` JSON). `doc` never parses this.
    pub note: Arc<Vec<u8>>,
    /// The camera this layer is traced through. Plain numbers: no 3D types in `doc`.
    pub camera: TraceCamera,
    /// Last render at document size; what compositing and the thumbnail use.
    pub cache: Surface,
    /// Set when `note` or `camera` changed and `cache` is stale.
    pub dirty: bool,
}

/// A camera, in `doc`'s own terms. Mirrors `wobbleworks_3d::camera::Camera`.
pub struct TraceCamera {
    pub target: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub focal_mm: f32,
    pub orthographic: bool,
    /// How the 3D camera is bound to this layer (4.2).
    pub link: CameraLink,
    /// How a 3D aspect is fitted to the document's aspect (4.3).
    pub fit: Fit,
}
```

A new crate, **`crates/trace3d`**, depends on both `wobbleworks-3d` and `photocraft-doc` and owns
every conversion. It is the only place the two worlds meet:

```
photocraft-doc ──┐
                 ├── trace3d ── apps/wobbleworks
wobbleworks-3d ──┘
```

`doc` keeps its current dependencies; `wobble3d` keeps knowing nothing about documents; the
dependency graph stays acyclic and the bridge is one reviewable crate.

### 4.2 The camera link, the heart of the request

> *whatever the camera views in 3d mode we can trace it in 2d, without changing whatever is on the
> 3d canvas*

Three link modes, one enum:

```rust
pub enum CameraLink {
    /// The layer follows the 3D viewport camera. Orbit in 3D, the 2D reference follows.
    Live,
    /// Frozen. `TraceCamera`'s own numbers are used; orbiting in 3D does not touch this layer.
    Pinned,
    /// Follows a saved shot in the note's `Sequence`.
    Shot(u32),
}
```

`Pinned` is the mode the request is really asking for, and it is what makes the two modes
independent: **"Shoot" copies the live 3D camera into the layer and freezes it.** You then keep
orbiting in 3D freely, and the 2D reference holds the angle you chose. Several pinned 3D layers
can sit in one document at different angles off one scene, which is the three-quarter-plus-profile
set an artist actually wants.

`Shot` is free reuse: the note's `Sequence` already stores cameras, `export::shot_camera(seq, t,
viewport)` already resolves one, and `space3d/shotstrip.rs` already draws the strip. A shot *is* a
saved camera, so the shot strip becomes the 2D side's camera picker with no new storage.

Nothing here writes to `ed.scene`. The invariant is worth asserting in a test:

```rust
#[test]
fn tracing_never_touches_the_note() {
    let before = (ed.scene.clone(), ed.camera);
    let _ = trace3d::render_layer(&mut ed, &layer, 2480, 3508)?;
    assert_eq!(ed.scene, before.0);
    assert_eq!(ed.camera, before.1);   // `picture()` already restores it
}
```

### 4.3 Document size and aspect

`export::picture` sizes output as `camera.viewport * scale`, with `scale` clamped to 1–4. A
document is whatever the user made it (A4 at 300 dpi is 2480 x 3508), so add a sibling that takes
the size directly and let the existing one call it:

```rust
pub fn picture_at(ed: &mut Editor, camera: &Camera, frame: u32,
                  width: u32, height: u32, transparent: bool) -> Result<Picture, String>;
```

Same save/restore body, with `cam.viewport = Viewport { width, height }` instead of a scale.
`MAX_SIDE` is 8192, which covers A3 at 600 dpi; above that, render at 8192 and let PhotoCraft
scale.

Changing the viewport aspect changes the framing, because `View::aspect = width / height` while
the vertical FOV is held. That is *fit-height* — correct, but it must be a stated choice, so:

```rust
pub enum Fit {
    /// Keep vertical FOV; the document's width decides how much is seen sideways.
    /// (`View`'s current behaviour, and the default.)
    Height,
    /// Keep horizontal FOV; adjust the lens so the 3D view's width is preserved.
    Width,
    /// Keep everything the 3D viewport showed, letterboxing into the document.
    Contain,
}
```

`Width` and `Contain` are a `focal_mm` adjustment on the copied camera before rendering, so the
renderer does not change.

### 4.4 Re-render, and keeping the UI responsive

On `dirty`, `trace3d` re-renders and replaces `cache`. Two paths, because a 94-megapixel A3 CPU
raster is not a per-frame operation:

- **Interactive** — while the 3D camera moves in `Live` mode, render at a capped size (long edge
  about 2048) and let the compositor scale. Reuses the existing GPU path (`space3d/gpu.rs`) when a
  render state is available.
- **Committed** — on release, and before any LT conversion or export, render at full document size
  through `picture_at`.

`Pinned` layers never re-render from camera motion at all; only an edit to the note dirties them.
This is what makes multiple pinned angles cheap.

### 4.5 LT conversion, where we can beat CSP

CSP's LT conversion is an *image* operation: it rasterises the 3D layer and runs edge detection,
which is why its line quality depends on render resolution and why creases come out broken.

We do not have to do that, because `render()` returns **geometry, not pixels**:
`Frame { batches: Vec<Batch> }` of screen-space triangles carrying depth (`z`), surface flags and
`layer::UNDER / PAINT / OVER`. So the three CSP presets map onto three differently-sourced
implementations:

| CSP preset | Our source | Output |
|---|---|---|
| **3D Lines** | Silhouette and crease edges from the tessellated geometry, projected through `View::project` | **Vector paths** into a `LayerContent::Shape` via `crates/vector`: resolution-independent, closed, editable, and they scale with the document instead of being baked at render size |
| **Texture Lines** | Edge detection on the rasterised `Picture` (the model treated as a flat image) | Raster line layer, matching CSP's behaviour, for textures and image resources where there is no geometry to read |
| **Tone** | Threshold the shaded render into bands, then fill each band with a halftone | Raster or pattern fill. The dot/line/cross/terrazzo/stippled generators already exist in `raster.rs` as *material* patterns and can be reused as *tone* patterns |

CSP's three controls map across directly: *Threshold of fill in black*, *Extract line* and *tone
density* become crease-angle threshold, minimum edge length, and tone band count.

Vector output for *3D Lines* is the real win: CSP users spend their cleanup time fixing raster
lines that LT conversion produced at the wrong resolution.

### 4.6 Drawing back onto the model (CSP's 3D texture paint)

The reverse direction needs no new maths either. A 2D stroke at `(sx, sy)` on a document backed by
a 3D layer:

1. `View::ray(sx, sy)` gives the ray through that document pixel.
2. Intersect it with the note's geometry — `crates/wobble3d/src/ops.rs` already does picking for
   the 3D tools.
3. Add the stroke to the note at the hit, exactly as the 3D mode's own brush does.

And `View::world_per_px(depth)` keeps a 2D brush's apparent size constant as the surface recedes.

This one *does* write to the note, so it must be an explicit tool ("Paint on 3D"), never a side
effect of tracing. Tracing stays strictly read-only.

### 4.7 Snapping and guides

`View::project` gives the 2D canvas every 3D feature's document-space position, so the 2D tools can
snap to projected geometry — CSP's 3D-aware snapping, and a free perspective ruler: project the
scene's three axis vanishing points and hand them to PhotoCraft's existing perspective ruler.

## 5. Order of work

Each step is useful on its own and testable before the next.

| # | Step | Size |
|---|---|---|
| 1 | `picture_at` in `space3d/export.rs`; `picture` calls it. Test: a known camera at two document sizes. | **S** |
| 2 | `crates/trace3d`: `TraceCamera` to/from `Camera`, `Picture` to `Surface`, `Fit`. Test: the no-mutation assertion of 4.2. | **S** |
| 3 | `LayerContent::Model3d` + `Model3dLayer` in `doc`; compositing reads `cache`; thumbnails. | **M** |
| 4 | Camera link UI: "Shoot" (pin), Live toggle, shot picker off the existing shot strip. | **M** |
| 5 | Interactive vs committed render sizes (4.4). | **S** |
| 6 | LT conversion: *Texture Lines* first (raster, simplest), then *Tone*, then *3D Lines* (vector, the valuable one). | **M**, **M**, **L** |
| 7 | Paint on 3D (4.6) as an explicit tool. | **M** |
| 8 | Snapping and the derived perspective ruler (4.7). | **M** |
| 9 | PSD round-trip: a 3D layer has no PSD equivalent, so write the flattened `cache` as a raster layer and keep the note in `Layer::psd_blocks` so our own round-trip stays lossless. | **M** |

Steps 1–4 are the whole user-visible request: *orbit in 3D, press Shoot, trace in 2D, 3D canvas
untouched.*

## 6. Open questions

- **Alpha convention.** `Picture.rgba` is straight RGBA8 and `Surface::from_interleaved` takes
  interleaved RGBA8, but whether `Surface` stores straight or premultiplied alpha needs confirming
  before step 2; a mismatch shows up as dark fringing on soft 3D edges.
- **Undo.** 3D edits are commands in `Editor`'s own history (`editor::Editor::run`); 2D edits are
  in PhotoCraft's. A camera pin is a *document* edit and belongs in PhotoCraft's history. Whether
  the two histories stay separate (simplest, and it matches having two modes) or merge is a
  product call.
- **Document storage.** A `.wob3d` note embedded per layer is simple but duplicates the scene
  across several pinned angles of the same model. One note per document, with layers referencing
  it, is leaner but needs a lifetime story.
