# 0286: Camera shots during play

Date: 2026-11-16. Status: accepted. Focuses 1 (modularity), 3 (performance) and 4 (graphics).

## Context

Games take pictures from cameras that are not the screen's: an in-game photo saved to disk, a
handheld camera's live screen, a security monitor. The renderer drew only the scene's first
camera into the window or headset, and a picture from elsewhere needed a second renderer
(the offscreen one) with its own copy of every model and texture.

## Decision

1. **`marionette_core::shots::Shots`** (a resource, not saved or hashed): game code queues a
   `ShotRequest` (pose, field of view, near and far, size, an id) with `Shots::ask`. An empty
   `image` asks for a read-back: the picture comes back in `Shots::done` as tight sRGB RGBA8
   rows. A non-empty `image` names a camera screen: the picture stays on the GPU and is shown
   by sprites and screen UI under the image name `shot:<name>`; it is black until the first.
   A size of 0 by 0 means the screen's (the window, or one eye), so a photo mode needs no
   window size of its own.
2. **Drawn in the same frame as the main view.** The window, the OpenXR loop and
   `marionette_render::shots::take_now` (offscreen, tests) call the same `ShotDraws` steps: plan
   the waiting requests, draw them after the frame's own views in one
   `Renderer::render_views_ui` call (per-frame work shared, culling covers every view), copy
   read-backs to mappable buffers, show each screen's new picture. Shots get no screen UI, no
   render scale and no foveation. Shadows still follow the frame's first view.
3. **No frame waits for a shot.** Buffers are mapped after the submit and collected by a
   non-blocking poll on a later frame. Camera screens keep two textures and alternate, so no
   pass samples the image it draws into.
4. **Bounded.** At most `Shots::LIMIT` (8) requests wait and 8 finished pictures are held; a
   ninth `ask` is refused. Headless runs draw nothing, so requests wait there. At most six shots
   are drawn a frame; the rest wait. At most 8 camera screens keep textures (the least recently
   drawn goes first). Sizes are clamped to 16..4096 pixels.
5. **Bloom per size.** The bloom settings uniform moved into each size's targets, since its
   intensity is divided by that size's level count and a shot can differ in size from the
   screen.
6. **Run-time images.** The `generated:` image registry holds 32 images instead of 8, so a
   game can show a few dozen small pictures (photo prints) it decodes itself.

## Rejected

- A second `Offscreen` renderer for pictures: a second copy of every GPU asset, and its
  read-back blocks the frame.
- A blocking read-back in the window loop: a visible hitch at 4K for every photo.
- Camera entities that render to textures every frame: most screens need a picture a few
  times a second, and a game knows when; requests keep the cost where the game asks for it.

## Not done yet

- Shots are not drawn in the editor or by `marionette screenshot`.
- A shot's shadows come from the main view's shadow map; a shot far from the main view has
  no shadows beyond its range.
