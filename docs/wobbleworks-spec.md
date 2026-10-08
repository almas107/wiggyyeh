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

- (empty: the next session starts at stage 1)
