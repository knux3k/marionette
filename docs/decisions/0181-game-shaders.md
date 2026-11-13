# 0181: Game shaders as hooks inside the PBR shader (`ShaderMaterial`, `GameShaders`)

Date: 2026-11-13. Status: accepted. Focuses 1 (modularity, custom code), 2 (tokens), 3
(performance) and 4 (graphics), decision 0060.

## Context

Games need their own looks (terrain blending, water, holograms, stylized shading) without
forking the renderer. A shader that replaces the whole pipeline would have to re-implement
lights, shadow maps, environment, fog, tone mapping and bloom, and would break each time the
engine's lighting changes (ray tracing and cascades are being added in parallel).

## Decision

1. **Hooks, not whole shaders.** A game writes WGSL with `fn surface(s: ptr<function,
   Surface>, i: SurfaceIn)` and/or `fn vertex(v: ptr<function, Vertex>)`. `pbr.wgsl` has a
   marked hook block with empty `surface_hook` and `vertex_hook`; for a game shader the block
   is replaced by calls into the game's functions and the game source is appended after a
   prelude with its bindings. `surface` edits the standard material result (color, metallic,
   roughness, normal, emissive, unlit) before lighting; `vertex` edits the world-space vertex.
   The contract is in `docs/ai/SHADERS.md` (one page, read only for this task).
2. **Source**: `ShaderMaterial.shader` is a name stored in the `GameShaders` resource (code
   inside a game binary) or a `.wgsl` file relative to the scene (scenes and agents can use
   it without Rust). Options are comment lines (`// @blend alpha|additive`,
   `// @double_sided`, `// @no_shadows`), so one text carries everything.
3. **Material data**: group 3 holds 16 numbers (`game.params`) and 4 textures (`tex0`..`tex3`)
   per distinct `ShaderMaterial`; groups 0 to 2 are the engine's (view, lights, the model's own
   material), so a glTF model keeps its textures as the starting point.
4. **Pipelines** per shader: lit for the final and HDR (bloom) targets, plus a depth pipeline
   when the shader has `vertex` (shadows follow moved vertices). Built on first use, rebuilt
   when the stored source changes; compile errors are captured with a wgpu error scope and
   returned as `render.shader` diagnostics with the line where the game source starts.
5. **Drawing**: the shader slot is part of the instanced group key, so culling, level of
   detail, instancing and bulk draws work unchanged. Blended shaders draw after opaque ones
   without depth writes and never cast shadows.
6. **Game time**: the view uniform's spare `eye.w` carries game time (ticks plus the frame's
   interpolation, times the step), so animated shaders are deterministic for offscreen
   pictures. The standard shader ignores it.
7. **No change to existing pictures**: the standard path keeps its own vertex entry points;
   the fragment code was reorganized around a `Surface` value with the same arithmetic. All
   existing reference images were regenerated on Mesa lavapipe before and after and compared
   byte for byte: identical.
8. `primitive:grid` (the 1 m plane split into 64 x 64 squares) for scenes that move vertices
   without game code.

## Alternatives

- **Full custom pipelines** (game owns vertex and fragment stages): most freedom, but lighting
  and shadows would be copied into every game and drift; rejected for v1. A later decision
  can add an escape hatch for full-screen effects.
- **A node graph or material language**: more tokens to learn and a compiler to maintain;
  WGSL is already what agents know.
- **Specialization by `#ifdef`-style preprocessing**: WGSL has none; text splicing at one
  marked block is simpler and keeps `pbr.wgsl` valid on its own.

## Consequences

- One pipeline switch per change of shader within a pass (draws are grouped by key, so this
  is per group, not per object).
- Skinned parts keep the standard shader; blended draws are not depth-sorted; alpha cutout
  does not cut shadows; no sky or full-screen shaders yet (SHADERS.md, "Limits").
- Merge point with the lighting work: anything that changes `fs_main` should keep reading
  material values from `s` (the `Surface`) after the hook call.
