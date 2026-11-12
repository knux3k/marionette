# 0063: Opt-in looks: shadows, local lights, sky light, emissive, tone mapping, fog

Date: 2026-11-12. Status: accepted. Focus 4 (graphics, decision 0060).

## Context

The renderer drew forward PBR with up to four directional lights, a flat ambient color and
colors clamped at 1.0. No shadows, no point or spot lights, nothing that glows, no fog. Scenes
looked flat. Existing scenes, proof games and reference images must not change by surprise.

## Decision

Each look is turned on by scene data and is off when the data is absent:

| Look | Turned on by | Notes |
| --- | --- | --- |
| Shadows | resource `Shadows { distance, resolution, softness }` | First `DirectionalLight` (lowest id). One depth pass per frame (shared by both eyes) into a square shadow map covering `distance` m around a point ahead of the first view, snapped to texels; 3x3 filtered compare, slope and normal-offset bias. Static and skinned meshes cast. |
| Point and spot lights | components `PointLight`, `SpotLight` | Position from the entity's transform. Inverse-square falloff windowed to zero at `range`; spot cone smoothstep between inner and outer angles. First 16 by entity id. No shadows from these yet. |
| Sky light | resource `SkyLight { sky, ground, intensity }` | Replaces flat `AmbientLight` with a hemisphere blend by surface normal. |
| Emissive | component `Emissive { color, intensity }` | Added after lighting, per instance (instance attribute 12). |
| Tone mapping | resource `ToneMapping { exposure }` | Exposure then Narkowicz's ACES fit, in the PBR fragment shader. |
| Fog | resource `Fog { color, density }` | Exponential-squared by distance from the eye, after tone mapping. Models only. |

All of it lives in the existing PBR shader behind flag bits in the lights uniform, so with every
flag off the shader computes exactly what it did before. Checked: the showcase, sprites and UI
images are byte-identical before and after on the same GPU, and the existing reference tests
pass.

## Alternatives

- An HDR render target with a separate tone-map pass: needed for bloom, but costs a full-screen
  pass and memory per eye. Tone mapping in the forward shader gives the same result for opaque
  surfaces; an HDR target can come with bloom later.
- Cascaded shadow maps: better quality over long distances, several passes per frame. One map
  around the camera fits the proof games' sizes; cascades are a follow-up.
- Shader permutations per feature: no runtime branch, but many pipelines to build and cache.
  The branches are on uniform flags (the same for every pixel), which GPUs handle well.
- `Option` fields on one `RenderSettings` resource: scene files would need reflect enum
  encoding for each field. Separate resources are clearer to set with `resource_set`.

## Consequences

- Cost: one extra depth pass over all mesh groups when `Shadows` is present (draw calls rise by
  the number of groups), plus nine shadow taps per pixel. On a software rasterizer the lighting
  example took 13.1 ms against 5.6 ms without the looks at 640x360; hardware GPU numbers need the
  owner's PC.
- Sprites and UI are not lit, fogged or tone mapped.
- Follow-ups: shadows from point and spot lights, cascades, glTF emissive factors and textures,
  bloom with an HDR target, image-based environment lighting, a sky gradient background.
