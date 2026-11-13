# 0200: World-space UI is one entity per panel, drawn as distance-field quads through a generic hook

Date: 2026-11-13. Status: accepted.

## Context

The engine's UI (`UiNode`, `UiText`) is screen-space, uses a 5x7 bitmap font (capitals only)
and draws in pixels. VR games need panels and text placed in the world: readable at arm's
length from any angle, many of them, cheap per frame, pressable by a controller ray or a
finger. The renderer is contended and games must not need engine edits for it (focus 1).

## Decision

- **A new optional crate, `marionette-ui3d`** (cargo feature `ui3d` on `marionette-agent` and
  `marionette-cli`, off by default; or `Engine::builder().module(Ui3dModule)`). The renderer
  does not depend on it.
- **A panel is one entity**: `UiPanel` (size in meters, columns, colors, facing, range) plus
  `UiContent` (a list of items: label, button, toggle, slider, progress, list, spacer).
  Widget values live in the items, so they are scene state (saved, hashed, replayed). One
  entity per panel keeps scenes small and lets layout and meshes be cached per panel.
- **Generic render hook in core and render**: `QuadDrawHooks` (like `BulkDrawHooks`) lets any
  module hand the renderer world-space quads that share one texture. `marionette-render`
  draws them in `worldquads.rs` with its own shader: the texture's red channel is a signed
  distance field and the edge is smoothed over about one screen pixel (`fwidth`), so text
  and shapes stay sharp at any size, distance and angle. Depth tested, not depth written,
  back faces culled, drawn after sprites and before screen UI. The renderer change is small
  (one field, one upload call, one draw call) and nothing is game-specific.
- **One atlas for text and rectangles**: a distance-field atlas holds the glyphs and a solid
  block, so a whole world of panels is one texture and one draw call per font. Atlases made
  at run time are passed by name (`generated:<key>`, `marionette_assets::generated`, at most
  8 held, key = content hash).
- **Fonts**: no font file ships. The built-in 5x7 bitmap font is converted to a distance field
  (capitals only, soft corners); a game sets `UiFontConfig.font` to its own TrueType or
  OpenType file (baked with `fontdue` at 48 px per em, ASCII, Latin-1 and a few symbols).
- **Facing and anchoring run in the simulation** (`Stage::Logic`, only additions,
  multiplications and square roots), so a pointer hits what is drawn and runs replay.
- **Cost model**: layout and meshes are cached per panel and rebuilt only when the panel's
  components or pointer highlight change. Per frame the hook transforms cached quads of
  visible panels (range and back-face culled, far to near). Numbers: `docs/ai/proofs/ui3d.md`.

## Alternatives

- One entity per widget with parent links: more flexible, but there is no entity hierarchy in
  the engine and thousands of text entities would cost scene size and tick time. Rejected.
- Render panels to textures and draw one quad each: crisp only at the baked size, needs a
  render-to-texture pass in the contended renderer, memory per panel. Rejected.
- A bitmap atlas per size: blurry between sizes. Rejected for VR where distance varies.
- Multi-channel distance fields (sharp corners): better at huge magnification, needs a
  generator we do not have; the single-channel field is enough at arm's length. Revisit with
  measurements in a headset.
- Shipping a default font: adds bytes and a license to every game. Rejected.
