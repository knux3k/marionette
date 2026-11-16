# 0202: Screen UI on a world canvas

Date: 2026-11-16. Status: accepted.

## Context

A game's menus are often built once as screen UI (`UiNode` with absolute pixel layout, clips
and buttons). In VR there is no screen: the same menu has to hang in the world and be pressed
by a controller ray. Rebuilding every menu as `UiPanel` flow items doubles the work and the
two layouts drift apart. Unity solves this with a world-space canvas.

## Decision

- **`UiOnCanvas { canvas }`** (core, next to `UiNode`) moves a screen UI node onto the canvas
  entity with that scene id. The screen renderer's extract and the window's mouse skip such
  nodes, so core and the renderer need no world UI code.
- **`UiCanvas { size, pixels, max_distance, interactive }`** (crate `marionette-ui3d`): a flat
  rectangle placed by its `Transform`. Its nodes are laid out with the usual anchor rules on
  a `pixels` screen and scaled to `size`.
- Drawing: the ui3d quad hook draws each visible canvas after the panels, far ones first,
  nodes in screen order (layer, then scene id). Rects use the font's solid texel; text uses
  the panel font (the built-in one is made from the screen's bitmap font), upper case as on
  the screen; clips cut quads and their texture corners with core's `clip_quad`.
- Input: a pointer whose nearest interactive hit is a canvas (nearer than every panel, and
  not in a panel drag) clicks on the press edge the top-most `UiButton` under it (layer, then
  scene id), exactly as `button_at` picks for the mouse. Like a mouse click, the action is
  pressed for the whole next tick: the click is kept and set in `Stage::Input` right after
  the input module (`marionette_input::InputApplied`), so menu code that runs before
  `Stage::Logic` sees it as just pressed. (Panel buttons still set theirs inside
  `Stage::Logic`, decision 0201.)
- `UiImage` nodes are not drawn on a canvas yet (no game needs it; each image would be its
  own quad batch).

## Alternatives

- Rendering the screen UI into an offscreen texture shown on a quad: needs a render target
  per canvas, mip filtering and a GPU path for every frame; text loses the distance field.
  Rejected for now.
- A game-side converter from screen nodes to `UiPanel` items: absolute layouts do not map to
  the flow layout, and every game would write it again. Rejected.
