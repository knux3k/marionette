# 0080: Frustum culling always on; level of detail opt-in (`LevelOfDetail`)

Date: 2026-11-13. Status: accepted. Focuses 3 (performance) and 4 (graphics), decision 0060.

## Context

The renderer drew every mesh instance in every view, both eyes included, whatever the camera
saw. A large VR factory game is mostly out of view at any moment (behind the player, other
halls, other planets), and far machines are a few pixels tall but cost their full triangle
count. The target is to beat a commercial engine's frame time with no visible tradeoff, so the
first savings must be the ones that cannot change the picture.

## Decision

1. **Frustum culling, always on** (`crates/marionette-render/src/cull.rs`). Each model part gets
   a bounding sphere at upload (box center, farthest vertex). Each frame every static instance
   is classed against every view's frustum (planes from the view-projection matrix, which works
   for perspective and orthographic cameras):
   - inside any view: drawn in all views and shadow maps;
   - outside every view but inside a shadow volume (the directional shadow box, or the range
     of a point or spot light with `CastShadows`): drawn into shadow maps only, so shadows of
     things behind the camera stay;
   - otherwise not written to the instance buffer.
   Within each instanced group the seen instances come first, so views draw a prefix of the
   group and shadow passes draw all of it: no extra buffers or draw calls. The tests are
   conservative, so pixels are identical with culling on and off (test
   `culling_drops_unseen_instances_without_changing_the_image`). `Renderer::set_culling(false)`
   and `marionette bench --no-cull` exist only for measuring.
2. **Level of detail, opt-in**: resource `LevelOfDetail { medium_below, low_below, hide_below }`.
   Sizes are the part's bounding sphere diameter over the screen height, measured from the first
   view (both eyes use the same level, so they never disagree). With the resource present, each
   part of 64 triangles or more gets up to two reduced index lists (about 1/2 and 1/4 of its
   triangles, meshoptimizer's edge-collapse simplifier at 5% error, sharing the full vertex
   buffer) when its model loads, on the background loader thread while streaming. A level that
   saves under 10% over the previous one is dropped. Parts below `hide_below` are not drawn in
   views (shadow maps keep them if they are in a shadow volume). The level is part of the draw
   group key, so a level is one more instanced draw per part in use, not one per instance.
   Turning the resource on in a running scene reloads models once.
3. `RenderStats.culled` counts static instances not drawn in views; `instances + culled` is
   every static instance of the frame. `instances` and `triangles` now count what views draw.

Skinned and morphed parts are not culled and keep one level (their rest-pose bounds do not hold
while animated). The simplifier is behind the `lod` cargo feature of `marionette-render`
(default on); without it `LevelOfDetail` still hides small far parts.

## Alternatives

- GPU culling (compute pass writing indirect draws): scales further but needs indirect-draw
  plumbing and a depth pyramid for occlusion. On the CPU the sphere tests cost less than
  they save: at the 32x factory size the render record phase went from 1.12 to 0.99 ms. Kept as the next step if instance counts
  grow past a few hundred thousand.
- Occlusion culling (hierarchical depth): large wins indoors, but needs a depth prepass or last
  frame's depth and can pop; later.
- Per-eye culling with two instance lists: saves the slice only one eye sees, at the price of
  separate buffers and draws per eye. Both eyes share almost all of their view.
- LOD by distance in meters: simpler to state, but wrong for very big or very small parts and
  for different fields of view; screen share is what decides visible detail.
- Ship pre-made LOD meshes from glTF (`MSFT_lod`): better quality when artists make them; not
  read yet. Generated levels need no asset work.

## Consequences

- Frames where much of a scene is out of view record and draw only what is seen. Measured
  numbers are in `docs/ai/proofs/render.md` (section Culling and level of detail).
- Reduced levels use extra index memory (about 75% of the part's index buffer) only for scenes
  with `LevelOfDetail`.
- Level switches can pop at the thresholds; no cross-fade yet.
