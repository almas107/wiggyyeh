# Tracing 3D in 2D: Clip Studio Paint's 3D layer, mapped onto PhotoCraft

What Clip Studio Paint (CSP) does with 3D models on a 2D canvas, what we already have, and the
design for bridging WobbleWorks 3D to the PhotoCraft canvas.

The one-line version: **the bridge is small, because the camera and the renderer already do the
hard part.** `wobbleworks_3d::camera` is a complete projective camera, and
`space3d::export::picture` already renders the note through *an arbitrary camera* while putting
the live one back. What is missing is a place in the document to keep a 3D scene, and a surface
for it to render into.

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

> **Superseded by section 7.2.** The `CameraLink` enum below was the first design. CSP turns out
> to keep a camera *list* on the scene and a `MainCameraID` on the layer, which is strictly better
> and also settles where the note is stored. Read 7.2 for the model being built; this section is
> kept because the reasoning about *why* pinning matters still holds.

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

### Resolved

- **Alpha convention — settled, and no conversion is needed.** Both sides are *straight*
  (unassociated) alpha, so `Picture.rgba` can go into a `Surface` verbatim.
  - `crates/wobble3d/src/raster.rs:259` divides the alpha back out when it emits the picture
    (`un = |v| v / a`), so the premultiplied float canvas it composites in does not leak into the
    output.
  - `crates/raster/src/lib.rs` `to_rgba` / `from_rgba` carry alpha as an independent channel and
    never multiply or divide RGB by it.
  - There is therefore no dark-fringing risk on soft 3D edges, which was the worry.
- **A size mismatch panics, so the bridge must validate.**
  `Surface::write_interleaved` (`crates/raster/src/lib.rs:464`) asserts
  `bytes.len() == w * h * bpp`. `trace3d` must check the rasterised picture's length against the
  target rect before handing it over, rather than trusting it.
- **Document storage** — answered by CSP's own model; see 7.2. The note is stored once with a
  camera list, and each layer names a camera by id.

### Still open

- **Undo.** 3D edits are commands in `Editor`'s own history (`editor::Editor::run`); 2D edits are
  in PhotoCraft's. A camera pin is a *document* edit and belongs in PhotoCraft's history. Whether
  the two histories stay separate (simplest, and it matches having two modes) or merge is a
  product call. This is the one genuine unknown left, and it is a product decision rather than a
  technical one.
- **Off-centre frustum.** Ours derives a symmetric frustum from `focal_mm`; CSP stores six planes
  (7.3). Only matters for a shifted / tilt-shift camera, so it is deferred, not solved.

## 7. Verified against the CSP binary (2026-10-13)

Evidence: 14,591 MSVC RTTI type descriptors recovered from `CLIPStudioPaint.exe` (90 MB), plus
`kSerializeLabel*` identifiers. CSP is **not stripped of RTTI**, so its class names are readable
directly. No decompilation was needed for any claim below.

### 7.1 CSP's four layers, and what they correspond to here

CSP's internal codenames:

| CSP module | Role | Our counterpart |
|---|---|---|
| **`C3DCore`** | The 3D engine. Pure data + maths, no UI: `SMLCamera`, `SMLModelData`, `SMLMeshData`, `SMLMeshGroup`, `SMLMeshItem`, `SMLBone`, `SMLSkin`, `SMLHalfEdge`, `SMLMaterial`, `SMLPart`, `SMLPartsList`, `C3DMaterial`, `C3DTextureInfo`, `C3DRoomParts`, and a `C3DIK*` inverse-kinematics family for posing figures | `crates/wobble3d` |
| **`Planeswalker`** | App framework and document model: `PW3DLayerDoCommand`, `PW3DLayerUtility`, `PW3DRenderSettingDialogBox`, `PWRender`, `PWIO3DCore` (`PWIO3DMaterial`, `PWIO3DTextureInfo`) | `photocraft-doc` + `photocraft-engine` |
| **`Odyssey`** | Serialisation and presets: `ODDPresetCamera`, `ODDPresetSceneCamera`, `ODICatalogAngle`, `ODDCatalogInfoAngle`, `ODDPresetPose`, `ODCCameraMouseControllerT<Move\|Rotate\|Target>` | `crates/format` + `crates/io` |
| **`Urza`** | UI: `URUX3DLayerEditController`, `URUX3DLayerEditView`, `URUX3DLayerEditCommandExecutor`, `URUX3DCameraAnglePresetSelectView`, `URNewLayer3DDessindollDialogBox` | `apps/wobbleworks/src/space3d` |

