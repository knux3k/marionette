# Render cost controls

What lowers the GPU and render-CPU cost of a frame, how to turn each on, and its proofs. Read
when a scene is too slow to draw or a headset frame is over budget. Budgets and general
measuring: [PERFORMANCE.md](PERFORMANCE.md).

| Control | Default | Changes pixels | Decision |
| --- | --- | --- | --- |
| Frustum culling | on | no (tested identical) | 0080 |
| `LevelOfDetail` resource | off | slightly, far away | 0080 |
| 4x MSAA (`--msaa 4`, window always) | on | anti-aliasing | 0005 |

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

## Proofs

In [proofs/render.md](proofs/render.md#culling-and-level-of-detail-decision-0080).
