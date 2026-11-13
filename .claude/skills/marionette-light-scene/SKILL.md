---
name: marionette-light-scene
description: Make a Marionette 3D scene look good with the opt-in lighting and post-processing components (shadows, point and spot lights, sky and environment lighting, emissive, tone mapping, bloom, fog). Use when a scene looks flat, dark, washed out or unlit, or when adding lamps, glow, reflections or atmosphere.
---

# Light a scene

Every look is opt-in scene data: nothing changes until a component or resource is added, so
existing scenes and reference images stay as they were. Add one thing, take a screenshot,
look, then add the next. Use `project_schema` with the type name for exact fields.

## Order that works

1. Sun: one `DirectionalLight` entity. The lowest id is the one that casts shadows.
2. Resource `Shadows`: sun shadows over a square `distance` meters around the camera. Keep
   `distance` as small as the play area allows; shadow detail is spread over it.
3. Ambient: resource `SkyLight` (sky colour from above, ground colour from below) instead of
   flat `AmbientLight`. For reflections on shiny surfaces add resource `Environment`
   (`image` empty builds one from the sky colours; or a 2:1 equirectangular PNG).
4. Resource `ToneMapping`: bright light rolls off instead of clipping at white. Add it
   before judging brightness; re-tune light strengths after.
5. Local lights: `PointLight` (lamp, torch) and `SpotLight` (flashlight). The renderer uses
   the first 16 by entity id. Add `CastShadows` only to the few that matter: a spot light
   costs one shadow map, a point light six, at most 24 maps per frame.
6. Glow: `Emissive` on screens, lava, lit windows (it does not light other objects), then
   resource `Bloom` so light above `threshold` spills. Bloom needs `ToneMapping` to look right.
7. Resource `Fog` for depth; set the camera's `clear_color` to the fog colour.

## Look at it

`marionette op observe_screenshot '{"scene": "scenes/main.scene.json", "size": "1280x720"}' --json`
returns a PNG path: open it. Compare before and after each step.

## Materials

`Material` overrides a model with `base_color`, `metallic`, `roughness`. Rough, non-metal
surfaces read as matte; metal needs `Environment` to have something to reflect. glTF models
keep their own textures and emissive maps when no `Material` is set.

## Cost

Each look costs frame time: shadows and bloom the most. In VR every view pays again. After
lighting a scene, measure it (skill `marionette-find-slowdown`) and drop what the budget
cannot carry: shadow-casting local lights first, then bloom.

## Checks

A scenario with an `image` reference keeps the look from changing by accident
(skill `marionette-test-game`).
