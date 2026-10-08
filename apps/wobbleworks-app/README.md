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
- `shell.rs`: dots and a paper-sheet outline around the picture, and the colour strip (current
  colour + mixer, recently painted colours, the user's palette; kept between sessions).

Plan and progress: `docs/wobbleworks-spec.md`.

```sh
cargo run -p wobbleworks-app [-- picture.psd]     # desktop
cd apps/wobbleworks-app && trunk serve --release  # web
cargo test -p wobbleworks-app
# Offscreen screenshot (WOBBLE_MIXER=1, WOBBLE_WIDTH=390, WOBBLE_HOVER=x,y, WOBBLE_FLAT=1,
# WOBBLE_PLAIN_FONT=1, WOBBLE_SVG_ICONS=1 to compare against PhotoCraft's own look):
WOBBLE_SNAPSHOT=/tmp/shot.png cargo test -p wobbleworks-app snapshot -- --ignored
```

`apps/wobbleworks` is the earlier standalone attempt (no PhotoCraft); it is mined for its wobble
brush and `.wob` import, then deleted when this app replaces it.
