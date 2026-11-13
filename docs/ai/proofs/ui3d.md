# Proofs: World-space UI and text (decisions 0200, 0201)

Crate `marionette-ui3d`; renderer part `crates/marionette-render/src/worldquads.rs` and
`shaders/worldquads.wgsl`.

Logic and pictures (cloud machine, no hardware GPU; GPU test on Mesa lavapipe):
- `cargo test -p marionette-ui3d`: distance-field edge at 0.5; built-in font atlas has glyphs
  and a solid area; layout across columns with wrapped text; ray click, toggle, slider drag
  (clamps off the panel), list scroll and select; fingertip push presses; panels face the
  viewer and follow an anchor; quads drawn front-only and far panels first; two runs give
  the same state hash; the cache forgets removed panels; `a_panel_renders_with_crisp_text`
  renders twice with identical bytes. `UI3D_PICTURES=<dir>` writes front, 0.9 rad angled and
  3 m views to look at.
- `cargo test -p marionette-agent --features ui3d --test ui3d_ops`: `ui3d_panels` and
  `ui3d_point` on a live session.

CPU cost, 100 panels with about 47,800 quads (a 600-letter label, a button, a toggle, a
slider and a 3-row list each), release build, 4-core cloud CPU, from
`cargo test --release -p marionette-ui3d -- --ignored --nocapture heavy`:
- first frame (layout and meshes of all 100 panels): 8.1 ms;
- later frames (cached meshes, world transform of every visible quad): 1.17 ms;
- a tick with panels and no pointer: 0.019 ms;
- one panel's value changing every frame, tick plus extract: 0.99 ms (only that panel is
  rebuilt).

Not measured: GPU frame time on the RTX 2080 (needs the shared PC), upload of about 190,000
vertices (6.9 MB) per frame in that case, a headset, any font file other than the built-in
one (the TrueType path compiles and is used by `UiFontConfig`, but no test font is in the
repository), minified text far away (no mip chain on the atlas; the far view shimmers less
than a bitmap font but is not filtered).
