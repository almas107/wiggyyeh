# WobbleWorks

A cute drawing app where every line boils. WobbleWorks takes the idea of
[WigglyPaint](https://internet-janitor.itch.io/wigglypaint) and builds it in Rust: each stroke
is drawn a few times with tiny differences, and the drawings play in a loop so everything wiggles.

It is written in Rust only: a native egui/eframe app on wgpu, and the same code compiled to
WebAssembly for the browser. There is no JavaScript.

## Run it

```sh
# Desktop
cargo run --release -p wobbleworks

# Browser (needs trunk: `cargo install trunk`)
cd apps/wobbleworks
trunk serve --release        # http://127.0.0.1:8766
trunk build --release        # static site in dist/wobbleworks
```

## What it does

**Drawing**
- 13 brushes, each with its own wobble: Marker, Dither, Fuzz, Shaky, Rowdy, Sketch, Ribbon,
  Spray, Beads, Chalk (grain that shimmers), Nib (slanted calligraphy), Blob fill (draw a loop
  and it fills in), Steady. Plus an Eraser.
- Five tip shapes: round, square, diamond, star and heart.
- Shape tools: Line, Box and Oval draw with the current brush (Shift snaps). Blob fill with Box
  or Oval gives a solid wobbly shape.
- Symmetry: left/right, top/bottom, four-way, and a six-way kaleidoscope.
- Steady-hand smoothing, and pen pressure (touch force, or pen pressure in the browser).
- Fill with tolerance and grow, worked out per frame so fills boil with their outlines. You can
  sample all layers or just the current one.
- Lasso: pick something up, then move, scale, turn, flip, recolour, stamp copies, apply or
  delete it. Move with nothing selected lifts the whole layer. Arrow keys nudge.
- Pick colours from the canvas. Use the colour picker or type a hex code. Palettes are
  editable, with presets (Wobble, Pastel, Pico-8, Game Boy, Sunset, Ink & paper), and recent
  colours are remembered.

**Animation**
- Wiggle amount (0–400%), speed, 2–8 frames, and playback modes: Loop, Ping-pong or Jumble.
- Pause, step one frame, or click a frame dot to hold that frame.

**Layers**
- Up to 64 layers, each with visibility, opacity, blend mode (Normal, Multiply, Screen, Add,
  Darken, Lighten), alpha lock and clipping.
- Rename, duplicate (the copy boils on its own), merge down, clear and delete. Reorder by
  dragging or with the arrow buttons.

**Files**
- Projects autosave about 1.5 s after a change. Each project has two slots, so a crash or power
  cut mid-save can't lose both. Autosaves missing from the project list are recovered on start.
- `.wob` import and export. Files from the original Wobbleworks web page open unchanged and boil
  exactly as before, because the same noise is used bit for bit.
- Export a PNG of the current frame, an animated GIF, or a sprite sheet, at 1–4× with crisp
  pixels. Import pictures by button or by dropping them on the window.

**Make it yours (Settings)**
- 7 themes (Bubblegum, Mint Choc, Lemonade, Grape Soda, Peach Fuzz, Midnight Snack, Arcade),
  or set all 8 colours yourself.
- Roundness, outline width, shadow depth, text size and UI scale.
- "Boiling UI": button outlines wiggle with the drawing. Reduce motion keeps the UI still.
- Tool names under icons, tools on either side, panel width, and cards that fold away.
- Backdrop pattern (polka dots, graph paper, candy stripes, plain), checkerboard colours,
  pixel grid and brush cursor.
- Every keyboard shortcut can be changed (Settings → Keys).

## How it stays fast

- Strokes are kept as vectors, and each layer is rendered once per frame and cached. A cache is
  rebuilt only when that layer's content or the wiggle changes. Native builds render frames in
  parallel.
- Playback re-renders nothing: each frame is already composited and uploaded as its own GPU
  texture, so a tick just shows a different texture.
- While you draw, only the new part of the stroke is stamped. The incremental render is tested
  to match a full render exactly. Only the touched rectangle is recomposited and uploaded.
- Floating selections are drawn through a transformed GPU quad, so dragging and turning them
  costs no CPU.
- Undo snapshots share stroke lists and pixels through `Arc`s, so they are nearly free.
- Autosave caches each layer's PNG encoding and only re-encodes layers that changed.

## Code map

| File | What |
|---|---|
| `model.rs` | Document, layers, strokes, brushes, tips |
| `brush.rs` | Noise, stamps and stroke rendering (incremental) |
| `render.rs` | Layer caches, compositing, GPU upload |
| `fill.rs`, `select.rs`, `geom.rs` | Fill, floating selections, paths and shapes |
| `history.rs` | Undo/redo |
| `project.rs`, `store.rs` | `.wob` format, PNG/GIF export, crash-safe autosave |
| `app.rs`, `canvas.rs`, `panels.rs` | App state and actions, canvas input, panels and dialogs |
| `theme.rs`, `widgets.rs`, `icons.rs`, `settings.rs` | The cartoon look, widgets, icons, preferences |
| `platform.rs` | Desktop/browser differences (files, clock, pen) |

## Tests

```sh
cargo test -p wobbleworks
# Offscreen screenshot (needs a GPU or a software Vulkan driver such as lavapipe):
WOBBLE_SNAPSHOT=shot.png cargo test -p wobbleworks snapshot -- --ignored
```