The split is the same one this design proposes, which is reassuring: a UI-free 3D engine, a
document model that owns the layer, a serialisation layer for cameras and poses, and a separate UI.
`PW3DLayerUtility` is the direct analogue of the proposed `crates/trace3d`.

### 7.2 A 3D layer owns a camera — confirmed

Two independent pieces of evidence:

- `UR3DLayerMainCameraCollapsedErrorFormatter`, alongside `UR3DLayerSceneCollapsedErrorFormatter`
  and `UR3DLayerSceneNodeLostErrorFormatter`.
- `kSerializeLabelMainCameraID` and `kSerializeLabelCameraList`.

So a CSP 3D layer persists a **camera list** plus a **main camera ID** pointing into it. This
confirms section 4.1 (the layer stores its own camera) and **supersedes section 4.2's
`CameraLink`**: rather than three link modes, CSP keeps one camera list on the scene and has each
layer name a camera by id.

That is the better model, and it resolves the third open question in section 6. Revised:

```rust
/// On the note: cameras live here, once each.
pub struct CameraList { pub cameras: Vec<(u32, TraceCamera)> }

/// On the layer: which camera this layer is traced through.
pub struct Model3dLayer {
    pub note: Arc<Vec<u8>>,
    pub main_camera_id: u32,   // CSP's MainCameraID
    pub follow_viewport: bool, // true = Live; false = pinned to main_camera_id
    pub cache: Surface,
    pub dirty: bool,
}
```

Several pinned angles of one model then cost one camera each instead of a duplicated note, and
"Shoot" becomes *append a camera to the list and point this layer at it*. `Shot(u32)` collapses
into the same mechanism, since the note's `Sequence` is already a list of cameras.

### 7.3 CSP's camera stores more than ours

From the `ODDPresetCamera` serialise labels:

`Position`, `Rotation`, `TargetDistance`, `Twist`, `UpGuide`, `IsOrthographic`, `RotationPivot`,
`BillboardRotation`, and a full frustum: `FrustumLeft/Right/Top/Bottom/Near/Far/Origin`
(plus `FrustumStock` on `ODDPresetSceneCamera`).

Two real gaps against `wobbleworks_3d::camera::Camera`:

1. **No twist (roll).** CSP persists `Twist` and an explicit `UpGuide`; our camera derives `up`
   from yaw/pitch and cannot roll. A traced camera that cannot match a tilted reference angle is a
   genuine limitation — adding a `twist: f32` applied to `right`/`up` in `Camera::view()` is small
   and should go in at step 2, before `TraceCamera` is serialised, to avoid a format change later.
2. **Frustum versus focal length.** CSP stores six frustum planes; we store `focal_mm` and derive
   a symmetric frustum. Ours is a strict subset — it cannot express an *off-centre* frustum, which
   is what a shifted/tilt-shift camera needs. Worth knowing before claiming camera parity; not
   needed for step 1–4.

`RotationPivot` corroborates `Camera::orbit_about(pivot, …)`, which already exists here.

### 7.4 LT conversion: 2D and 3D lines are separate code paths

The dialog classes are:

- `URNewLTConvertLayerDialogBox` — the container
- `URNewLTConvertLayer3DLineView` — **3D Lines**
- `URNewLTConvertLayer2DLineView` — **Texture Lines** (2D/flat-image treatment)
- `URNewLTConvertLayerToneView` — **Tone**
- `URNewLTConvertLayerCategorySelector` / `…SelectorList` — the preset categories
- `UREditLTConvertPresetController`, `LTConvertPreset` — user-editable presets

