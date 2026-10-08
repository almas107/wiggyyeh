# WobbleWorks: spec for the rebuild

Read this file, `AGENTS.md`, and only the parts of `docs/` you need. Don't re-read the whole repo.

## What the user wants

A **brand-new app**, built **on PhotoCraft**, with:

1. **All of PhotoCraft's features.** Every engine command, tool, panel, menu item and file
   format must be reachable. Build on PhotoCraft's crates (`photocraft-engine` `Session` +
   command registry, `photocraft-io`, `photocraft-compose`/`gpu`, and so on). Never
   reimplement them. Dispatch PhotoCraft commands by id, as the rest of the repo does.
   Coverage target: everything listed in `docs/parity.md` that PhotoCraft has live.
2. **A game-like, polished, cute UI in the spirit of WigglyPaint**, the animated drawing
   program by Internet Janitor (https://internet-janitor.itch.io/wigglypaint).
   - **Licence:** the user says WigglyPaint is MIT licensed. Fetch its licence text and
     confirm the exact terms before reusing anything. If it is MIT, its look, layout and
     assets may be reused. Credit it in `ATTRIBUTION.md` and keep its licence file next to
     any reused asset.
   - Its boiling-line animation (strokes redrawn over a few frames that loop) applies to
     everything you draw.
3. **"Game juice"**, the way polished indie games feel. This is required, not optional.
   - Animated transitions everywhere: panels and dialogs slide or bounce in, tools pop.
   - Squash and stretch on press, hover wobble, easing (no linear snaps), screen shake on
     big actions (clear, merge, delete), particles on stamp, fill, undo and save.
   - **Sound**: synthesized in code (no sound files), with clicks, pops, brush scratch
     sounds that follow speed, and undo/redo blips. Volume and mute go in settings.
     Native: `cpal` (or `rodio`). Web: WebAudio through `web-sys`. Both stay Rust only.
   - **Personality**: a mascot or character that reacts to what you do (idle, cheering,
     surprised on undo, sleepy when idle), cheerful copy, and celebratory moments
     (first stroke, saving, exporting).
   - Responsive: input feedback the same frame; no UI hitches over 16 ms.
4. **Extremely polished.** Consistent spacing, no clipped or overflowing panels, works at
   phone width on web, light and dark themes, reduce-motion and mute options.
5. Native desktop and web (wasm), Rust only, no Tauri or webviews (`AGENTS.md`). Never panic
   on input (`AGENTS.md` › Never crash).

## Layout plan (agree with the user before deviating)

- The main screen looks and feels like WigglyPaint: big canvas, brush strip, palette, a few
  chunky buttons, the mascot.
- PhotoCraft's full feature set opens from it: a juicy menu bar or tool drawer, panels as
  animated pop-out cards, and an "Advanced editor" mode that slides in PhotoCraft's full
  panels and menus, restyled cute. Nothing from PhotoCraft may be unreachable.
- The wiggle brushes and boiling playback must work on PhotoCraft documents (layers, masks,
  blend modes and so on). Export to an animated GIF, a PNG sequence and PSD (per-frame layers
  or groups).

## Reusable pieces from the previous (wrong) attempt

`apps/wobbleworks` is a standalone app that does **not** use PhotoCraft. It was the wrong
deliverable. Mine it, then delete it once the new app replaces it:

- `src/brush.rs`: the wobble noise (bit-exact with the original Wobbleworks), stamps, and
  incremental stroke rendering (tested equal to a full render).
- `src/project.rs`: reading the original `.wob` files (keep import support).
- `src/theme.rs`, `src/widgets.rs`, `src/icons.rs`: the cartoon theme, widgets and icons (a
  starting point only).
- The offscreen screenshot test pattern (`egui_kittest` + wgpu). In this container, install
  `mesa-vulkan-drivers` first, or there is no adapter. Web testing used `wasm-bindgen-cli`
  0.2.129 plus Playwright Chromium (`/opt/pw-browsers`), and works.

## Mistakes made last time: don't repeat them

1. **Misread the request and built for an hour without confirming.** "All the features" and
   "the UI of the project I forked" meant *PhotoCraft's* features, on PhotoCraft. Instead
   the agent built a standalone WigglyPaint clone that shares no PhotoCraft code. When a
   request says "the project I forked", it means build on it. Before any large build,
   restate the plan in 3–5 lines (what it is built on, which features, what the UI looks
   like) and get a yes.
2. **Picked the narrowest reading of an ambiguous scope** ("all the features" = the small
   app's features) without asking. If two readings differ by weeks of work, ask.
3. **Claimed WigglyPaint was not open source** without checking. It is MIT (per the user).
   Verify licences from the source; never assert them from memory.
4. **Spent a huge context on one pass.** The previous session passed 400k tokens. Work in
   stages, commit and push after each, and keep a short progress log in this file
   (below), so a new session can resume cheaply.
5. **No sound, no transitions, no personality.** A "cute" UI with static widgets felt
   dead. Juice and sound are core requirements, not polish to add later.
6. **Layout bug shipped to the first screenshot:** a side panel's content was wider than the
   panel, and egui's `Panel::right` then shifted the panel off-screen. Always set slider
   widths from `ui.available_width()`, and screenshot every screen before calling it done.

## Stages (push working code after each)

1. App crate on `photocraft-engine`: open, draw and save a PSD; register it in
   `xtask/src/layers.rs`; native and web build.
2. Wiggle engine as PhotoCraft tools and commands: boil playback; GIF/PNG-sequence export.
3. WigglyPaint-style main screen plus the theme, with screenshots reviewed.
4. Juice: transitions, easing, particles, shake; then sound (native and web); then the
   mascot.
5. Every PhotoCraft feature reachable (check against `docs/parity.md`), restyled.
6. Polish pass: phone-width web, reduce-motion, mute, perf (`AGENTS.md` › Performance), tests,
   `panic_hunt` for any new commands.

## Progress log

- **Stage 1 (2026-10-08): done.** Plan agreed with the user: wrap PhotoCraft's editor
  (`PhotocraftApp`, which owns the engine `Session`) instead of a separate canvas; the new crate is
  `apps/wobbleworks-app` (renamed to `wobbleworks` when the old app is deleted).
  - `src/shell.rs` `WobbleApp`: its own bar (New, Open, Save PSD, Brush/Eraser, palette, Undo/Redo,
    "Advanced editor") over PhotoCraft's editor. Simple mode = PhotoCraft's `fullScreen` screen mode
    (canvas only); Advanced = `standard` (every menu and panel). Everything dispatches commands by
    id (`file.new`, `paint.stroke`, `tools.setColors`, `edit.undo`, …); saving goes through
    `PhotocraftApp::save_as` with a `.psd` name.
  - `src/io.rs`: `photocraft-io` import/export as `Services`; `src/native.rs` (rfd, atomic writes,
    opens a path argument) and `src/web.rs` (file picker + drops via the inbox, downloads).
  - Registered in `xtask/src/layers.rs` (exempt, like `photocraft`).
  - Tests (`tests/stage1.rs`): draw → save PSD → reimport pixels, open PSD → draw → save, palette,
    bad input → `Err`, a real pointer drag paints through PhotoCraft's Brush tool, both modes at
    desktop and phone width. Screenshot: `WOBBLE_SNAPSHOT=… cargo test -p wobbleworks-app snapshot -- --ignored`.
  - Still open for later stages: simple mode's backdrop is PhotoCraft's full-screen black (stage 3
    restyles it, probably via a backdrop hook in `ui-egui`); Esc in simple mode drops to the
    advanced editor (PhotoCraft's full-screen exit); no preferences persistence yet; the bar is
    plain egui (stage 3).
- **Stage 3 (2026-10-08): first pass done, pulled forward at the user's request** ("looks like a
  Photoshop ripoff"; asked for a different font and colours and hand-painted containers and
  buttons). The user chose a chunky pixel font defined in code (no font files: craft-fonts has no
  playful Latin face) and the Bubblegum palette.
  - `pixfont.rs` (5 × 7 glyphs + descenders, all printable ASCII), `rough.rs` (wobbly marker
    outlines, smooth-noise edges that boil over 3 frames, hard shadows, blobs), `widgets.rs`
    (buttons that rise on hover and squash on press, tool tiles, paint-blob swatches, sliders,
    painted cards, boiling logo), `icons.rs` (hand-drawn tool icons), `theme.rs` (presets;
    PhotoCraft's `Tokens` recoloured over its light Studio layout, re-applied whenever PhotoCraft
    resets its theme).
  - Simple mode: top bar (New, Open, Save PSD, Export PNG, Undo/Redo, Advanced), a 20-tool strip
    plus a "More" drawer with all 49 PhotoCraft tools, a paint dock (16 blobs, colour mixer, size and
    opacity via `tools.setBrush`, tool name, picture switcher, status), the canvas as a paper sheet
    (dots, wobbly outline, hard shadow; PhotoCraft's pasteboard set to the paper colour). Phones:
    icon-only top bar, tools in a scrolling row above the dock.
  - WigglyPaint licence checked on its itch.io page: code MIT, assets CC0. Nothing copied; credited
    in `ATTRIBUTION.md` as inspiration, with rows for the pixel font and icons.
  - Still open: WigglyPaint-style brush strip (wiggle brushes are stage 2), mascot and sound (stage
    4), theme picker and dark mode in settings, a restyle of PhotoCraft's own panel widgets beyond
    colours (stage 5), the logo's ink letters read a bit jumbled while boiling.
- **UI rework 2 (2026-10-08), user feedback on stage 3:** liked the wobbliness and the colour strip;
  disliked the "All the tools" drawer, the header and the loud pink icons; picked the full editor
  (the old "advanced" screen) as the one and only screen, hand-drawn, with medium pixel text.
  - Simple mode, header and drawer removed. The window is PhotoCraft's editor plus a bottom colour
    strip (current colour + mixer, recent colours tracked when the picture changes, user palette:
    "+" adds, right-click removes; saved through eframe persistence).
  - `ttf.rs`: the pixel font as an in-memory TrueType font (`FONT_SCALE` 1.0 at PhotoCraft's
    12.5 pt; 1.2 overflowed the options bar). `handdrawn.rs`: post-pass over every egui layer
    (`Context::graphics_mut`) turning Rect/LineSegment/Circle shapes into wobbly boiling ones,
    skipping the canvas area, textured and blurred rects. `svgicon.rs`: Lucide SVG subset parser
    (paths with arcs, circles, rects, lines, polylines) drawing icons as wobbly lines with pastel
    washes via a new `photocraft_ui_egui::icons::set_painter` hook (+ `svg`, `names`).
  - Accents calmed (sun wash + ink outline for the current tool, teal toggles); tooltips at 0.12 s
    show tool names beside the pointer, in a hand-drawn bubble.
  - Lesson: egui mitres sharp corners, so a folded or zero-length polygon edge draws a long spike.
    Tiny icon parts are drawn as plain dots, near-duplicate points are dropped, and closed outlines
    don't repeat their first point.
  - Still open: PhotoCraft's title bar still shows its kitsune icon and "Discord"; the colour
    picker inside the mixer is egui's; performance of the post-pass not measured yet (stage 6).
- **UI rework 3 (2026-10-08), user feedback:** side panels looked better without wobbly outer
  frames; the colour strip looked better without labels and should open into a card.
  - `handdrawn.rs`: boxes at least 100 × 100 (panels, cards, canvas surround) keep straight edges;
    their contents still wobble.
  - `colour.rs`: the strip (current colour, up to 8 recent, a dot, the palette; no labels) opens on
    a click into a 372 × 420 card that grows from the strip with an ease-out-back pop, content
    fading in. Bottom tabs: Colour (Wheel / HSV / Hex modes, "+ Palette"), Palette (recent and
    palette with labels, add/remove), Reference (load an image through the platform picker, shown
    fitted; hover previews and click/drag picks its colour). Click away or Esc closes it.
- **Stage 2 (2026-10-08): wiggle engine done.**
  - `crates/engine/src/wiggle_cmds.rs` (registered in `commands.rs`): a wiggle layer is a plain
    group of pixel layers `Boil 1..N`, so blend modes, opacity, masks and effects work and it
    round-trips through PSD as per-frame layers. Commands: `wiggle.new {frames 2..12=3}`,
    `wiggle.apply {command, params, amount}` (runs `paint.stroke|paint.pencil|paint.bucket|
    paint.gradient|paint.mixerBrush|edit.fill` on every frame as one history step, rolling back on
    failure), `wiggle.stroke`, `wiggle.showFrame {frame}` (playback: no history step, document stays
    saved, the active frame follows so live strokes stay visible), `wiggle.info`; helpers
    `at_frame`, `frame_count`, `wiggle_of`, `frames`. Noise is the original Wobbleworks `rnd`/`jr`
    (tests pin the original values); strokes are resampled every 8 px before wobbling so whole
    lines boil. `panic_hunt` and all 778 engine tests pass.
  - App: new pictures get a wiggle layer; strokes and fills made with PhotoCraft's own tools on a
    boil frame are taken back and re-applied to every frame (`spread_to_frames`, watching
    `Session::journal`); playback at ~7.7 fps in step with the UI boil, held while the pointer is
    down; recent colours now follow history length, not revision. `anim.rs`: animated GIF (image
    crate, loops forever) and PNG-sequence export of the frames, capped at 4096² pixels. Wiggle
    dock next to the colour strip: Boil on/off, Wiggle amount 0–10, + Layer, GIF, PNGs.
  - Tests: `tests/stage2.rs` (frames painted as one undo step, spread from a plain paint.stroke,
    playback without history or dirtiness, GIF/PNG/PSD export) and the real pointer drag in
    `stage1.rs` now checks every frame got the stroke.
  - Disk: the 30 GB target dir fills the session allowance; build with `CARGO_INCREMENTAL=0` and
    delete test executables in `target/debug/deps` after big test runs.
  - Pre-existing, not ours: clippy `manual_range_contains` in `crates/engine/src/fill_cmds.rs:409`
    (engine `--all-targets`).
  - Still open: `.wob` import from the old app, wiggle brush presets (WigglyPaint's brush strip),
    onion skin, per-layer boil speed.
- **Stage 4 (2026-10-08): juice, sound and the mascot done.**
  - `juice.rs`: every Middle/Foreground/Tooltip layer (PhotoCraft windows, menus, popups,
    tooltips) pops in with an ease-out-back scale over 0.22 s (shape transform after the frame is
    drawn, reset when a window closes); screen shake (decaying noise translate of every layer,
    0.32 s); paint-splat and confetti particles (capped at 400) on a Tooltip layer.
  - `handdrawn.rs`: small boxes under the pointer jiggle (fast boil frame, 1.8× wobble) and
    squash when pressed: every PhotoCraft button gets hover and press feedback.
  - `audio.rs`: synthesized Click, Pop, Scratch{speed} (low-passed noise, louder/brighter with
    pointer speed, paced every 45 ms while drawing on the canvas), Undo/Redo blips, Chime, Thud,
    Boing; no clicks at onset (tested), never above 0.95. Native output `CpalOut` (cpal 0.16, mixed
    in the device callback, silent without a device; Linux builds need `libasound2-dev`); web
    `WebAudioOut` (one-shot AudioBuffer sources via web-sys, no JS).
  - `mascot.rs`: Wob, a boiling blob on the paper's bottom-right corner: breathes, eyes follow the
    pointer, cheers (first stroke, save, export, poke), surprised (undo, big actions), sleepy after
    30 s idle ("z Z z"); pixel-font speech bubbles with cheerful lines; poke = boing + confetti.
  - Triggers (`shell.rs::effects`): new journal entries (undo/redo, fills, stamps, delete/clear/
    merge/flatten, the first stroke), status "Saved…"/"Exported…" (also PhotoCraft's File menu),
    tool changes, the colour card opening, clicks off the canvas.
  - Settings card (dock "Settings"): volume, mute, reduce motion (stops UI boil, pops, shake,
    particles, hops), Wob on/off, hand-drawn UI on/off; saved with eframe persistence.
  - Tests: `tests/stage4.rs` drives real frames (first-stroke chime, undo blip + surprised Wob,
    save confetti + cheer, delete thud + shake, reduce motion, settings round trip) plus unit
    tests for sounds, particles, pops and moods.
  - Disk: build into `CARGO_TARGET_DIR=target/ww` with `CARGO_PROFILE_DEV_DEBUG=line-tables-only`
    and `CARGO_INCREMENTAL=0`: the whole app plus tests is ~4 GB instead of ~30 GB.
  - Pre-existing, not ours: wasm `-D warnings` stops in `photocraft-cms` (unused `PAR_*` consts).
  - Still open: Wob idle animations beyond breathing, sound for the boil playback itself, haptic-y
    "tool pop" on PhotoCraft's toolbar button (we sparkle at the pointer instead).
- **Stage 5 (2026-10-08): every PhotoCraft feature reachable, restyled.**
  - WobbleWorks runs PhotoCraft's full editor (standard screen mode: menu bar, options bar, tools,
    panels), so every live menu item (`docs/parity.md`, ≥ `FLOOR` 627) is reachable.
    `tests/stage5.rs::every_live_menu_item_opens_inside_wobbleworks` (ignored by default, ~80 s)
    invokes every live catalog item inside WobbleWorks (hand-drawn pass, pixel font, juice on)
    one frame at a time: no panics.
  - Dialogs reviewed by screenshot (New, Image Size, Canvas Size, Gaussian Blur, Levels, Curves,
    Hue/Saturation, Preferences, Export As, Unsharp Mask, File and Select menus): readable, no
    clipping beyond a tight "Canvas extension color:" label. Default buttons (OK, Create, Export)
    are now sunshine with ink instead of hot pink. Wob ducks out of sight while a window, menu or
    popup overlaps its corner (it covered Preferences' OK/Cancel and could eat their clicks).
  - Fixed in PhotoCraft (`crates/ui-egui/src/canvas.rs`): the canvas scrollbar ids now include
    the layer, so a document shown twice (Window › Arrange › New Window for Document, embedded
    where there are no extra OS windows: web, tests) no longer trips egui's "widget changed
    layer" debug assertion. Regression test: `stage5.rs::a_second_window_on_the_document_renders`
    (fails without the fix; a plain-PhotoCraft version of it did not reproduce).
- **Stage 6 (2026-10-08), part 1:**
  - The old standalone `apps/wobbleworks` is deleted and `apps/wobbleworks-app` took its place:
    package, library and binary are now `wobbleworks` (earlier log entries say
    `wobbleworks-app`). Its `.wob` projects open through `wob.rs` (each layer → a wiggle layer,
    strokes redrawn on every frame with their seeds, raster frames copied), wired into the import
    service so PhotoCraft's File › Open and drag-and-drop take `.wob`.
  - `wiggle.new` while a boil frame is active now inserts above that wiggle layer (it nested
    inside it, breaking "+ Layer" and imports); engine test added.
  - Frame cost (`stage5.rs::frame_cost`, dev profile, no GPU): PhotoCraft's own look 25.4 ms,
    WobbleWorks' look 27.0 ms per frame, so the hand-drawn pass, juice, icons and Wob add ~1.6 ms.
- **Stage 6, part 2: web smoke test.** `cargo build -p wobbleworks --target wasm32-unknown-unknown`
  + `wasm-bindgen 0.2.129 --target web` + a page with the trunk loader replaced by
  `import init from "./wobbleworks.js"`, served locally and opened in Playwright's Chromium
  (`/opt/pw-browsers/chromium-1194`, WebGL2 through SwiftShader): starts in ~8 s at 1280 × 820
  and 390 × 760, no errors (only "no WebGPU, using WebGL2" and "SetTheme not implemented"
  warnings); pixel font, hand-drawn UI, dots, Wob and the dock all render. Fixed on the way:
  PhotoCraft's background-layer marks are clipped to the area above the colour strip (its tool
  column's edge line ran down across the strip on phones).
