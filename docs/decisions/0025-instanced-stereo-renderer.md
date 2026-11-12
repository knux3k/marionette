# 0025: Instanced, multi-view renderer and bulk draws

Date: 2026-11-12. Status: accepted.

## Decision

- PBR draws are instanced. Per instance: model matrix, base color, material params (96 bytes)
  in one vertex buffer (step mode Instance). The normal matrix is the cofactor of the model's
  3x3, computed in the shader (no CPU inverse). Draws are grouped by (asset, part, material
  override) in a `BTreeMap`, so order is deterministic; one draw call per group.
- `Renderer::render_views` renders several views (eyes) per frame. Instances, lights and sprites
  are prepared once; each view has its own slot in a dynamic-offset view uniform (up to
  `MAX_VIEWS` = 8, world and UI slot per view). It returns `RenderStats` (views, draw calls,
  instances, triangles).
- `Offscreen::render_eyes(gpu, frame, camera, eyes, ipd)` renders an emulated headset (eyes
  offset along the camera's right axis). `marionette bench --headset` uses 2 eyes of
  2160 x 2160, 64 mm IPD and a slow head yaw, and reports phase medians (sim, extract, record,
  GPU wait).
- Bulk draws: modules with many identical objects (factory items) add a `BulkDrawHook` to the
  `BulkDrawHooks` resource (`crates/marionette-core/src/spatial.rs`). Extract calls each hook
  with the interpolation alpha; the hook returns transforms, with no entity per object.
- `Registry::init_world` registers every component type with a new world, so queries over
  optional components work before any entity has them (extract relied on this silently).

## Why

One draw call per object (with a per-draw uniform) cost 153 ms of CPU for 10,000 cubes on the
reference software rasterizer; instancing brought it to 12 ms with one draw call. Stereo
without re-extracting halves the per-eye CPU cost. Entities per belt item would multiply
extract and hash cost by the item count for no gameplay benefit.

## Consequences

- No frustum culling yet; every instance is drawn in every view.
- Real OpenXR output (swapchains, poses) is still later work (W9); the multi-view path is what
  it will call.
