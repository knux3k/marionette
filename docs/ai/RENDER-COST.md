# Render cost controls

What lowers the GPU and render-CPU cost of a frame, how to turn each on, and its proofs. Read
when a scene is too slow to draw or a headset frame is over budget. Budgets and general
measuring: [PERFORMANCE.md](PERFORMANCE.md).

| Control | Default | Changes pixels | Decision |
| --- | --- | --- | --- |
| Frustum culling | on | no (tested identical) | 0080 |
| `LevelOfDetail` resource | off | slightly, far away | 0080 |
| 4x MSAA (`--msaa 4`, window always) | on | anti-aliasing | 0005 |
| `RenderScale` resource | off | softer below 1, cleaner above 1 | 0081 |
| `Foveation` resource (headsets) | off | edges of the view only | 0082 |
| DLSS | not available | | 0083 |

## Frustum culling

Always on. Instances outside every view are not drawn in views; those inside a shadow volume
(directional shadow box, range of a light with `CastShadows`) still draw into shadow maps.
Skinned and morphed parts are not culled. `RenderStats.culled` counts what views skipped.
Compare with `marionette bench <scene> --no-cull`.

## Level of detail

```bash
marionette op resource_set '{"scene":"s.scene.json","resource":"LevelOfDetail","value":{"medium_below":0.2,"low_below":0.06,"hide_below":0.0}}'
```

Sizes are a part's bounding-sphere diameter over the screen height (1.0 fills it). Parts of 64
or more triangles get levels at about 1/2 and 1/4 of their triangles when the model loads
(feature `lod` of `marionette-render`, on by default). `hide_below` above zero hides tiny far
parts in views. Both eyes use the level picked from the first view.

## Render scale and upscaling

```bash
marionette op resource_set '{"scene":"s.scene.json","resource":"RenderScale","value":{"scale":0.75,"sharpness":0.3}}'
```

The world is drawn at `scale` times the output size (0.5 to 2.0) and resolved to it; UI stays
at full resolution. Below 1: Catmull-Rom clamped against ringing, plus `sharpness` (0 to 1).
Above 1: supersampled and box-filtered down (cleanest edges, costs about `scale` squared).

## Foveated rendering

```bash
marionette op resource_set '{"scene":"s.scene.json","resource":"Foveation","value":{"inner_radius":0.7}}'
```

Outside an ellipse (`inner_radius` of the half width and height) every other 2x2 pixel quad is
not shaded and is filled from its neighbours. For headset views; combine with `RenderScale`
freely. wgpu has no variable-rate shading, so this is done with a depth mask.

## Proofs

- `culling_drops_unseen_instances_without_changing_the_image` (`marionette-render` GPU tests):
  pixels identical with culling on and off; 121 of 123 instances not drawn in view; a box out of
  view still shadows the view.
- `level_of_detail_reduces_far_triangles_only_when_asked`: far spheres draw a reduced mesh
  (over 20% fewer triangles, mean pixel difference under 2 of 255), a tiny one is hidden.
- Unit tests in `crates/marionette-render/src/cull.rs` (frustum planes, sphere transform,
  shadow-only class, level choice, simplifier levels).
- 32x factory (`perf_factory_scene` scale 32), `bench --headset --size 1024x1024`, release,
  Mesa lavapipe in a 4-core cloud container: instances drawn per eye 9,792 -> 4,895,
  triangles 117,504 -> 58,740, record phase p50 1.12 -> 0.99 ms. GPU wait on lavapipe is fill
  bound (53.5 -> 50.5 ms) and says nothing about a hardware GPU; hardware numbers UNVERIFIED.