This directly confirms the three presets, and more usefully it confirms that **CSP implements 2D
line and 3D line extraction as two different views over two different code paths**, not one shared
edge detector with a flag.

That validates the split in section 4.5: *3D Lines* sourced from geometry and *Texture Lines*
sourced from the raster are genuinely different operations, and implementing them separately is
following CSP's own architecture rather than diverging from it.

It also means the plan to emit *3D Lines* as **vector** paths is a clean improvement at the point
where CSP is weakest, without touching the *Texture Lines* path at all.

### 7.5 Unchanged by this evidence

Sections 1–5 stand. The `picture_at` + `trace3d` + `Model3d` + Shoot sequence is unaffected except
for the camera-id refinement in 7.2 and the `twist` field in 7.3, both of which land in steps 1–3.

## 8. Implementation status (2026-10-13)

Steps 1–4 and 9 are in the tree and green (8.3). `cargo test -p wobbleworks-3d --lib` passes
**110 tests**, the four twist tests below among them.

### 8.1 Done: camera twist (roll)

The gap found in 7.3. Done first and deliberately, because `twist` has to exist *before* anything
serialises a camera for a document — adding it later would be a file-format change.

`crates/wobble3d/src/camera.rs`:

- `Camera::twist: f32`, degrees, `#[serde(default)]`.
- Applied in `Camera::view()` as a quaternion roll about the view axis, reusing
  `math::Quat::from_axis_angle`:

  ```rust
  if self.twist.is_finite() && self.twist != 0.0 {
      let roll = Quat::from_axis_angle(back, self.twist.to_radians());
      right = roll.rotate(right).normalized();
      up = roll.rotate(up).normalized();
  }
  ```

  `back` is deliberately left alone, so the eye position, every depth and all six perfect views are
  bit-identical to before at `twist == 0`.
- `sanitize()` wraps it to −180..=180 and falls back to 0 when non-finite.

`crates/wobble3d/src/sequence.rs`: `shot_camera` now interpolates `twist`. Since both `yaw` and
`twist` are angles needing shortest-arc interpolation, the ad-hoc yaw wrap was factored into one
`lerp_angle` closure used by both — a leg from +170° to −170° crosses ±180 the short way instead of
swinging 340° the wrong way.

Tests added:

| Test | What it pins down |
|---|---|
| `twist_rolls_the_screen_axes_without_moving_the_eye` | 180° of roll reflects the picture exactly through the screen centre; eye, view axis and depth unchanged; `project`/`unproject_at` still invert each other at 0/30/−115/180° in both perspective and orthographic; `right`,`up`,`back` stay a right-handed orthonormal frame |
| `twist_is_wrapped_and_survives_hostile_numbers` | 540° wraps to 180°; `NaN` falls back to upright |
| `a_note_saved_before_twist_existed_loads_upright` | A camera JSON with no `twist` key still deserialises, at 0 — the back-compatibility guarantee, asserted rather than assumed |
| `twist_flies_between_shots_the_short_way_too` | Shot interpolation crosses ±180 the short way |

### 8.2 Done: `picture_at` (step 1)

`apps/wobbleworks/src/space3d/export.rs`:

```rust
pub fn picture_at(ed: &mut Editor, camera: &Camera, frame: u32,
                  width: u32, height: u32, transparent: bool) -> Result<Picture, String>
```

Renders at an exact pixel size instead of a 1–4× multiple of the 3D pane, which is what a document
of arbitrary size needs. It keeps the existing save/restore contract (`ed.camera` is put back, and
`ed.scene` is only read) and adds a `cam.sanitize()` so a hostile camera cannot reach the renderer.

`picture` is now a thin wrapper that computes `width`/`height` from `scale` and delegates, so there
is exactly one implementation of the render path. Its signature is unchanged and all five existing
call sites (`png`, the boil/turntable/shots exporters) are untouched.

### 8.3 Done: steps 2, 3, 4 and 9 (2026-10-13)

