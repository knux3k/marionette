# 0102: Each sun shadow map draws only the casters inside it

Date: 2026-11-14. Status: accepted. Focuses 3 (performance) and 4 (graphics), decision 0060.
Part of the optimization pass (decision 0100) that starts from the realistic benchmark
(decision 0284).

## Context

Culling (decision 0080) sorted instances into seen, shadow-only and hidden, but every shadow
map then drew every non-hidden caster. With four sun cascades the realistic scene drew its
roughly 18 million triangles four times into shadow maps on top of the view, which made up
most of its GPU time on the owner's desktop (10.5 ms GPU against Unity's about 4.3 ms).

## Decision

- `Culler::shadow_maps` gives each instance a bit per sun shadow map it touches, using the
  same conservative sphere test as the view, against the map's own clip volume.
- After the instance buffer is laid out, each map gets a copy of the instances it touches,
  appended after the main ones, one range per draw (`DrawCall::cascades`). Map `k` draws only
  that range. Point and spot light maps keep drawing every caster.
- The copies are whole `InstanceData` records, so every depth pipeline (plain, cut-out, game
  shader) works unchanged. The extra upload is the size of the instances in the maps, which
  is far smaller than the triangles it saves.
- With culling off (`--no-cull`) or in ray-traced frames, the maps draw everything as before.
- `RenderStats.shadow_instances` counts instances drawn into shadow maps, summed over maps.

## Why the picture cannot change

A caster outside a map's clip volume produces no fragments in that map: shadow pipelines
clip depth (no depth clamp), and depth-only writes with `LessEqual` do not depend on draw
order. Leaving such casters out therefore gives the same map texel for texel. The test
`shadow_cascades_draw_only_their_own_casters_without_changing_the_image` renders four
cascades with and without culling and requires identical bytes.

## Rejected

- An index list per map read from a storage buffer in the shadow shaders: less upload, but it
  changes every depth pipeline, including game shader shadow pipelines. Kept as a follow-up
  if the copies ever show up in a profile.
- Ordering instances so each map's casters are contiguous: not possible in general for four
  overlapping volumes plus the view.
