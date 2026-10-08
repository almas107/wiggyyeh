# WobbleWorks (on PhotoCraft)

A cute, wiggly drawing app built on PhotoCraft: it wraps PhotoCraft's editor and engine, so every
PhotoCraft command, tool, panel and file format is available. The simple screen shows the canvas
and WobbleWorks' bar; **Advanced editor** shows PhotoCraft's full menus and panels on the same
pictures. Plan and progress: `docs/wobbleworks-spec.md`.

```sh
cargo run -p wobbleworks-app [-- picture.psd]     # desktop
cd apps/wobbleworks-app && trunk serve --release  # web
cargo test -p wobbleworks-app
# Offscreen screenshot (WOBBLE_ADVANCED=1 for the advanced editor, WOBBLE_WIDTH=390 for a phone):
WOBBLE_SNAPSHOT=/tmp/shot.png cargo test -p wobbleworks-app snapshot -- --ignored
```

`apps/wobbleworks` is the earlier standalone attempt (no PhotoCraft); it is mined for its wobble
brush, `.wob` import and theme, then deleted when this app replaces it.