Steps 1–4 (the whole user-visible request) and step 9 are in, on 7.2's camera-list model.
In WobbleWorks: **View › Trace in 2D › Shoot / Shoot live / Shoot from a saved camera**, with
Fit (Height / Width / All) and a **Roll** slider for twist.

**`crates/trace3d`** (`wobbleworks-trace3d`, layer 2 in `xtask/src/layers.rs`): the render path
moved here as planned (`picture_at`; `export::picture_at` delegates, so there is one copy), plus
`fit_camera`, `layer_camera`, `to_surface` (length-checked, see section 6), `render_surface`
(sides over `MAX_SIDE` render at 8192 and scale up), `snapshot`, `new_layer`, `refresh_layer` and
`render_saved` (re-renders a layer from its own saved note in a scratch editor, with nothing live
open). 7 tests, including `tracing_never_touches_the_note`.

**`LayerContent::Model3d(Model3dLayer)`** in `photocraft-doc`:

```rust
pub struct Model3dLayer {
    pub scene: Arc<Vec<u8>>,   // the .wob3d note, opaque to doc; shared by the note's layers
    pub scene_id: u64,         // which note (Scene::uid), so its layers can be kept in step
    pub main_camera_id: u64,   // CSP's MainCameraID: an id in the note's camera list
    pub follow_view: bool,     // follow the note's view camera instead
    pub fit: Model3dFit,       // Height / Width / Contain (4.3)
    pub cache: Surface,        // the last render: what everything else composites
    pub dirty: bool,
}
```

- **The camera list is the note's camera shots.** As 7.2 predicted, `Shot(u32)` collapsed into
  the same mechanism: "Shoot" appends the view to the shots as "Trace N" (one 3D undo step;
  `shot.add` now takes a name) and points the layer at it. The shot strip is the camera list's
  editor for free: re-aim or rename a camera there and its layers follow; delete it and its
  layers keep their last picture, marked `dirty`.
- **The note is stored once** (open question 3): every layer of a note shares one
  `Arc<Vec<u8>>`, and `.pcraft` stores blobs by content hash, so one copy per file however many
  layers (tested with incompressible bytes). `Scene::uid` (new, `#[serde(default)]`) ties
  layers to their note; `Editor::note_uid` stamps it into the undo and redo snapshots too, so
  undoing in 3D can't orphan layers.
- **Compositing** reads `cache` through `Layer::surface()` (CPU and GPU paths alike);
  proxies downsample it; it is not paintable (`surface_mut` is `None`). **Layer › Rasterize › 3D**
  (`layer.rasterize.3d`, new menu item, live) turns it into pixels. Moving the layer moves its
  render; the next re-render frames it by its camera again.
- **Keeping layers current:** leaving the 3D mode re-renders the open note's layers: all of them
  when the note changed, only `follow_view` ones when just the view moved; all of them get the
  newest note bytes. Going to 3D with a 3D layer active opens its note on the layer's camera
  (CSP's double-click), refused if the note open now is kept nowhere else.
- **Files:** `.pcraft` has a `Model3d` manifest variant (scene as a blob). **PSD (step 9):** the
  cache is written as the layer's pixels, and a private `wb3D` block holds the header (ids,
  follow, fit) and the note; import turns it back into a 3D layer, and a damaged block leaves
  plain pixels (`crates/io/src/model3d_map.rs`, `crates/io/tests/model3d.rs`).
- **Agents:** `w3d.shoot {camera: "new" | "view" | id, fit}`, `w3d.retrace {layer?}`,
  `w3d.edit {layer}`, `camera.roll {to | by}`, `camera.set {twist}`.

Still open: the interactive capped-size render while a live layer follows the camera (4.4:
today live layers update when you leave 3D), LT conversion (4.5; the RTTI's `C3DRidgeRender`
and `C3DLTRenderOffscreen` suggest CSP's *3D Lines* render ridges from geometry, as planned),
Paint on 3D (4.6), snapping and the derived perspective ruler (4.7), an off-centre frustum
(6), a twist *tool* on the canvas like `C3DTwistTool` (the Roll slider and `camera.roll` do it
by number), and undo across the two histories (6: they stay separate; a Shoot is one step in
each).
