# 0110: Bulk draw hooks get the frame's camera (`BulkDrawContext`)

Date: 2026-11-13. Status: accepted.

## Context

`BulkDrawHook` (decision 0025) received only `alpha`, the fraction of the way to the next tick.
Porting a large VR factory game's item drawing showed that real games place bulk copies by
where the viewer is: items past a distance are skipped, items beyond another distance are drawn
as flat icons turned toward the camera, and a level of detail is picked per copy. Without the
camera, a hook would have to find it itself, which each module would do differently, and the
renderer already knows it.

## Decision

- Hooks are `fn(&World, &BulkDrawContext, &mut Vec<BulkDraw>)`. `BulkDrawContext` holds
  `alpha`, `eye` (world position of the first camera by scene id; `None` without a camera) and
  `forward` (its unit view direction). It is a struct so later fields (views, frustum) do not
  break hooks again.
- `extract_at` fills it from the cameras it has already extracted, so it costs nothing extra.
- The built-in hooks (`marionette-factory` items, `JobDraw`) are unchanged in behaviour.
- Test: `crates/marionette-render/tests/bulk_context.rs` (alpha, eye and forward reach a hook).

## Alternatives

- **Pass the whole `FrameData`:** more than hooks need and ties modules to renderer internals.
- **Hooks query the camera themselves:** repeated, inconsistent (which camera, `LookAt` or not)
  and slower.
- **Per-view hooks (one call per eye):** doubles hook cost in a headset; copies culled by
  distance are the same for both eyes, so one call with the head position is enough.

## Consequences

Game modules with their own hooks must change the signature (one line). Stereo views share the
head position; a frustum per view can be added to the context later.
