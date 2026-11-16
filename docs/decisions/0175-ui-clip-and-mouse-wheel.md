# 0175: Screen UI clips to a rectangle; the mouse wheel is an input

Date: 2026-11-15. Status: accepted.

## Context

Screen UI (`UiNode` with `UiRect`, `UiText`, `UiImage`) drew every node whole. A list longer
than its panel, or a large map or tree panned and zoomed inside a window, had no way to hide
what lies outside its view: a game would have to cut rectangles itself and could not cut text
or images at all. Zooming such a view with the mouse wheel was not possible either: the window
fed mouse movement and buttons, but not the wheel.

## Decision

- New built-in component `UiClip { anchor, offset, size }` in `marionette_core::ui`, placed
  like a `UiNode`. A node with it is drawn only inside that rectangle: the renderer cuts each
  quad (rectangles, image quads and every glyph of text) on the CPU and moves its texture
  coordinates in step (`clip_quad`), so nothing stretches and no extra draw call or GPU state
  is needed. Clicks outside the clip miss the node (`hit`, used by `button_at`).
- The window feeds the wheel as the device input `axis:MouseWheel` for one tick, like the
  mouse axes: notches since the last tick, up positive, touchpad pixels at 40 per notch,
  clamped to -4..4. It goes through the same queue as other input, so recordings carry it.

## Consequences

Scenes without `UiClip` draw exactly as before, so no reference image changes and the drift
hash is unchanged. The clip is per node, not inherited (screen UI has no hierarchy): a view
puts the same clip on every node inside it. Cutting on the CPU costs a few comparisons per
quad. Rotated UI is still not supported. Not yet seen on a real screen.
