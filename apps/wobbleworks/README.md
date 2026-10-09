# WobbleWorks (on PhotoCraft)

A cute, wiggly drawing app built on PhotoCraft: it runs PhotoCraft's full editor (every command,
tool, panel and file format) and redraws it by hand:

- `theme.rs`: Bubblegum colours published as PhotoCraft's theme tokens (calm accents, no loud pink).
- `pixfont.rs` + `ttf.rs`: a chunky pixel font defined in code and built into a TrueType font in
  memory at startup, put first in every font stack PhotoCraft uses.
- `handdrawn.rs`: after each frame is laid out, every box, line and circle PhotoCraft drew is
  swapped for a wobbly marker version that boils (text, images and the canvas stay exact).
- `svgicon.rs`: PhotoCraft's Lucide icons parsed from their SVG and drawn as soft wobbly lines with
  pastel washes, through `photocraft_ui_egui::icons::set_painter`.
- `juice.rs`, `audio.rs`, `mascot.rs`: pop-ins, shake, particles, synthesized sound (cpal on the
  desktop, WebAudio on the web) and Wob the mascot; Settings has volume, mute and reduce motion.
- `shell.rs`: dots and a paper-sheet outline around the picture, and the colour strip (current
  colour + mixer, recently painted colours, the user's palette; kept between sessions).

- `space3d/`: **WobbleWorks 3D** (the dock's 3D button): Feather-style drawing on 3D Guides,
  driven with Blender's mouse and keys, with boiling lines and painterly brushes. The note, tools
  and renderer are the `wobbleworks-3d` crate; this folder is only its UI. See
  `docs/wobbleworks-3d.md`.

Plan and progress: `docs/wobbleworks-spec.md` (2D) and `docs/wobbleworks-3d.md` (3D).

Linux builds need ALSA's headers for sound (`libasound2-dev` on Debian/Ubuntu).

```sh
cargo run -p wobbleworks [-- picture.psd]     # desktop
cd apps/wobbleworks && trunk serve --release  # web
cargo test -p wobbleworks
# Offscreen screenshot (WOBBLE_MIXER=1, WOBBLE_WIDTH=390, WOBBLE_HOVER=x,y, WOBBLE_FLAT=1,
# WOBBLE_PLAIN_FONT=1, WOBBLE_SVG_ICONS=1 to compare against PhotoCraft's own look):
WOBBLE_SNAPSHOT=/tmp/shot.png cargo test -p wobbleworks snapshot -- --ignored
# The 3D mode (WOBBLE3D_RENDER=1 for render mode):
WOBBLE3D_SNAPSHOT=/tmp/shot3d.png cargo test -p wobbleworks --test three_d snapshot -- --ignored
```

It replaced an earlier standalone WobbleWorks (no PhotoCraft); that app's `.wob` projects open
here (`wob.rs`), and its wobble noise lives on in `photocraft_engine::wiggle_cmds`.
