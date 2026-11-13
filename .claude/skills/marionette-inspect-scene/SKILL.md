---
name: marionette-inspect-scene
description: See a Marionette scene as the scene editor shows it (`editor_view`), which entity is at a pixel, and the selected entity's components. Use when asked what is on screen or where an object is, or to show a scene from a free camera.
---

# Inspect a scene through the editor view

`marionette edit <scene>` is the window a person uses: fly with the right mouse button and
WASD, click to select, panels on both sides. An agent has no screen, so it uses the operation
`editor_view`, which draws the same picture offscreen and answers the same questions.
Both need the `editor` cargo feature (on by default).

## Steps

1. Picture from the scene's camera, with an entity selected:
   `marionette op editor_view '{"scene": "scenes/main.scene.json", "select": 3}'`.
   The PNG goes to `.marionette/screenshots/<name>-editor.png` (the only place allowed in
   read-only mode); look at it before describing it.
2. What is at a pixel (y down, same `size` as the image, default 1280x720): add
   `"pick": [[640, 360]], "image": false`. Each pick lists entities nearest first; clicking
   in the window cycles through the same list.
3. A free camera: `"eye": [6, 4, 8], "look_at": [0, 0.5, 0]`. To fit the selection (or the
   whole scene when nothing is selected) add `"frame": true`; the output's `camera` is where
   it ended up, reusable as `eye`.
4. Components of the selected entity come back under `selected`. For many entities use
   `observe_query` instead of one call each.
5. A live session works too: pass `session` instead of `scene` (the editor views a copy; the
   session is not changed).

## Traps

- Models are picked by their bounding box, so a pick near a model's edge can hit it in empty
  space. Check with a second pick or by framing it.
- Entities with no model or sprite (cameras, lights, spawn points) are small markers: pick
  within 10 pixels of them.
- `"panels": false` and `"grid": false` give a clean picture for a person; the panels cover
  both sides of a 1280-wide image.
- Edits in the window (arrow keys move, Ctrl+D copies, Del deletes, Ctrl+Z undoes) are
  written to the scene file through `component_set`, `entity_create` and `entity_delete`.
  From an agent, call those operations directly; the window is for the person.
- Opening `marionette edit` on someone's screen needs their OK.
