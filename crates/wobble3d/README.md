# wobbleworks-3d

Feather-style 3D curve drawing with boiling lines, as a library with no UI. The WobbleWorks app
(`apps/wobbleworks/src/space3d`) is one front end; tests and agents drive it the same way.

```rust
use wobbleworks_3d::editor::Editor;
use serde_json::json;

let mut ed = Editor::new();
ed.set_viewport(1280.0, 800.0);
ed.run("camera.view", &json!({"view": "front"}))?;
ed.run("guide.draw", &json!({"points": [[500, 400], [640, 300], [780, 400]]}))?;   // a 3D Guide
ed.run("camera.view", &json!({"view": "right"}))?;
ed.run("stroke.draw", &json!({"points": [[640, 300, 0.5], [640, 500, 1.0]]}))?;   // draw on it
let frame = ed.render(0, true);          // depth-sorted screen triangles for boil frame 0
```

| Module | |
|---|---|
| `editor` | `Editor`: state, tools, pointer events, sessions, undo, `run(id, params)`, `commands()` |
| `model` | the note: curves, brushes, groups, resources, environment, boil, presets, shots |
| `camera` | turntable camera, perfect views, projection and rays |
| `guide` | 3D Guides: drawn, bent, lofted, primitives; raycasts |
| `assist` | stabiliser, Draw Shape recognition, mirror |
| `transform` | Blender-style modal transforms and the gizmo |
| `ops` | picking, erase / vacuum, isolation, area select, liquify, duplicates |
| `render`, `texture`, `raster` | tessellation, the procedural brush atlas, CPU fill |
| `io`, `keys` | .wob3d, OBJ, GLB; the Blender keymap |

Docs: `docs/wobbleworks-3d.md`.
