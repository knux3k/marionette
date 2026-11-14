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
Each sun shadow cascade draws only the casters inside its own volume (decision 0102).
Skinned and morphed parts are not culled. `RenderStats.culled` counts what views skipped;
`RenderStats.shadow_instances` counts instances drawn into shadow maps.
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

The resolve is one extra full-size pass per view (about 0.2 to 0.35 ms for two 2160x2160 eyes
on a desktop GPU), so scale pays only when shading the world costs more than that: on a
desktop GPU, 0.5 saved 26% on a bloom scene and 0.75 broke even; on a light scene 0.75 was
slower. Measure your scene before keeping it (numbers in proofs/render.md).

## Foveated rendering

```bash
marionette op resource_set '{"scene":"s.scene.json","resource":"Foveation","value":{"inner_radius":0.7}}'
```

Outside an ellipse (`inner_radius` of the half width and height) every other 2x2 pixel quad is
not shaded and is filled from its neighbours. For headset views; combine with `RenderScale`
freely (with scale below 1 the periphery is upscaled bilinearly). wgpu has no variable-rate
shading, so this is done with a depth mask. Without `RenderScale` it adds a copy pass, so it
pays only where edge shading is costly; on the small example scenes it was slightly slower.

## Proofs

In [proofs/render.md](proofs/render.md#culling-and-level-of-detail-decision-0080) and
[desktop GPU numbers](proofs/render.md#render-cost-on-a-desktop-gpu-decisions-0080-to-0082).
