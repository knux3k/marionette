# 0065: Shadows from point and spot lights, per light

Date: 2026-11-13. Status: accepted. Focus 4 (graphics, decision 0060).

## Context

Decision 0063 gave shadows to the first directional light only. Lamps, torches and flashlights
lit through walls and props, which is the biggest visible difference from other engines in
interiors and at night.

## Decision

Component `CastShadows { resolution, softness }` on an entity with a `PointLight` or
`SpotLight`. Absent, the light is unshadowed as before.

- One depth texture array holds every local shadow map of a frame. A spot light takes one layer
  (a perspective view as wide as its outer cone); a point light takes six (90-degree cube faces
  +x, -x, +y, -y, +z, -z, each widened by two texels so the 3x3 filter stays inside). The shader
  picks the face by the major axis of the direction from the light.
- At most 24 layers per frame (`MAX_LOCAL_SHADOW_LAYERS`), given to lights in entity id order;
  a light whose layers do not fit is unshadowed. All layers share the largest resolution asked
  for (128 to 1024; 24 layers of 1024 is 100 MB). The array shrinks to one texel when no light
  casts.
- Bias: the depth pass's slope-scaled bias, plus a normal offset of about one shadow texel at
  the receiver's distance, growing with the angle between surface and light. Filter: 3x3
  comparison taps scaled by `softness`.
- A model on the same entity as the shadow-casting light does not cast into local shadow maps
  (it still casts the sun's shadow). A lantern model around its own light would otherwise
  block all of it. Such models are drawn in their own instanced groups.
- Each layer is a depth pass over every mesh group (no culling by light range yet).

The sun's shadow normal offset now also scales with the surface's angle to the light (none
when the light is straight on), which removes the offset where it only moved shadows away from
their casters.

## Alternatives

- Cube depth textures for point lights: WGSL has `texture_depth_cube_array`, but not every
  backend supports cube arrays, and one 2D array for spot and point lights keeps one binding.
- Dual-paraboloid maps for point lights: two passes instead of six, but they bend straight edges
  and need tessellated casters.
- A `shadows` field on `PointLight` and `SpotLight`: changes every existing scene's canonical
  text and state hash; a separate component keeps them unchanged.
- Culling casters per layer by light range: fewer draws, but instanced groups would need to be
  split per light every frame. Left for when a scene needs it (counted in `RenderStats`).

## Consequences

- Draw calls rise by the mesh group count per layer: six times for a point light. The looks
  example (two shadowed lights, seven layers) is listed in PROOFS.md with its cost.
- The tuning note from decision 0063 ("a thin block's shadow starts slightly away from its
  base") was checked: the gap is the block's own point light lighting the ground beside it.
  With that light removed the shadow meets the base. With `CastShadows` on such a light the
  block no longer blocks it, and the sun's shadow still shows.
